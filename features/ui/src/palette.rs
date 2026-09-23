//! Command palette. Trigger on `\`, fuzzy filter the catalog,
//! dispatch the chosen command as a `TransactionSpec`.
//!
//! The trigger is a BACKSLASH, not the slash most editors use. This
//! editor is embedded in Keyflow, where `/` is ordinary text and
//! constantly typed: `G/B` is a slash chord, `V/V` a secondary dominant,
//! `4/4` a time signature, `////` a bar of rhythm, and `/ Title /` a
//! header. A palette on `/` opened on almost every line of a chart.
//! Backslash means nothing in that language, so it is free to mean
//! "command".
//!
//! Ported from `~/Development/Task/crates/editor/src/handler/commands.rs`
//! and adapted for our editor's `EditorState` / `Changes` API
//! instead of Task's block-based document model. The
//! `CommandKind` enum mirrors the Task code closely; the
//! catalog is markdown-flavored (callouts, code fences, math
//! blocks, headings, etc.).
//!
//! Architecture follows `CodeMirror`'s `@codemirror/autocomplete`:
//! a `PaletteState` signal holds the open menu + query; each
//! state update re-runs `detect_trigger` against the current
//! line; selection nav fires `move_selection`; pick fires
//! `run_command` and clears the state.

use dioxus::prelude::*;
use editor_state::{Changes, EditorState, Selection, TextSlice, TransactionSpec};

/// Slash-command popup. Reads the open state from the
/// `palette` signal threaded down from the host (typically the
/// playground or whichever shell embeds the editor). Renders
/// rows grouped by `group`; clicks pick a command. Keyboard
/// nav lives in `Editor`'s `onkeydown` rather than here so it
/// works without the menu element being focused.
//
// Dioxus 0.7 flags `key:` on non-first nodes in a block as
// deprecated; the menu-row rsx! emits an optional header
// followed by the row div, which trips the lint. Suppressed
// at the component level; refactoring to a single keyed
// outer element would lose the leading group divider.
#[expect(
    deprecated,
    reason = "rsx! emits an optional header before the row div; a single keyed \
              outer element would lose the leading group divider"
)]
#[component]
pub fn CommandPalette(
    state: Signal<EditorState>,
    palette: Signal<Option<PaletteState>>,
    /// Optional transaction sink — pass the same callback given to
    /// `Editor`'s `on_transaction` so command picks made by *mouse*
    /// (clicking a row) report through the host's sink exactly like
    /// keyboard picks (which route via the editor's keydown handler).
    #[props(default)]
    on_transaction: Option<Callback<crate::TransactionEvent>>,
) -> Element {
    let snapshot = palette.read().clone();
    let Some(current) = snapshot else {
        return rsx! { Fragment {} };
    };
    // Anchor the menu just under the current caret on every
    // render. Lets the menu track the user's typing position
    // instead of docking at the bottom of the editor frame.
    use_effect(|| {
        let script = r"(()=>{
            const menu = document.querySelector('.ed-menu');
            if (!menu) return;
            const sel = window.getSelection && window.getSelection();
            if (!sel || sel.rangeCount === 0) return;
            const r = sel.getRangeAt(0);
            const rects = r.getClientRects();
            const rect = rects.length
                ? rects[rects.length - 1]
                : r.getBoundingClientRect();
            if (!rect) return;
            const w = menu.offsetWidth;
            const h = menu.offsetHeight;
            const vw = window.innerWidth;
            const vh = window.innerHeight;
            let top = rect.bottom + 6;
            if (top + h > vh - 8) top = rect.top - h - 6;
            let left = rect.left;
            if (left + w > vw - 8) left = vw - w - 8;
            if (left < 8) left = 8;
            menu.style.top = top + 'px';
            menu.style.left = left + 'px';
        })();";
        let _ = document::eval(script);
    });
    let hits = filter_commands(&current.query);
    if hits.is_empty() {
        return rsx! {
            div { class: "ed-menu",
                div { class: "ed-menu-empty", "No commands match." }
            }
        };
    }
    let selected = current.selected.min(hits.len().saturating_sub(1));
    let mut last_group: Option<&str> = None;
    let mut row_idx: usize = 0;
    rsx! {
        div { class: "ed-menu",
            for entry in hits.iter().cloned() {
                {
                    let show_header = last_group != Some(entry.group);
                    last_group = Some(entry.group);
                    let is_selected = row_idx == selected;
                    let idx_for_click = row_idx;
                    let entry_for_click = entry.clone();
                    let state_for_click = state;
                    let mut palette_for_click = palette;
                    let sink_for_click = on_transaction;
                    let current_for_click = current.clone();
                    row_idx += 1;
                    rsx! {
                        {
                            if show_header {
                                rsx! { div { class: "ed-menu-group", "{entry.group}" } }
                            } else { rsx! {} }
                        }
                        div {
                            key: "{idx_for_click}",
                            class: if is_selected { "ed-menu-row selected" } else { "ed-menu-row" },
                            // Mousedown.preventDefault keeps the
                            // editor's caret from blurring as the
                            // click lands, so the next render keeps
                            // selection state coherent.
                            onmousedown: move |e: Event<MouseData>| e.prevent_default(),
                            onclick: move |_| {
                                let cur = state_for_click.read().clone();
                                let end = current_for_click.trigger_start + 1 + current_for_click.query.len();
                                if let Some(spec) = run_command(
                                    &cur,
                                    current_for_click.trigger_start..end,
                                    entry_for_click.kind,
                                ) {
                                    crate::event::apply_tx(state_for_click, &cur, spec, sink_for_click);
                                }
                                palette_for_click.set(None);
                            },
                            div { class: "ed-menu-icon", "{entry.icon}" }
                            div { class: "ed-menu-body",
                                div { class: "ed-menu-label", "{entry.label}" }
                                if !entry.desc.is_empty() {
                                    div { class: "ed-menu-desc", "{entry.desc}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One menu entry. Title shows in the row, group is the header
/// above it ("Heading", "Format", "Callout", …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandEntry {
    pub label: &'static str,
    pub group: &'static str,
    pub desc: &'static str,
    pub kind: CommandKind,
    /// Short ASCII badge shown left of the label, in the mono face.
    ///
    /// It is the SYNTAX the command inserts — `[ ]` for a task, `$$` for a
    /// math block, `[[` for a wikilink — which reads better in a markdown
    /// editor than a pictogram would, and teaches the markup while it is
    /// being pointed at. Deliberately ASCII: the previous set reached for
    /// `☐ ❝ ▦ 𝒯 ⤳ ⑃ ∑ ∫ ⌘ 🦀 🔗 🖼 ⁿ`, almost none of which exist in the
    /// host's font stack, so the column rendered as a row of tofu boxes.
    pub icon: &'static str,
}

/// What the command does. Each variant carries the data the
/// runner needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Splice a literal snippet at the palette range. `caret_back`
    /// is how many bytes to move the caret back from the end of
    /// the insert (e.g. `("[[]]", 2)` lands the caret between
    /// the brackets).
    InsertSnippet(&'static str, usize),
    /// Replace the palette range with a block-shaped snippet that
    /// starts on its own line. If the line containing the palette
    /// has other content, the runner inserts a newline first.
    /// Same `caret_back` semantics as `InsertSnippet`.
    InsertBlockSnippet(&'static str, usize),
    /// Set the heading level of the current line (1-6, or 0 to
    /// strip). Strips any existing `#…#` prefix first.
    SetHeading(u8),
    /// Promote the current line to a list item of the given
    /// kind. Replaces any existing list marker.
    SetList(ListKind),
    /// Toggle the current line's task checkbox (`[ ]` ↔ `[x]`).
    ToggleTask,
    /// Add a UUID block id to the block at the caret (Logseq
    /// style) so it can be referenced via `((uuid))`.
    AddBlockId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Unordered,
    Ordered,
    Task,
}

/// Open-state of the palette menu. `None` when the menu's closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteState {
    /// Byte offset of the [`TRIGGER`] that opened the menu.
    pub trigger_start: usize,
    /// Body typed after the palette (does NOT include the `/`).
    pub query: String,
    /// Currently highlighted row.
    pub selected: usize,
}

/// Scan back from the caret for a [`TRIGGER`] that hasn't been closed by
/// whitespace, returning its position and the query typed after it.
///
/// Operates on the slice from the start of the current line up to the
/// caret, so a trigger deep in the doc doesn't hold the menu open across
/// line breaks.
/// The character that opens the palette.
///
/// Backslash rather than slash: see the module docs. `/` is ordinary,
/// frequent text in a Keyflow chart, and a palette bound to it fired
/// constantly against the user's intent.
pub const TRIGGER: char = '\\';

#[must_use]
pub fn detect_trigger(doc: &str, caret: usize) -> Option<(usize, String)> {
    let caret = caret.min(doc.len());
    let line_start = doc
        .before(caret)
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let segment = doc.slice(line_start..caret);
    let bytes = segment.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        let c = *bytes.get(i.saturating_sub(1))?;
        if c == TRIGGER as u8 {
            // A doubled trigger is an escaped backslash (`\\`), which is a
            // literal backslash in markdown and not a command.
            if i >= 2 {
                let prev = *bytes.get(i.saturating_sub(2))?;
                if prev == TRIGGER as u8 {
                    return None;
                }
            }
            let query = segment.after(i).to_string();
            // The escape rule, from the other side. CommonMark escapes are
            // `\` + ASCII PUNCTUATION and nothing else — a backslash before
            // any other character is a literal backslash — and
            // `markdown::escape_span` here implements exactly that. So the
            // two uses of `\` partition cleanly and this is the line that
            // divides them: punctuation after the trigger is an escape the
            // author is writing (`\*`, `\_`, `\[`), a letter is a command
            // they are looking for (`\quote`, `\h1`).
            //
            // A bare trigger with nothing after it yet still opens the menu —
            // that is how anyone discovers what is in it — and the very next
            // character decides which of the two this was.
            if !query.chars().next().is_none_or(char::is_alphanumeric) {
                return None;
            }
            return Some((line_start.saturating_add(i).saturating_sub(1), query));
        }
        if char::from(c).is_whitespace() {
            return None;
        }
        i = i.saturating_sub(1);
    }
    None
}

/// Case-insensitive substring filter over label / group / desc.
#[must_use]
pub fn filter_commands(query: &str) -> Vec<CommandEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all_commands();
    }
    all_commands()
        .into_iter()
        .filter(|c| {
            c.label.to_lowercase().contains(&q)
                || c.group.to_lowercase().contains(&q)
                || c.desc.to_lowercase().contains(&q)
        })
        .collect()
}

/// Resolve a picked command into a `TransactionSpec`. Removes
/// the `/query` first, then either splices a snippet or runs a
/// line-level transform (heading / list / task).
#[must_use]
pub fn run_command(
    state: &EditorState,
    slash_range: std::ops::Range<usize>,
    cmd: CommandKind,
) -> Option<TransactionSpec> {
    let doc = state.doc.to_string();
    if slash_range.end > doc.len() || slash_range.start > slash_range.end {
        return None;
    }
    match cmd {
        CommandKind::InsertSnippet(text, caret_back) => {
            let new_caret = slash_range
                .start
                .saturating_add(text.len())
                .saturating_sub(caret_back);
            Some(
                TransactionSpec::new()
                    .changes(Changes::replace(slash_range, text))
                    .selection(Selection::caret(new_caret))
                    .annotate("origin", "palette"),
            )
        }
        CommandKind::InsertBlockSnippet(text, caret_back) => {
            Some(insert_block_snippet(&doc, slash_range, text, caret_back))
        }
        CommandKind::SetHeading(level) => {
            // Replace the whole line containing the palette range
            // with a heading-prefixed version of its non-palette
            // content. Doing this as one atomic replace avoids
            // the bug where stripping the palette separately and
            // then calling `set_heading` on a synthetic doc
            // produces offsets that don't map back to the real
            // doc.
            let (line_start, line_end) = line_bounds(&doc, &slash_range);
            let body = line_without_slash(&doc, line_start, line_end, &slash_range);
            let body = strip_heading(&body);
            let new_line = if level == 0 {
                body.to_string()
            } else {
                let prefix = "#".repeat(usize::from(level));
                format!("{prefix} {body}")
            };
            let caret = line_start.saturating_add(new_line.len());
            Some(
                TransactionSpec::new()
                    .changes(Changes::replace(line_start..line_end, new_line))
                    .selection(Selection::caret(caret))
                    .annotate("origin", "palette"),
            )
        }
        CommandKind::SetList(kind) => {
            let target = match kind {
                ListKind::Unordered => "- ",
                ListKind::Ordered => "1. ",
                ListKind::Task => "- [ ] ",
            };
            let (line_start, line_end) = line_bounds(&doc, &slash_range);
            let body = line_without_slash(&doc, line_start, line_end, &slash_range);
            let body = strip_list_marker(&body);
            let new_line = format!("{target}{body}");
            let caret = line_start.saturating_add(new_line.len());
            Some(
                TransactionSpec::new()
                    .changes(Changes::replace(line_start..line_end, new_line))
                    .selection(Selection::caret(caret))
                    .annotate("origin", "palette"),
            )
        }
        CommandKind::ToggleTask => {
            let (line_start, line_end) = line_bounds(&doc, &slash_range);
            let body = line_without_slash(&doc, line_start, line_end, &slash_range);
            // If the line already starts with `- [ ]`/`- [x]`,
            // flip it; otherwise promote to task.
            let b = body.as_bytes();
            let is_task = matches!(
                (b.first(), b.get(1), b.get(2), b.get(4)),
                (Some(b'-' | b'*' | b'+'), Some(b' '), Some(b'['), Some(b']'))
            ) && b.len() >= 5;
            let new_line = if is_task {
                let new_inner = if b.get(3) == Some(&b' ') { b'x' } else { b' ' };
                let mut bs = b.to_vec();
                if let Some(slot) = bs.get_mut(3) {
                    *slot = new_inner;
                }
                String::from_utf8(bs).unwrap_or_else(|_| body.clone())
            } else {
                format!("- [ ] {body}")
            };
            let caret = line_start.saturating_add(new_line.len());
            Some(
                TransactionSpec::new()
                    .changes(Changes::replace(line_start..line_end, new_line))
                    .selection(Selection::caret(caret))
                    .annotate("origin", "palette"),
            )
        }
        CommandKind::AddBlockId => {
            // Strip the palette first by reconstructing the line,
            // then run `add_block_id` on the result.
            let (line_start, line_end) = line_bounds(&doc, &slash_range);
            let body = line_without_slash(&doc, line_start, line_end, &slash_range);
            // Build a synthetic state with the palette removed
            // and run the helper. Inline transcription avoids a
            // direct dep on the helper's `(spec, ref_str)`
            // shape — we just compute the final line.
            if body.trim().is_empty() {
                return None;
            }
            let uuid = uuid_v4_string();
            let new_text = format!("{body}\nid:: {uuid}");
            let caret = line_start.saturating_add(body.len());
            Some(
                TransactionSpec::new()
                    .changes(Changes::replace(line_start..line_end, new_text))
                    .selection(Selection::caret(caret))
                    .annotate("origin", "palette-block-id"),
            )
        }
    }
}

fn uuid_v4_string() -> String {
    // Re-export point — keep `uuid` as an editor-state dep only.
    uuid::Uuid::now_v7().to_string()
}

/// Byte-range of the (single) line containing the palette. The
/// palette range is guaranteed to live on one line because the
/// parser closes on newlines.
fn line_bounds(doc: &str, slash_range: &std::ops::Range<usize>) -> (usize, usize) {
    let line_start = doc
        .before(slash_range.start)
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let line_end = doc
        .after(slash_range.end)
        .find('\n')
        .map_or(doc.len(), |n| slash_range.end.saturating_add(n));
    (line_start, line_end)
}

/// Reconstruct the line text with the `/query` removed.
fn line_without_slash(
    doc: &str,
    line_start: usize,
    line_end: usize,
    slash_range: &std::ops::Range<usize>,
) -> String {
    let mut out = String::with_capacity(line_end.saturating_sub(line_start));
    out.push_str(doc.slice(line_start..slash_range.start));
    out.push_str(doc.slice(slash_range.end..line_end));
    out
}

/// Strip a leading `#…# ` heading marker (1-6 hashes + space).
fn strip_heading(line: &str) -> &str {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes).copied() == Some(b' ') {
        line.after(hashes.saturating_add(1))
    } else {
        line
    }
}

fn strip_list_marker(line: &str) -> &str {
    let b = line.as_bytes();
    // task: `- [X] `
    if b.len() >= 6
        && matches!(b.first(), Some(b'-' | b'*' | b'+'))
        && b.get(1) == Some(&b' ')
        && b.get(2) == Some(&b'[')
        && b.get(4) == Some(&b']')
        && b.get(5) == Some(&b' ')
    {
        return line.after(6);
    }
    // ordered: `N. `
    if b.len() >= 3
        && b.first().is_some_and(u8::is_ascii_digit)
        && b.get(1) == Some(&b'.')
        && b.get(2) == Some(&b' ')
    {
        return line.after(3);
    }
    // unordered: `- `
    if b.len() >= 2 && matches!(b.first(), Some(b'-' | b'*' | b'+')) && b.get(1) == Some(&b' ') {
        return line.after(2);
    }
    line
}

/// The process-wide catalog override, if a host has registered one.
///
/// A registry rather than a prop, and for the same reason as
/// `editor_state::fence_renderer`: `filter_commands` is called from the
/// editor's KEYDOWN handler as well as from this component — it has to know
/// the hit count to move the selection and to pick on Enter — and threading
/// a catalog through both would put a command table in the signature of
/// every layer between. The catalog is fixed at application start and never
/// varies per document.
fn catalog() -> &'static std::sync::RwLock<Option<Vec<CommandEntry>>> {
    static CATALOG: std::sync::OnceLock<std::sync::RwLock<Option<Vec<CommandEntry>>>> =
        std::sync::OnceLock::new();
    CATALOG.get_or_init(|| std::sync::RwLock::new(None))
}

/// Replace the palette's catalog wholesale. Call once at application start.
///
/// The built-in catalog is markdown's — headings, lists, callouts, math,
/// wikilinks — which is right when the buffer holds a note and wrong when it
/// holds something else. An editor embedded in Keyflow is editing a CHART:
/// there are no headings in a chart, and "Numbered list" is not a thing
/// anyone reaches for while writing one. A host that knows what its buffer
/// contains supplies the commands for it.
///
/// Passing an empty vec is meaningful — it says "this host has no commands",
/// and the palette will say so rather than offering markdown's.
pub fn register_catalog(commands: Vec<CommandEntry>) {
    if let Ok(mut slot) = catalog().write() {
        *slot = Some(commands);
    }
}

/// The full catalog: whatever the host registered, else markdown's.
#[must_use]
pub fn all_commands() -> Vec<CommandEntry> {
    if let Ok(slot) = catalog().read()
        && let Some(custom) = slot.as_ref()
    {
        return custom.clone();
    }
    markdown_commands()
}

/// The built-in catalog, for a buffer that holds markdown.
#[must_use]
pub fn markdown_commands() -> Vec<CommandEntry> {
    let mut out = Vec::new();
    push_headings(&mut out);
    push_lists(&mut out);
    push_code_and_math(&mut out);
    push_callouts(&mut out);
    push_block_refs(&mut out);
    push_embeds_and_links(&mut out);
    out
}

/// Heading levels 1-6.
///
/// `H1`…`H6`, so the row says which level it sets rather than all six
/// wearing the same `H`.
const fn heading_badge(level: u8) -> &'static str {
    match level {
        1 => "H1",
        2 => "H2",
        3 => "H3",
        4 => "H4",
        5 => "H5",
        _ => "H6",
    }
}

/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_headings(out: &mut Vec<CommandEntry>) {
    // ── Headings ───────────────────────────────────────────
    for (level, label) in [
        (1u8, "Heading 1"),
        (2, "Heading 2"),
        (3, "Heading 3"),
        (4, "Heading 4"),
        (5, "Heading 5"),
        (6, "Heading 6"),
    ] {
        out.push(CommandEntry {
            label,
            group: "Heading",
            desc: "",
            kind: CommandKind::SetHeading(level),
            icon: heading_badge(level),
        });
    }
}

/// Bullet, numbered, and task lists.
///
/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_lists(out: &mut Vec<CommandEntry>) {
    // ── Lists ──────────────────────────────────────────────
    out.extend([
        CommandEntry {
            label: "Bulleted list",
            group: "Structure",
            desc: "- item",
            kind: CommandKind::SetList(ListKind::Unordered),
            icon: "•",
        },
        CommandEntry {
            label: "Numbered list",
            group: "Structure",
            desc: "1. item",
            kind: CommandKind::SetList(ListKind::Ordered),
            icon: "1.",
        },
        CommandEntry {
            label: "Task",
            group: "Structure",
            desc: "- [ ] task",
            kind: CommandKind::SetList(ListKind::Task),
            icon: "[ ]",
        },
        CommandEntry {
            label: "Toggle task",
            group: "Structure",
            desc: "Mark current line as / un-task",
            kind: CommandKind::ToggleTask,
            icon: "[x]",
        },
        CommandEntry {
            label: "Quote",
            group: "Structure",
            desc: "> blockquote",
            kind: CommandKind::InsertSnippet("> ", 0),
            icon: ">",
        },
        CommandEntry {
            label: "Horizontal rule",
            group: "Structure",
            desc: "---",
            kind: CommandKind::InsertBlockSnippet("---\n", 0),
            icon: "---",
        },
        CommandEntry {
            label: "Table",
            group: "Structure",
            desc: "GFM pipe table skeleton",
            kind: CommandKind::InsertBlockSnippet(
                "| col1 | col2 |\n| ---- | ---- |\n|      |      |\n",
                21,
            ),
            icon: "tbl",
        },
    ]);
}

/// Fenced code blocks, inline/display math, and diagram fences.
///
/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_code_and_math(out: &mut Vec<CommandEntry>) {
    // ── Code & math ────────────────────────────────────────
    out.extend([
        CommandEntry {
            label: "Code block",
            group: "Code",
            desc: "```lang \\n … \\n```",
            kind: CommandKind::InsertBlockSnippet("```\n\n```\n", 5),
            icon: "{}",
        },
        CommandEntry {
            label: "Rust code block",
            group: "Code",
            desc: "```rust",
            kind: CommandKind::InsertBlockSnippet("```rust\n\n```\n", 5),
            icon: "rs",
        },
        CommandEntry {
            label: "TypeScript code block",
            group: "Code",
            desc: "```ts",
            kind: CommandKind::InsertBlockSnippet("```ts\n\n```\n", 5),
            icon: "ts",
        },
        CommandEntry {
            label: "Typst block",
            group: "Code",
            desc: "Compiled Typst — math, diagrams, layout",
            kind: CommandKind::InsertBlockSnippet("```typst\n\n```\n", 5),
            icon: "typ",
        },
        CommandEntry {
            label: "Mermaid diagram",
            group: "Code",
            desc: "```mermaid",
            kind: CommandKind::InsertBlockSnippet("```mermaid\n\n```\n", 5),
            icon: "~>",
        },
        CommandEntry {
            label: "Tabs",
            group: "Code",
            desc: "Switchable tab panels",
            // Caret lands after the first `=== Tab 1\n` so the
            // user types straight into the opening panel.
            kind: CommandKind::InsertBlockSnippet("```tabs\n=== Tab 1\n\n=== Tab 2\n\n```\n", 16),
            icon: "tab",
        },
        CommandEntry {
            label: "Inline math",
            group: "Math",
            desc: "$x$",
            kind: CommandKind::InsertSnippet("$$", 1),
            icon: "$",
        },
        CommandEntry {
            label: "Math block",
            group: "Math",
            desc: "$$\\n…\\n$$",
            kind: CommandKind::InsertBlockSnippet("$$\n\n$$\n", 4),
            icon: "$$",
        },
        CommandEntry {
            label: "Keyboard shortcut",
            group: "Code",
            desc: "`kbd:<C-s>` — rendered as key caps",
            kind: CommandKind::InsertSnippet("`kbd:`", 1),
            icon: "kbd",
        },
        CommandEntry {
            label: "Shortcut for action",
            group: "Code",
            desc: "`kbd:@action` — the keys currently bound to an action id",
            kind: CommandKind::InsertSnippet("`kbd:@`", 1),
            icon: "kbd",
        },
    ]);
}

/// Obsidian-style `> [!type]` callout headers.
///
/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_callouts(out: &mut Vec<CommandEntry>) {
    // ── Callouts ────────────────────────────────────────────
    // All 13 canonical Obsidian types. Snippet always has a
    // trailing space + newline + body so `caret_back = 3` lands
    // the caret on the title line (right after `> [!type] `),
    // ready for the user to type the title. Next Enter takes
    // them to the body.
    // No `Todo` callout — that's what the `- [ ]` task list
    // is for. Picking `/task` is the right path for actionable
    // items; the callout was redundant with it.
    // The snippet rides in the table rather than being derived from `kind` by
    // a second match: `InsertBlockSnippet` needs a `&'static str`, so it can't
    // be `format!`ed, and the parallel match it replaces had a wildcard arm
    // that could only ever be dead code.
    for (snippet, label, icon) in [
        ("> [!note] ", "Note", "📝"),
        ("> [!abstract] ", "Abstract", "📄"),
        ("> [!info] ", "Info", "ⓘ"),
        ("> [!tip] ", "Tip", "💡"),
        ("> [!success] ", "Success", "✅"),
        ("> [!question] ", "Question", "❓"),
        ("> [!warning] ", "Warning", "⚠"),
        ("> [!failure] ", "Failure", "❌"),
        ("> [!danger] ", "Danger", "⚡"),
        ("> [!bug] ", "Bug", "🐞"),
        ("> [!example] ", "Example", "🧪"),
        ("> [!quote] ", "Quote", "❞"),
    ] {
        // Just the header line with a trailing space — caret
        // ends right after `> [!type] ` ready for the title.
        // User hits Enter to drop into the body; the existing
        // `enter_continue_list` command picks up the
        // blockquote prefix and inserts `> ` on the next line.
        out.push(CommandEntry {
            label,
            group: "Callout",
            desc: "",
            kind: CommandKind::InsertBlockSnippet(snippet, 0),
            icon,
        });
    }
}

/// Logseq-style block ids and references.
///
/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_block_refs(out: &mut Vec<CommandEntry>) {
    // ── Block IDs / refs (Logseq-style) ─────────────────────
    // `Mod-Shift-K` is the keymap binding; the palette entry is
    // a discoverability path.
    out.push(CommandEntry {
        label: "Block id",
        group: "Block",
        desc: "Give this block an id so it can be referenced",
        kind: CommandKind::AddBlockId,
        icon: "#",
    });
}

/// Wikilinks, embeds, and external links.
///
/// One of the sections of [`all_commands`], split out so that function stays
/// a readable table of contents rather than a 240-line body.
fn push_embeds_and_links(out: &mut Vec<CommandEntry>) {
    // ── Embeds & links ─────────────────────────────────────
    out.extend([
        CommandEntry {
            label: "Link",
            group: "Link",
            desc: "[text](url)",
            kind: CommandKind::InsertSnippet("[]()", 3),
            icon: "url",
        },
        CommandEntry {
            label: "Wikilink",
            group: "Link",
            desc: "[[Page]]",
            kind: CommandKind::InsertSnippet("[[]]", 2),
            icon: "[[",
        },
        CommandEntry {
            label: "Embed",
            group: "Link",
            desc: "![[file]] — image / audio / video / pdf",
            kind: CommandKind::InsertSnippet("![[]]", 2),
            icon: "![[",
        },
        CommandEntry {
            label: "Footnote ref",
            group: "Link",
            desc: "[^id]",
            kind: CommandKind::InsertSnippet("[^]", 1),
            icon: "[^",
        },
        CommandEntry {
            label: "Inline footnote",
            group: "Link",
            desc: "^[note]",
            kind: CommandKind::InsertSnippet("^[]", 1),
            icon: "^[",
        },
    ]);
}

/// Insert a block-level snippet, replacing the palette query.
///
/// Split out of [`run_command`]'s `match`: this arm carries the block-context
/// handling (leading blank line, list-prefix stripping) that the inline
/// snippet arm does not, and it dominated that function's length.
fn insert_block_snippet(
    doc: &str,
    slash_range: std::ops::Range<usize>,
    text: &str,
    caret_back: usize,
) -> TransactionSpec {
    // Snap to the start of the current line. If there's
    // other text before the palette on this line, drop the
    // whole block on a fresh line below.
    let line_start = doc
        .before(slash_range.start)
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let prefix_text = doc.slice(line_start..slash_range.start);
    let line_has_content = !prefix_text.trim().is_empty();
    let snippet = if line_has_content {
        format!("\n{text}")
    } else {
        text.to_string()
    };
    // Build the final doc directly: strip the `/query`,
    // then insert the block snippet at the line-aware
    // anchor.
    let before = doc.before(slash_range.start);
    let after = doc.after(slash_range.end);
    let stripped = format!("{before}{after}");
    let anchor = if line_has_content {
        slash_range.start
    } else {
        line_start
    };
    let head = stripped.before(anchor);
    let tail = stripped.after(anchor);
    let final_doc = format!("{head}{snippet}{tail}");
    let new_caret = anchor
        .saturating_add(snippet.len())
        .saturating_sub(caret_back);
    TransactionSpec::new()
        .changes(Changes::replace(0..doc.len(), final_doc))
        .selection(Selection::caret(new_caret))
        .annotate("origin", "palette")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_at_start_of_doc() {
        assert_eq!(detect_trigger("\\cal", 4), Some((0, "cal".to_string())));
    }

    #[test]
    fn detect_after_whitespace() {
        assert_eq!(
            detect_trigger("hi \\code", 8),
            Some((3, "code".to_string()))
        );
    }

    #[test]
    fn a_slash_never_opens_the_palette() {
        // The whole reason the trigger moved. `/` is ordinary text in a
        // Keyflow chart — a slash chord, a time signature, a bar of
        // rhythm — and every one of these used to open the menu.
        assert_eq!(detect_trigger("G/B", 3), None);
        assert_eq!(detect_trigger("4/4", 3), None);
        assert_eq!(detect_trigger("V/V", 3), None);
        assert_eq!(detect_trigger("////", 4), None);
        assert_eq!(detect_trigger("https://", 8), None);
    }

    #[test]
    fn a_markdown_escape_is_not_a_command() {
        // CommonMark escapes are `\` + ASCII punctuation, and nothing else
        // can be escaped — so punctuation after the trigger is always the
        // author escaping a character, never a command they are hunting for.
        for esc in ["\\*", "\\_", "\\[", "\\#", "\\`", "\\!", "\\.", "\\-"] {
            assert_eq!(detect_trigger(esc, esc.len()), None, "{esc} is an escape");
        }
    }

    #[test]
    fn a_letter_after_the_trigger_is_a_command() {
        assert_eq!(detect_trigger("\\q", 2), Some((0, "q".to_string())));
        assert_eq!(detect_trigger("\\h1", 3), Some((0, "h1".to_string())));
    }

    #[test]
    fn a_bare_trigger_opens_the_menu() {
        // Nothing typed after it yet: the menu opens so the catalog is
        // discoverable, and the next character decides what this was.
        assert_eq!(detect_trigger("\\", 1), Some((0, String::new())));
    }

    #[test]
    fn an_escaped_backslash_is_a_literal() {
        // `\\` is markdown for one literal backslash, not a command.
        assert_eq!(detect_trigger("\\\\", 2), None);
    }

    #[test]
    fn detect_triggers_after_ordinary_text() {
        // Typing the trigger at the end of prose opens the menu, the way
        // Notion and most modern editors behave.
        assert_eq!(detect_trigger("hello\\", 6), Some((5, String::new())));
        assert_eq!(
            detect_trigger("hello\\cal", 9),
            Some((5, "cal".to_string()))
        );
    }

    #[test]
    fn detect_closes_on_space() {
        assert_eq!(detect_trigger("\\foo bar", 8), None);
    }

    #[test]
    fn detect_scoped_to_current_line() {
        // A trigger on a previous line shouldn't keep the menu open
        // across newlines.
        assert_eq!(detect_trigger("\\old\nnew here", 13), None);
    }

    #[test]
    fn a_host_catalog_replaces_the_markdown_one() {
        // Serialised against the other tests only by running last-ish; the
        // registry is process-wide, so this restores it before returning.
        assert!(
            markdown_commands().iter().any(|c| c.label == "Heading 1"),
            "the built-in catalog is markdown's"
        );
        register_catalog(vec![CommandEntry {
            label: "Quarter note",
            group: "Rhythm",
            desc: "_4",
            kind: CommandKind::InsertSnippet("_4", 0),
            icon: "_4",
        }]);
        let hits = all_commands();
        assert_eq!(hits.len(), 1, "the host catalog replaces, not extends");
        assert_eq!(hits[0].label, "Quarter note");
        // Put it back so catalog-agnostic tests are unaffected.
        if let Ok(mut slot) = catalog().write() {
            *slot = None;
        }
        assert!(all_commands().iter().any(|c| c.label == "Heading 1"));
    }

    #[test]
    fn filter_matches_group() {
        let hits = filter_commands("call");
        // After the label rename, callouts are matched by group
        // (`"Callout"`) rather than by the label itself.
        assert!(hits.iter().any(|c| c.group == "Callout"));
    }
}
