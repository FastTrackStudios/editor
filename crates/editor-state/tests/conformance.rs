//! Differential test: our markdown pass against a `CommonMark` reference.
//!
//! `editor-state` hand-rolls its markdown parser, because the live preview
//! needs *source byte ranges* to hang decorations off rather than a tree to
//! render. That is a real constraint — but it also means nothing was
//! checking the parser against the specification, and constructs quietly
//! rotted: `![alt](url)` rendered as a literal `!` followed by a link,
//! `\*escapes\*` emphasised the text they were escaping, and sub-lists
//! came out flat.
//!
//! `pulldown-cmark` is the oracle. It is a dev-dependency only, and this
//! is deliberately a *structural* comparison, not a byte comparison — the
//! two renderers legitimately differ (we emit one `<div class="cm-line">`
//! per source line and keep the source text intact underneath). What must
//! agree is whether a construct was recognised at all.
//!
//! If `pulldown-cmark` ever becomes the parser rather than the oracle,
//! `OffsetIter` is the seam: it yields `(Event, Range<usize>)`, and those
//! ranges are exactly what `Decoration` wants.

use editor_state::html::render_markdown_html;
use pulldown_cmark::{Options, Parser};

/// Does the reference parser see this construct in the source?
fn reference_has(src: &str, want: fn(&pulldown_cmark::Event<'_>) -> bool) -> bool {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    Parser::new_ext(src, opts).any(|e| want(&e))
}

fn is_image(e: &pulldown_cmark::Event<'_>) -> bool {
    matches!(
        e,
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
    )
}

fn is_link(e: &pulldown_cmark::Event<'_>) -> bool {
    matches!(
        e,
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. })
    )
}

fn is_emphasis(e: &pulldown_cmark::Event<'_>) -> bool {
    matches!(
        e,
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis)
    )
}

fn is_hard_break(e: &pulldown_cmark::Event<'_>) -> bool {
    matches!(e, pulldown_cmark::Event::HardBreak)
}

fn is_table(e: &pulldown_cmark::Event<'_>) -> bool {
    matches!(
        e,
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Table(_))
    )
}

/// Every case here is a construct both parsers must agree exists.
///
/// `(source, our marker, does the reference see it)`.
const CASES: &[(&str, &str, fn(&pulldown_cmark::Event<'_>) -> bool)] = &[
    ("![alt](pic.png)", "<img", is_image),
    ("![a](p.png \"T\")", "<img", is_image),
    ("[a](http://x)", "md-link", is_link),
    ("[a](u \"T\")", "md-link", is_link),
    ("[a][r]\n\n[r]: http://x", "md-link", is_link),
    ("[r]\n\n[r]: http://x", "md-link", is_link),
    ("*em*", "md-italic", is_emphasis),
    ("a  \nb", "md-hard-break", is_hard_break),
    ("| a |\n|---|\n| 1 |", "md-table", is_table),
    ("| a |\n|:-:|\n| 1 |", "md-table", is_table),
];

#[test]
fn we_recognise_what_the_reference_recognises() {
    let mut missed = Vec::new();
    for (src, marker, want) in CASES {
        let ours = render_markdown_html(src).contains(marker);
        let theirs = reference_has(src, *want);
        if theirs && !ours {
            missed.push(format!(
                "{src:?} — reference sees it, we emit no {marker:?}"
            ));
        }
    }
    assert!(
        missed.is_empty(),
        "constructs we drop:\n{}",
        missed.join("\n")
    );
}

#[test]
fn an_escape_suppresses_the_construct_it_escapes() {
    // Both parsers must treat `\*` as a literal asterisk. Ours used to
    // emphasise the text *and* leak the backslash into the output.
    let src = r"\*not emphasis\*";
    assert!(!reference_has(src, is_emphasis), "oracle disagrees");
    let ours = render_markdown_html(src);
    assert!(
        !ours.contains("md-italic"),
        "we emphasised an escape:\n{ours}"
    );
    assert!(!ours.contains('\\'), "the backslash leaked:\n{ours}");
}

#[test]
fn a_reference_link_without_a_definition_is_not_a_link() {
    let src = "[a][nope]";
    assert!(!reference_has(src, is_link), "oracle disagrees");
    let ours = render_markdown_html(src);
    assert!(
        !ours.contains("md-link"),
        "we linked an undefined label:\n{ours}"
    );
    assert!(
        ours.contains("[a][nope]"),
        "the source should survive:\n{ours}"
    );
}
