//! Native-renderer (Blitz / `dioxus-native`) input + caret seam.
//!
//! On the web/desktop path a real `contenteditable` element does the
//! heavy lifting: the browser inserts typed text, paints the caret, and
//! moves the selection on click/arrow, and our [`crate::bridge`] observes
//! those DOM mutations and folds them back into `editor-state`
//! transactions.
//!
//! Blitz has none of that. Its built-in text editing is form-control only
//! (`<input>`/`<textarea>` via parley); there is no `contenteditable` on
//! arbitrary element subtrees, no JS engine (so every `document::eval`
//! no-ops), and no OS caret on a plain `<div>`. So on native the data flow
//! is *inverted*: `editor-state` is the sole source of truth, the
//! component renders the tile tree as rsx ([`crate::tile::render_dx`]),
//! and this module supplies the two things the browser used to give us for
//! free —
//!
//! 1. **A painted caret.** [`native_caret_decoration`] drops a thin
//!    zero-width bar at the selection head when the editor would
//!    otherwise show the native insert caret (insert mode, or vim
//!    disabled). The block/underscore vim carets are already painted by
//!    [`crate::editor::modal_caret_decoration`] on every platform, so this
//!    only fills the insert-mode gap.
//! 2. **Default text input.** [`handle_text_input`] is the keydown
//!    fallthrough the web delegates to `contenteditable`: a printable
//!    character (or `Enter`) with no modifier that nothing else claimed
//!    gets inserted over the current selection as a transaction.
//! 3. **IME composition.** An input method (Pinyin, Kana, dead-key
//!    accents, …) composes text before committing it. The FTS Blitz fork
//!    turns the platform IME on for this element (`inputmode="text"`) and
//!    dioxus-native-dom delivers its preedit and commit as
//!    `compositionupdate` / `compositionend`. [`ime_step`] folds those into
//!    a [`Composition`] held in a signal beside the state, and
//!    [`native_preedit_decoration`] draws it at the caret until it commits
//!    (one transaction) or is cancelled (none).

use editor_state::{
    Changes, DecoratedRange, EditorState, KeySpec, Range, Selection, TransactionSpec,
};

/// Painted insert caret for the native renderer. A 1px-wide zero-content
/// widget at the selection head, styled by `.ed-native-caret` in the
/// stylesheet. Only emitted when the editor is focused, not in reading
/// mode, and the active vim mode (if any) is one that would normally show
/// the *bar* caret — Normal/Visual/Replace already paint their own block /
/// underscore via [`crate::editor::modal_caret_decoration`], so painting a
/// bar there too would double up.
///
/// A non-empty selection still gets a head caret: the selected range is
/// rendered via its own mark decoration by the caller's decoration source;
/// the caret marks the active edge so shift-arrow growth is visible.
#[must_use]
pub fn native_caret_decoration(
    s: &EditorState,
    vim: Option<dioxus::prelude::Signal<editor_vim::VimState>>,
    focused: dioxus::prelude::Signal<bool>,
) -> Vec<DecoratedRange> {
    use dioxus::prelude::*;

    if !focused() || s.reading_mode {
        return Vec::new();
    }
    // Skip when a painted modal caret already owns this position.
    if let Some(vim) = vim {
        match vim.read().mode {
            editor_vim::Mode::Insert | editor_vim::Mode::Command => {}
            // Normal / Visual* / Replace paint their own block / underscore
            // via `crate::editor::modal_caret_decoration`.
            _ => return Vec::new(),
        }
    }
    // Before a character, the caret is a mark on that character (Blitz paints
    // marks, as it does the syntax colours). Where there is no character to
    // mark (an empty line, or the end of the document) it is a widget span
    // holding a no-break space, which gives the inline-block caret a box.
    let text = s.doc.to_string();
    let head = s.selection.primary().head.min(text.len());
    let next = text[head..].chars().next();
    if matches!(next, Some(c) if c != '\n') {
        let end = head + next.map_or(0, char::len_utf8);
        vec![DecoratedRange::mark(head..end, "ed-native-caret")]
    } else if next.is_some() || text[..head].ends_with('\n') || head == text.len() {
        vec![DecoratedRange::widget(
            head,
            "<span class=\"ed-native-caret\">\u{a0}</span>".to_string(),
        )]
    } else if let Some(prev) = text[..head].chars().next_back() {
        let start = head - prev.len_utf8();
        vec![DecoratedRange::mark(start..head, "ed-native-caret-end")]
    } else {
        Vec::new()
    }
}

/// An IME composition in progress: the preedit text the input method is
/// still composing.
///
/// It is *not* in the document: it lives in a signal
/// beside the editor state and is drawn at the caret, so a re-render (a
/// remote edit, the idle decoration pass, a theme change) neither drops it
/// nor commits it, and it follows the caret when an edit elsewhere moves
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    pub text: String,
}

/// One IME event, as the native view receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// `compositionupdate`: the preedit is now this text. Empty means it
    /// was cleared — a cancel, or the clear that precedes a commit.
    Preedit(String),
    /// `compositionend`: insert this text; the composition is over.
    Commit(String),
}

/// What an [`ImeEvent`] does: the composition that follows it, and the
/// transaction (if any) to apply first.
#[derive(Debug)]
pub struct ImeOutcome {
    pub composition: Option<Composition>,
    pub spec: Option<TransactionSpec>,
}

/// The pure IME state machine, split out so it is testable without a
/// Dioxus runtime.
///
/// - A non-empty preedit starts or updates the composition without changing
///   the document or selection.
/// - An empty preedit ends the composition with no edit, preserving any
///   selected text if composition is cancelled.
/// - A commit inserts its text over the selection as one transaction,
///   tagged `origin: native-ime`, with the caret after it. The IME's
///   literal text is inserted as is: no bracket auto-pairing or list
///   continuation.
#[must_use]
pub fn ime_step(cur: &EditorState, event: ImeEvent) -> ImeOutcome {
    let p = cur.selection.primary();
    let (from, to) = (p.from(), p.to());
    match event {
        ImeEvent::Preedit(text) if text.is_empty() => ImeOutcome {
            composition: None,
            spec: None,
        },
        ImeEvent::Preedit(text) => ImeOutcome {
            spec: None,
            composition: Some(Composition { text }),
        },
        ImeEvent::Commit(text) => ImeOutcome {
            spec: (!text.is_empty()).then(|| {
                TransactionSpec::new()
                    .changes(Changes::replace(from..to, &text))
                    .selection(Selection::caret(from.saturating_add(text.len())))
                    .annotate("origin", "native-ime")
            }),
            composition: None,
        },
    }
}

/// The preedit, drawn at the selection head as an inline widget.
///
/// It is styled by `.ed-preedit` (the `--ed-preedit-*` theme tokens). The painted insert
/// caret sits at the same offset and is pushed after it, so it reads at
/// the end of the composed text.
#[must_use]
pub fn native_preedit_decoration(
    s: &EditorState,
    composition: Option<&Composition>,
) -> Vec<DecoratedRange> {
    let Some(c) = composition.filter(|c| !c.text.is_empty()) else {
        return Vec::new();
    };
    let head = s.selection.primary().from().min(s.doc.len());
    vec![DecoratedRange::widget(
        head,
        format!("<span class=\"ed-preedit\">{}</span>", escape_text(&c.text)),
    )]
}

/// Escape text for the widget's HTML body.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Selection highlight for the native renderer. The web path shows the
/// browser's own `contenteditable` selection; Blitz has none, so we paint
/// the primary selection range ourselves as a background mark (the same
/// `draw_inline_backgrounds` mechanism the block caret uses, so it never
/// shifts text). Empty selections (a plain caret) paint nothing. Spans
/// crossing line boundaries are split per line by the tile builder.
#[must_use]
pub fn native_selection_decoration(
    s: &EditorState,
    vim: Option<dioxus::prelude::Signal<editor_vim::VimState>>,
    focused: dioxus::prelude::Signal<bool>,
) -> Vec<DecoratedRange> {
    use dioxus::prelude::*;
    if !focused() {
        return Vec::new();
    }
    // In vim VISUAL LINE mode the selection covers WHOLE lines, not just
    // the caret-to-anchor char range — expand each range to its line
    // bounds so the highlight fills every touched line.
    let line_wise = matches!(
        vim.map(|v| v.read().mode),
        Some(editor_vim::Mode::VisualLine)
    );
    let rope = s.doc.rope();
    let mut out = Vec::new();
    for r in s.selection.ranges().iter().filter(|r| r.from() != r.to()) {
        if line_wise {
            // Whole-row highlight: a LINE decoration per touched line sets
            // the `.cm-line` block background, which Blitz paints edge to
            // edge (an inline mark would stop at the last glyph, leaving a
            // ragged right edge on short lines).
            let start_line = rope.byte_to_line(r.from().min(rope.len_bytes()));
            let end_line = rope.byte_to_line(r.to().min(rope.len_bytes()));
            for line in start_line..=end_line {
                out.push(DecoratedRange::line(
                    rope.line_to_byte(line),
                    "ed-selection-line",
                ));
            }
        } else {
            out.push(DecoratedRange::mark(r.from()..r.to(), "ed-selection"));
        }
    }
    out
}

/// Default text-input action for the native keydown handler — the work the
/// web path hands to `contenteditable`. Returns `true` if the key was
/// consumed (caller should `prevent_default` and stop).
///
/// Only fires for unmodified printable characters and `Enter`; everything
/// structural (Backspace/Delete/Tab/Mod-\*) is already handled by the
/// keymap before this fallthrough runs. Ctrl/Alt/Meta-modified keys are
/// left alone so they can't smuggle control chars into the doc.
#[must_use]
pub fn handle_text_input(
    state: dioxus::prelude::Signal<EditorState>,
    cur: &EditorState,
    press: &KeySpec,
    sink: Option<dioxus::prelude::Callback<crate::TransactionEvent>>,
) -> bool {
    // Control/super-modified chords are never literal text. Alt is left to
    // the keymap (it may bind Alt-motions); a bare AltGr-composed glyph
    // arrives as `Character` without the alt flag on most layouts.
    match text_input_spec(cur, press) {
        Some(spec) => {
            crate::event::apply_tx(state, cur, spec, sink);
            true
        }
        None => false,
    }
}

/// Pure decision half of [`handle_text_input`]: the transaction a printable
/// key produces, or `None` if the key isn't literal text. Split out so the
/// behavior is unit-testable without a Dioxus runtime.
#[must_use]
pub fn text_input_spec(cur: &EditorState, press: &KeySpec) -> Option<TransactionSpec> {
    // Word-wise deletion (Ctrl-Backspace / Ctrl-Delete) — shared-core
    // mirror of the browser's deleteGroupBackward/Forward defaults.
    if (press.ctrl || press.meta) && !press.alt {
        let tag = |spec: TransactionSpec| Some(spec.annotate("origin", "native-input"));
        match press.key.as_str() {
            "Backspace" => return editor_state::commands::delete_word_backward(cur).and_then(tag),
            "Delete" => return editor_state::commands::delete_word_forward(cur).and_then(tag),
            _ => {}
        }
    }
    if press.ctrl || press.meta {
        return None;
    }
    let p = cur.selection.primary();
    let (from, to) = (p.from(), p.to());

    // Default actions the web path gets from contenteditable +
    // `beforeinput` interception, expressed through the SAME shared
    // commands (`editor_state::commands`) the web bridge calls — one
    // implementation, two renderers:
    //   Enter     → list/task/blockquote continuation (plain \n fallback)
    //   Backspace → bracket-pair aware, char-wise backward delete
    //   Delete    → char-wise forward delete
    //   brackets  → auto-pair / skip-over-closer
    let tag = |spec: TransactionSpec| Some(spec.annotate("origin", "native-input"));
    match press.key.as_str() {
        "Enter" => return editor_state::commands::enter_continue_list(cur).and_then(tag),
        "Backspace" => return editor_state::commands::delete_backward(cur).and_then(tag),
        "Delete" => return editor_state::commands::delete_forward(cur).and_then(tag),
        _ => {}
    }

    let inserted: &str = match press.key.as_str() {
        // Tab on a list/task line indents the item (Shift-Tab outdents);
        // elsewhere it inserts a literal tab (Shift-Tab does nothing).
        "Tab" => {
            if let Some(spec) = editor_state::commands::tab_list_indent(cur, press.shift) {
                return tag(spec);
            }
            if press.shift {
                return None;
            }
            "\t"
        }
        // A single typed grapheme arrives as `Key::Character`. Named keys
        // ("ArrowLeft", "Escape", …) are multi-char strings we must not
        // insert; gate on "exactly one char, not a control".
        other if is_text_input(other) => other,
        _ => return None,
    };

    if let Some(spec) = editor_state::commands::insert_bracket(cur, inserted) {
        return tag(spec);
    }

    Some(
        TransactionSpec::new()
            .changes(Changes::replace(from..to, inserted))
            .selection(Selection::caret(from + inserted.len()))
            .annotate("origin", "native-input"),
    )
}

/// Caret movement for the native keydown handler — the work the web hands
/// to `contenteditable` + the `selectionchange` bridge. Handles
/// Arrow{Left,Right,Up,Down}, Home, End; `Shift` extends the selection
/// (anchor fixed, head moves) instead of collapsing it. Returns `true`
/// when the key was a movement key (caller should `prevent_default`).
///
/// Movement is over **byte offsets** (the selection's native unit),
/// stepping by `char` via the rope's char index. Up/Down move by *logical*
/// line and re-derive the target column each press (no sticky goal column
/// yet) — wrap-aware visual-line movement needs Blitz layout geometry and
/// lands later. `Ctrl`/`Meta`-modified arrows are left for the keymap
/// (word/document motions) by returning `false`.
#[must_use]
pub fn handle_navigation(
    state: dioxus::prelude::Signal<EditorState>,
    cur: &EditorState,
    press: &KeySpec,
    sink: Option<dioxus::prelude::Callback<crate::TransactionEvent>>,
) -> bool {
    match nav_target(cur, press) {
        Some(range) => {
            let spec = TransactionSpec::new()
                .selection(Selection::single(range))
                .annotate("origin", "native-nav");
            crate::event::apply_tx(state, cur, spec, sink);
            true
        }
        None => false,
    }
}

/// Pure decision half of [`handle_navigation`]: the new primary selection
/// range a movement key produces, or `None` if the key isn't a movement
/// key. `Shift` keeps the anchor (extend); otherwise the range collapses
/// to a caret at the new head. Split out for unit testing.
#[must_use]
pub fn nav_target(cur: &EditorState, press: &KeySpec) -> Option<Range> {
    if press.alt {
        return None;
    }
    let head = cur.selection.primary().head.min(cur.doc.len());
    // Mod (ctrl/meta) + horizontal arrows: word-group motion, the
    // shared-core mirror of the browser's Ctrl-ArrowLeft/Right.
    if press.ctrl || press.meta {
        let new_head = match press.key.as_str() {
            "ArrowLeft" => editor_state::commands::word_boundary_left(cur, head),
            "ArrowRight" => editor_state::commands::word_boundary_right(cur, head),
            _ => return None,
        };
        let anchor = if press.shift {
            cur.selection.primary().anchor
        } else {
            new_head
        };
        return Some(Range::new(anchor, new_head));
    }
    let rope = cur.doc.rope();
    let char_idx = rope.byte_to_char(head);

    let new_head: usize = match press.key.as_str() {
        "ArrowLeft" => {
            if char_idx > 0 {
                rope.char_to_byte(char_idx - 1)
            } else {
                0
            }
        }
        "ArrowRight" => {
            if char_idx < rope.len_chars() {
                rope.char_to_byte(char_idx + 1)
            } else {
                head
            }
        }
        "Home" => rope.line_to_byte(rope.byte_to_line(head)),
        "End" => line_end_byte(rope, rope.byte_to_line(head)),
        "ArrowUp" => vertical(rope, head, char_idx, Dir::Up),
        "ArrowDown" => vertical(rope, head, char_idx, Dir::Down),
        _ => return None,
    };

    let anchor = if press.shift {
        cur.selection.primary().anchor
    } else {
        new_head
    };
    Some(Range::new(anchor, new_head))
}

enum Dir {
    Up,
    Down,
}

/// Byte offset of the end of `line` (just before its `\n`, or the doc end
/// for the last line).
fn line_end_byte(rope: &ropey::Rope, line: usize) -> usize {
    let next_start = if line + 1 < rope.len_lines() {
        rope.line_to_byte(line + 1)
    } else {
        rope.len_bytes()
    };
    // Strip a single trailing '\n' (its char is one byte).
    if next_start > rope.line_to_byte(line)
        && rope.get_char(rope.byte_to_char(next_start).saturating_sub(1)) == Some('\n')
    {
        rope.char_to_byte(rope.byte_to_char(next_start) - 1)
    } else {
        next_start
    }
}

/// Move the caret up or down one logical line, preserving the column (in
/// chars from the line start) clamped to the target line's length.
fn vertical(rope: &ropey::Rope, head: usize, char_idx: usize, dir: Dir) -> usize {
    let line = rope.byte_to_line(head);
    let col = char_idx - rope.line_to_char(line);
    let target = match dir {
        Dir::Up if line == 0 => return rope.line_to_byte(0),
        Dir::Up => line - 1,
        Dir::Down if line + 1 >= rope.len_lines() => return head,
        Dir::Down => line + 1,
    };
    let target_start_char = rope.line_to_char(target);
    let target_end_char = rope.byte_to_char(line_end_byte(rope, target));
    let new_char = (target_start_char + col).min(target_end_char);
    rope.char_to_byte(new_char)
}

/// True when `key` is a literal text grapheme rather than a named key.
/// Dioxus reports printable input as `Key::Character(s)` where `s` is the
/// composed grapheme (usually one `char`, but combining sequences can be
/// longer); named keys are PascalCase identifiers like `"ArrowLeft"`. We
/// accept any string whose chars are all non-control, which covers ASCII,
/// Unicode letters, emoji, and combining sequences while rejecting the
/// named keys (which are alphabetic and would otherwise slip through — so
/// we additionally reject the empty string and require at least one char
/// that isn't ASCII-control). The keydown handler only reaches here after
/// the keymap and vim have had their pass, so motions/commands are gone.
fn is_text_input(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|c| !c.is_control())
        // Named keys ("Enter", "Tab", "ArrowLeft", "Home", …) are handled
        // explicitly above or elsewhere; they're >1 char and start with an
        // uppercase ASCII letter. A literal typed capital ("A") is one
        // char, so length-1 strings always pass.
        && (key.chars().count() == 1 || !is_named_key(key))
}

/// Heuristic: Dioxus named keys are multi-char PascalCase identifiers
/// (`ArrowLeft`, `PageDown`, `Escape`). A real multi-grapheme text input
/// (an emoji ZWJ sequence, say) contains a non-ASCII char, so we treat a
/// >1-char all-ASCII-alphabetic string as a named key.
fn is_named_key(key: &str) -> bool {
    key.chars().all(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `EditorState` with `text` and the primary caret at byte
    /// `head` (no anchor offset — a collapsed caret).
    fn at(text: &str, head: usize) -> EditorState {
        let s = EditorState::new(text.to_string());
        s.update(TransactionSpec::new().selection(Selection::caret(head)))
    }

    /// Build a state with an explicit (anchor, head) selection.
    fn sel(text: &str, anchor: usize, head: usize) -> EditorState {
        let s = EditorState::new(text.to_string());
        s.update(TransactionSpec::new().selection(Selection::single(Range::new(anchor, head))))
    }

    fn key(k: &str) -> KeySpec {
        KeySpec {
            key: k.to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            r#mod: false,
        }
    }

    fn shift(k: &str) -> KeySpec {
        KeySpec {
            shift: true,
            ..key(k)
        }
    }

    // ── text input ───────────────────────────────────────────────

    #[test]
    fn types_a_char_over_a_collapsed_caret() {
        let st = at("hi", 1);
        let spec = text_input_spec(&st, &key("x")).expect("char is text");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "hxi");
        assert_eq!(next.selection.primary().head, 2);
    }

    #[test]
    fn typing_replaces_a_selection() {
        let st = sel("hello", 1, 4); // "ell" selected
        let spec = text_input_spec(&st, &key("Y")).expect("char is text");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "hYo");
        assert_eq!(next.selection.primary().head, 2);
    }

    #[test]
    fn enter_inserts_newline() {
        let st = at("ab", 1);
        let spec = text_input_spec(&st, &key("Enter")).expect("enter is text");
        assert_eq!(st.update(spec).doc.to_string(), "a\nb");
    }

    #[test]
    fn named_keys_and_modified_keys_are_not_text() {
        assert!(text_input_spec(&at("a", 1), &key("ArrowLeft")).is_none());
        assert!(text_input_spec(&at("a", 1), &key("Escape")).is_none());
        // Ctrl/Meta chords are commands, never literal text.
        let ctrl_a = KeySpec {
            ctrl: true,
            ..key("a")
        };
        assert!(text_input_spec(&at("a", 1), &ctrl_a).is_none());
    }

    #[test]
    fn backspace_deletes_char_backward() {
        let st = at("abc", 2);
        let spec = text_input_spec(&st, &key("Backspace")).expect("backspace edits");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "ac");
        assert_eq!(next.selection.primary().head, 1);
        // At doc start there is nothing to delete.
        assert!(text_input_spec(&at("abc", 0), &key("Backspace")).is_none());
    }

    #[test]
    fn delete_deletes_char_forward() {
        let st = at("abc", 1);
        let spec = text_input_spec(&st, &key("Delete")).expect("delete edits");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "ac");
        assert_eq!(next.selection.primary().head, 1);
        // At doc end there is nothing to delete.
        assert!(text_input_spec(&at("abc", 3), &key("Delete")).is_none());
    }

    #[test]
    fn deletion_keys_eat_a_selection_whole() {
        let mut st = at("hello", 0);
        st.selection = Selection::single(Range::new(1, 4));
        let spec = text_input_spec(&st, &key("Backspace")).expect("selection deletes");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "ho");
        assert_eq!(next.selection.primary().head, 1);
    }

    #[test]
    fn backspace_is_charwise_over_multibyte() {
        let st = at("aé", 3); // 'é' is 2 bytes
        let spec = text_input_spec(&st, &key("Backspace")).expect("backspace edits");
        let next = st.update(spec);
        assert_eq!(next.doc.to_string(), "a");
        assert_eq!(next.selection.primary().head, 1);
    }

    #[test]
    fn unicode_and_emoji_are_text() {
        assert!(text_input_spec(&at("", 0), &key("é")).is_some());
        assert!(text_input_spec(&at("", 0), &key("世")).is_some());
        assert!(text_input_spec(&at("", 0), &key("🚀")).is_some());
    }

    // ── IME ──────────────────────────────────────────────────────

    fn preedit(text: &str) -> ImeEvent {
        ImeEvent::Preedit(text.to_string())
    }

    fn commit(text: &str) -> ImeEvent {
        ImeEvent::Commit(text.to_string())
    }

    #[test]
    fn preedit_composes_without_touching_the_doc() {
        let st = at("ab", 1);
        let out = ime_step(&st, preedit("ni"));
        assert!(out.spec.is_none());
        assert_eq!(out.composition.unwrap().text, "ni");
        let out = ime_step(&st, preedit("你"));
        assert!(out.spec.is_none());
        assert_eq!(out.composition.unwrap().text, "你");
    }

    #[test]
    fn commit_inserts_once_and_ends_composition() {
        let st = at("ab", 1);
        // winit clears the preedit just before it commits.
        let cleared = ime_step(&st, preedit(""));
        assert!(cleared.spec.is_none() && cleared.composition.is_none());
        let out = ime_step(&st, commit("你好"));
        assert!(out.composition.is_none());
        let next = st.update(out.spec.expect("commit edits"));
        assert_eq!(next.doc.to_string(), "a你好b");
        assert_eq!(next.selection.primary().head, 1 + "你好".len());
    }

    #[test]
    fn cancel_leaves_doc_unchanged() {
        let st = at("ab", 1);
        let out = ime_step(&st, preedit(""));
        assert!(out.spec.is_none());
        assert!(out.composition.is_none());
    }

    #[test]
    fn composing_over_a_selection_replaces_it_once() {
        let st = sel("hello", 1, 4);
        let out = ime_step(&st, preedit("k"));
        assert!(out.spec.is_none());
        let next = st.update(ime_step(&st, commit("か")).spec.unwrap());
        assert_eq!(next.doc.to_string(), "hかo");
        assert_eq!(next.selection.primary().head, 1 + "か".len());
    }

    #[test]
    fn commit_is_literal_text() {
        // No bracket auto-pair for an IME commit.
        let st = at("", 0);
        let next = st.update(ime_step(&st, commit("(")).spec.unwrap());
        assert_eq!(next.doc.to_string(), "(");
        // An empty commit edits nothing.
        assert!(ime_step(&st, commit("")).spec.is_none());
    }

    #[test]
    fn preedit_decoration_sits_at_the_head_escaped() {
        let st = at("ab", 1);
        assert!(native_preedit_decoration(&st, None).is_empty());
        let c = Composition {
            text: "<x&>".into(),
        };
        let d = native_preedit_decoration(&st, Some(&c));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].from, 1);
        assert_eq!(
            d[0].kind,
            editor_state::DecorationKind::Widget {
                html: "<span class=\"ed-preedit\">&lt;x&amp;&gt;</span>".into()
            }
        );
    }

    // ── navigation ───────────────────────────────────────────────

    #[test]
    fn arrows_move_by_char() {
        let st = at("abc", 1);
        assert_eq!(nav_target(&st, &key("ArrowRight")).unwrap().head, 2);
        assert_eq!(nav_target(&st, &key("ArrowLeft")).unwrap().head, 0);
    }

    #[test]
    fn arrows_clamp_at_doc_bounds() {
        assert_eq!(
            nav_target(&at("abc", 0), &key("ArrowLeft")).unwrap().head,
            0
        );
        assert_eq!(
            nav_target(&at("abc", 3), &key("ArrowRight")).unwrap().head,
            3
        );
    }

    #[test]
    fn home_and_end_hit_line_edges() {
        // "ab\ncde", caret in the middle of line 2 (byte 5 = between d,e)
        let st = at("ab\ncde", 5);
        assert_eq!(nav_target(&st, &key("Home")).unwrap().head, 3); // start of "cde"
        assert_eq!(nav_target(&st, &key("End")).unwrap().head, 6); // end of doc
        // End on the first line stops before the '\n'.
        assert_eq!(nav_target(&at("ab\ncde", 1), &key("End")).unwrap().head, 2);
    }

    #[test]
    fn vertical_preserves_column() {
        // Two equal-length lines; Down from col 1 of line 0 → col 1 of line 1.
        let st = at("abc\ndef", 1); // after 'a'
        assert_eq!(nav_target(&st, &key("ArrowDown")).unwrap().head, 5); // after 'd'
        let st2 = at("abc\ndef", 5); // after 'd'
        assert_eq!(nav_target(&st2, &key("ArrowUp")).unwrap().head, 1); // after 'a'
    }

    #[test]
    fn vertical_clamps_column_to_shorter_line() {
        // From col 4 of "hello" down to "hi" (len 2) → clamps to end of "hi".
        let st = at("hello\nhi", 4);
        let down = nav_target(&st, &key("ArrowDown")).unwrap().head;
        assert_eq!(down, 8); // "hello\n" = 6 bytes, + "hi" end = 8
    }

    #[test]
    fn shift_arrow_extends_selection() {
        let st = at("abcd", 1);
        let r = nav_target(&st, &shift("ArrowRight")).unwrap();
        assert_eq!(r.anchor, 1, "anchor stays put");
        assert_eq!(r.head, 2, "head moves");
        // Without shift, the range collapses.
        let c = nav_target(&st, &key("ArrowRight")).unwrap();
        assert_eq!(c.anchor, c.head);
    }

    #[test]
    fn non_movement_keys_return_none() {
        assert!(nav_target(&at("a", 0), &key("x")).is_none());
        assert!(nav_target(&at("a", 0), &key("Enter")).is_none());
    }
}
