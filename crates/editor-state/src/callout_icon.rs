//! The icon each callout type wears, as inline SVG.
//!
//! Obsidian draws a Lucide glyph in the callout title bar — a pencil on
//! `[!note]`, a flame on `[!tip]`, a zap on `[!danger]`. The mapping here
//! is Obsidian's own (obsidian.md/Callouts), and the path data is Lucide
//! 0.536.0 verbatim (ISC). The rest of the FTS stack draws the same set
//! through `dioxus-icons::lucide`; this crate has no Dioxus dependency,
//! so it carries the markup directly.
//!
//! `currentColor` on the stroke is the point: the title line already
//! carries the type's colour via `.md-callout-header.md-callout-<kind>`,
//! so the glyph tints itself and a new callout type needs no icon CSS.

/// Inline SVG for a canonical callout kind, ready to drop into a widget
/// decoration. Returns `None` for a kind that isn't one of the thirteen —
/// callers render no icon rather than a placeholder.
///
/// The kind strings are the canonical ones from
/// [`canonical_callout_kind`](crate::markdown::canonical_callout_kind),
/// already alias-resolved: `summary` and `tldr` both arrive as
/// `abstract`.
#[must_use]
pub fn callout_icon(kind: &str) -> Option<&'static str> {
    Some(match kind {
        // lucide `pencil`
        "note" => NOTE,
        // lucide `clipboard-list`
        "abstract" => ABSTRACT,
        // lucide `info`
        "info" => INFO,
        // lucide `circle-check-big`
        "todo" => TODO,
        // lucide `flame`
        "tip" => TIP,
        // lucide `check`
        "success" => SUCCESS,
        // lucide `help-circle`
        "question" => QUESTION,
        // lucide `triangle-alert`
        "warning" => WARNING,
        // lucide `x`
        "failure" => FAILURE,
        // lucide `zap`
        "danger" => DANGER,
        // lucide `bug`
        "bug" => BUG,
        // lucide `list`
        "example" => EXAMPLE,
        // lucide `quote`
        "quote" => QUOTE,
        _ => return None,
    })
}

/// Wrap an icon body in the `<svg>` element the CSS expects.
macro_rules! icon {
    ($body:expr) => {
        concat!(
            "<svg class=\"md-callout-icon\" xmlns=\"http://www.w3.org/2000/svg\" ",
            "viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" ",
            "stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" ",
            "aria-hidden=\"true\">",
            $body,
            "</svg>"
        )
    };
}

const NOTE: &str = icon!(
    "<path d=\"M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z\" /> <path d=\"m15 5 4 4\" />"
);
const ABSTRACT: &str = icon!(
    "<rect width=\"8\" height=\"4\" x=\"8\" y=\"2\" rx=\"1\" ry=\"1\" /> <path d=\"M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2\" /> <path d=\"M12 11h4\" /> <path d=\"M12 16h4\" /> <path d=\"M8 11h.01\" /> <path d=\"M8 16h.01\" />"
);
const INFO: &str = icon!(
    "<circle cx=\"12\" cy=\"12\" r=\"10\" /> <path d=\"M12 16v-4\" /> <path d=\"M12 8h.01\" />"
);
const TODO: &str =
    icon!("<path d=\"M21.801 10A10 10 0 1 1 17 3.335\" /> <path d=\"m9 11 3 3L22 4\" />");
const TIP: &str = icon!(
    "<path d=\"M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z\" />"
);
const SUCCESS: &str = icon!("<path d=\"M20 6 9 17l-5-5\" />");
const QUESTION: &str = icon!(
    "<circle cx=\"12\" cy=\"12\" r=\"10\" /> <path d=\"M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3\" /> <path d=\"M12 17h.01\" />"
);
const WARNING: &str = icon!(
    "<path d=\"m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3\" /> <path d=\"M12 9v4\" /> <path d=\"M12 17h.01\" />"
);
const FAILURE: &str = icon!("<path d=\"M18 6 6 18\" /> <path d=\"m6 6 12 12\" />");
const DANGER: &str = icon!(
    "<path d=\"M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z\" />"
);
const BUG: &str = icon!(
    "<path d=\"m8 2 1.88 1.88\" /> <path d=\"M14.12 3.88 16 2\" /> <path d=\"M9 7.13v-1a3.003 3.003 0 1 1 6 0v1\" /> <path d=\"M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6\" /> <path d=\"M12 20v-9\" /> <path d=\"M6.53 9C4.6 8.8 3 7.1 3 5\" /> <path d=\"M6 13H2\" /> <path d=\"M3 21c0-2.1 1.7-3.9 3.8-4\" /> <path d=\"M20.97 5c0 2.1-1.6 3.8-3.5 4\" /> <path d=\"M22 13h-4\" /> <path d=\"M17.2 17c2.1.1 3.8 1.9 3.8 4\" />"
);
const EXAMPLE: &str = icon!(
    "<path d=\"M3 12h.01\" /> <path d=\"M3 18h.01\" /> <path d=\"M3 6h.01\" /> <path d=\"M8 12h13\" /> <path d=\"M8 18h13\" /> <path d=\"M8 6h13\" />"
);
const QUOTE: &str = icon!(
    "<path d=\"M16 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z\" /> <path d=\"M5 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z\" />"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_callout_kind_has_an_icon() {
        for kind in [
            "note", "abstract", "info", "todo", "tip", "success", "question", "warning", "failure",
            "danger", "bug", "example", "quote",
        ] {
            let svg = callout_icon(kind).unwrap_or_else(|| panic!("no icon for {kind}"));
            assert!(svg.starts_with("<svg"), "{kind}: {svg}");
            assert!(svg.ends_with("</svg>"), "{kind}: {svg}");
            assert!(svg.contains("currentColor"), "{kind} must tint itself");
        }
    }

    #[test]
    fn an_unknown_kind_has_no_icon() {
        assert!(callout_icon("nosuchtype").is_none());
    }
}
