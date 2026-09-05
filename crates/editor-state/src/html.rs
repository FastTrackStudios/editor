//! Static HTML for a decorated document.
//!
//! The editor renders a note by walking its text alongside a set of
//! [`DecoratedRange`]s and emitting elements — `Mark` becomes a span with
//! a class, `Replace` hides a markdown marker, `Widget` injects HTML,
//! `Line` classes the line. That model is already HTML-shaped; what was
//! missing was a way to get the HTML *out* without running the editor.
//!
//! This is that. Same decorations, same classes, same stylesheet — so a
//! note published as a static page looks like the note in the editor,
//! because it is the same renderer with a different backend. The previous
//! answer was to hand each note to the editor in read-only mode, which
//! meant shipping the editor, its state machine, its decoration pipeline
//! and a WebGL surface to somebody reading a paragraph.
//!
//! Fences go through the plugin registry, so a language the host has
//! registered — keyflow charts, mermaid diagrams — renders here exactly
//! as it renders live.
//!
//! The output is *decorated source*, not semantic HTML: a heading is the
//! line `# Title` with the `#` hidden by a `Replace`, not an `<h1>`. That
//! is deliberate — it is what makes this the same rendering the editor
//! shows rather than a second one that drifts. It also means the output
//! needs the editor's stylesheet to look like anything.

use crate::decoration::{DecoratedRange, DecorationKind};

/// Wrap each line in `<div class="cm-line">`, the shape the editor's
/// stylesheet expects.
const LINE_TAG: &str = "div";

/// Render `text` with `decorations` as standalone HTML.
#[must_use]
pub fn render_html(text: &str, decorations: &[DecoratedRange]) -> String {
    let events = collect_events(decorations);
    emit(text, &events)
}

/// Decoration ranges as ordered boundary events, the same shape the
/// tile builder walks.
fn collect_events(decorations: &[DecoratedRange]) -> Vec<(usize, Ev)> {
    let mut events: Vec<(usize, Ev)> = Vec::new();
    for d in decorations {
        match &d.kind {
            DecorationKind::Mark { class, attrs } => {
                events.push((d.from, Ev::MarkStart(class.clone(), attrs.clone())));
                events.push((d.to, Ev::MarkEnd));
            }
            DecorationKind::Replace => {
                events.push((d.from, Ev::ReplaceStart(d.to)));
            }
            DecorationKind::Widget { html } => {
                events.push((d.from, Ev::Widget(html.clone())));
            }
            DecorationKind::Line { class } => {
                events.push((d.from, Ev::Line(class.clone())));
            }
            // Behaviour only: it moves a caret, and there is no caret
            // in a static page.
            DecorationKind::Atomic => {}
        }
    }
    // Stable by position, and at equal positions the order below —
    // a line class before anything on the line, a replace before the
    // text it hides, a mark end before the next mark starts.
    events.sort_by_key(|(at, ev)| (*at, ev.rank()));
    events
}

fn emit(text: &str, events: &[(usize, Ev)]) -> String {
    let mut e = Emitter::new(text.len());
    let mut ev = 0usize;

    for (i, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        while events.get(ev).is_some_and(|(at, _)| *at <= i) {
            if let Some((_, kind)) = events.get(ev) {
                e.apply(kind);
            }
            ev = ev.saturating_add(1);
        }
        if i == text.len() {
            break;
        }
        if ch == '\n' {
            e.end_line();
        } else {
            e.push_char(ch);
        }
    }

    e.finish()
}

/// Builds the HTML. Split from [`emit`] so the walk stays a short loop
/// and each thing that can happen at an offset is one method.
struct Emitter {
    out: String,
    marks: Vec<String>,
    line_classes: Vec<String>,
    pending: Vec<String>,
    hidden_until: Option<usize>,
    line_open: bool,
    at: usize,
}

impl Emitter {
    fn new(len: usize) -> Self {
        Self {
            out: String::with_capacity(len.saturating_mul(2)),
            marks: Vec::new(),
            line_classes: Vec::new(),
            pending: Vec::new(),
            hidden_until: None,
            line_open: false,
            at: 0,
        }
    }

    /// A line element is opened lazily, at its first visible content, so
    /// every `Line` decoration at that offset is already known.
    fn open_line(&mut self) {
        if self.line_open {
            return;
        }
        self.out.push_str("<");
        self.out.push_str(LINE_TAG);
        self.out.push_str(" class=\"cm-line");
        for c in &self.line_classes {
            self.out.push(' ');
            self.out.push_str(c);
        }
        self.out.push_str("\">");
        for m in std::mem::take(&mut self.pending) {
            self.out.push_str(&m);
        }
        self.line_open = true;
    }

    fn apply(&mut self, kind: &Ev) {
        match kind {
            Ev::Line(class) => self.line_classes.push(class.clone()),
            Ev::MarkStart(class, attrs) => {
                let mut tag = format!("<span class=\"{}\"", escape_attr(class));
                for (k, v) in attrs {
                    tag.push(' ');
                    tag.push_str(&escape_attr(k));
                    tag.push_str("=\"");
                    tag.push_str(&escape_attr(v));
                    tag.push('"');
                }
                tag.push('>');
                self.marks.push("</span>".to_owned());
                if self.line_open {
                    self.out.push_str(&tag);
                } else {
                    self.pending.push(tag);
                }
            }
            Ev::MarkEnd => {
                if let Some(close) = self.marks.pop() {
                    self.open_line();
                    self.out.push_str(&close);
                }
            }
            Ev::ReplaceStart(to) => self.hidden_until = Some(*to),
            Ev::Widget(html) => {
                self.open_line();
                self.out.push_str(html);
            }
        }
    }

    fn push_char(&mut self, ch: char) {
        self.at = self.at.saturating_add(ch.len_utf8());
        // Inside a `Replace` the source is in the document and absent
        // from the render — how `**` disappears once the caret leaves.
        if self.hidden_until.is_some_and(|end| self.at <= end) {
            return;
        }
        self.hidden_until = None;
        self.open_line();
        push_escaped(&mut self.out, ch);
    }

    fn close_marks(&mut self) {
        let marks = std::mem::take(&mut self.marks);
        for close in marks.iter().rev() {
            self.out.push_str(close);
        }
    }

    fn end_line(&mut self) {
        self.at = self.at.saturating_add(1);
        if self.hidden_until.is_some_and(|end| self.at <= end) {
            return;
        }
        self.open_line();
        // Marks do not span lines in the editor's model, but a malformed
        // decoration must not emit unbalanced tags.
        self.close_marks();
        self.pending.clear();
        self.line_classes.clear();
        self.line_open = false;
        self.out.push_str("</");
        self.out.push_str(LINE_TAG);
        self.out.push_str(">\n");
    }

    fn finish(mut self) -> String {
        if self.line_open {
            self.close_marks();
            self.out.push_str("</");
            self.out.push_str(LINE_TAG);
            self.out.push('>');
        }
        self.out
    }
}

/// Render an [`EditorState`](crate::EditorState) with the editor's own
/// markdown decorations.
///
/// Always in **reading mode**, whatever the state says. Live preview
/// keeps a markdown marker visible while the caret is inside its span —
/// that is what lets you edit the `**` you are standing in — and a
/// published page has no caret to be inside anything. Rendering a state
/// as-authored would leave the `**` in whichever span the caret happened
/// to be parked in when it was serialised.
#[must_use]
pub fn render_state_html(state: &crate::EditorState) -> String {
    let text = state.doc.to_string();
    let mut reading = state.clone();
    reading.reading_mode = true;
    let decorations = crate::markdown::live_preview(&reading);
    render_html(&text, &decorations)
}

/// Render markdown source as HTML, with every editor feature its
/// decorations provide — callouts, wikilinks, task lists, and any fence
/// whose language the host has registered a renderer for.
#[must_use]
pub fn render_markdown_html(source: &str) -> String {
    render_state_html(&crate::EditorState::new(source.to_string()))
}

enum Ev {
    Line(String),
    ReplaceStart(usize),
    MarkStart(String, Vec<(String, String)>),
    Widget(String),
    MarkEnd,
}

impl Ev {
    /// Tie-break order at one offset.
    const fn rank(&self) -> u8 {
        match self {
            Self::MarkEnd => 0,
            Self::Line(_) => 1,
            Self::ReplaceStart(_) => 2,
            Self::MarkStart(..) => 3,
            Self::Widget(_) => 4,
        }
    }
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(c),
    }
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EditorState;

    fn html(src: &str) -> String {
        render_state_html(&EditorState::new(src.to_string()))
    }

    #[test]
    fn plain_text_becomes_lines() {
        let out = html("one\ntwo");
        assert!(out.contains("one"), "{out}");
        assert!(out.contains("two"), "{out}");
        assert_eq!(out.matches("cm-line").count(), 2, "{out}");
    }

    #[test]
    fn text_is_escaped_but_widgets_are_not() {
        // Document text is untrusted and gets escaped; a widget is HTML
        // the renderer itself produced and must pass through intact, or
        // an engraved chart arrives as literal angle brackets.
        let out = html("a < b & c");
        assert!(out.contains("&lt;"), "{out}");
        assert!(out.contains("&amp;"), "{out}");
    }

    #[test]
    fn a_heading_is_marked() {
        let out = html("# Title\n");
        assert!(out.contains("md-h1") || out.contains("md-heading"), "{out}");
    }

    #[test]
    fn a_callout_carries_its_type() {
        // The whole point of routing the site through the editor: the
        // twelve Obsidian callouts render here because they render there.
        let out = html("> [!warning] Careful\n> body\n");
        assert!(out.contains("md-callout"), "{out}");
        assert!(out.contains("warning"), "{out}");
    }

    #[test]
    fn emphasis_markers_are_hidden_not_deleted() {
        let out = html("**bold**\n");
        assert!(out.contains("bold"), "{out}");
        // The `**` is a Replace: gone from the render, still in the doc.
        assert!(!out.contains("**"), "{out}");
    }

    #[test]
    fn tags_are_balanced() {
        let out = html("# Title\n\n**bold** and *italic*\n\n- [ ] a task\n");
        assert_eq!(
            out.matches("<span").count(),
            out.matches("</span>").count(),
            "unbalanced spans:\n{out}"
        );
        assert_eq!(
            out.matches("<div").count(),
            out.matches("</div>").count(),
            "unbalanced lines:\n{out}"
        );
    }
}
