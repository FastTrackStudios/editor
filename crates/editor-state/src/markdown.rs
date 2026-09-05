//! Markdown live-preview decoration source.
//!
//! Scans the doc for `**…**` (bold), `*…*` (italic), and
//! `` `…` `` (inline code) spans and emits decorations:
//!
//! - The body of the span gets a `MarkDecoration` with the
//!   corresponding class (`md-bold`, `md-italic`, `md-code`).
//! - The opening + closing markers are `Replace`d **only when
//!   the primary cursor is outside the span**. While the cursor
//!   is on the span, markers stay visible so the user sees
//!   the raw markdown and can edit it directly.
//!
//! This is exactly Obsidian's "Live Preview" mode in spirit —
//! it's a renderer trick, not a document-model change.
//!
//! The parser is single-pass and intentionally tiny. Not a real
//! `CommonMark` implementation; just enough to demo the
//! decoration pipeline. A future commit can swap in a proper
//! markdown parser (pulldown-cmark or a port of CM6's
//! lang-markdown) without touching the decoration shape.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::decoration::{DecoratedRange, Decoration};
use crate::selection::Range;
use crate::state::EditorState;
use crate::text::{ByteSlice, TextSlice};

/// The full live-preview decoration source.
///
/// Suitable to register as `editor_view::DecorationSource`. Trait the
/// editor uses to resolve cross-file references — `((uuid))`, `[[Page]]`,
/// `![[Page#Heading]]`, etc. — without pulling a vault implementation into
/// `editor-state`. The `vault` crate provides the canonical impl; tests /
/// single-file uses pass `None`.
pub trait VaultLookup {
    /// Find a block by full UUID across the vault. Returns the
    /// containing page's basename and a short preview.
    fn lookup_block(&self, uuid: &str) -> Option<VaultBlockHit>;
    /// Find a page by basename (case-insensitive). Returns a
    /// content preview suitable for an embed card.
    fn lookup_page(&self, name: &str) -> Option<VaultPageHit>;
    /// Find a section `Page#Heading`. Returns the body of the
    /// section (heading line + content until next same-or-
    /// higher heading), or None when the page or heading is
    /// missing.
    fn lookup_section(&self, page: &str, heading: &str) -> Option<String>;
    /// Song metadata when `name` resolves to a `type: song` note —
    /// `None` (default) renders the wikilink normally.
    fn lookup_song(&self, _name: &str) -> Option<VaultSongHit> {
        None
    }
    /// The target note's frontmatter `type:` ("song", "setlist",
    /// "contact", "event", …) — drives kind-specific wikilink rendering
    /// (setlist cards, contact chips). `None` (default) = plain link.
    fn lookup_note_kind(&self, _name: &str) -> Option<String> {
        None
    }
    /// Setlist metadata when `name` resolves to a `type: setlist` note.
    fn lookup_setlist(&self, _name: &str) -> Option<VaultSetlistHit> {
        None
    }
    /// Scripture reference resolution: when `target` parses as a verse
    /// reference (`John 3:16`, `John 3:16-20`, `Rom 5:8@ESV`) the host
    /// returns display info + (possibly still loading) verse text, and
    /// the link renders as a scripture chip / verse card instead of an
    /// unresolved wikilink. Only consulted when no page matches the
    /// target, so a real `John 3:16.md` note still wins. `None`
    /// (default) = plain link.
    fn lookup_scripture(&self, _target: &str) -> Option<VaultScriptureHit> {
        None
    }
    /// Find a block by Obsidian short-id `Page#^id`.
    fn lookup_block_short(&self, page: &str, short_id: &str) -> Option<String>;
}

#[derive(Clone, Debug)]
pub struct VaultBlockHit {
    pub page: String,
    pub preview: String,
}

#[derive(Clone, Debug)]
pub struct VaultPageHit {
    pub preview: String,
}

/// Setlist metadata for a wikilink that targets a `type: setlist` note —
/// drives the inline SETLIST CARD (a standalone `[[Setlist]]` line embeds
/// the set as a compact card).
#[derive(Clone, Debug, PartialEq)]
pub struct VaultSetlistHit {
    pub title: String,
    pub song_count: usize,
    pub total_seconds: f64,
    /// The set's songs, in order — the embed renders the full reference
    /// player (header + one row per song).
    pub songs: Vec<VaultSetlistSongRow>,
}

/// One song row inside a setlist embed.
#[derive(Clone, Debug, PartialEq)]
pub struct VaultSetlistSongRow {
    /// The wikilink target (note name) — drives navigation + play.
    pub link: String,
    pub artist: Option<String>,
    pub duration_sec: f64,
    pub stem_count: usize,
}

/// A wikilink target that parses as a scripture reference.
///
/// Drives the inline SCRIPTURE CHIP (any `[[John 3:16]]` in running text) and the
/// VERSE CARD (a standalone `[[John 3:16]]` line embeds the verse text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultScriptureHit {
    /// Canonical display reference, e.g. `John 3:16–20`.
    pub display: String,
    /// OSIS id / range (`John.3.16`), the stable anchor key.
    pub osis: String,
    /// The verse text (range text joined), or `None` while the host is
    /// still fetching — the card shows a loading row, the chip renders
    /// resolved either way.
    pub text: Option<String>,
    /// Translation id the text came from (`WEB`, `ESV`).
    pub translation: String,
}

/// Song metadata for a wikilink that targets a `type: song` note —
/// drives the inline SONG STRIP widget (a standalone `[[Song]]` line
/// renders as a playable row instead of a plain link).
#[derive(Clone, Debug, PartialEq)]
pub struct VaultSongHit {
    pub title: String,
    pub artist: Option<String>,
    pub duration_sec: f64,
    pub stem_count: usize,
}

/// Host-supplied resolver for `kbd:@action` inline shortcuts.
///
/// Maps an action id (numeric or named command, e.g. `40044` or `_FTS_SESSION_TAKE_RANK_PLAYPOS_1`)
/// to the key sequence currently bound to it (`"<C-S-space>"`, `"n d"`). Mirrors [`VaultLookup`]:
/// `editor-state` stays app-agnostic, the host owns the keymap. Unresolved refs render as a
/// distinct "unbound" cap.
pub trait KbdLookup {
    fn keys_for_action(&self, action: &str) -> Option<String>;
}

/// Key-caps widget for a `kbd:` code span. `spec` is what follows the
/// prefix: a literal key sequence (`<C-s>`, `n d`) or `@action`.
/// Returns `None` when the spec is empty/unparseable so the caller
/// falls back to plain inline-code styling.
fn kbd_widget_html(spec: &str, kbd: Option<&dyn KbdLookup>) -> Option<String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let keys: String = if let Some(action) = spec.strip_prefix('@') {
        let action = action.trim();
        if action.is_empty() {
            return None;
        }
        match kbd.and_then(|k| k.keys_for_action(action)) {
            Some(keys) => keys,
            // Unresolved action ref: a distinct "unbound" cap showing
            // the action id, rather than breaking the note.
            None => {
                return Some(format!(
                    r#"<span class="md-kbd md-kbd-unbound" title="No key currently bound to this action"><kbd class="md-kbd-key">@{}</kbd></span>"#,
                    escape_html(action),
                ));
            }
        }
    } else {
        spec.to_string()
    };

    let chords: Vec<Vec<String>> = keys.split_whitespace().map(kbd_chord_labels).collect();
    if chords.is_empty() || chords.iter().any(Vec::is_empty) {
        return None;
    }
    let mut html = String::from(r#"<span class="md-kbd">"#);
    for (ci, chord) in chords.iter().enumerate() {
        if ci > 0 {
            html.push_str(r#"<span class="md-kbd-then">then</span>"#);
        }
        for (ki, key) in chord.iter().enumerate() {
            if ki > 0 {
                html.push_str(r#"<span class="md-kbd-plus">+</span>"#);
            }
            let _ = write!(
                html,
                r#"<kbd class="md-kbd-key">{}</kbd>"#,
                escape_html(key)
            );
        }
    }
    html.push_str("</span>");
    Some(html)
}

/// One chord token → display labels: `"<C-S-space>"` → `Ctrl Shift
/// Space`, `"r"` → `R`. `C`/`S`/`A` are Ctrl/Shift/Alt; `M` and `D`
/// both mean the platform Meta/Cmd key.
fn kbd_chord_labels(token: &str) -> Vec<String> {
    let inner = token
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(token);

    let mut parts = Vec::new();
    let mut rest = inner;
    while let Some((m, tail)) = rest.split_once('-') {
        let label = match m {
            "C" => "Ctrl",
            "S" => "Shift",
            "A" => "Alt",
            "M" | "D" => "Meta",
            _ => break,
        };
        parts.push(label.to_string());
        rest = tail;
    }

    // Bare-modifier chords like `<C->` have no tail key.
    if rest.is_empty() {
        return parts;
    }

    let key = match rest {
        "space" => "Space".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "esc" | "escape" => "Esc".to_string(),
        "tab" => "Tab".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" | "del" => "Delete".to_string(),
        "minus" => "-".to_string(),
        "plus" => "+".to_string(),
        "up" => "\u{2191}".to_string(),
        "down" => "\u{2193}".to_string(),
        "left" => "\u{2190}".to_string(),
        "right" => "\u{2192}".to_string(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_string(),
    };
    parts.push(key);
    parts
}

thread_local! {
    /// Set while a one-shot render is in progress. See
    /// [`render_everything`].
    static RENDER_EVERYTHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is a one-shot render in progress?
///
/// The per-pass compile budgets consult this: when it is set they arm
/// themselves to `u8::MAX` instead of their usual couple of cold compiles.
#[must_use]
pub fn rendering_everything() -> bool {
    RENDER_EVERYTHING.with(std::cell::Cell::get)
}

/// Run `f` with every per-pass compile budget lifted.
///
/// The budgets exist because the decoration pass runs on every keystroke:
/// a note full of fresh math would compile all of it per character typed,
/// so each pass spends a couple of cold compiles and the rest fall back to
/// source until the next pass. Over a few passes a live document
/// converges, and the cache carries what is already rendered.
///
/// A one-shot render — the static HTML the guide is built from — has no
/// next pass. Under the live budget the third equation on a page was
/// never compiled at all and shipped as raw `$…$` source, permanently.
/// Anything rendering once must call this.
pub fn render_everything<R>(f: impl FnOnce() -> R) -> R {
    RENDER_EVERYTHING.with(|c| c.set(true));
    let out = f();
    RENDER_EVERYTHING.with(|c| c.set(false));
    out
}

/// Decorations for `state`, with no vault or keybinding lookups.
#[must_use]
pub fn live_preview(state: &EditorState) -> Vec<DecoratedRange> {
    live_preview_with_lookups(state, None, None)
}

#[must_use]
pub fn live_preview_with(
    state: &EditorState,
    vault: Option<&dyn VaultLookup>,
) -> Vec<DecoratedRange> {
    live_preview_with_lookups(state, vault, None)
}

pub fn live_preview_with_lookups(
    state: &EditorState,
    vault: Option<&dyn VaultLookup>,
    kbd: Option<&dyn KbdLookup>,
) -> Vec<DecoratedRange> {
    // Per-pass compile budget for Typst — bounds the worst
    // case at a couple of cold compiles per render so a doc
    // full of fresh math doesn't block typing. See `typst`
    // submodule for the budget value and rationale.
    crate::plugin::ensure_builtins();
    reset_compile_budget();
    reset_mermaid_budget();
    reset_keyflow_budget();
    reset_tabs_budget();
    reset_block_index();

    let text = state.doc.to_string();
    // In reading mode, swap the primary selection for one that
    // can't touch any byte range — `cursor_touches` then always
    // returns false, so every marker stays hidden. Same effect
    // as Obsidian's preview-only mode.
    let primary = if state.reading_mode {
        Range::caret(usize::MAX)
    } else {
        state.selection.primary()
    };
    let mut out = Vec::new();

    // Per-step timing for the perf trace. The cost in this fn is
    // dominated by `emit_fence_tokens` (tree-sitter) on docs
    // with code fences; the rest is O(doc-length) byte walking.
    let t_blocks = now_ms_native();
    let fenced_ranges = scan_blocks(&text, primary, &mut out);
    let blocks_ms = now_ms_native() - t_blocks;

    let t_inline = now_ms_native();
    let inline_decs_before = out.len();
    emit_status_pills(&text, primary, &mut out);
    emit_roster_rows(&text, primary, vault, &mut out);
    // Lazily computed on the first song strip (resolver scans are cheap
    // and cached, but most documents have no strips at all).
    let mut strip_runs: Option<std::collections::HashMap<usize, StripRunCtx>> = None;
    decorate_inline_spans(
        &text,
        primary,
        &fenced_ranges,
        vault,
        kbd,
        &mut strip_runs,
        &mut out,
    );
    let inline_ms = now_ms_native() - t_inline;
    let inline_decs = out.len().saturating_sub(inline_decs_before);
    tracing::debug!(
        doc_len = text.len(),
        block_decs = inline_decs_before,
        inline_decs,
        fence_count = fenced_ranges.len(),
        blocks_ms = %format!("{:.2}", blocks_ms),
        inline_ms = %format!("{:.2}", inline_ms),
        "md.live_preview"
    );
    out
}

/// Wall-clock milliseconds. wasm-safe alias around the
/// view-layer `now_ms`; mirrored here so editor-state stays
/// free of dioxus deps.
fn now_ms_native() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        let s = START.get_or_init(std::time::Instant::now);
        s.elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now())
    }
}

// YAML frontmatter lives in its own submodule — parser,
// serializer, and Properties widget renderer.
pub mod frontmatter;
use frontmatter::render_properties_html;
pub use frontmatter::{FrontMatter, PropValue, Property, parse_frontmatter, serialize_property};

pub(crate) mod typst;
use typst::{TypstKind, render_typst, reset_compile_budget};

pub(crate) mod mermaid;
use mermaid::reset_compile_budget as reset_mermaid_budget;

mod keyflow;
use keyflow::{render_keyflow, reset_compile_budget as reset_keyflow_budget};

mod tabs;
use tabs::{render_tabs, reset_render_budget as reset_tabs_budget};

pub(crate) fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn in_fenced_code(ranges: &[std::ops::Range<usize>], pos: usize) -> bool {
    ranges.iter().any(|r| pos >= r.start && pos < r.end)
}

/// Layout context for one song-strip line: joined to a strip directly
/// above/below (no blank line between) + alternating parity within its
/// run — lets adjacent strips render as one flush, striped list.
#[derive(Clone, Copy, Default)]
struct StripRunCtx {
    joined_above: bool,
    joined_below: bool,
    odd: bool,
    /// 1-based position within the run — the setlist order number shown
    /// where the play control appears on hover.
    index: usize,
}

/// Scan the document for standalone `[[Song]]` lines (resolver-confirmed
/// songs) and compute each one's run context, keyed by line start.
fn song_strip_runs(
    text: &str,
    vault: Option<&dyn VaultLookup>,
) -> std::collections::HashMap<usize, StripRunCtx> {
    let Some(vault) = vault else {
        return HashMap::new();
    };
    // Collect candidate lines: (line_start, line_end).
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    let mut pos: usize = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let t = content.trim();
        if let Some(inner) = t.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
            let page = inner.split(['#', '|']).next().unwrap_or(inner).trim();
            if vault.lookup_song(page).is_some() {
                candidates.push((pos, pos.saturating_add(content.len())));
            }
        }
        pos = pos.saturating_add(line.len());
    }
    // Group into runs: consecutive candidates whose lines are ADJACENT
    // (exactly one newline between them).
    let mut out = std::collections::HashMap::new();
    let mut i = 0;
    while i < candidates.len() {
        let mut j = i;
        // Extend the run while the next candidate starts exactly one byte
        // after this one ends — i.e. they are separated by a single newline.
        while let (Some(&(next_start, _)), Some(&(_, cur_end))) =
            (candidates.get(j.saturating_add(1)), candidates.get(j))
        {
            if next_start != cur_end.saturating_add(1) {
                break;
            }
            j = j.saturating_add(1);
        }
        for (k, &(start, _)) in candidates.get(i..=j).unwrap_or_default().iter().enumerate() {
            out.insert(
                start,
                StripRunCtx {
                    joined_above: k > 0,
                    joined_below: i.saturating_add(k) < j,
                    odd: k % 2 == 1,
                    index: k.saturating_add(1),
                },
            );
        }
        i = j.saturating_add(1);
    }
    out
}

/// A small stroke-icon (Lucide-shaped, `currentColor`) for widget HTML —
/// inherits the role chip's color.
fn role_icon_svg(kind: &str) -> String {
    let body = match kind {
        "drum" => {
            r#"<path d="m2 2 8 8"/><path d="m22 2-8 8"/><ellipse cx="12" cy="9" rx="10" ry="5"/><path d="M7 13.4v7.9"/><path d="M12 14v8"/><path d="M17 13.4v7.9"/><path d="M2 9v8a10 5 0 0 0 20 0V9"/>"#
        }
        "guitar" => {
            r#"<circle cx="8" cy="16" r="5"/><path d="m11.8 12.2 7.2-7.2"/><path d="m18 3 3 3"/><path d="m19 4-2.5 2.5"/>"#
        }
        "keys" => {
            r#"<rect x="2" y="6" width="20" height="12" rx="1"/><path d="M7 6v7"/><path d="M12 6v7"/><path d="M17 6v7"/>"#
        }
        "mic" => {
            r#"<path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/>"#
        }
        "sliders" => {
            r#"<line x1="21" x2="14" y1="4" y2="4"/><line x1="10" x2="3" y1="4" y2="4"/><line x1="21" x2="12" y1="12" y2="12"/><line x1="8" x2="3" y1="12" y2="12"/><line x1="21" x2="16" y1="20" y2="20"/><line x1="12" x2="3" y1="20" y2="20"/><line x1="14" x2="14" y1="2" y2="6"/><line x1="8" x2="8" y1="10" y2="14"/><line x1="16" x2="16" y1="18" y2="22"/>"#
        }
        "bulb" => {
            r#"<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1.3.5 2.6 1.5 3.5.8.8 1.3 1.5 1.5 2.5"/><path d="M9 18h6"/><path d="M10 22h4"/>"#
        }
        "monitor" => {
            r#"<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>"#
        }
        "video" => {
            r#"<path d="m16 13 5.2 3.5a.5.5 0 0 0 .8-.4V7.9a.5.5 0 0 0-.8-.4L16 11"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#
        }
        // "music" and anything unrecognized.
        _ => {
            r#"<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>"#
        }
    };
    format!(
        r#"<svg class="md-role-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
    )
}

/// The FTS instrument color scheme + an icon per role — reflected on the
/// roster's role chips (Drums red, Bass yellow, Electric blue, Acoustic
/// cyan, Keys green, Synth purple, vocals pink, tech slate…).
fn role_style(role: &str) -> (&'static str, &'static str) {
    let r = role.to_ascii_lowercase();
    if r.contains("drum") || r.contains("perc") {
        ("md-role--red", "drum")
    } else if r.contains("bass") {
        ("md-role--yellow", "guitar")
    } else if r.contains("electric") {
        ("md-role--blue", "guitar")
    } else if r.contains("acoustic") {
        ("md-role--cyan", "guitar")
    } else if r.contains("key") || r.contains("piano") {
        ("md-role--green", "keys")
    } else if r.contains("synth") || r.contains("organ") {
        ("md-role--purple", "keys")
    } else if r.contains("vocal") || r.contains("worship leader") || r.contains("singer") {
        ("md-role--pink", "mic")
    } else if r.contains("music director") {
        ("md-role--orange", "music")
    } else if r.contains("foh") || r.contains("audio") || r.contains("sound") {
        ("md-role--slate", "sliders")
    } else if r.contains("light") {
        ("md-role--amber", "bulb")
    } else if r.contains("graphic") || r.contains("lyric") || r.contains("screen") {
        ("md-role--slate", "monitor")
    } else if r.contains("production") || r.contains("director") {
        ("md-role--orange", "video")
    } else {
        ("md-role--slate", "music")
    }
}

/// Roster rows: `Role - [[Name]] (Status)[, [[Name]] (Status)…]` where
/// every target is a `type: contact` note renders as a TEAM row widget —
/// role chip + one CONTACT CARD per person: initials avatar with a
/// status ring + badge (green ✓ confirmed / amber ? pending / red ✕
/// declined) and the name. Caret on the line = raw editable text.
fn emit_roster_rows(
    text: &str,
    primary: Range,
    vault: Option<&dyn VaultLookup>,
    out: &mut Vec<DecoratedRange>,
) {
    let Some(vault) = vault else { return };
    let mut pos: usize = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let line_from = pos;
        pos = pos.saturating_add(line.len());
        let line_to = line_from.saturating_add(content.len());
        let t = content.trim();
        let Some(dash) = t.find(" - [[") else {
            continue;
        };
        let role = t.before(dash).trim();
        if role.is_empty() || role.starts_with('#') || role.starts_with('[') {
            continue;
        }
        // Parse the people list: repeated `[[Name]]` + optional `(status)`.
        let mut rest = t.after(dash.saturating_add(3));
        let mut people: Vec<(String, &'static str, &'static str, &'static str)> = Vec::new();
        while let Some(open) = rest.find("[[") {
            let Some(close_rel) = rest.after(open).find("]]") else {
                break;
            };
            let name = rest
                .slice(open.saturating_add(2)..open.saturating_add(close_rel))
                .trim();
            let name = name.split(['#', '|']).next().unwrap_or(name).trim();
            rest = rest.after(open.saturating_add(close_rel).saturating_add(2));
            let (st_cls, badge, ring) = {
                let after = rest.trim_start().trim_start_matches(',').trim_start();
                after
                    .strip_prefix('(')
                    .and_then(|r| r.split_once(')').map(|(a, _)| a))
                    .map_or(("md-av--none", "", ""), |inner| {
                        match inner.trim().to_ascii_lowercase().as_str() {
                            "confirmed" => ("md-av--confirmed", "✓", "confirmed"),
                            "declined" => ("md-av--declined", "✕", "declined"),
                            _ => ("md-av--pending", "?", "pending"),
                        }
                    })
            };
            if vault.lookup_note_kind(name).as_deref() != Some("contact") {
                people.clear();
                break;
            }
            people.push((name.to_owned(), st_cls, badge, ring));
        }
        if people.is_empty() || cursor_touches(primary, line_from..line_to) {
            continue;
        }
        // `fold` + `write!` rather than `map(format!).collect()`: one buffer
        // for the whole row instead of a fresh String per contact.
        let cards = people
            .iter()
            .fold(String::new(), |mut acc, (name, st, badge, ring)| {
                let initials: String = name
                    .split_whitespace()
                    .take(2)
                    .filter_map(|w| w.chars().next())
                    .collect::<String>()
                    .to_uppercase();
                let badge_html = if badge.is_empty() {
                    String::new()
                } else {
                    format!(r#"<span class="md-av-badge md-av-badge--{ring}">{badge}</span>"#)
                };
                let _ = write!(
                    acc,
                    r#"<span class="md-contact-card" data-href="{n}"><span class="md-avatar {st}">{initials}{badge_html}</span><span class="md-contact-name">{n}</span></span>"#,
                    n = html_escape(name),
                );
                acc
            });
        let (role_cls, icon_kind) = role_style(role);
        let icon = role_icon_svg(icon_kind);
        out.push(Decoration::replace(line_from..line_to));
        out.push(Decoration::widget(
            line_from,
            format!(
                r#"<span class="md-roster-row"><span class="md-roster-role {role_cls}">{icon}{role}</span><span class="md-roster-people">{cards}</span></span>"#,
                role = html_escape(role),
            ),
        ));
        out.push(Decoration::atomic(line_from..line_to));
    }
}

/// Assignment-status pills: a line ending in `(Confirmed)` / `(Pending)`
/// / `(Declined)` (the event-planner roster convention:
/// `Drums - [[Name]] (Pending)`) renders the token as a colored pill and
/// hides the parens. Caret on the line keeps the raw text editable.
fn emit_status_pills(text: &str, primary: Range, out: &mut Vec<DecoratedRange>) {
    let mut pos: usize = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let line_from = pos;
        pos = pos.saturating_add(line.len());
        let trimmed_end = content.trim_end();
        let Some(open_rel) = trimmed_end.rfind('(') else {
            continue;
        };
        let Some(inner) = trimmed_end
            .after(open_rel)
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
        else {
            continue;
        };
        let status = match inner.trim().to_ascii_lowercase().as_str() {
            "confirmed" => "md-status--confirmed",
            "pending" => "md-status--pending",
            "declined" => "md-status--declined",
            _ => continue,
        };
        let line_to = line_from.saturating_add(content.len());
        if cursor_touches(primary, line_from..line_to) {
            continue;
        }
        let open_abs = line_from.saturating_add(open_rel);
        let close_abs = line_from
            .saturating_add(trimmed_end.len())
            .saturating_sub(1);
        let word_from = open_abs.saturating_add(1);
        let word_to = close_abs;
        out.push(Decoration::replace(open_abs..word_from));
        out.push(Decoration::mark(
            word_from..word_to,
            match status {
                "md-status--confirmed" => "md-status-pill md-status--confirmed",
                "md-status--pending" => "md-status-pill md-status--pending",
                _ => "md-status-pill md-status--declined",
            },
        ));
        out.push(Decoration::replace(word_to..close_abs.saturating_add(1)));
    }
}

/// The inline setlist-card widget for a standalone `[[Setlist]]` wikilink
/// — a compact embed: art tile · title · song count · duration. Clicking
/// navigates to the setlist note (`data-href`).
fn setlist_card_html(target: &str, setlist: &VaultSetlistHit) -> String {
    let safe = html_escape(target);
    let title = html_escape(&setlist.title);
    let n = setlist.song_count;
    // See the roster row above: fold into one buffer rather than allocating a
    // String per song.
    let rows = setlist
        .songs
        .iter()
        .enumerate()
        .fold(String::new(), |mut acc, (i, row)| {
            let link = html_escape(&row.link);
            let (disp_title, disp_artist) = split_title_artist(&row.link, row.artist.as_deref());
            let title = html_escape(&disp_title);
            let artist = disp_artist
                .map(|a| format!(r#"<span class="md-song-strip-artist">{}</span>"#, html_escape(&a)))
                .unwrap_or_default();
            let initial = html_escape(
                &disp_title.chars().next().unwrap_or('♪').to_uppercase().to_string(),
            );
            let mut cls = String::from("md-song-strip");
            if i > 0 {
                cls.push_str(" md-song-strip--ja");
            }
            if i.saturating_add(1)< setlist.songs.len() {
                cls.push_str(" md-song-strip--jb");
            }
            if i % 2 == 1 {
                cls.push_str(" md-song-strip--alt");
            }
            let _ = write!(
                acc,
                r#"<span class="{cls}" data-href="song-play:{link}"><span class="md-song-strip-num" data-href="song-play:{link}"><span class="md-ss-idx">{idx}</span><svg class="md-ss-play" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg></span><span class="md-ss-art"><span class="md-ss-art-i">{initial}</span><span class="md-ss-eq"><i></i><i></i><i></i><i></i></span></span><span class="md-ss-titles"><span class="md-song-strip-title">{title}</span>{artist}</span><span class="md-ss-open" data-href="{link}" title="Open song"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 17 17 7"/><path d="M9 7h8v8"/></svg></span><span class="md-ss-more" data-href="song-more:{link}"><svg viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.9"/><circle cx="12" cy="12" r="1.9"/><circle cx="19" cy="12" r="1.9"/></svg></span></span>"#,
                idx = i.saturating_add(1),
            );
            acc
        });
    format!(
        r#"<span class="md-setlist-embed"><span class="md-setlist-card" data-href="{safe}"><span class="md-setlist-card-art">🎵</span><span class="md-setlist-card-titles"><span class="md-setlist-card-title">{title}</span><span class="md-setlist-card-sub">Setlist · {n} songs</span></span><span class="md-setlist-card-open">Open ›</span></span>{rows}</span>"#
    )
}

/// Split a `"Title - Artist"` display string into (title, artist). Falls back
/// to `fallback` for the artist when there's no ` - ` separator, so the strip
/// shows a clean title with the artist as a subtitle rather than repeating
/// `"Song - Artist"` on the title line.
fn split_title_artist(raw: &str, fallback: Option<&str>) -> (String, Option<String>) {
    if let Some((t, a)) = raw.split_once(" - ") {
        let a = a.trim();
        (t.trim().to_string(), (!a.is_empty()).then(|| a.to_string()))
    } else {
        (
            raw.trim().to_string(),
            fallback.map(std::string::ToString::to_string),
        )
    }
}

/// The inline song-strip widget for a standalone `[[Song]]` wikilink.
/// The whole strip navigates (`data-href` = the link target); the play
/// control carries `data-href="song-play:<target>"` — the host's
/// `on_link_click` intercepts the scheme and drives playback.
fn song_strip_html(target: &str, song: &VaultSongHit, ctx: StripRunCtx) -> String {
    let safe = html_escape(target);
    let (disp_title, disp_artist) = split_title_artist(&song.title, song.artist.as_deref());
    let title = html_escape(&disp_title);
    let artist = disp_artist
        .map(|a| {
            format!(
                r#"<span class="md-song-strip-artist">{}</span>"#,
                html_escape(&a)
            )
        })
        .unwrap_or_default();
    let mut cls = String::from("md-song-strip");
    if ctx.joined_above {
        cls.push_str(" md-song-strip--ja");
    }
    if ctx.joined_below {
        cls.push_str(" md-song-strip--jb");
    }
    if ctx.odd {
        cls.push_str(" md-song-strip--alt");
    }
    let idx = ctx.index.max(1);
    let initial = html_escape(
        &disp_title
            .chars()
            .next()
            .unwrap_or('♪')
            .to_uppercase()
            .to_string(),
    );
    format!(
        r#"<span class="{cls}" data-href="song-play:{safe}"><span class="md-song-strip-num" data-href="song-play:{safe}"><span class="md-ss-idx">{idx}</span><svg class="md-ss-play" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg></span><span class="md-ss-art"><span class="md-ss-art-i">{initial}</span><span class="md-ss-eq"><i></i><i></i><i></i><i></i></span></span><span class="md-ss-titles"><span class="md-song-strip-title">{title}</span>{artist}</span><span class="md-ss-open" data-href="{safe}" title="Open song"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 17 17 7"/><path d="M9 7h8v8"/></svg></span><span class="md-ss-more" data-href="song-more:{safe}"><svg viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.9"/><circle cx="12" cy="12" r="1.9"/><circle cx="19" cy="12" r="1.9"/></svg></span></span>"#
    )
}

/// The inline verse-card widget for a standalone `[[John 3:16]]`
/// wikilink: the verse text as a block quote with the reference +
/// translation as the caption. The card carries
/// `data-href="scripture-open:<target>"` — the host routes it to the
/// scripture reader anchored at the verse.
fn scripture_card_html(target: &str, sc: &VaultScriptureHit) -> String {
    let safe = html_escape(target);
    let display = html_escape(&sc.display);
    let tx = html_escape(&sc.translation);
    let body = sc
        .text
        .as_ref()
        .map_or_else(|| "Loading…".to_string(), |t| html_escape(t));
    format!(
        r#"<span class="md-scripture-card" data-href="scripture-open:{safe}"><span class="md-scripture-card-text">{body}</span><span class="md-scripture-card-ref"><span class="md-scripture-card-display">{display}</span><span class="md-scripture-card-tx">{tx}</span><span class="md-scripture-card-open">Study ›</span></span></span>"#
    )
}

/// Render the HTML for an `![[file|opts]]` embed when the
/// target's extension maps to a media kind we know. Returns
/// `None` for embeds we don't yet support (notes, unknown
/// formats) — the caller then falls back to a wikilink-style
/// mark so the source stays visible. Quartz reference: same
/// dispatch in `ofm.ts:233-265`.
fn embed_widget_html(raw: &str, doc: &str, vault: Option<&dyn VaultLookup>) -> String {
    let (target, opts) = match raw.split_once('|') {
        Some((t, o)) => (t.trim(), Some(o.trim())),
        None => (raw.trim(), None),
    };
    let ext = target.rsplit_once('.').map(|x| x.1.to_ascii_lowercase());
    let ext = ext.as_deref().unwrap_or("");
    let safe_target = html_escape(target);
    let style = opts.and_then(parse_size_opts).unwrap_or_default();
    // 1. Media extensions first (image / video / audio / pdf).
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "avif" | "bmp" => {
            return format!(
                r#"<img class="md-embed-image" src="{safe_target}" alt="{safe_target}"{style}>"#
            );
        }
        "mp4" | "webm" | "mov" | "ogv" => {
            return format!(
                r#"<video class="md-embed-video" src="{safe_target}" controls{style}></video>"#
            );
        }
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => {
            return format!(
                r#"<audio class="md-embed-audio" src="{safe_target}" controls></audio>"#
            );
        }
        "pdf" => {
            return format!(r#"<iframe class="md-embed-pdf" src="{safe_target}"{style}></iframe>"#);
        }
        _ => {}
    }
    // 2. Note-style embeds. Split into page + fragment parts.
    //    Shapes:
    //      `![[Page]]`            — whole-page embed
    //      `![[Page#Heading]]`    — section embed
    //      `![[Page#^short-id]]`  — block embed (Obsidian short id)
    //      `![[#Heading]]`        — section in current doc
    //      `![[#^short-id]]`      — block in current doc
    let (page_part, frag_part) = match target.split_once('#') {
        Some((p, f)) => (p.trim(), Some(f.trim())),
        None => (target.trim(), None),
    };
    let is_intra_doc = page_part.is_empty();
    let safe_page = if page_part.is_empty() {
        "this page".to_string()
    } else {
        html_escape(page_part)
    };
    // Section / short-id fragment.
    if let Some(frag) = frag_part {
        if let Some(short_id) = frag.strip_prefix('^') {
            // Block embed via short id. Intra-doc resolution
            // first; cross-doc through the vault.
            let resolved = if is_intra_doc {
                resolve_block_short_id(doc, short_id)
            } else {
                vault.and_then(|v| v.lookup_block_short(page_part, short_id))
            };
            return render_embed_card_short(
                "🔗",
                &safe_page,
                &html_escape(frag),
                resolved.as_deref(),
            );
        }
        // Section embed. Intra-doc walks this file's headings;
        // cross-doc asks the vault.
        let resolved = if is_intra_doc {
            resolve_heading_section(doc, frag)
        } else {
            vault.and_then(|v| v.lookup_section(page_part, frag))
        };
        return render_embed_card_section(
            "📄",
            &safe_page,
            &html_escape(frag),
            resolved.as_deref(),
        );
    }
    // 3. Whole-page embed. Cross-doc resolution via vault;
    //    intra-doc has no meaningful behavior (a page embedding
    //    itself), so falls back to placeholder.
    let resolved = if is_intra_doc {
        None
    } else {
        vault
            .and_then(|v| v.lookup_page(page_part))
            .map(|h| h.preview)
    };
    render_embed_card_page("📄", &safe_page, resolved.as_deref())
}

/// Walk the doc for a heading whose text matches `name` (case-
/// insensitive trim). Returns the heading body content (up to
/// the next same-or-higher heading) when found.
fn resolve_heading_section(doc: &str, name: &str) -> Option<String> {
    let needle = name.trim().to_lowercase();
    let lines: Vec<(usize, usize)> = line_ranges(doc);
    let mut start_idx: Option<(usize, usize)> = None; // (line_index, level)
    for (i, (lf, lt)) in lines.iter().enumerate() {
        let line = doc.slice(*lf..*lt);
        if let Some((level, marker_end)) = parse_heading(line) {
            let title = line.after(marker_end).trim();
            if title.to_lowercase() == needle {
                start_idx = Some((i, level));
                break;
            }
        }
    }
    let (start_i, start_level) = start_idx?;
    // Collect content lines until the next heading of level
    // <= start_level.
    let mut body = String::new();
    for (lf, lt) in lines.iter().skip(start_i.saturating_add(1)) {
        let line = doc.slice(*lf..*lt);
        if let Some((level, _)) = parse_heading(line)
            && level <= start_level
        {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    Some(body.trim().to_string())
}

/// Find an Obsidian short block-id `^id` in the doc and return
/// the containing paragraph's text.
fn resolve_block_short_id(doc: &str, short_id: &str) -> Option<String> {
    let needle = format!("^{short_id}");
    let pos = doc.find(&needle)?;
    let line_start = doc
        .before(pos)
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let line_end = doc
        .after(pos)
        .find('\n')
        .map_or(doc.len(), |n| pos.saturating_add(n));
    let line = doc.slice(line_start..line_end);
    // Strip the trailing `^id` so the embed shows the body text.
    Some(
        line.before(line.len().saturating_sub(needle.len()))
            .trim_end()
            .to_string(),
    )
}

/// Render the body of an embed card: the same inline subset table cells
/// get, applied line by line.
///
/// Card bodies used to be `html_escape`d, so an embedded block showed its
/// own source — `[Anthropic](https://anthropic.com)`, `[[An Introduction]]`
/// and `**bold**` as literal text — which is exactly what an embed is
/// supposed to spare the reader.
fn render_embed_preview(body: &str) -> String {
    body.lines()
        .map(render_table_cell)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_embed_card_page(icon: &str, page: &str, resolved: Option<&str>) -> String {
    let body = resolved.map_or_else(
        || r#"<span class="md-embed-placeholder">multi-file lookup pending</span>"#.to_string(),
        render_embed_preview,
    );
    format!(
        r#"<div class="md-embed-card md-embed-page"><div class="md-embed-head">{icon} <span class="md-embed-title">{page}</span></div><div class="md-embed-body">{body}</div></div>"#
    )
}

fn render_embed_card_section(
    icon: &str,
    page: &str,
    heading: &str,
    resolved: Option<&str>,
) -> String {
    let body = resolved.map_or_else(
        || r#"<span class="md-embed-placeholder">multi-file lookup pending</span>"#.to_string(),
        render_embed_preview,
    );
    format!(
        r#"<div class="md-embed-card md-embed-section"><div class="md-embed-head">{icon} <span class="md-embed-title">{page}</span> <span class="md-embed-sep">›</span> <span class="md-embed-frag">{heading}</span></div><div class="md-embed-body">{body}</div></div>"#
    )
}

fn render_embed_card_short(icon: &str, page: &str, short: &str, resolved: Option<&str>) -> String {
    let body = resolved.map_or_else(
        || r#"<span class="md-embed-placeholder">multi-file lookup pending</span>"#.to_string(),
        render_embed_preview,
    );
    format!(
        r#"<div class="md-embed-card md-embed-block"><div class="md-embed-head">{icon} <span class="md-embed-title">{page}</span> <span class="md-embed-sep">›</span> <span class="md-embed-frag">{short}</span></div><div class="md-embed-body">{body}</div></div>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Parse Obsidian's `|WxH`, `|W`, or `|HxW` opts on an embed.
/// Returns an inline `style` snippet to drop into the widget.
fn parse_size_opts(opts: &str) -> Option<String> {
    let (w, h) = match opts.split_once('x') {
        Some((w, h)) => (w.parse::<u32>().ok(), h.parse::<u32>().ok()),
        None => (opts.parse::<u32>().ok(), None),
    };
    let mut style = String::new();
    if let Some(w) = w {
        let _ = write!(style, " style=\"width:{w}px");
        if let Some(h) = h {
            let _ = write!(style, ";height:{h}px");
        }
        style.push('"');
    }
    if style.is_empty() {
        return None;
    }
    Some(style)
}

struct Span {
    /// Includes the opening + closing markers.
    outer: std::ops::Range<usize>,
    /// Just the inner content.
    body: std::ops::Range<usize>,
    /// CSS class to apply to the body. Static for now; later
    /// callers may want to inject their own class names.
    class: &'static str,
}

/// Did the primary selection touch any byte in `range`? A caret
/// *adjacent* to the span counts as touching — so cursors at
/// either edge keep the markers visible (matches Obsidian).
fn cursor_touches(primary: Range, range: std::ops::Range<usize>) -> bool {
    let (sel_from, sel_to) = (primary.from(), primary.to());
    sel_to >= range.start && sel_from <= range.end
}

/// Walk every inline span in `text` and push its decorations onto `out`.
///
/// The inline half of [`live_preview_with_lookups`], split out so that
/// function reads as the pipeline it is (blocks, then inline, then timing)
/// rather than a single 300-line body. The loop and its arms are unchanged.
fn decorate_inline_spans(
    text: &str,
    primary: Range,
    fenced_ranges: &[std::ops::Range<usize>],
    vault: Option<&dyn VaultLookup>,
    kbd: Option<&dyn KbdLookup>,
    strip_runs: &mut Option<std::collections::HashMap<usize, StripRunCtx>>,
    out: &mut Vec<DecoratedRange>,
) {
    let defs = link_definitions(text);
    for span in find_spans(text) {
        if in_fenced_code(fenced_ranges, span.outer.start) {
            continue;
        }
        if !span.body.is_empty() {
            // Embed: `![[file.png|opts]]` etc. Render an `<img>` /
            // `<video>` / `<audio>` / `<iframe>` widget when the
            // caret is off the span. While the caret is on the
            // span, the inner Mark + visible source bytes win so
            // the user can edit. Matches Obsidian / Quartz
            // `ofm.ts:233-265`.
            // Math — `$x$` inline or `$$x$$` display. Source
            // stays visible when the caret's on the span (so
            // the user can edit), otherwise replaced with a
            // rendered Typst SVG widget.
            if decorate_embed_like_span(&span, text, primary, vault, out) {
                continue;
            }
            // Inline footnotes `^[body]` — Obsidian renders them
            // as an auto-numbered superscript reference; the body
            // is hidden until the user mouses over or clicks. We
            // don't auto-number yet (no footnote registry), so
            // collapse to a generic `[*]` marker when caret is
            // away. Source stays visible while editing.
            if decorate_leaf_span(&span, text, primary, vault, kbd, out) {
                continue;
            }
            if !decorate_link_span(&span, text, primary, &defs, vault, strip_runs, out) {
                continue;
            }
        }
        if !cursor_touches(primary, span.outer.clone()) {
            // Hide the opening bracket(s) and, for an aliased wikilink,
            // the `target|` prefix up to the display text.
            let hide_left_to = if span.class == "md-wikilink" {
                match text.slice(span.body.clone()).find('|') {
                    Some(rel) => span.body.start.saturating_add(rel).saturating_add(1),
                    None => span.body.start,
                }
            } else {
                span.body.start
            };
            if hide_left_to > span.outer.start {
                out.push(Decoration::replace(span.outer.start..hide_left_to));
            }
            if span.outer.end > span.body.end {
                out.push(Decoration::replace(span.body.end..span.outer.end));
            }
        }
    }
}

/// `$inline$` and `$$display$$` math, rendered to a Typst SVG widget when the
/// caret is off the span.
///
/// The source stays visible while the caret is on it so the user can edit.
/// Returns `true` when the span was consumed. Split out of
/// [`decorate_embed_like_span`]; the body is unchanged.
fn decorate_math_span(
    span: &Span,
    text: &str,
    primary: Range,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if span.class == "md-math-inline" || span.class == "md-math-block" {
        if !cursor_touches(primary, span.outer.clone()) {
            let body = text.slice(span.body.clone());
            let kind = if span.class == "md-math-inline" {
                TypstKind::MathInline
            } else {
                TypstKind::MathBlock
            };
            if let Some(svg) = render_typst(kind, body) {
                out.push(Decoration::replace(span.outer.clone()));
                // `data-focus-pos` lets the JS click
                // handler route a click on the widget
                // back to a caret inside the source
                // span, so the user can edit math by
                // tapping the rendered output.
                let html = format!(
                    r#"<span class="{cls}" data-focus-pos="{pos}">{svg}</span>"#,
                    cls = if kind == TypstKind::MathInline {
                        "md-math-widget md-math-widget-inline"
                    } else {
                        "md-math-widget md-math-widget-block"
                    },
                    pos = span.body.start,
                );
                out.push(Decoration::widget(span.outer.start, html));
                return true;
            }
        }
        // Source visible (caret on, or compile failed).
        out.push(Decoration::mark(span.body.clone(), span.class));
        return true;
    }
    false
}

/// The span classes that render as a replacement widget when the caret is
/// off them: `$math$`, `%%comments%%`, `((block-refs))` and `![[embeds]]`.
///
/// Returns `true` when the span was consumed and the caller should move to the
/// next one — the `continue`s these arms used inside
/// [`decorate_inline_spans`]' loop. Split out to keep that loop readable; the
/// arms are unchanged.
fn decorate_embed_like_span(
    span: &Span,
    text: &str,
    primary: Range,
    vault: Option<&dyn VaultLookup>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if decorate_math_span(span, text, primary, out) {
        return true;
    }
    // Comments — `%%…%%` source is hidden entirely
    // (body + markers) when the caret is away. Only the
    // body stays visible while editing.
    if span.class == "md-comment" {
        if cursor_touches(primary, span.outer.clone()) {
            out.push(Decoration::mark(span.body.clone(), "md-comment"));
        } else {
            out.push(Decoration::replace(span.outer.clone()));
        }
        return true;
    }
    // `((uuid))` block reference — render as an atomic
    // chip showing the target block's first-line
    // content. UUID source is never visible (would
    // invite editing → broken refs). Always render the
    // widget; the chip itself is the only visible form.
    if span.class == "md-block-ref" {
        let uuid = text.slice(span.body.clone());
        // Resolve in this order: intra-doc block index →
        // vault lookup → unresolved. The vault hit
        // brings its own preview (target page may live
        // anywhere); intra-doc hits read from this
        // doc's text directly.
        let (preview, source_page, is_resolved) = block_anchor_for_uuid(uuid).map_or_else(
            || {
                if let Some(hit) = vault.and_then(|v| v.lookup_block(uuid)) {
                    (hit.preview, Some(hit.page), true)
                } else {
                    (
                        format!("unresolved {}", uuid.before(8.min(uuid.len()))),
                        None,
                        false,
                    )
                }
            },
            |anchor| (block_preview(text, anchor), None, true),
        );
        let cls = if is_resolved {
            "md-block-ref-chip"
        } else {
            "md-block-ref-chip md-block-ref-unresolved"
        };
        let page_hint = source_page.map(|p| format!(" › {p}")).unwrap_or_default();
        let html = format!(
            r#"<span class="{cls}" data-uuid="{uuid}" title="{full}">{glyph} {preview}{page}</span>"#,
            glyph = "🔗",
            full = escape_html(uuid),
            preview = escape_html(&preview),
            page = escape_html(&page_hint),
        );
        out.push(Decoration::replace(span.outer.clone()));
        out.push(Decoration::widget(span.outer.start, html));
        out.push(Decoration::atomic(span.outer.clone()));
        return true;
    }
    // `{{embed ((uuid))}}` — render the target block's
    // content inline in a bordered card. Same atomic +
    // hidden-source treatment as block refs.
    if span.class == "md-block-embed" {
        let uuid = text.slice(span.body.clone());
        let (content, source_page, is_resolved) = block_anchor_for_uuid(uuid).map_or_else(
            || {
                if let Some(hit) = vault.and_then(|v| v.lookup_block(uuid)) {
                    (hit.preview, Some(hit.page), true)
                } else {
                    (
                        format!("unresolved {}", uuid.before(8.min(uuid.len()))),
                        None,
                        false,
                    )
                }
            },
            |anchor| (block_preview(text, anchor), None, true),
        );
        let cls = if is_resolved {
            "md-block-embed-card"
        } else {
            "md-block-embed-card md-block-ref-unresolved"
        };
        let page_chip = source_page
            .map(|p| format!(
                r#"<div class="md-embed-head">📄 <span class="md-embed-title">{title}</span></div>"#,
                title = escape_html(&p),
            ))
            .unwrap_or_default();
        let html = format!(
            r#"<div class="{cls}" data-uuid="{uuid}">{page_chip}{content}</div>"#,
            uuid = escape_html(uuid),
            // An embed exists to show the block as it reads, not as it is
            // written; escaping it put `[a link](url)` and `**bold**` on
            // the page verbatim.
            content = render_embed_preview(&content),
        );
        out.push(Decoration::replace(span.outer.clone()));
        out.push(Decoration::widget(span.outer.start, html));
        out.push(Decoration::atomic(span.outer.clone()));
        return true;
    }
    false
}

/// A `[[wikilink]]` alone on its line, resolved against the vault into a
/// richer embed — a setlist card, a song strip, a scripture verse card, or an
/// unresolved-page chip.
///
/// Returns `true` when it emitted an embed and the caller should stop; an
/// inline wikilink (one with text around it) falls through to the ordinary
/// link decoration. Split out of [`decorate_link_span`]; the body is
/// unchanged.
fn decorate_standalone_wikilink(
    span: &Span,
    text: &str,
    primary: Range,
    vault: Option<&dyn VaultLookup>,
    href: Option<&str>,
    strip_runs: &mut Option<std::collections::HashMap<usize, StripRunCtx>>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if span.class == "md-wikilink"
        && !cursor_touches(primary, span.outer.clone())
        && let Some(h2) = href
    {
        let page_part = h2.split(['#', '|']).next().unwrap_or(h2).trim();
        let line_start = text
            .before(span.outer.start)
            .rfind('\n')
            .map_or(0, |i| i.saturating_add(1));
        let line_end = text
            .after(span.outer.end)
            .find('\n')
            .map_or(text.len(), |i| span.outer.end.saturating_add(i));
        let standalone = text.slice(line_start..span.outer.start).trim().is_empty()
            && text.slice(span.outer.end..line_end).trim().is_empty();
        if standalone {
            if let Some(setlist) = vault.and_then(|v| v.lookup_setlist(page_part)) {
                out.push(Decoration::replace(span.outer.clone()));
                out.push(Decoration::widget(
                    span.outer.start,
                    setlist_card_html(page_part, &setlist),
                ));
                out.push(Decoration::atomic(span.outer.clone()));
                return true;
            }
            if let Some(song) = vault.and_then(|v| v.lookup_song(page_part)) {
                let ctx = strip_runs
                    .get_or_insert_with(|| song_strip_runs(text, vault))
                    .get(&line_start)
                    .copied()
                    .unwrap_or_default();
                out.push(Decoration::replace(span.outer.clone()));
                out.push(Decoration::widget(
                    span.outer.start,
                    song_strip_html(page_part, &song, ctx),
                ));
                out.push(Decoration::atomic(span.outer.clone()));
                return true;
            }
            // VERSE CARD: a standalone scripture reference
            // embeds the verse text. Real pages win (checked
            // above via setlist/song; the general page check
            // below keeps ordinary links untouched).
            if vault.is_some_and(|v| v.lookup_page(page_part).is_none())
                && let Some(sc) = vault.and_then(|v| v.lookup_scripture(page_part))
            {
                out.push(Decoration::replace(span.outer.clone()));
                out.push(Decoration::widget(
                    span.outer.start,
                    scripture_card_html(page_part, &sc),
                ));
                out.push(Decoration::atomic(span.outer.clone()));
                return true;
            }
        }
    }
    false
}

/// Decorate the link family — `[text](url)`, `[[wikilink]]` and their vault
/// resolutions (song strips, setlist cards, scripture chips, contact cards).
///
/// Runs after the embed and leaf handlers have declined the span. Split out of
/// [`decorate_inline_spans`]' loop; the body is unchanged.
fn decorate_link_span(
    span: &Span,
    text: &str,
    primary: Range,
    defs: &std::collections::HashMap<String, (String, Option<String>)>,
    vault: Option<&dyn VaultLookup>,
    strip_runs: &mut Option<std::collections::HashMap<usize, StripRunCtx>>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    let mut link_title: Option<String> = None;
    let href = match span.class {
        "md-link" => {
            let dest =
                text.slice(span.body.end.saturating_add(2)..span.outer.end.saturating_sub(1));
            let (url, title) = split_destination(dest);
            link_title = title;
            Some(url)
        }
        // `[text][label]`, `[text][]` and bare `[label]`. The
        // label is whatever sits in the second bracket pair, or
        // the display text itself for the collapsed / shortcut
        // forms. An unresolved label is not a link at all.
        "md-reflink" => {
            let label = text
                .slice(span.body.end.saturating_add(2)..span.outer.end.saturating_sub(1))
                .trim();
            let key = if label.is_empty() {
                normalize_label(text.slice(span.body.clone()))
            } else {
                normalize_label(label)
            };
            match defs.get(&key) {
                Some((url, title)) => {
                    link_title.clone_from(title);
                    Some(url.clone())
                }
                None => None,
            }
        }
        "md-wikilink" => Some(text.slice(span.body.clone()).to_string()),
        _ => None,
    };
    // An unmatched reference label is literal text — no styling,
    // and the brackets stay visible.
    if span.class == "md-reflink" && href.is_none() {
        return false;
    }
    // For `[[target|display]]` only the display text is shown —
    // the `target|` prefix is hidden (like the brackets). The
    // display range is the body after the first `|`; without an
    // alias it's the whole body. `#Heading`-only links keep
    // their body verbatim (Obsidian shows "Page#Heading").
    let display = if span.class == "md-wikilink" {
        // `[[Page|Alias]]` displays only the alias; without one the whole body
        // is the display text.
        text.slice(span.body.clone()).find('|').map_or_else(
            || span.body.clone(),
            |rel| span.body.start.saturating_add(rel).saturating_add(1)..span.body.end,
        )
    } else {
        span.body.clone()
    };
    // SONG STRIP: a wikilink ALONE on its line whose target is a
    // `type: song` note renders as a playable song row (title ·
    // artist · stems · duration, with a play control the host
    // wires via `data-href="song-play:<target>"`). Caret on the
    // line falls through to the normal editable link.
    if decorate_standalone_wikilink(span, text, primary, vault, href.as_deref(), strip_runs, out) {
        return true;
    }
    if let Some(h) = href {
        // Wikilinks: consult the vault to decide
        // resolved (purple, default) vs unresolved
        // (red). Without a vault the link stays
        // unresolved — `#Heading` / `#^id` suffixes are
        // stripped before the page-name lookup so
        // `[[Page#Section]]` resolves when Page exists.
        let mut scripture_hit: Option<VaultScriptureHit> = None;
        let cls = if span.class == "md-wikilink" {
            let page_part = h.split(['#', '|']).next().unwrap_or(&h).trim();
            let resolved = vault.is_some_and(|v| v.lookup_page(page_part).is_some());
            if resolved {
                // Kind-specific styling: contact links render as
                // person chips wherever they appear (inline too).
                match vault.and_then(|v| v.lookup_note_kind(page_part)).as_deref() {
                    Some("contact") => "md-wikilink md-contact-chip",
                    _ => "md-wikilink",
                }
            } else if let Some(sc) = vault.and_then(|v| v.lookup_scripture(page_part)) {
                // Scripture reference: resolved chip, verse text
                // as hover tooltip once it lands.
                scripture_hit = Some(sc);
                "md-wikilink md-scripture-chip"
            } else {
                "md-wikilink md-wikilink-unresolved"
            }
        } else if span.class == "md-reflink" {
            "md-link"
        } else {
            span.class
        };
        let mut attrs = vec![("data-href".into(), h)];
        if let Some(t) = link_title {
            attrs.push(("title".into(), t));
        }
        if let Some(text) = scripture_hit.and_then(|sc| {
            sc.text
                .map(|t| format!("{} ({})\n{}", sc.display, sc.translation, t))
        }) {
            attrs.push(("title".into(), text));
        }
        out.push(Decoration::mark_with_attrs(display, cls, attrs));
        if !cursor_touches(primary, span.outer.clone()) {
            // Caret elsewhere: treat the link as one
            // atomic unit. Clicks anywhere inside snap
            // to the nearer edge so the user never lands
            // in the hidden marker bytes (`](url)` etc.).
            out.push(Decoration::atomic(span.outer.clone()));
        }
    } else {
        out.push(Decoration::mark(span.body.clone(), span.class));
    }
    true
}

/// Span classes whose decoration is self-contained: `^[inline footnotes]`,
/// `![[embeds]]` of files, and `` `code` ``.
///
/// Returns `true` when the span was consumed — see
/// [`decorate_embed_like_span`] for the same convention. Split out of
/// [`decorate_inline_spans`]; the arms are unchanged.
/// Split a `CommonMark` link destination — the bytes between `(` and `)` —
/// into the URL and its optional title.
///
/// `(/u "T")`, `(/u 'T')` and `(/u (T))` all carry a title; the title is
/// advisory and becomes a `title=` attribute. Without this the whole
/// payload went into `href`, so `[a](u "T")` linked to `u "T"`.
/// Angle-bracketed destinations (`(<u v>)`) are unwrapped, which is how
/// `CommonMark` spells a URL containing spaces.
fn split_destination(raw: &str) -> (String, Option<String>) {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('<')
        && let Some(url) = rest.split_once('>')
    {
        let title = title_of(url.1);
        return (url.0.to_owned(), title);
    }
    match raw.split_once(char::is_whitespace) {
        Some((url, rest)) => (url.to_owned(), title_of(rest)),
        None => (raw.to_owned(), None),
    }
}

/// The quoted title trailing a link destination, unwrapped.
fn title_of(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let inner = rest
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .or_else(|| rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')))?;
    Some(inner.to_owned())
}

/// Collect `[label]: url "title"` link-reference definitions.
///
/// `CommonMark` lets a link name its destination once and refer to it by
/// label anywhere — including *before* the definition — so this runs as a
/// pre-pass over the whole document. Labels are matched case-insensitively
/// with whitespace collapsed, as the spec requires.
fn link_definitions(text: &str) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut defs = std::collections::HashMap::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix('[') else {
            continue;
        };
        let Some((label, tail)) = rest.split_once("]:") else {
            continue;
        };
        if label.is_empty() || label.starts_with('^') {
            continue;
        }
        let (url, title) = split_destination(tail);
        if !url.is_empty() {
            defs.insert(normalize_label(label), (url, title));
        }
    }
    defs
}

/// A link label, folded for comparison: case-insensitive, with internal
/// whitespace runs collapsed to one space.
/// Is this line a `[label]: url "title"` link-reference definition?
fn is_link_definition(line: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix('[')
        .and_then(|r| r.split_once("]:"))
        .is_some_and(|(label, tail)| {
            !label.is_empty() && !label.starts_with('^') && !tail.trim().is_empty()
        })
}

fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn decorate_leaf_span(
    span: &Span,
    text: &str,
    primary: Range,
    vault: Option<&dyn VaultLookup>,
    kbd: Option<&dyn KbdLookup>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if span.class == "md-inline-footnote" {
        if cursor_touches(primary, span.outer.clone()) {
            out.push(Decoration::mark(span.body.clone(), "md-inline-footnote"));
        } else {
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(
                span.outer.start,
                format!(
                    r#"<sup class="md-inline-footnote-marker" data-focus-pos="{}">[*]</sup>"#,
                    span.body.start,
                ),
            ));
        }
        return true;
    }
    // `![alt](url)` — a CommonMark image. Rendered as a real
    // `<img>` when the caret is off it, the same treatment
    // `![[file.png]]` gets; Obsidian's `|width` / `|WxH` suffix
    // on the alt text is honoured here too.
    if span.class == "md-image" {
        let alt_raw = text.slice(span.body.clone());
        let dest = text.slice(span.body.end.saturating_add(2)..span.outer.end.saturating_sub(1));
        let (url, title) = split_destination(dest);
        if !cursor_touches(primary, span.outer.clone()) {
            let (alt, size) = match alt_raw.split_once('|') {
                Some((a, opts)) => (a, parse_size_opts(opts).unwrap_or_default()),
                None => (alt_raw, String::new()),
            };
            let title_attr = title
                .map(|t| format!(r#" title="{}""#, html_escape(&t)))
                .unwrap_or_default();
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(
                span.outer.start,
                format!(
                    r#"<img class="md-embed-image md-image-inline" src="{}" alt="{}"{title_attr}{size}>"#,
                    html_escape(&url),
                    html_escape(alt),
                ),
            ));
            out.push(Decoration::atomic(span.outer.clone()));
            return true;
        }
        // Caret on the span: leave the source editable, styled
        // like a link so it still reads as one thing.
        out.push(Decoration::mark(span.body.clone(), "md-link"));
        return true;
    }
    // `:rocket:` → 🚀. The source stays while the caret is on it.
    if span.class == "md-emoji" {
        if let Some(glyph) = crate::emoji::emoji_for(text.slice(span.body.clone()))
            && !cursor_touches(primary, span.outer.clone())
        {
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(
                span.outer.start,
                format!(r#"<span class="md-emoji">{glyph}</span>"#),
            ));
            out.push(Decoration::atomic(span.outer.clone()));
        }
        return true;
    }
    // Two trailing spaces — a hard line break. They occupy no
    // width, so hide them and leave the line wrap to do the work.
    if span.class == "md-hard-break" {
        if !cursor_touches(primary, span.outer.clone()) {
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(
                span.outer.start,
                r#"<span class="md-hard-break"></span>"#.to_owned(),
            ));
        }
        return true;
    }
    if span.class == "md-embed" {
        let raw = text.slice(span.body.clone());
        if !cursor_touches(primary, span.outer.clone()) {
            let html = embed_widget_html(raw, text, vault);
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(span.outer.start, html));
            return true;
        }
        // Fallback (or caret on): style the body like a
        // wikilink so it's still recognizable as a link.
        out.push(Decoration::mark(span.body.clone(), "md-wikilink"));
        if !cursor_touches(primary, span.outer.clone()) {
            if span.body.start > span.outer.start {
                out.push(Decoration::replace(span.outer.start..span.body.start));
            }
            if span.outer.end > span.body.end {
                out.push(Decoration::replace(span.body.end..span.outer.end));
            }
        }
        return true;
    }
    // `kbd:` inline shortcuts — code spans with a `kbd:` prefix
    // render as key caps. Two forms: literal `kbd:<C-s>` (keys
    // as written) and action-ref `kbd:@40044` (whatever keys the
    // host's [`KbdLookup`] says are currently bound). Caret on
    // the span shows the raw source for editing, like links.
    if span.class == "md-code" {
        let body = text.slice(span.body.clone());
        if let Some(spec) = body.strip_prefix("kbd:")
            && !cursor_touches(primary, span.outer.clone())
            && let Some(html) = kbd_widget_html(spec, kbd)
        {
            out.push(Decoration::replace(span.outer.clone()));
            out.push(Decoration::widget(span.outer.start, html));
            out.push(Decoration::atomic(span.outer.clone()));
            return true;
        }
        // Caret inside (or empty spec): fall through to the
        // normal inline-code styling with raw source.
    }
    false
}

/// The two line kinds handled before the main block dispatch: setext heading
/// underlines (and the lines they underline), and Logseq-style `^block-id`
/// trailers.
///
/// Returns `true` when the line is fully handled and the caller should move
/// on. Split out of [`scan_blocks`]' loop; the body is unchanged apart from
/// `continue` becoming `return true`.
fn emit_setext_and_block_id(
    text: &str,
    line_from: usize,
    line_to: usize,
    setext_underline: &std::collections::HashSet<usize>,
    setext_content_level: &std::collections::HashMap<usize, u8>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if setext_underline.contains(&line_from) {
        out.push(Decoration::line(line_from, "md-setext-underline"));
        return true;
    }
    // Setext content line — render with the heading class.
    // The underline isn't part of the heading text so we
    // don't bring it into the Replace; the active-state
    // tooling still treats the body as plain inline.
    if let Some(&level) = setext_content_level.get(&line_from) {
        let class = heading_class(usize::from(level));
        out.push(Decoration::line(line_from, class));
        return true;
    }
    let line = text.slice(line_from..line_to);

    // `id:: <uuid>` block-id property line (Logseq form).
    // Whole line is hidden from the rendered view; we never
    // want the user to accidentally edit a UUID (it'd break
    // every ref). Atomic so arrow-keys / Backspace treat
    // the hidden range as a single unit.
    if let Some(uuid_range) = parse_block_id_line(line, line_from) {
        // Replace the whole line content + its trailing
        // newline so neighbouring lines collapse together.
        let end = if line_to < text.len() {
            line_to.saturating_add(1)
        } else {
            line_to
        };
        out.push(Decoration::replace(line_from..end));
        out.push(Decoration::atomic(line_from..end));
        // Index this block id against the byte offset of the
        // line above (its block content). Stashed for
        // cross-line resolution + the `🔗` widget.
        register_block_id(text.slice(uuid_range), find_block_anchor(text, line_from));
        return true;
    }
    false
}

/// Decorate the remaining block-level line kinds: horizontal rules,
/// blockquotes and callouts, and list / task-list markers.
///
/// These share the property that they neither open nor close a fence, so
/// unlike the earlier sections they can run to completion on any line the
/// fence handling did not claim. Split out of [`scan_blocks`]' loop; the body
/// is unchanged apart from `continue` becoming `return`.
fn emit_block_line(
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    callout_stack: &mut Vec<&'static str>,
    out: &mut Vec<DecoratedRange>,
) {
    if is_hr(line) {
        // Two states:
        //   `md-hr-active` while the caret is on the line —
        //     just dim the `---` text, leave it on one row
        //     so the user can edit it.
        //   `md-hr` otherwise — hide the text bytes and let
        //     CSS render a single-row horizontal rule via
        //     the line's own border-top.
        if cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::line(line_from, "md-hr-active"));
        } else {
            out.push(Decoration::line(line_from, "md-hr"));
            out.push(Decoration::replace(line_from..line_to));
        }
        return;
    }

    // ── Task list ──────────────────────────────────────
    if let Some((prefix_end, checked)) = parse_task_marker(line) {
        let abs_prefix_end = line_from.saturating_add(prefix_end);
        out.push(Decoration::line(line_from, "md-task"));
        // The checkbox widget is always emitted so it stays
        // clickable regardless of caret position. The source
        // bytes are hidden via Replace only when the caret is
        // off the line — when the user is editing the line
        // we keep them visible so they can mutate the marker
        // directly and clicks/motions land on real text.
        let html = if checked {
            format!(
                r#"<span class="md-task-checkbox checked" data-task-pos="{line_from}">✓</span>"#
            )
        } else {
            format!(r#"<span class="md-task-checkbox" data-task-pos="{line_from}"></span>"#)
        };
        // Two states: source-visible when the caret is on
        // the line (so `- [ ]` is editable), checkbox-widget
        // otherwise. Rendering both at once gave you the
        // checkbox + the literal source overlapping.
        if cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::mark(
                line_from..abs_prefix_end,
                "md-task-marker-active",
            ));
        } else {
            out.push(Decoration::replace(line_from..abs_prefix_end));
            out.push(Decoration::widget(line_from, html));
        }
        return;
    }

    // ── Blockquote / Callout (with nesting) ────────────
    if let Some((depth, marker_end)) = parse_blockquote_depth(line) {
        let abs_marker_end = line_from.saturating_add(marker_end);
        // Lines with fewer `>` markers close any deeper
        // callouts. e.g. on `> > body` after `> [!a]\n> >
        // [!b]\n> >body` we'd be at depth 2 still — only a
        // depth-1 or 0 line pops back.
        while callout_stack.len() > depth {
            callout_stack.pop();
        }
        let after_marker = line.after(marker_end);
        // Callout header at the deepest open level.
        if parse_callout_header(after_marker).is_some() {
            emit_callout_header(line, line_from, line_to, primary, callout_stack, out);
            return;
        }
        // Plain blockquote or callout body — pick the kind
        // of the deepest currently-open callout (if any).
        let line_class = callout_stack
            .last()
            .copied()
            .map_or("md-blockquote", |kind| callout_class(kind, false));
        out.push(Decoration::line(line_from, line_class));
        if depth > 1 {
            out.push(Decoration::line(line_from, callout_depth_class(depth)));
        }
        if !cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::mark(
                line_from..abs_marker_end,
                "md-quote-marker",
            ));
        }
        // A list inside the quote is still a list. The body after
        // the `>` markers is ordinary block content, so hand it to
        // the same emitter the top level uses — with positions
        // shifted past the markers.
        let body = line.after(marker_end);
        if parse_list_marker(body).is_some() {
            emit_list_line(body, abs_marker_end, line_to, primary, out);
        }
        return;
    }
    // A line without `>` drains the whole nesting stack.
    callout_stack.clear();

    // ── List (unordered or ordered) ────────────────────
    if parse_list_marker(line).is_some() {
        emit_list_line(line, line_from, line_to, primary, out);
    }
}

/// Emit the decorations for a callout's header line — the type classes,
/// the Lucide icon, a default title when the source gives none, and the
/// fold control for a collapsible callout.
///
/// Split out of [`emit_block_line`], which outgrew its line budget once
/// callouts learned icons and folding.
fn emit_callout_header(
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    callout_stack: &mut Vec<&'static str>,
    out: &mut Vec<DecoratedRange>,
) {
    // Re-derived rather than passed: the caller has already matched
    // this line as a blockquote, and two fewer parameters is worth one
    // more walk over a handful of `>` bytes.
    let Some((depth, marker_end)) = parse_blockquote_depth(line) else {
        return;
    };
    let abs_marker_end = line_from.saturating_add(marker_end);
    let after_marker = line.after(marker_end);
    if let Some((kind, header_end_off, fold)) = parse_callout_header(after_marker) {
        // Extend the stack with synthetic ancestors if
        // the user opens a depth-3 callout without
        // having opened a depth-2 first. Real docs
        // almost never hit this; the fallback keeps
        // indexing safe.
        while callout_stack.len() < depth.saturating_sub(1) {
            callout_stack.push("note");
        }
        if callout_stack.len() == depth.saturating_sub(1) {
            callout_stack.push(kind);
        } else if let Some(slot) = callout_stack.get_mut(depth.saturating_sub(1)) {
            *slot = kind;
        }
        let line_class = callout_class(kind, true);
        out.push(Decoration::line(line_from, line_class));
        if let Some(folded) = fold {
            out.push(Decoration::line(line_from, "md-callout-collapsible"));
            if folded {
                out.push(Decoration::line(line_from, "md-callout-folded"));
            }
        }
        if depth > 1 {
            out.push(Decoration::line(line_from, callout_depth_class(depth)));
        }
        // Hide the `> > [!type] Title` markers when
        // caret is off the line — the icon widget and the
        // line class stand in for them. The marker span
        // covers all `>` chars + their spaces.
        let abs_header_end = abs_marker_end.saturating_add(header_end_off);
        if !cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::mark(
                line_from..abs_marker_end,
                "md-quote-marker",
            ));
            out.push(Decoration::replace(abs_marker_end..abs_header_end));
            // Obsidian's Lucide glyph, at the head of the
            // title. Anchored at the *start* of the replaced
            // syntax so it lands before the title text.
            if let Some(svg) = crate::callout_icon::callout_icon(kind) {
                out.push(Decoration::widget(abs_marker_end, svg.to_owned()));
            }
            // Collapsible callout: Obsidian puts a chevron at
            // the end of the title bar and folds the body. The
            // host toggles `md-callout-collapsed` on the
            // header; CSS hides the following body lines.
            if let Some(folded) = fold {
                out.push(Decoration::widget(
                        line_to,
                        format!(
                            r#"<button class="md-callout-fold" data-folded="{folded}" aria-label="Toggle callout">&rsaquo;</button>"#
                        ),
                    ));
            }
            // A bare `> [!tip]` has no title text. Obsidian
            // titles it with the type name; without this the
            // header line renders as an empty coloured bar.
            if after_marker.after(header_end_off).trim().is_empty() {
                out.push(Decoration::widget(
                    abs_header_end,
                    callout_default_title(kind).to_owned(),
                ));
            }
        }
    }
}

/// Emit the decorations for one list item — the `md-list-item` line class,
/// its indent depth, and the bullet / number widget.
///
/// Split out of [`emit_block_line`] so a list inside a blockquote or a
/// callout gets the same treatment; before, `> - a` kept a literal `-`
/// because only the top-level branch ever ran.
fn emit_list_line(
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    out: &mut Vec<DecoratedRange>,
) {
    if let Some(marker_end) = parse_list_marker(line) {
        let abs_marker_end = line_from.saturating_add(marker_end);
        out.push(Decoration::line(line_from, "md-list-item"));
        // Indent depth. A sub-list is indented by two spaces per
        // level (or one tab); the class drives padding in CSS so
        // the marker column steps in with it. Without this every
        // level rendered flush left and nesting was invisible.
        if let Some(cls) = list_depth_class(line) {
            out.push(Decoration::line(line_from, cls));
        }
        // Caret on the line: keep the raw `- ` / `1. ` source
        // visible (muted) so clicks land on real text and vim
        // motions don't fall through the Replace into the
        // line-tile end fallback. Off the line: hide source
        // and render the bullet/number widget.
        if cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::mark(
                line_from..abs_marker_end,
                "md-list-marker-active",
            ));
        } else {
            let kind_byte = line.trim_start().as_bytes().at(0);
            let widget_html = if kind_byte.is_ascii_digit() {
                let leading = line.len().saturating_sub(line.trim_start().len());
                let num_end = marker_end.saturating_sub(2).saturating_sub(leading);
                let num = line.trim_start().before(num_end);
                format!(r#"<span class="md-list-marker">{num}.&nbsp;</span>"#)
            } else {
                // A real bullet, not the source's `-`/`*`/`+`. Obsidian
                // steps the glyph by depth; so does this, which is the
                // only cue a reader gets that a sub-list is a sub-list
                // once the indent is small.
                let glyph = match list_depth_class(line) {
                    Some("md-list-depth-1") => "◦",
                    Some("md-list-depth-2") => "▪",
                    _ => "•",
                };
                format!(r#"<span class="md-list-marker">{glyph}&nbsp;</span>"#)
            };
            out.push(Decoration::replace(line_from..abs_marker_end));
            out.push(Decoration::widget(line_from, widget_html));
        }
    }
}

/// What the note's frontmatter declares it to be — the two document kinds
/// that give their first `#` heading a rendered header widget.
#[derive(Clone, Copy)]
struct DocKind<'a> {
    is_setlist: bool,
    is_event: bool,
    event_date: &'a str,
}

/// Decorate an ATX heading line (`# Title`), hiding the `#` markers when the
/// caret is elsewhere and applying the per-level class.
///
/// Also carries the setlist/event document special-cases, which turn the first
/// `#` of such a note into a header widget.
///
/// Returns `true` when the caller should skip to the next line. Split out of
/// [`scan_blocks`]' loop; the body is unchanged.
fn emit_heading(
    text: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    doc: DocKind<'_>,
    setlist_h1_done: &mut bool,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    let line = text.slice(line_from..line_to);
    let DocKind {
        is_setlist: doc_is_setlist,
        is_event: doc_is_event,
        event_date,
    } = doc;
    if let Some((level, marker_end)) = parse_heading(line) {
        let abs_marker_end = line_from.saturating_add(marker_end);
        if doc_is_setlist
            && level == 1
            && !(*setlist_h1_done)
            && !cursor_touches(primary, line_from..line_to)
        {
            (*setlist_h1_done) = true;
            let title = html_escape(line.after(marker_end).trim());
            out.push(Decoration::replace(line_from..line_to));
            out.push(Decoration::widget(
            line_from,
            format!(
                r#"<span class="md-setlist-header"><span class="md-setlist-art">🎵</span><span class="md-setlist-titles"><span class="md-setlist-title">{title}</span><span class="md-setlist-kind">Setlist</span></span><span class="md-setlist-playbtn" data-href="setlist-play:">▶</span><span class="md-setlist-openbtn" data-href="setlist-open:">Open</span></span>"#
            ),
        ));
            out.push(Decoration::atomic(line_from..line_to));
            return true;
        }
        if doc_is_event
            && level == 1
            && !(*setlist_h1_done)
            && !cursor_touches(primary, line_from..line_to)
        {
            (*setlist_h1_done) = true;
            let title = html_escape(line.after(marker_end).trim());
            let date = html_escape(event_date);
            let date_html = if date.is_empty() {
                String::new()
            } else {
                format!(r#"<span class="md-event-date">{date}</span>"#)
            };
            out.push(Decoration::replace(line_from..line_to));
            out.push(Decoration::widget(
            line_from,
            format!(
                r#"<span class="md-setlist-header md-event-header"><span class="md-setlist-art">📅</span><span class="md-setlist-titles"><span class="md-setlist-title">{title}</span><span class="md-setlist-kind">Event{date_sep}{date_html}</span></span></span>"#,
                date_sep = if date.is_empty() { "" } else { " · " },
            ),
        ));
            out.push(Decoration::atomic(line_from..line_to));
            return true;
        }
        if (doc_is_setlist || doc_is_event) && level == 1 && !(*setlist_h1_done) {
            // Caret on the title: keep it editable, but mark it
            // consumed so a SECOND h1 renders normally.
            (*setlist_h1_done) = true;
        }
        let class = heading_class(level);
        out.push(Decoration::line(line_from, class));
        // Marker stays visible (muted) any time the caret
        // is anywhere on the line — typing inside the
        // heading body shouldn't make the marker disappear.
        if cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::mark(
                line_from..abs_marker_end,
                "md-heading-marker",
            ));
        } else {
            out.push(Decoration::replace(line_from..abs_marker_end));
        }
        return true;
    }
    false
}

/// Render a fence whose language has a registered renderer — typst, mermaid,
/// or whatever the host registered through `fence_renderer` — as an SVG widget
/// in place of its source.
///
/// Returns `true` when the fence was rendered and the caller should move on.
/// Split out of [`open_fence_at_line`]; the body is unchanged.
fn emit_rendered_fence(
    text: &str,
    info: &str,
    line_from: usize,
    content_start: usize,
    // (marker char, run length) — the ``` or ~~~ that opened this fence.
    marker: (u8, usize),
    primary: Range,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    let (mc, mlen) = marker;
    // A language renders as a widget if something registered a plugin for
    // it. `kf*` and `tabs` build their widget markup inline below rather
    // than through a plugin, so they are named; nothing else is.
    if crate::plugin::get(info).is_some()
        || info.eq_ignore_ascii_case("kf")
        || info.eq_ignore_ascii_case("kf+")
        || info.eq_ignore_ascii_case("kf-")
        || info.eq_ignore_ascii_case("tabs")
    {
        let body_end = find_fence_close(text, content_start, mc, mlen);
        let body = text.slice(content_start..body_end);
        // Extend the replace range to cover the closing
        // ``` line so the rendered output stands alone
        // when caret is away.
        let bytes = text.as_bytes();
        let mut close_end = body_end;
        while close_end < bytes.len() && bytes.at(close_end) != b'\n' {
            close_end = close_end.saturating_add(1);
        }
        let fence_range = line_from..close_end;
        if !cursor_touches(primary, fence_range.clone()) && !body.trim().is_empty() {
            // The keyflow fence family — the AUTHOR picks per
            // snippet what shows, portable in the markdown:
            //   ```kf   → engraved chart only (default)
            //   ```kf+  → chart AND keyflow-highlighted source
            //   ```kf-  → highlighted source only, no chart
            // All three shed the code frame and carry a header
            // (the `kf` tag + a copy button, top-right). kf/kf+
            // also get a `</>` hover toggle to flip the source.
            let kf_kind = if info.eq_ignore_ascii_case("kf") {
                Some((false, true)) // (show_source, has_chart)
            } else if info.eq_ignore_ascii_case("kf+") {
                Some((true, true))
            } else if info.eq_ignore_ascii_case("kf-") {
                Some((true, false))
            } else {
                None
            };
            if let Some((show_source, has_chart)) = kf_kind {
                // Chart (kf/kf+) needs a successful engrave;
                // source-only (kf-) always renders.
                let svg = if has_chart {
                    render_keyflow(body)
                } else {
                    None
                };
                if has_chart && svg.is_none() {
                    // Bad chart source — leave the raw fence
                    // (falls through to the code path below).
                } else {
                    let show = if show_source {
                        " md-keyflow-show-source"
                    } else {
                        ""
                    };
                    let only = if has_chart {
                        ""
                    } else {
                        " md-keyflow-source-only"
                    };
                    // Header (fence tag + copy, plus the source
                    // toggle when a chart is present). It lives
                    // INSIDE the display block — the chart's
                    // top-right corner (or the source block's,
                    // when there's no chart) — anchored there,
                    // not overlaid on the widget. Copy grabs the
                    // raw body.
                    let toggle = if has_chart {
                        r#"<button type="button" class="md-keyflow-toggle" title="Show source">&lt;/&gt;</button>"#
                    } else {
                        ""
                    };
                    let header = format!(
                        r#"<div class="md-keyflow-header"><span class="md-keyflow-lang">{tag}</span><button class="md-code-copy" data-copy-from="{content_start}" data-copy-to="{body_end}" title="Copy">⧉</button>{toggle}</div>"#,
                        tag = escape_html(info),
                    );
                    let highlighted = keyflow::highlight_keyflow(body);
                    let html = svg.map_or_else(
                    || {
                        // Source only: header anchors to the
                        // source block's top-right.
                        format!(
                            r#"<div class="md-keyflow-widget{show}{only}" data-focus-pos="{content_start}"><div class="md-keyflow-sourcebox">{header}<pre class="md-keyflow-source"><code class="kf-code">{highlighted}</code></pre></div></div>"#,
                        )
                    },
                    |svg| {
                        // Chart present: header anchors to the
                        // chart's top-right; the source block (if
                        // shown) stacks above it.
                        format!(
                            r#"<div class="md-keyflow-widget{show}{only}" data-focus-pos="{content_start}"><div class="md-keyflow-sourcebox"><pre class="md-keyflow-source"><code class="kf-code">{highlighted}</code></pre></div><div class="md-keyflow-render">{header}{svg}</div></div>"#,
                        )
                    },
                );
                    out.push(Decoration::replace(fence_range.clone()));
                    out.push(Decoration::widget(fence_range.start, html));
                }
            } else if info.eq_ignore_ascii_case("tabs") {
                // ```tabs — split the body into `=== Tab`
                // panels and render one self-contained,
                // CSS-only tab widget (hidden radios +
                // `<label>` strip + `:checked ~ .panel`
                // rules — no JS, since the widget is a
                // static injected HTML string). The scope
                // hash folds in `content_start` so two
                // blocks never share a radio group.
                if let Some(inner) = render_tabs(body, content_start) {
                    let html = format!(
                        r#"<div class="md-tabs-widget" data-focus-pos="{content_start}">{inner}</div>"#,
                    );
                    out.push(Decoration::replace(fence_range.clone()));
                    out.push(Decoration::widget(fence_range.start, html));
                }
            } else if let Some(plugin) = crate::plugin::get(info) {
                // Every other renderable fence, by registration rather
                // than by name. This arm does not know that mermaid and
                // typst exist — it asks the registry what `info` means,
                // and a language nobody registered falls through to an
                // ordinary code block, which is the right answer for a
                // fence tag that is just a label.
                if let Some(svg) = plugin.render(body) {
                    let class = plugin.widget_class();
                    let html = format!(
                        r#"<div class="{class}" data-focus-pos="{content_start}">{svg}</div>"#,
                    );
                    out.push(Decoration::replace(fence_range.clone()));
                    out.push(Decoration::widget(fence_range.start, html));
                }
            }
        }
    }
    false
}

/// Handle a line that opens a fenced code block.
///
/// Emits the lang/copy header (or, for a fence language with a registered
/// renderer, the rendered widget), records the fence's content range, and
/// leaves `fence` set so following lines are treated as inside it.
///
/// Returns `true` when the caller should skip to the next line — the
/// `continue`s this block used inside [`scan_blocks`]' loop. Split out of that
/// loop, where it was the single largest section; the body is unchanged.
fn open_fence_at_line(
    text: &str,
    trimmed: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    fence: &mut Option<(usize, u8, usize, bool)>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    if let Some((mc, mlen, info_start)) = opens_fence(trimmed) {
        let info_peek = trimmed.after(info_start).trim();
        let is_kf_fence = info_peek.eq_ignore_ascii_case("kf")
            || info_peek.eq_ignore_ascii_case("kf+")
            || info_peek.eq_ignore_ascii_case("kf-");
        out.push(Decoration::line(line_from, "md-code-block"));
        if is_kf_fence {
            out.push(Decoration::line(line_from, "md-keyflow-bare"));
        }
        let caret_on_opener = cursor_touches(primary, line_from..line_to);
        // Caret on opener: leave the `\`\`\`lang` source
        // visible so it's editable. Off: hide source +
        // overlay the lang/copy widget.
        if !caret_on_opener {
            out.push(Decoration::replace(line_from..line_to));
        }
        let info = trimmed.after(info_start).trim();
        let content_start = if line_to < text.len() {
            line_to.saturating_add(1)
        } else {
            line_to
        };
        *fence = Some((line_from, mc, mlen, is_kf_fence));
        // The lang+copy header overlays the opener line for
        // ordinary code fences. Skip it for fences we
        // render as a widget (typst, mermaid) — the
        // rendered SVG already speaks for itself, and the
        // floating header would be a leftover when the user
        // moves the caret onto the fence to edit source.
        // Keyflow fences build their own header (tag + copy) inside
        // the widget, so skip the absolute-positioned code header
        // that ordinary fences overlay.
        // Asked of the registry, not a list of names: any language with
        // a plugin draws its own widget, so the floating header would be
        // a leftover sitting on top of it. `kf*` and `tabs` are the two
        // that build their widgets inline in `emit_rendered_fence`
        // rather than through a plugin, so they still say so here.
        let is_rendered_fence = crate::plugin::get(info).is_some()
            || info.eq_ignore_ascii_case("kf")
            || info.eq_ignore_ascii_case("kf+")
            || info.eq_ignore_ascii_case("kf-")
            || info.eq_ignore_ascii_case("tabs");
        if !caret_on_opener && !is_rendered_fence {
            let body_end_estimate = find_fence_close(text, content_start, mc, mlen);
            let header_html = format!(
                r#"<span class="md-code-header"><span class="md-code-lang">{lang}</span><button class="md-code-copy" data-copy-from="{from}" data-copy-to="{to}" title="Copy">⧉</button></span>"#,
                lang = if info.is_empty() { "plain" } else { info },
                from = content_start,
                to = body_end_estimate,
            );
            out.push(Decoration::widget(line_from, header_html));
        }
        if let Some(lang) = editor_syntax::Lang::from_fence_tag(info) {
            emit_fence_tokens(text, content_start, mc, mlen, lang, out);
        }
        // ```typst — render the body as a Typst document
        // and emit a single SVG widget on the closing fence
        // line so the rendered output sits below the source
        // code. Skip when the caret is anywhere inside the
        // fence range (so the user sees the raw source while
        // editing).
        if emit_rendered_fence(
            text,
            info,
            line_from,
            content_start,
            (mc, mlen),
            primary,
            out,
        ) {
            return true;
        }
        return true;
    }
    false
}

/// Recognise a GFM pipe table starting at `line_from` and emit it as one
/// rendered `<table>` widget covering every row.
///
/// Returns the byte range the table occupies, so the caller can register it as
/// a no-inline-parsing region and skip past it. Split out of [`scan_blocks`]'s
/// line loop; the body is unchanged.
fn emit_table_widget(
    text: &str,
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    eligible: bool,
    out: &mut Vec<DecoratedRange>,
) -> Option<std::ops::Range<usize>> {
    if line.trim().is_empty() || !eligible || !is_table_header(line) {
        return None;
    }
    let rows = try_parse_table(text, line_from, line_to)?;
    let table_end = rows.last().map_or(line_to, |r| r.1);
    // Header / separator + body cells.
    let cells = collect_table_cells(text, &rows);
    let align = rows
        .get(1)
        .map(|(f, t)| column_alignments(text.slice(*f..*t)))
        .unwrap_or_default();
    let html = render_table_html(&cells, &align);
    // When caret is anywhere in the table, leave the source visible
    // (Obsidian behavior — typing in tables works against the source).
    // Otherwise replace + widget.
    if !cursor_touches(primary, line_from..table_end) {
        out.push(Decoration::replace(line_from..table_end));
        out.push(Decoration::widget(line_from, html));
    }
    // The outer loop drives off `line_ranges`, so it can't skip lines from in
    // here. Returning the range marks it as a "fenced-like" zone: the inline
    // scanner won't reparse cell contents as bold/italic at odd positions, and
    // later iterations see those lines as inside-fence.
    Some(line_from..table_end)
}

/// Render the YAML frontmatter block as Obsidian's "Properties" panel.
///
/// The widget is emitted whenever the block parses — including with the caret
/// inside the YAML, so vim row-navigation has something to look at. The active
/// row is flagged in the HTML for CSS to highlight. Split out of
/// [`scan_blocks`]; the body is unchanged.
fn emit_frontmatter_widget(
    text: &str,
    primary: Range,
    out: &mut Vec<DecoratedRange>,
) -> Option<std::ops::Range<usize>> {
    let fm = parse_frontmatter(text)?;
    let caret = primary.head;
    let active_idx = fm
        .props
        .iter()
        .position(|p| caret >= p.range.start && caret < p.range.end);
    let html = render_properties_html(&fm.props, active_idx);
    out.push(Decoration::replace(fm.outer.clone()));
    out.push(Decoration::widget(fm.outer.start, html));
    Some(fm.outer)
}

/// Pre-pass for setext headings — the `===` / `---` underline form.
///
/// Returns the heading level keyed by the underlined line's start offset, and
/// the set of offsets that are themselves underlines. Split out of
/// [`scan_blocks`], which needs both maps before its main line loop starts.
fn scan_setext_underlines(
    text: &str,
) -> (
    std::collections::HashMap<usize, u8>,
    std::collections::HashSet<usize>,
) {
    let all_lines = line_ranges(text);
    let mut setext_content_level: std::collections::HashMap<usize, u8> =
        std::collections::HashMap::new();
    let mut setext_underline: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for win in all_lines.windows(2) {
        let [(lf, lt), (ulf, ult)] = *win else {
            continue;
        };
        let above = text.slice(lf..lt);
        let under = text.slice(ulf..ult).trim_end();
        if above.trim().is_empty() || under.is_empty() {
            continue;
        }
        let setext_ok_above = !above.starts_with('#')
            && !above.starts_with('>')
            && !above.starts_with('-')
            && !above.starts_with('*')
            && !above.starts_with('+')
            && !above.starts_with("```")
            && !above.starts_with("~~~")
            && !above.starts_with("---")
            && !above.starts_with("===");
        if !setext_ok_above {
            continue;
        }
        if under.chars().all(|c| c == '=') {
            setext_content_level.insert(lf, 1);
            setext_underline.insert(ulf);
        } else if under.chars().all(|c| c == '-') {
            setext_content_level.insert(lf, 2);
            setext_underline.insert(ulf);
        }
    }
    (setext_content_level, setext_underline)
}

/// Block-level scanner. Walks the doc line by line, recognizing
/// headings, blockquotes, lists, task lists, HRs and fenced code
/// blocks. Pushes the right `Decoration`s onto `out` and returns
/// the byte ranges occupied by fenced-code *content* so the
/// caller can skip inline parsing inside them.
/// The block-run state [`emit_plain_block_line`] needs: whether a new block
/// could start here, and whether we are inside a list or an indented code
/// block already.
struct BlockRun<'a> {
    prev_blank: bool,
    in_list: bool,
    in_indented_code: &'a mut bool,
}

/// The two line kinds handled before headings and fences: an indented
/// (four-space) code block, and a link-reference definition.
///
/// Returns `true` when the line is fully handled. Split out of
/// [`scan_blocks`], which was at its line budget.
fn emit_plain_block_line(
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    run: &mut BlockRun<'_>,
    out: &mut Vec<DecoratedRange>,
) -> bool {
    // An indented block runs on until the indent stops, so every line of
    // it is code — not just the first, which is all a head-of-block test
    // alone would catch. Inside a list the same indent is a continuation
    // line, never code.
    if (run.prev_blank || *run.in_indented_code)
        && !run.in_list
        && line.starts_with("    ")
        && !line.trim().is_empty()
    {
        out.push(Decoration::line(line_from, "md-code-block"));
        out.push(Decoration::line(line_from, "md-code-indented"));
        *run.in_indented_code = true;
        return true;
    }
    *run.in_indented_code = false;
    // A link-reference definition (`[label]: url "title"`) is
    // configuration, not content — it carries no output of its own, so it
    // is hidden the way frontmatter is.
    if is_link_definition(line) && !cursor_touches(primary, line_from..line_to) {
        out.push(Decoration::replace(line_from..line_to));
        out.push(Decoration::line(line_from, "md-link-def"));
        return true;
    }
    false
}

/// A line inside an open code fence: the code-block class, and — on the
/// closing fence — hiding the ```` ``` ```` and recording the fenced range
/// so the inline pass skips it.
///
/// Split out of [`scan_blocks`], which was at its line budget.
fn emit_fenced_line(
    line: &str,
    line_from: usize,
    line_to: usize,
    primary: Range,
    fence: &mut Option<(usize, u8, usize, bool)>,
    fenced_ranges: &mut Vec<std::ops::Range<usize>>,
    out: &mut Vec<DecoratedRange>,
) {
    let Some((_, mc, mlen, is_kf)) = *fence else {
        return;
    };
    out.push(Decoration::line(line_from, "md-code-block"));
    if is_kf {
        out.push(Decoration::line(line_from, "md-keyflow-bare"));
    }
    if is_closing_fence(line, mc, mlen) {
        // Caret on the closing fence: source stays visible so the user
        // can edit the ```` ``` ````. Off: hidden via Replace so the
        // line just shows the code-block background.
        if !cursor_touches(primary, line_from..line_to) {
            out.push(Decoration::replace(line_from..line_to));
        }
        if let Some((open_end, _, _, _)) = fence.take() {
            fenced_ranges.push(open_end..line_to);
        }
    }
}

fn scan_blocks(
    text: &str,
    primary: Range,
    out: &mut Vec<DecoratedRange>,
) -> Vec<std::ops::Range<usize>> {
    // `type: setlist` notes render their FIRST `# ` heading as the
    // setlist header widget (art tile · title · SETLIST · play) — the
    // note's own title IS the player header. Editable when the caret is
    // on the line; plain text in raw views (it is only a decoration).
    let doc_is_setlist = frontmatter_declares_setlist(text);
    // `type: event` notes: the first H1 renders as the EVENT header
    // (title · date · recurrence) — weekly events are distinguished by
    // their date, so it leads.
    let doc_is_event = frontmatter_scalar(text, "type").as_deref() == Some("event");
    let event_date = frontmatter_scalar(text, "date").unwrap_or_default();
    let mut setlist_h1_done = false;

    let mut fenced_ranges = Vec::new();
    // ── YAML frontmatter ───────────────────────────────────
    //
    // Obsidian renders `---\n…\n---` at the top of a note as a
    // "Properties" panel. Only the very start of the doc
    // counts — `---` mid-doc is a horizontal rule.
    //
    // When caret is outside the block, replace the source with
    // the rendered properties widget. When caret is inside,
    // leave the raw YAML visible (so the user can edit), and
    // still register the range so the inline scanner doesn't
    // interpret `key: value` colons as anything markdown.
    // Frontmatter widget is always shown once the block parses
    // — including when the caret is inside the YAML — so vim
    // row-navigation has something to look at. The active row
    // (the one containing the caret) is flagged in the HTML so
    // CSS can highlight it. Only the `---` delimiter lines stay
    // raw when caret is on them, in case the user wants to
    // collapse the block by deleting them.
    let frontmatter_range = parse_frontmatter(text).map(|fm| fm.outer);
    if let Some(fm_range) = emit_frontmatter_widget(text, primary, out) {
        fenced_ranges.push(fm_range);
    }

    // Fence-tracking state:
    //   - `Some((open_line_end, marker_char, marker_len))` while
    //     we're inside a fence; `open_line_end` is the byte AFTER
    //     the opening fence's `\n` (or end of doc if the fence is
    //     the last thing).
    // (open_pos, fence_char, fence_len, is_keyflow). The last flag marks
    // `kf`/`kf+` fences so every line sheds the grey code-block frame —
    // the engraved chart (and its own source block) stands full width,
    // not boxed like code. `kf-src` is NOT flagged: it stays a code block.
    let mut fence: Option<(usize, u8, usize, bool)> = None;
    // Callout stack: one entry per nesting depth. `> [!note]\n>
    // > [!warning]` pushes "note" then "warning"; a line with
    // fewer `>` markers pops back. Non-blockquote lines drain
    // the whole stack. Indexed by depth - 1 (depth 1 → [0]).
    let mut callout_stack: Vec<&'static str> = Vec::new();
    // Block context the indented-code rule needs: an indent means
    // "code" only at the start of a block, never inside a list.
    let mut prev_blank = true;
    let mut in_list = false;
    let mut in_indented_code = false;
    // Setext headings: a content line followed by `===` / `---`. `---`
    // is also a HR — setext wins when the line above is non-blank and
    // not itself a block-opening marker. See [`scan_setext_underlines`].
    let (setext_content_level, setext_underline) = scan_setext_underlines(text);

    for (line_from, line_to) in line_ranges(text) {
        // Setext underline — emit a styling class on the line so
        // it's visually paired with the heading above, then move
        // on. Without this skip, the `---` underline would be
        // matched as an HR by the block below.
        if emit_setext_and_block_id(
            text,
            line_from,
            line_to,
            &setext_underline,
            &setext_content_level,
            out,
        ) {
            continue;
        }
        let line = text.slice(line_from..line_to);

        // Lines inside the frontmatter range are handled above
        // (Replace + widget or delimiter marks) — skip block
        // parsing so the `---` opener isn't misread as a HR
        // and `key: value` lines aren't matched against block
        // patterns.
        if let Some(r) = &frontmatter_range
            && line_from < r.end
        {
            continue;
        }

        // ── Table (GFM pipe table) ─────────────────────────
        //
        // First-line check: `| header | … |` followed by a
        // separator row `|---|---|`. The separator's column
        // count must match the header. When we recognize a
        // table, jump the outer scan past its last row and emit
        // a single rendered `<table>` widget covering the whole
        // range. Quartz: `ofm.ts:123-126` via `remark-gfm`.
        if let Some(table_range) = emit_table_widget(
            text,
            line,
            line_from,
            line_to,
            primary,
            fence.is_none() && callout_stack.is_empty(),
            out,
        ) {
            fenced_ranges.push(table_range);
        }

        // ── Inside a fence ─────────────────────────────────
        if fence.is_some() {
            emit_fenced_line(
                line,
                line_from,
                line_to,
                primary,
                &mut fence,
                &mut fenced_ranges,
                out,
            );
            continue;
        }

        // ── Indented code block ────────────────────────────
        // Four spaces (or a tab) of indent is a code block, but
        // only where a *new block* can start: after a blank line
        // and not inside a list, where the same indent means a
        // continuation. Getting that wrong would turn every
        // wrapped list item into code.
        if emit_plain_block_line(
            line,
            line_from,
            line_to,
            primary,
            &mut BlockRun {
                prev_blank,
                in_list,
                in_indented_code: &mut in_indented_code,
            },
            out,
        ) {
            prev_blank = false;
            continue;
        }

        // ── Starting a fence ───────────────────────────────
        let trimmed = line.trim_start();
        if open_fence_at_line(text, trimmed, line_from, line_to, primary, &mut fence, out) {
            continue;
        }

        // ── Headings ───────────────────────────────────────
        if emit_heading(
            text,
            line_from,
            line_to,
            primary,
            DocKind {
                is_setlist: doc_is_setlist,
                is_event: doc_is_event,
                event_date: &event_date,
            },
            &mut setlist_h1_done,
            out,
        ) {
            continue;
        }

        // ── HR ─────────────────────────────────────────────
        emit_block_line(line, line_from, line_to, primary, &mut callout_stack, out);
        // Carried to the next iteration: an indented line is only
        // a code block at the head of one.
        in_list = parse_list_marker(line).is_some() || (in_list && line.starts_with("  "));
        prev_blank = line.trim().is_empty();
    }

    // EOF with unclosed fence — close it implicitly at doc end so
    // the inline parser still skips that range.
    if let Some((open_end, _, _, _)) = fence {
        fenced_ranges.push(open_end..text.len());
    }

    fenced_ranges
}

/// A frontmatter scalar (`key: value`) from the document's YAML fence.
fn frontmatter_scalar(text: &str, key: &str) -> Option<String> {
    let rest = text.strip_prefix("---")?;
    let (front, _) = rest.split_once("\n---")?;
    front.lines().find_map(|l| {
        l.trim_start()
            .strip_prefix(key)
            .and_then(|r| r.strip_prefix(':'))
            .map(|v| v.trim().trim_matches(['"', '\'']).trim().to_owned())
    })
}

/// Does the document's YAML frontmatter declare `type: setlist`?
fn frontmatter_declares_setlist(text: &str) -> bool {
    frontmatter_scalar(text, "type").as_deref() == Some("setlist")
}

const HEADING_CLASS: [&str; 6] = ["md-h1", "md-h2", "md-h3", "md-h4", "md-h5", "md-h6"];

/// CSS class for an ATX heading of `level` (1-6).
///
/// Levels come from the parser and are already range-checked, but the lookup
/// is written total anyway: a level outside 1..=6 is not a markdown heading,
/// and falling back to `md-h6` styles it as the deepest one rather than
/// panicking mid-render.
fn heading_class(level: usize) -> &'static str {
    HEADING_CLASS
        .get(level.saturating_sub(1))
        .copied()
        .unwrap_or("md-h6")
}

/// Match a callout header `[!type] [title]` after the `> ` of a
/// blockquote line. Returns the canonical callout kind (lower-
/// cased + alias-resolved) and the byte offset within `after`
/// where the `[!type]` syntax ends (excluding any title). The
/// type list mirrors Obsidian / Quartz: `ofm.ts:63-91`.
fn parse_callout_header(after: &str) -> Option<(&'static str, usize, Option<bool>)> {
    let b = after.as_bytes();
    if !b.starts_with(b"[!") {
        return None;
    }
    let close = b.iter().skip(2).position(|&c| c == b']')?;
    let raw = after.slice(2..close.saturating_add(2));
    // Strip the optional collapse suffix the user can add via
    // `+`/`-` on the closing bracket — `[!note]+` / `[!note]-`.
    let kind = canonical_callout_kind(raw)?;
    let mut end = close.saturating_add(3);
    // `[!note]+` starts expanded, `[!note]-` starts folded. Both
    // mark the callout as collapsible; a bare `[!note]` is not.
    let fold = match b.get(end) {
        Some(b'+') => Some(false),
        Some(b'-') => Some(true),
        _ => None,
    };
    if fold.is_some() {
        end = end.saturating_add(1);
    }
    // Consume the space that typically follows.
    if b.get(end) == Some(&b' ') {
        end = end.saturating_add(1);
    }
    Some((kind, end, fold))
}

/// The title Obsidian shows for a callout whose header carries no title
/// text of its own — the type name, capitalised.
const fn callout_default_title(kind: &str) -> &'static str {
    match kind.as_bytes() {
        b"note" => "Note",
        b"abstract" => "Abstract",
        b"info" => "Info",
        b"todo" => "Todo",
        b"tip" => "Tip",
        b"success" => "Success",
        b"question" => "Question",
        b"warning" => "Warning",
        b"failure" => "Failure",
        b"danger" => "Danger",
        b"bug" => "Bug",
        b"example" => "Example",
        _ => "Quote",
    }
}

fn canonical_callout_kind(raw: &str) -> Option<&'static str> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "note" => "note",
        "abstract" | "summary" | "tldr" => "abstract",
        "info" => "info",
        "todo" => "todo",
        "tip" | "hint" | "important" => "tip",
        "success" | "check" | "done" => "success",
        "question" | "help" | "faq" => "question",
        "warning" | "attention" | "caution" => "warning",
        "failure" | "missing" | "fail" => "failure",
        "danger" | "error" => "danger",
        "bug" => "bug",
        "example" => "example",
        "quote" | "cite" => "quote",
        _ => return None,
    })
}

/// Cheap "is this even a candidate?" check: a non-trivial pipe
/// table header must start (after optional spaces) with `|` and
/// contain at least one other `|`.
fn is_table_header(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('|') && t.after(1).contains('|')
}

/// Walk forward from the line that looks like a table header.
/// Returns the byte ranges of all table rows (header + sep +
/// body) when valid, or `None` if the structure doesn't hold.
fn try_parse_table(
    text: &str,
    header_from: usize,
    header_to: usize,
) -> Option<Vec<(usize, usize)>> {
    let bytes = text.as_bytes();
    // Find the separator line directly after the header.
    let sep_from = if header_to < bytes.len() && bytes.at(header_to) == b'\n' {
        header_to.saturating_add(1)
    } else {
        return None;
    };
    let mut sep_end = sep_from;
    while sep_end < bytes.len() && bytes.at(sep_end) != b'\n' {
        sep_end = sep_end.saturating_add(1);
    }
    let sep_line = text.slice(sep_from..sep_end);
    let header_cells = split_pipe_cells(text.slice(header_from..header_to));
    let sep_cells = split_pipe_cells(sep_line);
    if header_cells.len() != sep_cells.len() || header_cells.is_empty() {
        return None;
    }
    for cell in &sep_cells {
        let c = cell.trim();
        if c.is_empty() {
            return None;
        }
        if !c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')) {
            return None;
        }
    }
    let mut rows = vec![(header_from, header_to), (sep_from, sep_end)];
    let mut i = if sep_end < bytes.len() {
        sep_end.saturating_add(1)
    } else {
        sep_end
    };
    while i < bytes.len() {
        let row_from = i;
        let mut row_end = row_from;
        while row_end < bytes.len() && bytes.at(row_end) != b'\n' {
            row_end = row_end.saturating_add(1);
        }
        let row_line = text.slice(row_from..row_end);
        if row_line.trim().is_empty() || !row_line.trim_start().starts_with('|') {
            break;
        }
        rows.push((row_from, row_end));
        i = if row_end < bytes.len() {
            row_end.saturating_add(1)
        } else {
            row_end
        };
    }
    Some(rows)
}

fn split_pipe_cells(line: &str) -> Vec<&str> {
    let mut t = line.trim();
    if let Some(stripped) = t.strip_prefix('|') {
        t = stripped;
    }
    if let Some(stripped) = t.strip_suffix('|') {
        t = stripped;
    }
    t.split('|').map(str::trim).collect()
}

fn collect_table_cells(text: &str, rows: &[(usize, usize)]) -> Vec<Vec<String>> {
    rows.iter()
        .enumerate()
        .filter(|(idx, _)| *idx != 1) // drop the separator row
        .map(|(_, (f, t))| {
            split_pipe_cells(text.slice(*f..*t))
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect()
        })
        .collect()
}

/// Render inline markdown inside one table cell.
///
/// Tables are the one construct the live preview replaces with a widget
/// rather than decorating in place, so the inline pass that handles
/// `**bold**`, `` `code` `` and links everywhere else never reaches a
/// cell's contents. Without this a table renders its own source: the
/// keyflow guide's notation-systems table showed a literal
/// `**Letter name**` and `` `C`, `F#`, `Bb` ``.
///
/// Deliberately small — the subset that actually turns up in a table:
/// code spans, wikilinks, inline links, bold, italic. Code spans are
/// resolved first so their contents are never treated as markup, which
/// is what lets a cell document `` `**` `` without going bold.
///
/// Emits the same `md-*` classes the decoration path uses, so a table
/// cell and a paragraph style identically.
fn render_table_cell(cell: &str) -> String {
    let b = cell.as_bytes();
    let mut out = String::with_capacity(cell.len());
    let mut i = 0;

    while i < b.len() {
        // `code` — first, so nothing inside is interpreted.
        if b.at(i) == b'`'
            && let Some(end) = cell.after(i.saturating_add(1)).find('`')
        {
            let body = cell.slice(i.saturating_add(1)..i.saturating_add(1).saturating_add(end));
            out.push_str(r#"<code class="md-code">"#);
            out.push_str(&html_escape(body));
            out.push_str("</code>");
            i = i.saturating_add(end.saturating_add(2));
            continue;
        }

        // [[wikilink]] and [[target|label]]
        if cell.after(i).starts_with("[[")
            && let Some(end) = cell.after(i.saturating_add(2)).find("]]")
        {
            let body = cell.slice(i.saturating_add(2)..i.saturating_add(2).saturating_add(end));
            let (target, label) = body.split_once('|').unwrap_or((body, body));
            let _ = write!(
                out,
                r#"<span class="md-wikilink" data-href="{}">{}</span>"#,
                html_escape(target.trim()),
                html_escape(label.trim())
            );
            i = i.saturating_add(end.saturating_add(4));
            continue;
        }

        // [text](url)
        if b.at(i) == b'['
            && let Some(close) = cell.after(i.saturating_add(1)).find(']')
        {
            let rest = cell.after(i.saturating_add(1).saturating_add(close).saturating_add(1));
            if rest.starts_with('(')
                && let Some(paren) = rest.find(')')
            {
                let text =
                    cell.slice(i.saturating_add(1)..i.saturating_add(1).saturating_add(close));
                let href = rest.slice(1..paren);
                let _ = write!(
                    out,
                    r#"<a class="md-link" href="{}" data-href="{}">{}</a>"#,
                    html_escape(href),
                    html_escape(href),
                    html_escape(text)
                );
                // past "[", the link text, "](", the href and ")"
                i = i
                    .saturating_add(close)
                    .saturating_add(paren)
                    .saturating_add(3);
                continue;
            }
        }

        // **bold** before *italic*, or the opening `**` reads as an
        // empty emphasis followed by a stray `*`.
        if cell.after(i).starts_with("**")
            && let Some(end) = cell.after(i.saturating_add(2)).find("**")
        {
            out.push_str(r#"<span class="md-bold">"#);
            out.push_str(&html_escape(
                cell.slice(i.saturating_add(2)..i.saturating_add(2).saturating_add(end)),
            ));
            out.push_str("</span>");
            i = i.saturating_add(end.saturating_add(4));
            continue;
        }

        if b.at(i) == b'*' || b.at(i) == b'_' {
            let marker = char::from(b.at(i));
            if let Some(end) = cell.after(i.saturating_add(1)).find(marker) {
                let body = cell.slice(i.saturating_add(1)..i.saturating_add(1).saturating_add(end));
                if !body.is_empty() {
                    out.push_str(r#"<span class="md-italic">"#);
                    out.push_str(&html_escape(body));
                    out.push_str("</span>");
                    i = i.saturating_add(end.saturating_add(2));
                    continue;
                }
            }
        }

        // Ordinary text. Step by char, not byte, so multi-byte
        // characters are not split.
        // The loop guard keeps `i` inside `cell`; if a clamp ever put it at or
        // past the end there is no character left to emit, so stop.
        let Some(ch) = cell.after(i).chars().next() else {
            break;
        };
        out.push_str(&html_escape(&ch.to_string()));
        i = i.saturating_add(ch.len_utf8());
    }

    out
}

/// Per-column alignment from a GFM separator row: `:--` left, `--:` right,
/// `:-:` centre, `---` unset.
///
/// The separator was parsed for validity and then discarded, so
/// `|:-:|--:|` laid out exactly like `|---|---|`.
fn column_alignments(sep: &str) -> Vec<Option<&'static str>> {
    split_pipe_cells(sep)
        .into_iter()
        .map(|c| {
            let c = c.trim();
            match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Some("center"),
                (true, false) => Some("left"),
                (false, true) => Some("right"),
                (false, false) => None,
            }
        })
        .collect()
}

fn render_table_html(cells: &[Vec<String>], align: &[Option<&'static str>]) -> String {
    if cells.is_empty() {
        return String::new();
    }
    // `style="text-align:…"` for the column, or nothing.
    let style = |idx: usize| {
        align
            .get(idx)
            .copied()
            .flatten()
            .map(|a| format!(r#" style="text-align:{a}""#))
            .unwrap_or_default()
    };
    let mut s = String::from(r#"<table class="md-table">"#);
    let mut iter = cells.iter();
    if let Some(header) = iter.next() {
        s.push_str("<thead><tr>");
        for (i, c) in header.iter().enumerate() {
            let _ = write!(s, "<th{}>", style(i));
            s.push_str(&render_table_cell(c));
            s.push_str("</th>");
        }
        s.push_str("</tr></thead>");
    }
    s.push_str("<tbody>");
    for row in iter {
        s.push_str("<tr>");
        for (i, c) in row.iter().enumerate() {
            let _ = write!(s, "<td{}>", style(i));
            s.push_str(&render_table_cell(c));
            s.push_str("</td>");
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table>");
    s
}

/// Composite class for nested callouts so CSS can indent the
/// deeper levels. Depth `1` is the unnested base (no class
/// emitted by the caller); `2`+ each get a level-specific
/// class. Caps at 4 — anything deeper falls back to level 4
/// styling, which is fine for the rare deep-nesting edge case.
const fn callout_depth_class(depth: usize) -> &'static str {
    match depth {
        2 => "md-callout-nested-2",
        3 => "md-callout-nested-3",
        _ => "md-callout-nested-4",
    }
}

fn callout_class(kind: &str, is_header: bool) -> &'static str {
    // The decoration::Line variant takes a `String` so we have
    // to return a `&'static str` selected from a fixed table.
    // 13 kinds × 2 (header/body) — 26 entries; one match.
    match (kind, is_header) {
        ("note", true) => "md-callout md-callout-note md-callout-header",
        ("note", false) => "md-callout md-callout-note",
        ("abstract", true) => "md-callout md-callout-abstract md-callout-header",
        ("abstract", false) => "md-callout md-callout-abstract",
        ("info", true) => "md-callout md-callout-info md-callout-header",
        ("info", false) => "md-callout md-callout-info",
        ("todo", true) => "md-callout md-callout-todo md-callout-header",
        ("todo", false) => "md-callout md-callout-todo",
        ("tip", true) => "md-callout md-callout-tip md-callout-header",
        ("tip", false) => "md-callout md-callout-tip",
        ("success", true) => "md-callout md-callout-success md-callout-header",
        ("success", false) => "md-callout md-callout-success",
        ("question", true) => "md-callout md-callout-question md-callout-header",
        ("question", false) => "md-callout md-callout-question",
        ("warning", true) => "md-callout md-callout-warning md-callout-header",
        ("warning", false) => "md-callout md-callout-warning",
        ("failure", true) => "md-callout md-callout-failure md-callout-header",
        ("failure", false) => "md-callout md-callout-failure",
        ("danger", true) => "md-callout md-callout-danger md-callout-header",
        ("danger", false) => "md-callout md-callout-danger",
        ("bug", true) => "md-callout md-callout-bug md-callout-header",
        ("bug", false) => "md-callout md-callout-bug",
        ("example", true) => "md-callout md-callout-example md-callout-header",
        ("example", false) => "md-callout md-callout-example",
        ("quote", true) => "md-callout md-callout-quote md-callout-header",
        ("quote", false) => "md-callout md-callout-quote",
        _ => "md-blockquote",
    }
}

/// Iterate `(line_from, line_to)` byte ranges, exclusive of the
/// trailing `\n`. The last line (no trailing `\n`) is included.
fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push((start, i));
            start = i.saturating_add(1);
        }
    }
    if start <= bytes.len() {
        out.push((start, bytes.len()));
    }
    out
}

fn parse_heading(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let mut level = 0;
    while level < 6 && b.get(level) == Some(&b'#') {
        level = level.saturating_add(1);
    }
    if level == 0 {
        return None;
    }
    // Must be followed by a space (`# foo`) — otherwise it's a
    // tag (`#foo`) or just hashes.
    if b.get(level) != Some(&b' ') {
        return None;
    }
    Some((level, level.saturating_add(1)))
}

/// Count the depth of a nested blockquote (number of `>`
/// markers at the start of the line) and the byte offset where
/// the content body begins. Spaces between successive `>` are
/// tolerated — `> > [!note]` is the canonical Obsidian form.
/// Returns `None` when the line doesn't start with `>`.
fn parse_blockquote_depth(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    if b.first() != Some(&b'>') {
        return None;
    }
    let mut i = 0;
    let mut depth: usize = 0;
    while i < b.len() && b.at(i) == b'>' {
        depth = depth.saturating_add(1);
        i = i.saturating_add(1);
        // Eat the optional separator space — either between
        // successive `>` markers or before the body.
        if i < b.len() && b.at(i) == b' ' {
            i = i.saturating_add(1);
        }
    }
    Some((depth, i))
}

fn is_hr(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let c = t.as_bytes().at(0);
    if c != b'-' && c != b'*' && c != b'_' {
        return false;
    }
    t.bytes().all(|x| x == c)
}

fn parse_task_marker(line: &str) -> Option<(usize, bool)> {
    let b = line.as_bytes();
    // `- [ ]` / `- [x]` / `- [/]` / `- [>]` / `* [ ]` ... — must
    // be at least 5 bytes for the `- [X]` form.
    if b.len() < 5 {
        return None;
    }
    let bullet = b.at(0);
    if (bullet != b'-' && bullet != b'*' && bullet != b'+') || b.at(1) != b' ' || b.at(2) != b'[' {
        return None;
    }
    let inner = b.at(3);
    if b.at(4) != b']' {
        return None;
    }
    // Accept any single printable ASCII as a status — Obsidian's
    // Tasks plugin convention uses `x`, ` `, `/` (in-progress),
    // `>` (forwarded), `<` (scheduled), `-` (cancelled), `?`
    // (question), `!` (important). Anything else still parses
    // (treated as unchecked) so we don't break funky user
    // schemes; styling can specialize via data attrs later.
    let is_valid_status = inner == b' '
        || inner.is_ascii_alphanumeric()
        || matches!(inner, b'/' | b'>' | b'<' | b'-' | b'?' | b'!' | b'.' | b'*');
    if !is_valid_status {
        return None;
    }
    let checked = matches!(inner, b'x' | b'X');
    let end = if b.get(5) == Some(&b' ') { 6 } else { 5 };
    Some((end, checked))
}

/// The indent-depth class for a list line, or `None` at the top level.
///
/// Two spaces (or one tab) per level, which is what every editor that
/// writes markdown emits, capped at four so a runaway indent can't invent
/// classes the stylesheet doesn't have.
fn list_depth_class(line: &str) -> Option<&'static str> {
    let cols = line
        .bytes()
        .take_while(|&c| c == b' ' || c == b'\t')
        .map(|c| if c == b'\t' { 4 } else { 1 })
        .sum::<usize>();
    match cols / 2 {
        0 => None,
        1 => Some("md-list-depth-1"),
        2 => Some("md-list-depth-2"),
        3 => Some("md-list-depth-3"),
        _ => Some("md-list-depth-4"),
    }
}

fn parse_list_marker(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let leading = b.iter().take_while(|&&c| c == b' ').count();
    let after = b.after(leading);
    // Unordered: `- ` / `* ` / `+ `.
    if let Some(&c) = after.first()
        && (c == b'-' || c == b'*' || c == b'+')
        && after.get(1) == Some(&b' ')
    {
        return Some(leading.saturating_add(2));
    }
    // Ordered: `1. ` / `12. `.
    let digit_count = after.iter().take_while(|&&x| x.is_ascii_digit()).count();
    if digit_count > 0
        && after.get(digit_count) == Some(&b'.')
        && after.get(digit_count.saturating_add(1)) == Some(&b' ')
    {
        return Some(leading.saturating_add(digit_count).saturating_add(2));
    }
    None
}

/// Does this trimmed line *open* a fence? Returns
/// `(marker_char, marker_len, info_string_start_offset)`.
fn opens_fence(trimmed: &str) -> Option<(u8, usize, usize)> {
    let b = trimmed.as_bytes();
    if b.len() < 3 {
        return None;
    }
    let c = b.at(0);
    if c != b'`' && c != b'~' {
        return None;
    }
    let run = b.iter().take_while(|&&x| x == c).count();
    if run < 3 {
        return None;
    }
    Some((c, run, run))
}

/// Find the byte offset of the closing fence's `\n` (or doc end)
/// when walking forward from `content_start`. Used by both the
/// syntax-highlighter and the lang/copy header widget.
fn find_fence_close(text: &str, content_start: usize, marker_char: u8, marker_len: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = content_start;
    while i < bytes.len() {
        let line_from = i;
        while i < bytes.len() && bytes.at(i) != b'\n' {
            i = i.saturating_add(1);
        }
        let line = text.slice(line_from..i);
        if is_closing_fence(line, marker_char, marker_len) {
            return line_from;
        }
        if i < bytes.len() {
            i = i.saturating_add(1);
        }
    }
    text.len()
}

/// Slice the fenced-body bytes between `content_start` and the
/// matching closing fence (or doc end), run the syntax highlighter,
/// and emit one `Mark` decoration per token.
///
/// Memoized by `(lang, body)` — typing OUTSIDE a fence shouldn't
/// re-parse the fence with tree-sitter on every keystroke. The
/// cache is bounded so it doesn't grow forever; entries evict
/// LRU-style when the bound is hit.
fn emit_fence_tokens(
    text: &str,
    content_start: usize,
    marker_char: u8,
    marker_len: usize,
    lang: editor_syntax::Lang,
    out: &mut Vec<DecoratedRange>,
) {
    let t_find = now_ms_native();
    let end = find_fence_close(text, content_start, marker_char, marker_len);
    let find_ms = now_ms_native() - t_find;
    if end <= content_start {
        return;
    }
    let body = text.slice(content_start..end);
    let t_tok = now_ms_native();
    let cached = with_fence_cache(|cache| cache.get(lang, body));
    let was_cached = cached.is_some();
    let tokens = cached.unwrap_or_else(|| {
        let toks = editor_syntax::highlight(lang, body);
        with_fence_cache(|cache| cache.put(lang, body.to_string(), toks.clone()));
        toks
    });
    let tok_ms = now_ms_native() - t_tok;
    tracing::trace!(
        body_len = body.len(),
        token_count = tokens.len(),
        cache_hit = was_cached,
        find_ms = %format!("{:.2}", find_ms),
        tokenize_ms = %format!("{:.2}", tok_ms),
        "md.fence_tokens"
    );
    for tok in tokens {
        let abs_from = content_start.saturating_add(tok.start);
        let abs_to = content_start.saturating_add(tok.end);
        let class = format!("md-tok-{}", tok.tag);
        out.push(Decoration::mark(abs_from..abs_to, class));
    }
}

/// Bounded LRU-ish cache of `(lang, body) -> tokens`. Sized for
/// the common case of a handful of fences in a doc. Tree-sitter
/// parses are the per-keystroke cost we want to avoid.
struct FenceCache {
    entries: Vec<(editor_syntax::Lang, String, Vec<editor_syntax::Token>)>,
    cap: usize,
}

impl FenceCache {
    fn new(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
            cap,
        }
    }
    fn get(&mut self, lang: editor_syntax::Lang, body: &str) -> Option<Vec<editor_syntax::Token>> {
        let idx = self
            .entries
            .iter()
            .position(|(l, b, _)| *l == lang && b == body)?;
        // Move to back so this entry is "freshest".
        let hit = self.entries.remove(idx);
        let toks = hit.2.clone();
        self.entries.push(hit);
        Some(toks)
    }
    fn put(&mut self, lang: editor_syntax::Lang, body: String, toks: Vec<editor_syntax::Token>) {
        if self.entries.len() >= self.cap {
            self.entries.remove(0);
        }
        self.entries.push((lang, body, toks));
    }
}

fn with_fence_cache<R>(f: impl FnOnce(&mut FenceCache) -> R) -> R {
    thread_local! {
        static CACHE: std::cell::RefCell<FenceCache> =
            std::cell::RefCell::new(FenceCache::new(16));
    }
    CACHE.with(|c| f(&mut c.borrow_mut()))
}

fn is_closing_fence(line: &str, marker_char: u8, marker_len: usize) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < marker_len {
        return false;
    }
    let b = trimmed.as_bytes();
    let run = b.iter().take_while(|&&x| x == marker_char).count();
    run >= marker_len && b.after(run).iter().all(|&x| x == b' ')
}

/// Single-pass scanner. Walks bytes, recognizing the inline
/// markdown flavors supported by live-preview. Doesn't cross
/// newlines (a stray `*` on one line shouldn't pair with one on
/// the next). Top-level only — no nesting yet (e.g. `**a~~b~~c**`
/// gets the bold but not the strike inside).
/// Scan the paired-marker inline delimiters at `i`: `***bold-italic***`,
/// `**bold**`, `~~strike~~`, `==highlight==`, `%%comment%%` and `$$math$$`.
///
/// Ordered longest-marker-first so a triple `***` is not consumed as nested
/// bold plus italic. Pushes any span it recognises onto `out` and returns the
/// offset to resume from. Split out of [`find_spans`]; the arms are unchanged
/// apart from `i = x; continue;` becoming `return Some(x);`.
fn scan_paired_marker_span(b: &[u8], i: usize, out: &mut Vec<Span>) -> Option<usize> {
    // marker isn't consumed as nested bold + italic.
    if i.saturating_add(6) <= b.len()
        && b.slice(i..i.saturating_add(3)) == b"***"
        && let Some(end) = find_close(b, i.saturating_add(3), b"***")
    {
        out.push(Span {
            outer: i..end.saturating_add(3),
            body: i.saturating_add(3)..end,
            class: "md-bold-italic",
        });
        return Some(end.saturating_add(3));
    }
    // **bold**
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"**"
        && let Some(end) = find_close(b, i.saturating_add(2), b"**")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-bold",
        });
        return Some(end.saturating_add(2));
    }
    // ~~strikethrough~~
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"~~"
        && let Some(end) = find_close(b, i.saturating_add(2), b"~~")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-strike",
        });
        return Some(end.saturating_add(2));
    }
    // ==highlight==
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"=="
        && let Some(end) = find_close(b, i.saturating_add(2), b"==")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-highlight",
        });
        return Some(end.saturating_add(2));
    }
    // %% obsidian comment %% — body hidden when caret away,
    // styled subtly when revealed. Quartz: `ofm.ts:132`.
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"%%"
        && let Some(end) = find_close(b, i.saturating_add(2), b"%%")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-comment",
        });
        return Some(end.saturating_add(2));
    }
    // $$display math$$ — must precede the `$inline$` arm so
    // the doubled marker doesn't get consumed as two empty
    // inline maths. Body is the Typst math source.
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"$$"
        && let Some(end) = find_close(b, i.saturating_add(2), b"$$")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-math-block",
        });
        return Some(end.saturating_add(2));
    }
    // $inline math$ — single-dollar pair. Skip if the body
    // would be empty (`$$` already handled above) or starts
    // with whitespace (avoids matching prose like
    None
}

/// Scan the block-reference forms at `i`: Logseq's `{{embed ((uuid))}}`, a bare
/// `((uuid))` reference, and `<autolinks>`.
///
/// Pushes any span it recognises onto `out` and returns the offset to resume
/// from. Split out of [`scan_link_like_span`]; the arms are unchanged.
/// `[text][label]` / `[label][]` / `[label]` — reference links.
///
/// The destination lives in a `[label]: url` definition line elsewhere in
/// the document; [`link_definitions`] collects those in a pre-pass and
/// [`decorate_link_span`] resolves them. A label with no matching
/// definition is left as plain text, which is what `CommonMark` requires.
/// `:smile:` — an emoji shortcode.
///
/// The name must be one unbroken run of shortcode characters *and* has to
/// resolve in [`crate::emoji`]; both conditions together are what stop a
/// bare `10:30 - 11:00` from being eaten as a shortcode.
fn scan_emoji_span(text: &str, b: &[u8], i: usize, out: &mut Vec<Span>) -> Option<usize> {
    if b.at(i) != b':' || (i > 0 && b.at(i.saturating_sub(1)).is_ascii_alphanumeric()) {
        return None;
    }
    let mut j = i.saturating_add(1);
    while j < b.len() && (b.at(j).is_ascii_alphanumeric() || matches!(b.at(j), b'_' | b'+' | b'-'))
    {
        j = j.saturating_add(1);
    }
    if j > i.saturating_add(1)
        && b.get(j) == Some(&b':')
        && crate::emoji::emoji_for(text.slice(i.saturating_add(1)..j)).is_some()
    {
        out.push(Span {
            outer: i..j.saturating_add(1),
            body: i.saturating_add(1)..j,
            class: "md-emoji",
        });
        return Some(j.saturating_add(1));
    }
    None
}

fn scan_reference_link_span(b: &[u8], i: usize, out: &mut Vec<Span>) -> Option<usize> {
    if b.at(i) != b'[' {
        return None;
    }
    let close_text = find_close(b, i.saturating_add(1), b"]")?;
    let after = close_text.saturating_add(1);
    // Collapsed / full form: `[text][]` or `[text][label]`.
    if b.get(after) == Some(&b'[')
        && let Some(close_label) = find_close(b, after.saturating_add(1), b"]")
    {
        out.push(Span {
            outer: i..close_label.saturating_add(1),
            body: i.saturating_add(1)..close_text,
            class: "md-reflink",
        });
        return Some(close_label.saturating_add(1));
    }
    // Shortcut form: bare `[label]`, but only when it is not the head of
    // a definition line (`[label]: url`), which the block pass hides
    // wholesale.
    if b.get(after) != Some(&b':') && close_text > i.saturating_add(1) {
        out.push(Span {
            outer: i..after,
            body: i.saturating_add(1)..close_text,
            class: "md-reflink",
        });
        return Some(after);
    }
    None
}

fn scan_block_ref_span(text: &str, b: &[u8], i: usize, out: &mut Vec<Span>) -> Option<usize> {
    if i.saturating_add(13) <= b.len() && b.slice(i..i.saturating_add(9)) == b"{{embed (" {
        // Look for `))}}` closing.
        let payload_start = i.saturating_add(9); // after `{{embed (`
        if b.get(payload_start) == Some(&b'(') {
            let uuid_start = payload_start.saturating_add(1);
            if let Some(uuid_len) = peek_uuid(b.after(uuid_start)) {
                let uuid_end = uuid_start.saturating_add(uuid_len);
                if b.get(uuid_end..uuid_end.saturating_add(4)) == Some(b"))}}") {
                    out.push(Span {
                        outer: i..uuid_end.saturating_add(4),
                        body: uuid_start..uuid_end,
                        class: "md-block-embed",
                    });
                    return Some(uuid_end.saturating_add(4));
                }
            }
        }
    }
    // `((uuid))` — Logseq block reference. The body is the
    // 36-char UUID itself; outer adds the `(( ))` markers.
    if i.saturating_add(40) <= b.len() && b.slice(i..i.saturating_add(2)) == b"((" {
        let uuid_start = i.saturating_add(2);
        if let Some(uuid_len) = peek_uuid(b.after(uuid_start)) {
            let uuid_end = uuid_start.saturating_add(uuid_len);
            if b.get(uuid_end..uuid_end.saturating_add(2)) == Some(b"))") {
                out.push(Span {
                    outer: i..uuid_end.saturating_add(2),
                    body: uuid_start..uuid_end,
                    class: "md-block-ref",
                });
                return Some(uuid_end.saturating_add(2));
            }
        }
    }
    // <https://…> autolink (also matches mailto-shaped
    // `<foo@bar.baz>`). The body becomes the URL itself; the
    // angle brackets are styling-only.
    if b.at(i) == b'<'
        && let Some(end) = find_close(b, i.saturating_add(1), b">")
    {
        let body = text.slice(i.saturating_add(1)..end);
        let is_url = body.starts_with("http://")
            || body.starts_with("https://")
            || body.starts_with("mailto:")
            || (body.contains('@') && !body.contains(' ') && body.contains('.'));
        if is_url {
            out.push(Span {
                outer: i..end.saturating_add(1),
                body: i.saturating_add(1)..end,
                class: "md-autolink",
            });
            return Some(end.saturating_add(1));
        }
    }
    // `![alt](url)` — a CommonMark image. Checked before the link
    // arm and anchored on the `!`, so the bang is inside `outer`
    // and gets hidden with the rest of the syntax. Without this
    // the bang survived as literal text and the remainder
    // rendered as an ordinary link.
    if b.at(i) == b'!'
        && b.get(i.saturating_add(1)) == Some(&b'[')
        && let Some(close_text) = find_close(b, i.saturating_add(2), b"]")
        && b.get(close_text.saturating_add(1)) == Some(&b'(')
        && let Some(close_paren) = find_close(b, close_text.saturating_add(2), b")")
    {
        out.push(Span {
            outer: i..close_paren.saturating_add(1),
            body: i.saturating_add(2)..close_text,
            class: "md-image",
        });
        return Some(close_paren.saturating_add(1));
    }
    // [text](url) — find `]` then verify `(...)` follows.
    if b.at(i) == b'['
        && let Some(close_text) = find_close(b, i.saturating_add(1), b"]")
        && b.get(close_text.saturating_add(1)) == Some(&b'(')
        && let Some(close_paren) = find_close(b, close_text.saturating_add(2), b")")
    {
        out.push(Span {
            outer: i..close_paren.saturating_add(1),
            body: i.saturating_add(1)..close_text,
            class: "md-link",
        });
        return Some(close_paren.saturating_add(1));
    }
    if let Some(next) = scan_reference_link_span(b, i, out) {
        return Some(next);
    }
    if let Some(next) = scan_emoji_span(text, b, i, out) {
        return Some(next);
    }
    // #tag  — `#` at doc start or after non-word char,
    // followed by tag chars (alnum / `-` / `_` / `/`). The
    // body equals the outer (no markers to hide) so the
    // mark just colors the whole `#tag` string.
    if b.at(i) == b'#' && tag_boundary_before(b, i) {
        let start = i;
        let mut j = i.saturating_add(1);
        while j < b.len() && is_tag_char(b.at(j)) {
            j = j.saturating_add(1);
        }
        // Need at least one tag char after `#`.
        if j > i.saturating_add(1) {
            out.push(Span {
                outer: start..j,
                body: start..j,
                class: "md-tag",
            });
            return Some(j);
        }
    }
    None
}

fn find_spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b.at(i) == b'\n' {
            i = i.saturating_add(1);
            continue;
        }
        // `\*` — a backslash escape. CommonMark lets any ASCII
        // punctuation be escaped, and the escaped character is
        // literal: neither it nor the backslash may start a
        // marker. Claiming the pair here is what makes that true,
        // because every scan below starts at `i`.
        if b.at(i) == b'\\'
            && b.get(i.saturating_add(1))
                .is_some_and(u8::is_ascii_punctuation)
        {
            out.push(Span {
                outer: i..i.saturating_add(2),
                body: i.saturating_add(1)..i.saturating_add(2),
                class: "md-escape",
            });
            i = i.saturating_add(2);
            continue;
        }
        // Two or more spaces before a newline is a hard line
        // break. They render as nothing and mean `<br>`; left
        // alone they were emitted as literal trailing spaces.
        if b.at(i) == b' '
            && b.get(i.saturating_add(1)) == Some(&b' ')
            && let Some(nl) = b.after(i).iter().position(|&c| c == b'\n')
            && b.slice(i..i.saturating_add(nl)).iter().all(|&c| c == b' ')
        {
            out.push(Span {
                outer: i..i.saturating_add(nl),
                body: i..i.saturating_add(nl),
                class: "md-hard-break",
            });
            i = i.saturating_add(nl);
            continue;
        }
        // ***bold-italic***  — must precede `**` so the triple
        if let Some(next) = scan_paired_marker_span(b, i, &mut out) {
            i = next;
            continue;
        }
        // "Cost is $5 not $10").
        if b.at(i) == b'$'
            && i.saturating_add(2) < b.len()
            && b.at(i.saturating_add(1)) != b' '
            && b.at(i.saturating_add(1)) != b'$'
            && let Some(end) = find_close(b, i.saturating_add(1), b"$")
            && end > i.saturating_add(1)
            && b.at(end.saturating_sub(1)) != b' '
        {
            out.push(Span {
                outer: i..end.saturating_add(1),
                body: i.saturating_add(1)..end,
                class: "md-math-inline",
            });
            i = end.saturating_add(1);
            continue;
        }
        // `inline code`
        if b.at(i) == b'`'
            && let Some(end) = find_close(b, i.saturating_add(1), b"`")
        {
            out.push(Span {
                outer: i..end.saturating_add(1),
                body: i.saturating_add(1)..end,
                class: "md-code",
            });
            i = end.saturating_add(1);
            continue;
        }
        // *italic* — must not be `**` (handled above) and must
        // contain at least one char.
        if b.at(i) == b'*'
            && let Some(end) = find_close(b, i.saturating_add(1), b"*")
            && end > i.saturating_add(1)
            && b.after(end.saturating_add(1)).first() != Some(&b'*')
        {
            out.push(Span {
                outer: i..end.saturating_add(1),
                body: i.saturating_add(1)..end,
                class: "md-italic",
            });
            i = end.saturating_add(1);
            continue;
        }
        // ![[embed]]  — image/audio/video/pdf embed by file
        // extension on the target. Recognized before the plain
        // `[[wikilink]]` arm. Quartz: `ofm.ts:233-265`.
        if let Some(next) = scan_link_like_span(text, b, i, &mut out) {
            i = next;
            continue;
        }
        i = i.saturating_add(1);
    }
    out
}

fn tag_boundary_before(b: &[u8], i: usize) -> bool {
    if i == 0 {
        // A `#` at the very start of the doc is a heading if
        // followed by a space; otherwise treat it as a tag.
        return b.get(1) != Some(&b' ');
    }
    let prev = b.at(i.saturating_sub(1));
    // `#` immediately after a newline followed by a space is a
    // heading marker, not a tag.
    if prev == b'\n' && b.get(i.saturating_add(1)) == Some(&b' ') {
        return false;
    }
    !prev.is_ascii_alphanumeric() && prev != b'_' && prev != b'/'
}

const fn is_tag_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'/'
}

/// Find the next occurrence of `needle` in `b` starting at
/// `from`, returning the start byte offset. Stops at newlines
/// (a span can't cross a line boundary).
/// Parse a `id:: <uuid>` block-id property line. Returns the
/// byte range of the UUID within the doc (absolute) when the
/// line matches, else `None`. Leading whitespace is tolerated
/// so an indented bullet's block-id is recognized too.
fn parse_block_id_line(line: &str, line_from: usize) -> Option<std::ops::Range<usize>> {
    let trimmed_start = line.len().saturating_sub(line.trim_start().len());
    let rest = line.after(trimmed_start);
    let prefix = "id:: ";
    let rest = rest.strip_prefix(prefix)?;
    let rest_off = trimmed_start.saturating_add(prefix.len());
    let bytes = rest.as_bytes();
    let uuid_len = peek_uuid(bytes)?;
    // Allow trailing whitespace but nothing else after the UUID.
    if rest.len() > uuid_len && !rest.after(uuid_len).trim().is_empty() {
        return None;
    }
    Some(
        line_from.saturating_add(rest_off)
            ..line_from.saturating_add(rest_off).saturating_add(uuid_len),
    )
}

/// Walk back from a line offset to the start of the block the
/// `id::` line belongs to. For a paragraph or list item, that's
/// the line directly above (or the start of the nearest non-
/// empty block above). For v1 we just return the previous
/// non-empty line's start.
fn find_block_anchor(text: &str, id_line_from: usize) -> usize {
    if id_line_from == 0 {
        return 0;
    }
    let prefix = text.before(id_line_from);
    // Walk back over any blank lines (shouldn't be common — the
    // `id::` line should be flush against the block).
    let mut end = id_line_from;
    while end > 0 {
        let prev_nl = prefix
            .before(end.saturating_sub(1))
            .rfind('\n')
            .map_or(0, |n| n.saturating_add(1));
        let line = text.slice(prev_nl..end.saturating_sub(1));
        if !line.trim().is_empty() {
            return prev_nl;
        }
        end = prev_nl;
    }
    0
}

// Per-`live_preview`-pass registry of UUIDs in the current
// doc. Refreshed on each pass via `reset_block_index`. Used by
// the `((uuid))` chip renderer to look up the target block's
// first-line content and by the `🔗` indicator to know which
// blocks have ids.
thread_local! {
static BLOCK_INDEX: std::cell::RefCell<std::collections::HashMap<String, usize>> =
    std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(crate) fn reset_block_index() {
    BLOCK_INDEX.with(|m| m.borrow_mut().clear());
}

pub(crate) fn register_block_id(uuid: &str, block_anchor: usize) {
    BLOCK_INDEX.with(|m| {
        m.borrow_mut().insert(uuid.to_string(), block_anchor);
    });
}

/// First ~40 chars of the block at `anchor`, stripped of
/// markdown markers for chip display. Stops at the first
/// newline.
pub(crate) fn block_preview(text: &str, anchor: usize) -> String {
    let line_end = text
        .after(anchor)
        .find('\n')
        .map_or(text.len(), |n| anchor.saturating_add(n));
    let line = text.slice(anchor..line_end);
    let cleaned = line.trim_start_matches(|c: char| {
        c == '#'
            || c == '>'
            || c == '-'
            || c == '*'
            || c == '+'
            || c == ' '
            || c == '\t'
            || c == '['
    });
    let cleaned = cleaned.trim_end();
    let max = 40;
    if cleaned.chars().count() > max {
        let truncated: String = cleaned.chars().take(max).collect();
        format!("{truncated}…")
    } else {
        cleaned.to_string()
    }
}

/// Look up a block's anchor offset by UUID. Returns `None` when
/// the UUID isn't in the current doc — multi-file resolution
/// is a later slice.
pub(crate) fn block_anchor_for_uuid(uuid: &str) -> Option<usize> {
    BLOCK_INDEX.with(|m| m.borrow().get(uuid).copied())
}

/// Peek a UUID v4 string at the start of `bytes` and return its
/// length (always 36) if matched, else `None`. Accepted form is
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` — hex digits in the
/// 8-4-4-4-12 layout, hyphens at positions 8/13/18/23.
pub(crate) fn peek_uuid(bytes: &[u8]) -> Option<usize> {
    const UUID_LEN: usize = 36;
    if bytes.len() < UUID_LEN {
        return None;
    }
    for idx in 0..UUID_LEN {
        let c = bytes.at(idx);
        if matches!(idx, 8 | 13 | 18 | 23) {
            if c != b'-' {
                return None;
            }
        } else if !c.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(UUID_LEN)
}

fn find_close(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut i = from;
    while i.saturating_add(needle.len()) <= b.len() {
        if b.at(i) == b'\n' {
            return None;
        }
        if b.slice(i..i.saturating_add(needle.len())) == needle {
            return Some(i);
        }
        i = i.saturating_add(1);
    }
    None
}

/// Scan the link-shaped inline delimiters at `i`: `![[embeds]]`, `[[wikilinks]]`,
/// `[^footnotes]`, `^[inline footnotes]`, `^block-ids`, `{{embed (…)}}`,
/// `((block-refs))` and bare `<autolinks>`.
///
/// Pushes any span it recognises onto `out` and returns the offset to resume
/// from, or `None` if nothing here matched. Split out of [`find_spans`], whose
/// loop carried every delimiter in one body; the arms are unchanged apart from
/// `i = x; continue;` becoming `return Some(x);`.
fn scan_link_like_span(text: &str, b: &[u8], i: usize, out: &mut Vec<Span>) -> Option<usize> {
    if i.saturating_add(5) <= b.len()
        && b.slice(i..i.saturating_add(3)) == b"![["
        && let Some(end) = find_close(b, i.saturating_add(3), b"]]")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(3)..end,
            class: "md-embed",
        });
        return Some(end.saturating_add(2));
    }
    // [[wikilink]]  — keep before `[link]` so the `[[`
    // isn't misread as the start of a regular link.
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"[["
        && let Some(end) = find_close(b, i.saturating_add(2), b"]]")
    {
        out.push(Span {
            outer: i..end.saturating_add(2),
            body: i.saturating_add(2)..end,
            class: "md-wikilink",
        });
        return Some(end.saturating_add(2));
    }
    // [^footnote-ref]
    if i.saturating_add(4) <= b.len()
        && b.slice(i..i.saturating_add(2)) == b"[^"
        && let Some(end) = find_close(b, i.saturating_add(2), b"]")
    {
        out.push(Span {
            outer: i..end.saturating_add(1),
            body: i.saturating_add(2)..end,
            class: "md-footnote",
        });
        return Some(end.saturating_add(1));
    }
    // ^[inline footnote body] — Obsidian extension. Body is
    // styled like a footnote reference but inline (the text
    // is the footnote content, not a refnum). Must match
    // BEFORE the `^block-id` arm, which would otherwise eat
    // the leading `^`.
    if b.at(i) == b'^'
        && b.get(i.saturating_add(1)) == Some(&b'[')
        && let Some(end) = find_close(b, i.saturating_add(2), b"]")
    {
        out.push(Span {
            outer: i..end.saturating_add(1),
            body: i.saturating_add(2)..end,
            class: "md-inline-footnote",
        });
        return Some(end.saturating_add(1));
    }
    // ^block-id — an Obsidian block reference target,
    // emitted at the end of a paragraph / list-item. We
    // recognize it only when followed by EOL (or end of
    // doc), so a stray `^` inside a sentence isn't styled.
    // Boundary check on the left mirrors `tag_boundary_before`.
    if b.at(i) == b'^'
        && i.saturating_add(1) < b.len()
        && (b.at(i.saturating_add(1)).is_ascii_alphanumeric()
            || b.at(i.saturating_add(1)) == b'-'
            || b.at(i.saturating_add(1)) == b'_')
        && (i == 0 || matches!(b.at(i.saturating_sub(1)), b' ' | b'\t' | b'\n'))
    {
        let mut j = i.saturating_add(1);
        while j < b.len() && (b.at(j).is_ascii_alphanumeric() || b.at(j) == b'-' || b.at(j) == b'_')
        {
            j = j.saturating_add(1);
        }
        if j == b.len() || b.at(j) == b'\n' {
            out.push(Span {
                outer: i..j,
                body: i..j,
                class: "md-block-id",
            });
            return Some(j);
        }
    }
    // `{{embed ((uuid))}}` — block embed (Logseq form).
    // Must match before `((uuid))` so the outer `(` of the
    // embed's payload isn't consumed by the bare-ref arm.
    if let Some(next) = scan_block_ref_span(text, b, i, out) {
        return Some(next);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use crate::selection::Selection;

    /// A stand-in chart renderer.
    ///
    /// `editor-state` deliberately cannot reach the real one — the chart
    /// language lives above this crate — so what is testable *here* is the
    /// plumbing: that a `kf` fence resolves a renderer, embeds what it
    /// returns, and builds the right widget around it. That a real chart
    /// engraves is `editor-keyflow`'s test, where the real renderer lives.
    struct StubCharts;

    impl crate::fence_renderer::FenceRenderer for StubCharts {
        fn render_svg(&self, source: &str) -> Option<String> {
            // Mirrors the real contract: a body that is a syntax
            // illustration rather than a chart declines to render.
            (!source.contains('\u{2192}'))
                .then(|| format!("<svg data-src=\"{}\"></svg>", source.len()))
        }
        fn highlight_html(&self, source: &str) -> String {
            format!("<span class=\"kf-root\">{}</span>", escape_html(source))
        }
    }

    /// Install [`StubCharts`] for the `kf` family. Idempotent, and safe to
    /// call from every test: the registry is process-wide and nextest runs
    /// each test in its own process anyway.
    fn with_stub_charts() {
        crate::fence_renderer::register_fence_renderer(
            crate::markdown::keyflow::LANGUAGE,
            std::sync::Arc::new(StubCharts),
        );
    }

    fn state(text: &str, caret: usize) -> EditorState {
        EditorState {
            doc: Doc::new(text),
            selection: Selection::caret(caret),
            folds: Vec::new(),
            reading_mode: false,
        }
    }

    #[test]
    fn bold_with_caret_outside_hides_markers() {
        // "**hi**" at offset 0..6. Body is 2..4 ("hi").
        // Caret at 7 (past the span) — markers should be hidden.
        let s = state("**hi** there", 7);
        let decs = live_preview(&s);
        // Expect: mark(2..4 bold), replace(0..2), replace(4..6).
        assert!(decs.iter().any(|d| d.from == 0 && d.to == 2));
        assert!(decs.iter().any(|d| d.from == 4 && d.to == 6));
        assert!(decs.iter().any(|d| d.from == 2 && d.to == 4));
    }

    #[test]
    fn bold_with_caret_inside_keeps_markers() {
        // Caret at 3 — inside "hi". Markers should NOT be hidden.
        let s = state("**hi** there", 3);
        let decs = live_preview(&s);
        let replace_count = decs
            .iter()
            .filter(|d| matches!(d.kind, crate::decoration::DecorationKind::Replace))
            .count();
        assert_eq!(replace_count, 0, "caret inside span should keep markers");
        // But the body mark is still there.
        assert!(decs.iter().any(|d| d.from == 2 && d.to == 4));
    }

    #[test]
    fn caret_adjacent_to_span_counts_as_touching() {
        // Caret right after the closing `**` — adjacent.
        let s = state("**hi**", 6);
        let decs = live_preview(&s);
        let replace_count = decs
            .iter()
            .filter(|d| matches!(d.kind, crate::decoration::DecorationKind::Replace))
            .count();
        assert_eq!(replace_count, 0);
    }

    #[test]
    fn italic_recognized() {
        let s = state("hello *world*", 0);
        let decs = live_preview(&s);
        assert!(decs.iter().any(|d| matches!(
            &d.kind,
            crate::decoration::DecorationKind::Mark { class, .. } if class == "md-italic"
        )));
    }

    #[test]
    fn table_cells_render_inline_markdown() {
        // Tables are replaced by a widget rather than decorated in place,
        // so the inline pass never reaches a cell. Before this, the
        // keyflow guide's notation-systems table rendered a literal
        // `**Letter name**` and `` `C`, `F#`, `Bb` ``.
        let html = render_table_cell("**Letter name**");
        assert!(
            html.contains(r#"<span class="md-bold">Letter name</span>"#),
            "{html}"
        );

        let html = render_table_cell("`C`, `F#`, `Bb`");
        assert_eq!(
            html,
            r#"<code class="md-code">C</code>, <code class="md-code">F#</code>, <code class="md-code">Bb</code>"#
        );
    }

    #[test]
    fn table_cells_render_links_and_wikilinks() {
        let html = render_table_cell("[[chords|Chords]]");
        assert!(html.contains(r#"class="md-wikilink""#), "{html}");
        assert!(html.contains(r#"data-href="chords""#), "{html}");
        assert!(html.contains(">Chords<"), "{html}");

        let html = render_table_cell("[docs](https://example.com)");
        assert!(html.contains(r#"href="https://example.com""#), "{html}");
        assert!(html.contains(">docs<"), "{html}");
    }

    #[test]
    fn a_code_span_is_not_further_interpreted() {
        // A cell documenting the bold marker must not go bold.
        let html = render_table_cell("`**`");
        assert_eq!(html, r#"<code class="md-code">**</code>"#);
    }

    #[test]
    fn table_cells_escape_html() {
        let html = render_table_cell("<script>alert(1)</script>");
        assert!(!html.contains("<script"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn unterminated_markers_stay_literal() {
        // A lone marker is text, not the start of a run to the end of the
        // cell — and must not panic.
        assert_eq!(render_table_cell("2 * 3"), "2 * 3");
        assert_eq!(render_table_cell("`unclosed"), "`unclosed");
        assert_eq!(render_table_cell("**unclosed"), "**unclosed");
    }

    #[test]
    fn multibyte_cells_are_not_split() {
        // The scanner walks bytes; stepping by one on a multi-byte char
        // would panic on a non-boundary index.
        for cell in ["♭ and ♯", "café — nö", "🎹 **keys**", "→ `x`"] {
            let _ = render_table_cell(cell);
        }
        assert!(render_table_cell("🎹 **keys**").contains("md-bold"));
    }

    #[test]
    fn the_guide_table_row_renders_as_intended() {
        // The exact row that exposed this, end to end.
        let cells = vec![
            vec!["System".into(), "Example".into()],
            vec!["**Letter name**".into(), "`C`, `F#`, `Bb`".into()],
        ];
        let html = render_table_html(&cells, &[]);
        assert!(!html.contains("**"), "raw bold markers survived: {html}");
        assert!(!html.contains('`'), "raw code markers survived: {html}");
    }

    #[test]
    fn inline_code_recognized() {
        let s = state("see `let x = 1`", 0);
        let decs = live_preview(&s);
        assert!(decs.iter().any(|d| matches!(
            &d.kind,
            crate::decoration::DecorationKind::Mark { class, .. } if class == "md-code"
        )));
    }

    #[test]
    fn span_does_not_cross_newline() {
        let s = state("**a\nb**", 0);
        let decs = live_preview(&s);
        // No span — the opening `**` doesn't pair across the \n.
        assert!(
            !decs
                .iter()
                .any(|d| matches!(d.kind, crate::decoration::DecorationKind::Mark { .. }))
        );
    }

    fn mark_classes(decs: &[DecoratedRange]) -> Vec<&str> {
        decs.iter()
            .filter_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Mark { class, .. } => Some(class.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn bold_italic_triple_recognized() {
        let s = state("***hi***", 0);
        assert!(mark_classes(&live_preview(&s)).contains(&"md-bold-italic"));
    }

    #[test]
    fn strikethrough_recognized() {
        let s = state("~~gone~~", 0);
        assert!(mark_classes(&live_preview(&s)).contains(&"md-strike"));
    }

    #[test]
    fn highlight_recognized() {
        let s = state("==pop==", 0);
        assert!(mark_classes(&live_preview(&s)).contains(&"md-highlight"));
    }

    #[test]
    fn link_recognized() {
        let s = state("[text](https://x)", 0);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-link"));
        // Body is just "text" (offsets 1..5).
        assert!(decs.iter().any(|d| d.from == 1 && d.to == 5));
    }

    #[test]
    fn wikilink_recognized() {
        let s = state("[[Page Name]]", 0);
        let decs = live_preview(&s);
        // Class is space-separated `md-wikilink
        // md-wikilink-unresolved` until a vault layer resolves
        // the target, so check the prefix rather than equality.
        assert!(
            mark_classes(&decs)
                .iter()
                .any(|c| c.starts_with("md-wikilink"))
        );
        assert!(decs.iter().any(|d| d.from == 2 && d.to == 11));
    }

    #[test]
    fn wikilink_alias_shows_only_display_text() {
        // `[[structure|Structure]]` — the caret away, only "Structure"
        // is marked; the `[[structure|` prefix and `]]` are replaced.
        let s = state("[[structure|Structure]]", 30);
        let decs = live_preview(&s);
        // The display range is the alias part: byte 12 ("Structure")
        // .. 21. The link mark covers exactly that, not the target.
        let disp_start = "[[structure|".len();
        let disp_end = "[[structure|Structure".len();
        assert!(
            decs.iter().any(|d| d.from == disp_start
                && d.to == disp_end
                && matches!(&d.kind, crate::decoration::DecorationKind::Mark { class, .. }
                    if class.starts_with("md-wikilink"))),
            "only the display text is marked as the link"
        );
        // `[[structure|` (0..12) is replaced (hidden) along with `]]`.
        assert!(
            decs.iter().any(|d| d.from == 0
                && d.to == disp_start
                && matches!(d.kind, crate::decoration::DecorationKind::Replace)),
            "the target|alias prefix is hidden"
        );
    }

    #[test]
    fn footnote_recognized() {
        let s = state("see[^1] here", 0);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-footnote"));
    }

    #[test]
    fn tag_recognized() {
        let s = state("a #todo b", 0);
        assert!(mark_classes(&live_preview(&s)).contains(&"md-tag"));
    }

    #[test]
    fn tag_requires_word_boundary() {
        let s = state("foo#bar", 0);
        let decs = live_preview(&s);
        assert!(!mark_classes(&decs).contains(&"md-tag"));
    }

    fn has_line_class(decs: &[DecoratedRange], pos: usize, class: &str) -> bool {
        decs.iter().any(|d| {
            d.from == pos
                && d.to == pos
                && matches!(&d.kind,
                    crate::decoration::DecorationKind::Line { class: c } if c == class)
        })
    }

    #[test]
    fn heading_emits_line_class_and_hides_marker() {
        let s = state("# Title", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-h1"));
        // Marker `# ` (2 bytes) replaced when caret elsewhere.
        assert!(decs.iter().any(|d| {
            d.from == 0 && d.to == 2 && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        }));
    }

    #[test]
    fn heading_levels_2_through_6() {
        for level in 2..=6 {
            let prefix = "#".repeat(level);
            let s = state(&format!("{prefix} h"), 100);
            let class = format!("md-h{level}");
            assert!(has_line_class(&live_preview(&s), 0, &class));
        }
    }

    #[test]
    fn heading_with_caret_shows_marker() {
        let s = state("# Title", 0);
        let decs = live_preview(&s);
        // Line class still applied …
        assert!(has_line_class(&decs, 0, "md-h1"));
        // … but no Replace on the `# `.
        let replace_on_marker = decs.iter().any(|d| {
            d.from == 0 && d.to == 2 && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(!replace_on_marker);
    }

    #[test]
    fn blockquote_recognized() {
        let s = state("> quoted", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-blockquote"));
    }

    #[test]
    fn table_recognized() {
        let s = state("| A | B |\n|---|---|\n| 1 | 2 |", 100);
        let decs = live_preview(&s);
        let widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-table"))
        });
        assert!(widget);
    }

    #[test]
    fn table_with_caret_inside_keeps_source_visible() {
        // Caret at byte 5 ("| A | B" inside header) — table
        // recognized but no Replace, source stays editable.
        let s = state("| A | B |\n|---|---|\n| 1 | 2 |", 5);
        let decs = live_preview(&s);
        let has_replace = decs
            .iter()
            .any(|d| matches!(d.kind, crate::decoration::DecorationKind::Replace));
        assert!(!has_replace);
    }

    #[test]
    fn table_requires_separator_row() {
        // No separator → not a table.
        let s = state("| A | B |\n| 1 | 2 |", 100);
        let decs = live_preview(&s);
        let widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-table"))
        });
        assert!(!widget);
    }

    #[test]
    fn inline_footnote_recognized() {
        // Caret away from the span: source replaced + marker
        // widget shown.
        let s = state("see ^[a side note] here", 0);
        let decs = live_preview(&s);
        let has_marker = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-inline-footnote-marker"))
        });
        assert!(has_marker);
    }

    #[test]
    fn inline_footnote_source_visible_when_caret_on() {
        // Caret inside the body: Mark, no Replace.
        let s = state("see ^[a side note] here", 8);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-inline-footnote"));
        let has_replace = decs.iter().any(|d| {
            d.from == 4
                && d.to == 18
                && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(!has_replace);
    }

    #[test]
    fn embed_page_renders_card() {
        let s = state("![[OtherPage]]", 100);
        let decs = live_preview(&s);
        let has_card = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-embed-page"))
        });
        assert!(has_card);
    }

    #[test]
    fn embed_section_with_intra_doc_resolves() {
        // `![[#Section]]` looks up the heading in the current
        // doc.
        let src = "Before\n\n## Section\nbody line\nmore body\n\n## Next\n\n![[#Section]]";
        let s = state(src, 100);
        let decs = live_preview(&s);
        let card_html = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-embed-section") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("section card");
        assert!(card_html.contains("body line"));
        assert!(!card_html.contains("md-embed-placeholder"));
    }

    #[test]
    fn embed_section_unresolved_shows_placeholder() {
        // Cross-doc section reference — no multi-file lookup
        // yet, so renders the placeholder.
        let s = state("![[OtherPage#Section]]", 100);
        let decs = live_preview(&s);
        let card_html = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-embed-section") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("section card");
        assert!(card_html.contains("md-embed-placeholder"));
    }

    #[test]
    fn embed_block_via_short_id_resolves_intra_doc() {
        let src = "Paragraph body ^anchor-here\n\n![[#^anchor-here]]";
        let s = state(src, 100);
        let decs = live_preview(&s);
        let card_html = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-embed-block") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("block card");
        assert!(card_html.contains("Paragraph body"));
    }

    #[test]
    fn block_id_property_line_replaced() {
        // A line that's just `id:: <uuid>` is hidden via a
        // Replace covering the whole line.
        let uuid = "5f9c1234-abcd-4ef0-8123-fedcba012345";
        let src = format!("paragraph content\nid:: {uuid}\nnext line");
        let s = state(&src, 0);
        let decs = live_preview(&s);
        let id_line_start = src.find("id::").unwrap();
        let id_line_end = src
            .after(id_line_start)
            .find('\n')
            .map_or(src.len(), |n| id_line_start + n + 1);
        let has_replace = decs.iter().any(|d| {
            d.from == id_line_start
                && d.to == id_line_end
                && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(has_replace);
    }

    #[test]
    fn block_ref_rendered_as_chip_widget() {
        let uuid = "5f9c1234-abcd-4ef0-8123-fedcba012345";
        let src = format!("see (({uuid})) for details");
        let s = state(&src, 0);
        let decs = live_preview(&s);
        let has_chip = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-block-ref-chip"))
        });
        assert!(has_chip);
    }

    #[test]
    fn block_embed_rendered_as_card() {
        let uuid = "5f9c1234-abcd-4ef0-8123-fedcba012345";
        let src = format!("{{{{embed (({uuid}))}}}}\n");
        let s = state(&src, 0);
        let decs = live_preview(&s);
        let has_card = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-block-embed-card"))
        });
        assert!(has_card);
    }

    /// Stub vault for cross-doc resolution tests.
    #[derive(Default)]
    struct FakeVault {
        blocks: std::collections::HashMap<String, super::VaultBlockHit>,
        pages: std::collections::HashMap<String, super::VaultPageHit>,
        sections: std::collections::HashMap<(String, String), String>,
        scripture: std::collections::HashMap<String, super::VaultScriptureHit>,
    }
    impl super::VaultLookup for FakeVault {
        fn lookup_block(&self, u: &str) -> Option<super::VaultBlockHit> {
            self.blocks.get(u).cloned()
        }
        fn lookup_page(&self, n: &str) -> Option<super::VaultPageHit> {
            self.pages.get(n).cloned()
        }
        fn lookup_section(&self, p: &str, h: &str) -> Option<String> {
            self.sections.get(&(p.into(), h.into())).cloned()
        }
        fn lookup_block_short(&self, _p: &str, _id: &str) -> Option<String> {
            None
        }
        fn lookup_scripture(&self, t: &str) -> Option<super::VaultScriptureHit> {
            self.scripture.get(t).cloned()
        }
    }

    #[test]
    fn block_ref_resolves_across_pages_via_vault() {
        let uuid = "11111111-1111-4111-8111-111111111111";
        let s = state(&format!("see (({uuid})) for context"), 0);
        let mut blocks = std::collections::HashMap::new();
        blocks.insert(
            uuid.to_string(),
            super::VaultBlockHit {
                page: "OtherPage".into(),
                preview: "Target block content".into(),
            },
        );
        let vault = FakeVault {
            blocks,
            pages: HashMap::new(),
            sections: HashMap::new(),
            ..Default::default()
        };
        let decs = super::live_preview_with(&s, Some(&vault));
        let chip_html = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-block-ref-chip") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("chip widget");
        assert!(chip_html.contains("Target block content"));
        assert!(chip_html.contains("OtherPage"));
        assert!(!chip_html.contains("md-block-ref-unresolved"));
    }

    fn scripture_vault(target: &str, text: Option<&str>) -> FakeVault {
        let mut scripture = std::collections::HashMap::new();
        scripture.insert(
            target.to_string(),
            super::VaultScriptureHit {
                display: "John 3:16".into(),
                osis: "John.3.16".into(),
                text: text.map(str::to_string),
                translation: "WEB".into(),
            },
        );
        FakeVault {
            scripture,
            ..Default::default()
        }
    }

    #[test]
    fn inline_scripture_link_renders_chip() {
        let s = state("see [[John 3:16]] here", 0);
        let vault = scripture_vault("John 3:16", Some("For God so loved the world…"));
        let decs = super::live_preview_with(&s, Some(&vault));
        let chip = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Mark { class, .. }
                if class == "md-wikilink md-scripture-chip")
        });
        assert!(chip, "decs = {decs:?}");
    }

    #[test]
    fn standalone_scripture_link_renders_verse_card() {
        // Caret far from the line so the widget fires.
        let s = state("intro\n\n[[John 3:16]]\n\ntail", 0);
        let vault = scripture_vault("John 3:16", Some("For God so loved the world…"));
        let decs = super::live_preview_with(&s, Some(&vault));
        let card = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-scripture-card") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("verse card widget");
        assert!(card.contains("For God so loved the world…"));
        assert!(card.contains("scripture-open:John 3:16"));
        assert!(card.contains("WEB"));
    }

    #[test]
    fn wikilink_resolved_class_when_vault_finds_page() {
        let s = state("see [[OtherPage]]", 0);
        let mut pages = std::collections::HashMap::new();
        pages.insert(
            "OtherPage".into(),
            super::VaultPageHit {
                preview: "Body".into(),
            },
        );
        let vault = FakeVault {
            blocks: HashMap::new(),
            pages,
            sections: HashMap::new(),
            ..Default::default()
        };
        let decs = super::live_preview_with(&s, Some(&vault));
        // The wikilink's mark class should NOT carry the
        // unresolved suffix when the vault confirms existence.
        let has_resolved = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Mark { class, .. }
                if class == "md-wikilink")
        });
        assert!(has_resolved, "decs = {decs:?}");
    }

    #[test]
    fn cross_page_section_embed_resolves_via_vault() {
        // Caret well past the embed so the widget actually
        // fires (caret on the span keeps the source visible).
        let s = state("![[Project README#Goals]]\n\nbody", 50);
        let mut sections = std::collections::HashMap::new();
        sections.insert(
            ("Project README".into(), "Goals".into()),
            "Make notes good.\nShip quickly.".into(),
        );
        let vault = FakeVault {
            blocks: HashMap::new(),
            pages: HashMap::new(),
            sections,
            ..Default::default()
        };
        let decs = super::live_preview_with(&s, Some(&vault));
        let card = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-embed-section") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("section card");
        assert!(card.contains("Make notes good."));
        assert!(!card.contains("md-embed-placeholder"));
    }

    #[test]
    fn block_ref_resolves_when_target_block_has_id_above() {
        // When the doc contains a block with an `id::` line,
        // the `((uuid))` chip should render the target's
        // first-line content (not "unresolved").
        let uuid = "5f9c1234-abcd-4ef0-8123-fedcba012345";
        let src =
            format!("First block content here\nid:: {uuid}\n\nA later paragraph with (({uuid})).");
        let s = state(&src, 0);
        let decs = live_preview(&s);
        let chip_html = decs
            .iter()
            .find_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html }
                    if html.contains("md-block-ref-chip") =>
                {
                    Some(html.clone())
                }
                _ => None,
            })
            .expect("chip widget");
        assert!(
            chip_html.contains("First block content here"),
            "expected chip to preview target, got: {chip_html}"
        );
        assert!(!chip_html.contains("md-block-ref-unresolved"));
    }

    #[test]
    fn block_id_recognized_at_eol_only() {
        // `^id` at end of line is a block ref.
        let s = state("paragraph ^block-1\nnext line", 0);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-block-id"));
        // Mid-line `^` shouldn't trigger.
        let s = state("x^y not a block id", 0);
        let decs = live_preview(&s);
        assert!(!mark_classes(&decs).contains(&"md-block-id"));
    }

    #[test]
    fn autolink_recognized() {
        let s = state("see <https://anthropic.com> for more", 0);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-autolink"));
    }

    #[test]
    fn autolink_email_recognized() {
        let s = state("mail <a@b.co>", 0);
        let decs = live_preview(&s);
        assert!(mark_classes(&decs).contains(&"md-autolink"));
    }

    #[test]
    fn setext_h1_recognized() {
        let s = state("Big Title\n=========\nbody", 0);
        let decs = live_preview(&s);
        let has_h1 = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class == "md-h1")
        });
        assert!(has_h1);
    }

    #[test]
    fn setext_h2_recognized() {
        let s = state("Subtitle\n--------\nbody", 0);
        let decs = live_preview(&s);
        let has_h2 = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class == "md-h2")
        });
        assert!(has_h2);
    }

    #[test]
    fn custom_task_status_recognized() {
        // `- [/]` in-progress, `- [>]` forwarded — still parses
        // as a task line, just with a non-canonical status char.
        let s = state("- [/] working on it\n- [>] later", 0);
        let decs = live_preview(&s);
        let has_task_line = decs
            .iter()
            .filter(|d| {
                matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class == "md-task")
            })
            .count();
        assert!(has_task_line >= 2);
    }

    #[test]
    fn frontmatter_parsed() {
        let src =
            "---\ntitle: Hello\ntags: [a, b]\npublished: true\naliases:\n  - x\n  - y\n---\n# body";
        let fm = super::parse_frontmatter(src).expect("fm found");
        assert_eq!(fm.props.len(), 4);
        assert_eq!(fm.props[0].key, "title");
        assert!(matches!(&fm.props[0].value, super::PropValue::Text(s) if s == "Hello"));
        assert!(
            matches!(&fm.props[1].value, super::PropValue::List(v) if v == &vec!["a".to_string(), "b".to_string()])
        );
        assert!(matches!(&fm.props[2].value, super::PropValue::Bool(true)));
        assert!(matches!(&fm.props[3].value, super::PropValue::List(v) if v.len() == 2));
    }

    #[test]
    fn frontmatter_property_ranges_are_atomic() {
        let src = "---\ntitle: x\ntags:\n  - a\n  - b\nactive: true\n---\n";
        let fm = super::parse_frontmatter(src).unwrap();
        // `title` should span only its one line.
        let title = &fm.props[0];
        assert_eq!(src.slice(title.range.clone()), "title: x\n");
        // `tags` should span the key line + both list items.
        let tags = &fm.props[1];
        assert_eq!(src.slice(tags.range.clone()), "tags:\n  - a\n  - b\n");
        // `active` is a scalar bool.
        let active = &fm.props[2];
        assert_eq!(src.slice(active.range.clone()), "active: true\n");
        assert!(matches!(active.value, super::PropValue::Bool(true)));
    }

    #[test]
    fn serialize_property_round_trips() {
        let s = super::serialize_property(
            "tags",
            &super::PropValue::List(vec!["a".into(), "b: c".into()]),
        );
        // The second item must be quoted because it contains a
        // colon; otherwise the parser would split it as a map.
        assert!(s.contains("\"b: c\""));
        assert!(s.starts_with("tags:\n"));
    }

    #[test]
    fn multiline_scalar_round_trips() {
        // `|` block scalars carry newlines verbatim.
        let src = "---\ndescription: |\n  first line\n  second line\n  third\nactive: true\n---\n";
        let fm = super::parse_frontmatter(src).unwrap();
        let desc = &fm.props[0];
        assert_eq!(desc.key, "description");
        if let super::PropValue::Text(t) = &desc.value {
            assert_eq!(t, "first line\nsecond line\nthird");
        } else {
            panic!("expected multiline text, got {:?}", desc.value);
        }
        // Serialize back out — must produce a `|` block, not a
        // collapsed single-line.
        let s = super::serialize_property("description", &desc.value);
        assert!(s.starts_with("description: |\n"));
        assert!(s.contains("  first line\n"));
        // Range covers the block + the closing indent line.
        assert_eq!(
            src.slice(desc.range.clone()),
            "description: |\n  first line\n  second line\n  third\n"
        );
    }

    #[test]
    fn frontmatter_only_at_doc_start() {
        // `---` mid-doc is a horizontal rule, not frontmatter.
        let src = "# heading\n\n---\nfoo: bar\n---\n";
        assert!(super::parse_frontmatter(src).is_none());
    }

    #[test]
    fn frontmatter_emits_widget_when_caret_away() {
        let s = state("---\ntitle: x\n---\n# body", 20);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-properties"))
        });
        assert!(has_widget);
    }

    #[test]
    fn kf_fence_engraves_on_all_targets() {
        // The exact snippet the keyflow guide's chords chapter ships.
        // editor-keyflow wraps engraver's CPU-only svg tier, so this
        // path runs on wasm32 too (the old native-only gate is gone).
        with_stub_charts();
        let s = state("```kf\nCmaj7 | F#m7b5 | Bbmaj9 | G7b9\n```\n\ntail", 44);
        let decs = live_preview(&s);
        let widget = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-keyflow-widget") =>
            {
                Some(html.clone())
            }
            _ => None,
        });
        let html = widget.expect("kf fence should engrave a chart widget");
        assert!(
            html.contains("<svg"),
            "widget should embed the engraved SVG"
        );
        // Rendered-only default: the source ships hidden behind the
        // `</>` toggle (CSS hides .md-keyflow-source until
        // md-keyflow-show-source is on the widget).
        assert!(
            html.contains("md-keyflow-toggle"),
            "widget should carry the source toggle button"
        );
        assert!(
            html.contains("md-keyflow-source"),
            "source column still ships in the widget for the toggle"
        );
        assert!(
            !html.contains("md-keyflow-show-source"),
            "```kf defaults to the engraved chart only"
        );
        // Every fence line sheds the grey code-block frame (bare) so the
        // chart renders full width, not boxed like code.
        assert!(has_line_class(&decs, 0, "md-keyflow-bare"), "opener bare");
        let close_at = "```kf\nCmaj7 | F#m7b5 | Bbmaj9 | G7b9\n".len();
        assert!(
            has_line_class(&decs, close_at, "md-keyflow-bare"),
            "closer bare"
        );
    }

    #[test]
    fn kf_dash_fence_is_highlighted_source_only() {
        // ```kf- — highlighted source, NO chart, always shown.
        with_stub_charts();
        let s = state("```kf-\nCmaj7 | Dm7\n```\n\ntail", 40);
        let decs = live_preview(&s);
        let widget = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-keyflow-widget") =>
            {
                Some(html.clone())
            }
            _ => None,
        });
        let html = widget.expect("kf- should widgetize a source block");
        assert!(
            html.contains("md-keyflow-source-only"),
            "kf- is source-only"
        );
        assert!(
            html.contains("class=\"kf-root\""),
            "kf- source is keyflow-highlighted"
        );
        assert!(!html.contains("<svg"), "kf- has NO chart");
        assert!(
            !html.contains("md-keyflow-toggle"),
            "kf- has no source toggle"
        );
        // Header with the tag + copy button.
        assert!(html.contains("md-keyflow-header"), "kf- carries a header");
        assert!(
            html.contains("md-code-copy"),
            "kf- header has a copy button"
        );
        // Sheds the code frame like the other keyflow fences.
        assert!(has_line_class(&decs, 0, "md-keyflow-bare"), "kf- is bare");
    }

    #[test]
    fn kf_plus_fence_shows_source_and_chart() {
        // ```kf+ — the author opts into source + chart together; the
        // widget ships with the show-source class already on.
        with_stub_charts();
        let s = state("```kf+\nCmaj7 | F#m7b5\n```\n\ntail", 30);
        let decs = live_preview(&s);
        let widget = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-keyflow-widget") =>
            {
                Some(html.clone())
            }
            _ => None,
        });
        let html = widget.expect("kf+ fence should engrave a chart widget");
        assert!(
            html.contains("md-keyflow-show-source"),
            "kf+ starts with source visible"
        );
        assert!(html.contains("<svg"), "kf+ still embeds the engraved SVG");
        // The source block is keyflow-highlighted (not plain text) and
        // wrapped for the stacked layout — never the old flex split.
        assert!(
            html.contains("class=\"kf-root\""),
            "source is kf-highlighted"
        );
        assert!(html.contains("md-keyflow-source"), "source block present");
        assert!(!html.contains("md-keyflow-split"), "no side-by-side split");
        // Source comes BEFORE the rendered chart in the DOM (stacked).
        let src_at = html.find("md-keyflow-source").unwrap();
        let render_at = html.find("md-keyflow-render").unwrap();
        assert!(src_at < render_at, "source stacks above the chart");
    }

    #[test]
    fn kbd_literal_renders_key_caps() {
        // Caret away: the `kbd:` code span becomes a key-caps widget.
        let s = state("press `kbd:<C-S-space>` now", 0);
        let decs = live_preview(&s);
        let widget = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Widget { html } if html.contains("md-kbd") => {
                Some(html.clone())
            }
            _ => None,
        });
        let html = widget.expect("kbd widget emitted");
        for cap in ["Ctrl", "Shift", "Space"] {
            assert!(html.contains(cap), "missing cap {cap} in {html}");
        }
        // Sequences render a "then" separator.
        let s = state("do `kbd:g g` twice", 0);
        let decs = live_preview(&s);
        assert!(decs.iter().any(|d| matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-kbd-then"))));
    }

    #[test]
    fn kbd_caret_inside_shows_source() {
        // Caret inside the span: raw source stays editable (plain
        // inline-code styling, no widget).
        let src = "press `kbd:<C-s>` now";
        let caret = src.find("C-s").unwrap();
        let s = state(src, caret);
        let decs = live_preview(&s);
        assert!(!decs.iter().any(|d| matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-kbd"))));
    }

    #[test]
    fn kbd_action_ref_resolves_through_lookup() {
        struct FakeKbd;
        impl KbdLookup for FakeKbd {
            fn keys_for_action(&self, action: &str) -> Option<String> {
                (action == "40044").then(|| "<space>".to_string())
            }
        }
        let s = state("press `kbd:@40044` to play, `kbd:@99999` is unbound", 0);
        let decs = live_preview_with_lookups(&s, None, Some(&FakeKbd));
        let widgets: Vec<String> = decs
            .iter()
            .filter_map(|d| match &d.kind {
                crate::decoration::DecorationKind::Widget { html } if html.contains("md-kbd") => {
                    Some(html.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(widgets.len(), 2, "both refs should render widgets");
        assert!(
            widgets.iter().any(|h| h.contains("Space")),
            "resolved ref shows keys"
        );
        assert!(
            widgets
                .iter()
                .any(|h| h.contains("md-kbd-unbound") && h.contains("@99999")),
            "unresolved ref renders the unbound cap"
        );
    }

    #[test]
    fn inline_math_recognized() {
        // Caret away: source replaced + math widget emitted.
        let s = state("Cost is $x^2$ today", 0);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-math-widget"))
        });
        assert!(has_widget);
    }

    #[test]
    fn block_math_recognized() {
        // `mc^2` would fail to compile in Typst (`mc` reads as
        // an unknown identifier); use `m c^2` so the smoke test
        // exercises a real render.
        let s = state("Before\n$$E = m c^2$$\nAfter", 0);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-math-widget-block"))
        });
        assert!(has_widget);
    }

    #[test]
    fn math_with_caret_inside_shows_source() {
        // Caret inside the body: no Replace, source visible
        // as `md-math-inline` mark so the user can edit.
        let s = state("Cost $x^2$ here", 7);
        let decs = live_preview(&s);
        let has_replace = decs.iter().any(|d| {
            d.from == 5
                && d.to == 10
                && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(!has_replace);
    }

    #[test]
    fn mermaid_fence_recognized() {
        // Caret past the closing fence so cursor_touches is
        // false and the widget actually fires.
        let src = "```mermaid\nflowchart TD\n  A --> B\n```\nx";
        let s = state(src, src.len() - 1);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-mermaid-widget"))
        });
        assert!(has_widget);
    }

    #[test]
    fn typst_fence_recognized() {
        // Caret past the closing fence so cursor_touches is
        // false and the widget actually fires.
        let src = "```typst\n= Section\n```\nx";
        let s = state(src, src.len() - 1);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-typst-widget"))
        });
        assert!(has_widget);
    }

    #[test]
    fn comment_recognized() {
        // Caret away from the `%%…%%` span: whole range hidden.
        let s = state("a %% hidden %% b", 0);
        let decs = live_preview(&s);
        let has_replace = decs.iter().any(|d| {
            d.from == 2
                && d.to == 14
                && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(has_replace);
    }

    #[test]
    fn comment_revealed_when_caret_inside() {
        // Caret inside the comment: body styled as `md-comment`.
        let s = state("a %% hidden %% b", 6);
        let decs = live_preview(&s);
        let has_mark = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Mark { class, .. } if class == "md-comment")
        });
        assert!(has_mark);
    }

    #[test]
    fn image_embed_recognized() {
        let s = state("![[pic.png]]", 100);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-embed-image"))
        });
        assert!(has_widget);
    }

    #[test]
    fn image_embed_with_size_opts() {
        let s = state("![[pic.png|320x200]]", 100);
        let decs = live_preview(&s);
        let widget = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Widget { html } => Some(html),
            _ => None,
        });
        let html = widget.expect("widget");
        assert!(html.contains("width:320px"));
        assert!(html.contains("height:200px"));
    }

    #[test]
    fn video_embed_recognized() {
        let s = state("![[clip.mp4]]", 100);
        let decs = live_preview(&s);
        let has_video = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-embed-video"))
        });
        assert!(has_video);
    }

    #[test]
    fn unknown_extension_falls_back_to_wikilink() {
        // .md isn't a media kind — should NOT emit an embed widget.
        let s = state("![[other.md]]", 100);
        let decs = live_preview(&s);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html }
                if html.starts_with("<img") || html.starts_with("<video"))
        });
        assert!(!has_widget);
    }

    #[test]
    fn callout_note_emits_md_callout_class() {
        let s = state("> [!note] Title", 100);
        let decs = live_preview(&s);
        let has_callout = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-note"))
        });
        assert!(has_callout);
    }

    #[test]
    fn nested_callout_emits_depth_class() {
        let src = "> [!note] outer\n> > [!warning] inner\n> > inner body\n";
        let s = state(src, 100);
        let decs = live_preview(&s);
        // Inner header line gets both `md-callout-warning` and
        // a depth-2 class.
        let inner_line_from = src.find("> > [!warning]").unwrap();
        let has_warning = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-warning"))
                && d.from == inner_line_from
        });
        let has_depth = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class == "md-callout-nested-2")
                && d.from == inner_line_from
        });
        assert!(has_warning, "expected inner line to be warning-classed");
        assert!(has_depth, "expected inner line to carry depth-2 class");
    }

    #[test]
    fn nested_callout_body_inherits_inner_kind() {
        let src = "> [!note] outer\n> > [!warning] inner header\n> > body\n";
        let s = state(src, 100);
        let decs = live_preview(&s);
        let body_from = src.find("> > body").unwrap();
        let body_is_warning = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-warning"))
                && d.from == body_from
        });
        assert!(body_is_warning);
    }

    #[test]
    fn dedent_closes_inner_callout() {
        // After `> > [!warning] inner`, a `> ` line at depth 1
        // should fall back to the OUTER callout kind, not the
        // inner one.
        let src = "> [!note] outer\n> > [!warning] inner\n> back to outer\n";
        let s = state(src, 100);
        let decs = live_preview(&s);
        let back_from = src.find("> back to outer").unwrap();
        let back_is_note = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-note"))
                && d.from == back_from
        });
        assert!(back_is_note);
    }

    #[test]
    fn callout_warning_alias_resolves() {
        let s = state("> [!caution] Hey", 100);
        let decs = live_preview(&s);
        let has_warning = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-warning"))
        });
        assert!(has_warning);
    }

    #[test]
    fn callout_body_lines_inherit_kind() {
        let s = state("> [!note] T\n> body line", 100);
        let decs = live_preview(&s);
        // Both lines should have a `md-callout-note` class.
        let count = decs
            .iter()
            .filter(|d| {
                matches!(&d.kind,
            crate::decoration::DecorationKind::Line { class }
                if class.contains("md-callout-note"))
            })
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn non_blockquote_line_closes_callout() {
        let s = state("> [!note] T\n> body\nafter", 100);
        let decs = live_preview(&s);
        // Line at pos 21 ("after") should NOT have callout class.
        let after_class = decs.iter().find_map(|d| match &d.kind {
            crate::decoration::DecorationKind::Line { class } if d.from == 21 => Some(class),
            _ => None,
        });
        // Either no Line at "after" (it's a plain line) or one
        // without the callout class.
        assert!(after_class.is_none_or(|c| !c.contains("md-callout")));
    }

    #[test]
    fn hr_recognized() {
        let s = state("---", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-hr"));
    }

    #[test]
    fn hr_active_class_when_caret_on_line() {
        let s = state("---", 1);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-hr-active"));
        // And the `---` source isn't replaced — user can edit.
        let has_replace = decs.iter().any(|d| {
            d.from == 0 && d.to == 3 && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(!has_replace);
    }

    #[test]
    fn list_bullet_recognized() {
        let s = state("- item", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-list-item"));
    }

    #[test]
    fn list_ordered_recognized() {
        let s = state("1. first", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-list-item"));
    }

    #[test]
    fn list_with_caret_on_line_keeps_source_visible() {
        // Caret on the bullet line — no Replace, no widget; the
        // `- ` source stays editable. Same pattern as headings.
        let s = state("- item", 3);
        let decs = live_preview(&s);
        let has_replace = decs.iter().any(|d| {
            d.from == 0 && d.to == 2 && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(
            !has_replace,
            "marker source must stay visible while caret is on the line"
        );
        let has_widget = decs
            .iter()
            .any(|d| matches!(&d.kind, crate::decoration::DecorationKind::Widget { .. }));
        assert!(!has_widget, "no bullet widget while caret is on the line");
    }

    #[test]
    fn ordered_list_with_caret_on_line_keeps_source_visible() {
        let s = state("1. foo", 3);
        let decs = live_preview(&s);
        let has_replace = decs.iter().any(|d| {
            d.from == 0 && d.to == 3 && matches!(d.kind, crate::decoration::DecorationKind::Replace)
        });
        assert!(!has_replace);
    }

    #[test]
    fn task_with_caret_on_line_keeps_source_visible() {
        // Caret on the line: source bytes stay editable (no
        // Replace AND no widget — both at once would overlap).
        let s = state("- [ ] todo", 4);
        let decs = live_preview(&s);
        let has_replace = decs
            .iter()
            .any(|d| matches!(d.kind, crate::decoration::DecorationKind::Replace));
        assert!(!has_replace);
        let has_widget = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Widget { html } if html.contains("md-task-checkbox"))
        });
        assert!(!has_widget);
    }

    #[test]
    fn task_unchecked_recognized() {
        let s = state("- [ ] todo", 100);
        let decs = live_preview(&s);
        assert!(has_line_class(&decs, 0, "md-task"));
        // Widget emitted for the checkbox.
        let widget = decs.iter().any(|d| {
            matches!(&d.kind, crate::decoration::DecorationKind::Widget { html }
                if html.contains("md-task-checkbox") && !html.contains("checked"))
        });
        assert!(widget);
    }

    #[test]
    fn task_checked_recognized() {
        let s = state("- [x] done", 100);
        let decs = live_preview(&s);
        let widget = decs.iter().any(|d| {
            matches!(&d.kind, crate::decoration::DecorationKind::Widget { html }
                if html.contains("checked"))
        });
        assert!(widget);
    }

    #[test]
    fn code_fence_with_lang_emits_syntax_tokens() {
        let s = state("```rust\nfn main() {}\n```", 999);
        let decs = live_preview(&s);
        let has_token = decs.iter().any(|d| {
            matches!(&d.kind,
            crate::decoration::DecorationKind::Mark { class, .. }
                if class.starts_with("md-tok-"))
        });
        assert!(has_token, "expected at least one md-tok-* mark");
    }

    #[test]
    fn code_fence_spans_multiple_lines() {
        let s = state("```rust\nfn main() {}\n```", 999);
        let decs = live_preview(&s);
        // Every line gets md-code-block.
        assert!(has_line_class(&decs, 0, "md-code-block")); // open
        assert!(has_line_class(&decs, 8, "md-code-block")); // body
        // Closing fence at byte 21 (`...{}\n` ends at 20).
        let close_line_start = "```rust\nfn main() {}\n".len();
        assert!(has_line_class(&decs, close_line_start, "md-code-block"));
    }

    #[test]
    fn inline_inside_fence_is_skipped() {
        let s = state("```\n**bold**\n```", 999);
        let decs = live_preview(&s);
        // No bold mark should exist for the `**bold**` inside fence.
        let has_bold = decs.iter().any(|d| {
            matches!(&d.kind, crate::decoration::DecorationKind::Mark { class, .. }
                if class == "md-bold")
        });
        assert!(!has_bold);
    }

    #[test]
    fn tag_at_line_start_when_no_heading() {
        let s = state("#foo bar", 100);
        assert!(mark_classes(&live_preview(&s)).contains(&"md-tag"));
    }

    #[test]
    fn tag_has_no_hidden_markers() {
        let s = state("#todo", 100);
        let decs = live_preview(&s);
        let has_replace = decs
            .iter()
            .any(|d| matches!(d.kind, crate::decoration::DecorationKind::Replace));
        assert!(!has_replace, "tag should have no markers to hide");
    }
}
