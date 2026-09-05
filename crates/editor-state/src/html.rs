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
    emit(text, &events, None)
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

fn emit(
    text: &str,
    events: &[(usize, Ev)],
    link_href: Option<&dyn Fn(&str) -> Option<String>>,
) -> String {
    let mut e = Emitter::new(text.len(), link_href);
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
/// Is this one of the six heading line classes?
fn is_heading_class(class: &str) -> bool {
    matches!(
        class,
        "md-h1" | "md-h2" | "md-h3" | "md-h4" | "md-h5" | "md-h6"
    )
}

/// GitHub's heading-anchor slug: lowercase, spaces to hyphens, and every
/// other character that isn't alphanumeric, `-` or `_` dropped.
///
/// Matching GitHub matters because it is also what `ssg-vault` generates
/// on the static-site side; a link that works in the wiki has to work in
/// the editor's own HTML too.
fn slugify(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if ch.is_whitespace() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

struct Emitter<'a> {
    link_href: Option<&'a dyn Fn(&str) -> Option<String>>,
    out: String,
    marks: Vec<String>,
    line_classes: Vec<String>,
    pending: Vec<String>,
    hidden_until: Option<usize>,
    line_open: bool,
    /// Byte index in `out` where the open tag's attribute list can be
    /// extended, and the visible text seen on this line so far. Together
    /// they let [`Self::end_line`] give a heading an `id` derived from
    /// its own text — the anchor in-page links and a table of contents
    /// need, which cannot be known when the tag is opened.
    line_attr_at: usize,
    line_text: String,
    at: usize,
}

impl<'a> Emitter<'a> {
    fn new(len: usize, link_href: Option<&'a dyn Fn(&str) -> Option<String>>) -> Self {
        Self {
            link_href,
            out: String::with_capacity(len.saturating_mul(2)),
            marks: Vec::new(),
            line_classes: Vec::new(),
            pending: Vec::new(),
            hidden_until: None,
            line_open: false,
            line_attr_at: 0,
            line_text: String::new(),
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
        self.line_attr_at = self.out.len();
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
                // A wikilink becomes a real anchor when the host can say
                // where it points. The editor's own markup is a span the
                // app makes clickable with a listener, which on a static
                // page is a link that does nothing.
                let href = self.link_href.and_then(|f| {
                    class.contains("md-wikilink").then(|| {
                        attrs
                            .iter()
                            .find(|(k, _)| k == "data-href")
                            .and_then(|(_, target)| f(target))
                    })?
                });
                let (elem, close) = if href.is_some() {
                    ("a", "</a>")
                } else {
                    ("span", "</span>")
                };
                let mut tag = format!("<{elem} class=\"{}\"", escape_attr(class));
                if let Some(href) = &href {
                    tag.push_str(" href=\"");
                    tag.push_str(&escape_attr(href));
                    tag.push('"');
                }
                for (k, v) in attrs {
                    tag.push(' ');
                    tag.push_str(&escape_attr(k));
                    tag.push_str("=\"");
                    tag.push_str(&escape_attr(v));
                    tag.push('"');
                }
                tag.push('>');
                self.marks.push(close.to_owned());
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
        self.line_text.push(ch);
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
        self.stamp_heading_id();
        self.pending.clear();
        self.line_classes.clear();
        self.line_open = false;
        self.out.push_str("</");
        self.out.push_str(LINE_TAG);
        // No separator between line elements. `.editor-root` is
        // `white-space: pre-wrap`, so a newline here survives as an
        // anonymous line box and paints a phantom blank row between
        // every line — splitting callouts into two bars. The live
        // editor builds its DOM with no text nodes between lines;
        // this path must match it exactly.
        self.out.push('>');
    }

    /// Give a heading line an `id` slugged from its own text, so in-page
    /// links and a generated table of contents have something to point
    /// at. Runs at line close because the text is only known then — and
    /// from both close paths, since a document with no trailing newline
    /// ends in [`Self::finish`] rather than [`Self::end_line`].
    fn stamp_heading_id(&mut self) {
        if self.line_classes.iter().any(|c| is_heading_class(c)) {
            let slug = slugify(&self.line_text);
            if !slug.is_empty() {
                self.out
                    .insert_str(self.line_attr_at, &format!("\" id=\"{slug}"));
            }
        }
        self.line_text.clear();
    }

    fn finish(mut self) -> String {
        if self.line_open {
            self.close_marks();
            self.stamp_heading_id();
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
///
/// Wikilinks render, but every one of them is unresolved: with no vault
/// to ask, the renderer cannot know whether `[[Header]]` names a page
/// that exists. Use [`render_markdown_html_with`] where that matters,
/// which for a published site is always.
#[must_use]
pub fn render_markdown_html(source: &str) -> String {
    render_state_html(&crate::EditorState::new(source.to_string()))
}

/// Render markdown source as HTML, resolving wikilinks against `vault`.
///
/// The same lookup the live editor uses, so a link that resolves to a
/// card in the editor resolves to one on the page.
#[must_use]
pub fn render_markdown_html_with(source: &str, vault: &dyn crate::markdown::VaultLookup) -> String {
    render_markdown_html_opts(source, &HtmlOptions::new().vault(vault))
}

/// How to render a document to HTML.
#[derive(Default)]
pub struct HtmlOptions<'a> {
    vault: Option<&'a dyn crate::markdown::VaultLookup>,
    link_href: Option<&'a dyn Fn(&str) -> Option<String>>,
}

impl<'a> HtmlOptions<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            vault: None,
            link_href: None,
        }
    }

    /// Resolve `[[wikilinks]]` and embeds against this vault.
    #[must_use]
    pub const fn vault(mut self, vault: &'a dyn crate::markdown::VaultLookup) -> Self {
        self.vault = Some(vault);
        self
    }

    /// Turn wikilinks into real anchors, given the target name.
    ///
    /// Without this a wikilink renders the way the editor renders it: a
    /// `<span>` carrying `data-href`, which the app makes clickable with
    /// a listener. On a page with no listener that is not a link at all —
    /// it looks like one and does nothing. Returning `None` for a name
    /// leaves the span alone, which is the honest rendering for a target
    /// that does not exist.
    #[must_use]
    pub const fn link_href(mut self, f: &'a dyn Fn(&str) -> Option<String>) -> Self {
        self.link_href = Some(f);
        self
    }
}

/// Render markdown to HTML under `opts`.
#[must_use]
pub fn render_markdown_html_opts(source: &str, opts: &HtmlOptions<'_>) -> String {
    let mut state = crate::EditorState::new(source.to_string());
    state.reading_mode = true;
    let text = state.doc.to_string();
    let decorations = crate::markdown::live_preview_with(&state, opts.vault);
    let events = collect_events(&decorations);
    emit(&text, &events, opts.link_href)
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
    fn an_image_becomes_an_img_element() {
        let out = html("![alt](pic.png)");
        assert!(out.contains(r#"src="pic.png""#), "{out}");
        // Inline, not the block treatment `![[embeds]]` get.
        assert!(out.contains("md-image-inline"), "{out}");
        // The bang belongs to the syntax, not the text.
        assert!(!out.contains(">!"), "the bang leaked:\n{out}");
    }

    #[test]
    fn a_link_title_does_not_land_in_the_href() {
        let out = html(r#"[a](u "T")"#);
        assert!(out.contains(r#"data-href="u""#), "{out}");
        assert!(out.contains(r#"title="T""#), "{out}");
    }

    #[test]
    fn a_sub_list_carries_its_depth() {
        let out = html("- a\n  - b\n    - c");
        assert!(out.contains("md-list-depth-1"), "{out}");
        assert!(out.contains("md-list-depth-2"), "{out}");
    }

    #[test]
    fn a_list_inside_a_quote_is_still_a_list() {
        let out = html("> - a\n> - b");
        assert!(out.contains("md-list-marker"), "{out}");
    }

    #[test]
    fn a_table_keeps_its_column_alignment() {
        let out = html("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |");
        assert!(out.contains("text-align:left"), "{out}");
        assert!(out.contains("text-align:center"), "{out}");
        assert!(out.contains("text-align:right"), "{out}");
    }

    #[test]
    fn a_heading_gets_an_anchor_id() {
        // Slugged GitHub-style, and present on a document whose last
        // line has no trailing newline.
        assert!(html("## My Heading!").contains(r#"id="my-heading""#));
        assert!(html("## A\n").contains(r#"id="a""#));
    }

    #[test]
    fn an_emoji_shortcode_resolves_but_a_time_range_does_not() {
        let out = html(":rocket: at 10:30 - 11:00");
        assert!(out.contains("\u{1f680}"), "{out}");
        assert!(out.contains("10:30 - 11:00"), "the time was eaten:\n{out}");
    }

    #[test]
    fn an_indent_is_code_only_at_the_head_of_a_block() {
        assert!(html("para\n\n    let x = 1;").contains("md-code-indented"));
        // Every line of the block, not just the first.
        let out = html("para\n\n    one\n    two");
        assert_eq!(out.matches("md-code-indented").count(), 2, "{out}");
        // Inside a list the same indent is a continuation line.
        assert!(!html("- item\n    wrapped").contains("md-code-indented"));
    }

    #[test]
    fn a_collapsible_callout_gets_a_fold_control() {
        let out = html("> [!warning]- Folded\n> body");
        assert!(out.contains("md-callout-collapsible"), "{out}");
        assert!(out.contains("md-callout-folded"), "{out}");
        assert!(out.contains("md-callout-fold"), "{out}");
        // A plain callout has none of it.
        assert!(!html("> [!note] N\n> b").contains("md-callout-fold"));
    }

    #[test]
    fn a_bare_callout_titles_itself() {
        assert!(html("> [!tip]\n> body").contains("Tip"));
    }

    #[test]
    fn a_link_definition_line_renders_nothing() {
        let out = html("[r]: http://x");
        assert!(!out.contains("http://x"), "{out}");
    }

    #[test]
    fn lines_are_not_separated_by_whitespace() {
        // `.editor-root` is `white-space: pre-wrap`. Any text node
        // between two line elements survives as an anonymous line box
        // and paints a phantom blank row on the page.
        let out = html("one\ntwo\nthree");
        assert!(
            !out.contains("></"),
            "unexpected empty line element:\n{out}"
        );
        assert!(
            !out.contains(">\n<") && !out.contains("> <"),
            "whitespace between line elements:\n{out}"
        );
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

    struct OnePage;
    impl crate::markdown::VaultLookup for OnePage {
        fn lookup_block(&self, _uuid: &str) -> Option<crate::markdown::VaultBlockHit> {
            None
        }
        fn lookup_page(&self, name: &str) -> Option<crate::markdown::VaultPageHit> {
            name.eq_ignore_ascii_case("header")
                .then(|| crate::markdown::VaultPageHit {
                    preview: "The header".to_owned(),
                })
        }
        fn lookup_section(&self, _page: &str, _heading: &str) -> Option<String> {
            None
        }
        fn lookup_block_short(&self, _page: &str, _short_id: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn a_wikilink_resolves_against_the_vault() {
        // Without a vault every link is unresolved, which on a published
        // site would style every cross-reference as broken.
        let bare = html("see [[header]]");
        assert!(bare.contains("md-wikilink-unresolved"), "{bare}");

        let resolved = render_markdown_html_with("see [[header]]", &OnePage);
        assert!(resolved.contains("md-wikilink"), "{resolved}");
        assert!(!resolved.contains("md-wikilink-unresolved"), "{resolved}");
    }

    #[test]
    fn a_resolvable_wikilink_becomes_a_real_anchor() {
        // The editor's markup is a span the app makes clickable with a
        // listener. A published page has no listener, so a span is a
        // link that looks like one and does nothing.
        let to_url = |name: &str| -> Option<String> {
            (name.eq_ignore_ascii_case("header")).then(|| format!("/guide/{name}"))
        };
        let opts = HtmlOptions::new().link_href(&to_url);
        let out = render_markdown_html_opts("see [[header]]", &opts);
        assert!(out.contains(r#"<a class="md-wikilink"#), "{out}");
        assert!(out.contains(r#"href="/guide/header""#), "{out}");
        assert!(out.contains("</a>"), "{out}");
    }

    #[test]
    fn an_unresolvable_wikilink_stays_a_span() {
        let to_url = |_: &str| -> Option<String> { None };
        let opts = HtmlOptions::new().link_href(&to_url);
        let out = render_markdown_html_opts("see [[nowhere]]", &opts);
        assert!(!out.contains("<a "), "{out}");
        assert!(out.contains("md-wikilink"), "{out}");
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
