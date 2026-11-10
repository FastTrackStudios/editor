//! Diagnostics as data: what a host (an LSP client, a linter, a
//! compiler) hands the editor to draw.
//!
//! Pure data, no protocol: `editor-lsp` resolves a server's
//! `publishDiagnostics` into these ([`editor_lsp::diagnostics::resolve`]
//! re-exports the same types), and the code editor draws them
//! ([`to_decorations`] for the underlines, [`crate::code`] for the gutter
//! marks).
//!
//! [`editor_lsp::diagnostics::resolve`]: ../../editor_lsp/diagnostics/fn.resolve.html

use crate::decoration::{DecoratedRange, Decoration};

/// Severity, ordered most→least severe. Plain enum so hosts don't
/// need lsp-types to consume diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    /// CSS class the view layer styles (squiggle color etc.).
    #[must_use]
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Error => "cm-lsp-error",
            Self::Warning => "cm-lsp-warning",
            Self::Information => "cm-lsp-info",
            Self::Hint => "cm-lsp-hint",
        }
    }

    /// `error`, `warning`, `info`, `hint`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }
}

/// One diagnostic: a byte range into the current document plus display
/// data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnostic {
    /// Byte range into the document. A zero-width range is drawn under
    /// the character after it (`editor-lsp` widens server ranges when it
    /// resolves them; [`to_decorations`] does the same for any other).
    pub from: usize,
    pub to: usize,
    pub severity: Severity,
    pub message: String,
    /// Producer name (`"rustc"`, `"rust-analyzer"`, …).
    pub source: Option<String>,
    /// Diagnostic code (`"E0308"`, `"unused_variables"`, …).
    pub code: Option<String>,
}

impl Diagnostic {
    /// A diagnostic over `range` with no source or code.
    #[must_use]
    pub fn new(
        range: std::ops::Range<usize>,
        severity: Severity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            from: range.start,
            to: range.end,
            severity,
            message: message.into(),
            source: None,
            code: None,
        }
    }

    /// The text shown on hover: `message` with `[source code]` after it
    /// when they are known.
    #[must_use]
    pub fn label(&self) -> String {
        match (&self.source, &self.code) {
            (Some(s), Some(c)) => format!("{} [{s} {c}]", self.message),
            (Some(x), None) | (None, Some(x)) => format!("{} [{x}]", self.message),
            (None, None) => self.message.clone(),
        }
    }
}

/// Map diagnostics to view decorations: a [`Decoration::mark`] per
/// diagnostic with the severity class plus a shared `cm-lsp-diagnostic`
/// class.
///
/// The message is carried in a `data-lsp-message` attribute for
/// tooltip/hover use, and in `title`. A range past the end of a document
/// of `doc_len` bytes is clamped; a zero-width one covers one byte after
/// it (or before it, at the end) so the underline has something to sit
/// under. Pass `usize::MAX` when the length is not known.
#[must_use]
pub fn to_decorations(diagnostics: &[Diagnostic], doc_len: usize) -> Vec<DecoratedRange> {
    diagnostics
        .iter()
        .filter_map(|d| {
            let mut from = d.from.min(d.to).min(doc_len);
            let mut to = d.to.max(d.from).min(doc_len);
            if from == to {
                if to < doc_len {
                    to = to.saturating_add(1);
                } else if from > 0 {
                    from = from.saturating_sub(1);
                } else {
                    return None;
                }
            }
            Some(Decoration::mark_with_attrs(
                from..to,
                format!("cm-lsp-diagnostic {}", d.severity.css_class()),
                vec![
                    ("data-lsp-message".to_owned(), d.message.clone()),
                    ("title".to_owned(), d.label()),
                ],
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoration::DecorationKind;

    #[test]
    fn a_diagnostic_is_a_mark_with_its_severity_and_message() {
        let d = Diagnostic::new(2..5, Severity::Warning, "unused");
        let decs = to_decorations(&[d], 10);
        assert_eq!(decs.len(), 1);
        let DecorationKind::Mark { class, attrs } = &decs[0].kind else {
            panic!("a mark");
        };
        assert_eq!((decs[0].from, decs[0].to), (2, 5));
        assert_eq!(class, "cm-lsp-diagnostic cm-lsp-warning");
        assert!(attrs.contains(&("title".to_owned(), "unused".to_owned())));
    }

    #[test]
    fn zero_width_and_out_of_range_diagnostics_still_show() {
        let decs = to_decorations(
            &[
                Diagnostic::new(3..3, Severity::Error, "here"),
                Diagnostic::new(8..40, Severity::Error, "past the end"),
                Diagnostic::new(10..10, Severity::Error, "at the end"),
            ],
            10,
        );
        let ranges: Vec<_> = decs.iter().map(|d| (d.from, d.to)).collect();
        assert_eq!(ranges, vec![(3, 4), (8, 10), (9, 10)]);
        assert_eq!(
            to_decorations(&[Diagnostic::new(0..0, Severity::Hint, "x")], 0),
            Vec::new(),
            "nothing to underline in an empty document"
        );
    }

    #[test]
    fn the_label_carries_source_and_code() {
        let mut d = Diagnostic::new(0..1, Severity::Error, "mismatched types");
        d.source = Some("rustc".into());
        d.code = Some("E0308".into());
        assert_eq!(d.label(), "mismatched types [rustc E0308]");
    }
}
