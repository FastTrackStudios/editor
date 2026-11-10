//! Editing code: the pure half of the view's `CodeEditor`.
//!
//! - [`decorations`]: a whole buffer highlighted in one language (the
//!   same `md-tok-*` classes fences use, so every theme styles it), line
//!   numbers as gutter markers, the bracket pair at the caret, and the
//!   host's diagnostics as underlines with a mark in the gutter.
//! - Commands for the code keymap ([`keymap`]): [`newline_and_indent`]
//!   (Enter keeps the line's indent, one more after an opening bracket,
//!   and splits `{}` onto three lines), [`indent`] (Tab) and
//!   [`dedent`] (Shift-Tab).

use std::cell::RefCell;

use crate::change::Changes;
use crate::command::Keymap;
use crate::commands::{INDENT_UNIT, indent_less, indent_more};
use crate::decoration::{DecoratedRange, Decoration};
use crate::diagnostic::{Diagnostic, Severity};
use crate::selection::Selection;
use crate::state::EditorState;
use crate::transaction::TransactionSpec;
use editor_syntax::{Syntax, Token};

/// The class on every line-number gutter marker.
pub const LINE_NUMBER_CLASS: &str = "code-ln";

/// What [`decorations`] draws.
#[derive(Debug, Clone, Copy)]
pub struct CodeView<'a> {
    /// The buffer's language; `None` is plain text.
    pub language: Option<&'a Syntax>,
    /// The host's diagnostics, byte ranges into the buffer.
    pub diagnostics: &'a [Diagnostic],
    /// A number before every line.
    pub line_numbers: bool,
}

/// Everything the code editor draws for `state`.
#[must_use]
pub fn decorations(state: &EditorState, view: CodeView<'_>) -> Vec<DecoratedRange> {
    let text = state.doc.to_string();
    let mut out = Vec::new();
    if let Some(syntax) = view.language {
        out.extend(syntax_marks(syntax, &text));
    }
    out.extend(crate::bracket_match::bracket_match(state));
    out.extend(crate::diagnostic::to_decorations(
        view.diagnostics,
        text.len(),
    ));
    out.extend(gutter(&text, view.diagnostics, view.line_numbers));
    out
}

/// `text` highlighted as `syntax`: one `md-tok-<tag>` mark per token.
///
/// The last buffer's tokens are kept (per thread), so a pass that runs
/// again on the same text (a selection change, the view's second pass)
/// does not parse again.
#[must_use]
pub fn syntax_marks(syntax: &Syntax, text: &str) -> Vec<DecoratedRange> {
    thread_local! {
        static LAST: RefCell<Option<(Syntax, String, Vec<Token>)>> = const { RefCell::new(None) };
    }
    let tokens = LAST.with(|last| {
        let mut last = last.borrow_mut();
        if let Some((s, t, toks)) = last.as_ref()
            && s == syntax
            && t == text
        {
            return toks.clone();
        }
        let toks = syntax.highlight(text);
        *last = Some((syntax.clone(), text.to_owned(), toks.clone()));
        toks
    });
    tokens
        .into_iter()
        .map(|t| Decoration::mark(t.start..t.end, format!("md-tok-{}", t.tag)))
        .collect()
}

/// One gutter marker per line: its number (when `line_numbers`), marked
/// with the worst severity of the diagnostics that start on it
/// (`code-ln code-ln-error`). Without line numbers only lines with a
/// diagnostic get a marker, a dot.
#[must_use]
pub fn gutter(text: &str, diagnostics: &[Diagnostic], line_numbers: bool) -> Vec<DecoratedRange> {
    let starts = line_starts(text);
    let mut worst: Vec<Option<Severity>> = vec![None; starts.len()];
    for d in diagnostics {
        let at = d.from.min(d.to).min(text.len());
        let line = starts.partition_point(|&s| s <= at).saturating_sub(1);
        if let Some(slot) = worst.get_mut(line) {
            *slot = Some(slot.map_or(d.severity, |w| w.min(d.severity)));
        }
    }
    let mut out = Vec::new();
    for (i, (&start, sev)) in starts.iter().zip(worst).enumerate() {
        let class = sev.map_or_else(
            || LINE_NUMBER_CLASS.to_owned(),
            |s| format!("{LINE_NUMBER_CLASS} {LINE_NUMBER_CLASS}-{}", s.name()),
        );
        if line_numbers {
            let n = i.saturating_add(1).to_string();
            out.push(DecoratedRange::gutter(start, &n, &class));
        } else if sev.is_some() {
            out.push(DecoratedRange::gutter(start, "●", &class));
        }
    }
    out
}

/// The byte offset of every line's start (the first is 0).
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i.saturating_add(1)),
        )
        .collect()
}

const OPENERS: &[(u8, u8)] = &[(b'{', b'}'), (b'(', b')'), (b'[', b']')];

/// Enter in code: a newline that keeps the current line's indent, adds
/// one [`INDENT_UNIT`] after an opening bracket, and puts a closing
/// bracket right after the caret on a line of its own (`{|}` →
/// `{\n  |\n}`).
#[must_use]
pub fn newline_and_indent(state: &EditorState) -> Option<TransactionSpec> {
    let p = state.selection.primary();
    let (from, to) = (p.from(), p.to());
    let text = state.doc.to_string();
    let bytes = text.as_bytes();
    let line_start = text
        .get(..from)?
        .rfind('\n')
        .map_or(0, |i| i.saturating_add(1));
    let indent_end = bytes
        .get(line_start..from)?
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .map_or(from, |n| line_start.saturating_add(n));
    let indent = text.get(line_start..indent_end)?;
    let before = text.get(line_start..from)?.trim_end();
    let opener = before
        .as_bytes()
        .last()
        .and_then(|b| OPENERS.iter().find(|(o, _)| o == b));
    let mut inserted = format!("\n{indent}");
    if let Some(&(_, close)) = opener {
        inserted.push_str(INDENT_UNIT);
        let caret = from.saturating_add(inserted.len());
        if bytes.get(to) == Some(&close) {
            inserted.push('\n');
            inserted.push_str(indent);
        }
        return Some(
            TransactionSpec::new()
                .changes(Changes::replace(from..to, inserted))
                .selection(Selection::caret(caret)),
        );
    }
    let caret = from.saturating_add(inserted.len());
    Some(
        TransactionSpec::new()
            .changes(Changes::replace(from..to, inserted))
            .selection(Selection::caret(caret)),
    )
}

/// Tab in code: with a caret, insert one [`INDENT_UNIT`]; with a
/// selection, indent every line it touches.
#[must_use]
pub fn indent(state: &EditorState) -> Option<TransactionSpec> {
    let p = state.selection.primary();
    if p.from() != p.to() {
        return indent_more(state);
    }
    let at = p.head;
    Some(
        TransactionSpec::new()
            .changes(Changes::insert(at, INDENT_UNIT))
            .selection(Selection::caret(at.saturating_add(INDENT_UNIT.len()))),
    )
}

/// Shift-Tab in code: outdent every line the selection touches. A line
/// with no indent is left alone, and the key is still taken (it does not
/// move focus out of the editor).
#[must_use]
pub fn dedent(state: &EditorState) -> Option<TransactionSpec> {
    indent_less(state).or_else(|| Some(TransactionSpec::new()))
}

fn undo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("undo"))
}

fn redo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("redo"))
}

/// The code editor's keys: Enter, Tab and Shift-Tab as above, undo and
/// redo (`Mod-z`, `Mod-Shift-z`, `Mod-y`) and select all (`Mod-a`).
#[must_use]
pub fn keymap() -> Keymap {
    Keymap::new()
        .with("Enter", newline_and_indent)
        .with("Shift-Tab", dedent)
        .with("Tab", indent)
        .with("Mod-Shift-z", redo)
        .with("Mod-z", undo)
        .with("Mod-y", redo)
        .with("Mod-a", crate::commands::select_all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoration::DecorationKind;

    fn at(text: &str, caret: usize) -> EditorState {
        let mut s = EditorState::new(text);
        s.selection = Selection::caret(caret);
        s
    }

    fn apply(state: &EditorState, spec: TransactionSpec) -> (String, usize) {
        let next = state.update(spec);
        let head = next.selection.primary().head;
        (next.doc.to_string(), head)
    }

    fn gutters(decs: &[DecoratedRange]) -> Vec<(usize, String, String)> {
        decs.iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::Widget { html } => {
                    crate::decoration::gutter_of(html).map(|(class, text)| (d.from, class, text))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn enter_keeps_the_indent() {
        let s = at("    let x = 1;", 14);
        let (doc, head) = apply(&s, newline_and_indent(&s).unwrap());
        assert_eq!(doc, "    let x = 1;\n    ");
        assert_eq!(head, doc.len());
    }

    #[test]
    fn enter_after_an_opener_indents_once_more_and_splits_a_pair() {
        let s = at("fn f() {}", 8);
        let (doc, head) = apply(&s, newline_and_indent(&s).unwrap());
        assert_eq!(doc, "fn f() {\n  \n}");
        assert_eq!(head, 11, "the caret on the indented middle line");

        let s = at("  call(", 7);
        let (doc, _) = apply(&s, newline_and_indent(&s).unwrap());
        assert_eq!(doc, "  call(\n    ");
    }

    #[test]
    fn tab_inserts_a_unit_and_shift_tab_takes_it_back() {
        let s = at("x", 0);
        let (doc, head) = apply(&s, indent(&s).unwrap());
        assert_eq!((doc.as_str(), head), ("  x", 2));
        let s = at(&doc, head);
        let (doc, _) = apply(&s, dedent(&s).unwrap());
        assert_eq!(doc, "x");
        let s = at("x", 0);
        let (doc, _) = apply(&s, dedent(&s).expect("still taken"));
        assert_eq!(doc, "x", "nothing to outdent");
    }

    #[test]
    fn tab_over_a_selection_indents_every_line() {
        let mut s = EditorState::new("a\nb\nc");
        s.selection = Selection::single(crate::selection::Range::new(0, 3));
        let (doc, _) = apply(&s, indent(&s).unwrap());
        assert_eq!(doc, "  a\n  b\nc");
    }

    #[test]
    fn every_line_gets_a_number_and_a_diagnostic_marks_its_line() {
        let text = "a\nbb\n\nc";
        let diags = [
            Diagnostic::new(3..4, Severity::Warning, "w"),
            Diagnostic::new(2..3, Severity::Error, "e"),
        ];
        let g = gutters(&gutter(text, &diags, true));
        assert_eq!(
            g,
            vec![
                (0, "editor-gutter code-ln".to_owned(), "1".to_owned()),
                (
                    2,
                    "editor-gutter code-ln code-ln-error".to_owned(),
                    "2".to_owned()
                ),
                (5, "editor-gutter code-ln".to_owned(), "3".to_owned()),
                (6, "editor-gutter code-ln".to_owned(), "4".to_owned()),
            ]
        );
        let g = gutters(&gutter(text, &diags, false));
        assert_eq!(
            g,
            vec![(
                2,
                "editor-gutter code-ln code-ln-error".to_owned(),
                "●".to_owned()
            )]
        );
    }

    #[test]
    fn decorations_highlight_the_whole_buffer_and_match_brackets() {
        let rust = Syntax::Builtin(editor_syntax::Lang::Rust);
        let s = at("fn main() {}", 11);
        let decs = decorations(
            &s,
            CodeView {
                language: Some(&rust),
                diagnostics: &[],
                line_numbers: false,
            },
        );
        let marks: Vec<(usize, usize, &str)> = decs
            .iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::Mark { class, .. } => Some((d.from, d.to, class.as_str())),
                _ => None,
            })
            .collect();
        assert!(marks.contains(&(0, 2, "md-tok-k")), "{marks:?}");
        assert!(marks.contains(&(10, 11, "md-bracket-match")), "{marks:?}");
        assert!(gutters(&decs).is_empty(), "no line numbers asked for");
    }
}
