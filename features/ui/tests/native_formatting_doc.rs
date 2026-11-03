//! Renders the full formatting document (`playground_doc.md`) through the
//! native (Blitz) path, live-preview on, and writes PNGs for inspection.
//! Not asserted: it is the visual check for word-by-word formatting. The
//! assertions live in `native_deco.rs`.
#![cfg(feature = "native")]

mod common;
use common::*;

/// The same light theme `native_screenshot.rs` uses, so colours are real.
const THEME: &str = "
:root { --background:#ffffff; --muted:#f2f4f7; --foreground:#1a1c20;
        --muted-foreground:#6b7280; --primary:#1d4ed8; }
.cm-line { font-size: 22px; line-height: 1.6; }
";

const DOC: &str = include_str!("../../../crates/editor-state/tests/playground_doc.md");

fn out(name: &str) -> String {
    format!("{}/{name}", std::env::temp_dir().display())
}

/// Mount the whole document in a tall window so one PNG shows it all.
fn mount_tall(caret: usize) -> dioxus_test::DocumentTester {
    let tester = dioxus_test::render(Harness)
        .with_root_context(Setup::text(DOC).caret(caret).markdown().theme(THEME))
        .with_window_size(960, 9000)
        .build();
    tester
        .query(".editor-root")
        .immediately()
        .expect("editor root should render")
        .focus();
    tester
}

/// Mount lines `from..to` of the document, so each PNG is readable at full
/// size (the whole document in one image is too tall to inspect).
fn mount_section(from: usize, to: usize) -> dioxus_test::DocumentTester {
    let text: &'static str = Box::leak(
        DOC.lines()
            .skip(from)
            .take(to - from)
            .collect::<Vec<_>>()
            .join("\n")
            .into_boxed_str(),
    );
    let tester = dioxus_test::render(Harness)
        .with_root_context(Setup::text(text).caret(0).markdown().theme(THEME))
        .with_window_size(960, 1400)
        .build();
    tester
        .query(".editor-root")
        .immediately()
        .expect("editor root should render")
        .focus();
    tester
}

#[tokio::test]
async fn shot_section_1_frontmatter_and_inline() {
    let t = mount_section(0, 40);
    t.pump().await.ok();
    t.render_png(out("doc_s1_inline.png"));
}

#[tokio::test]
async fn shot_section_2_blocks_and_callouts() {
    let t = mount_section(40, 100);
    t.pump().await.ok();
    t.render_png(out("doc_s2_blocks.png"));
}

#[tokio::test]
async fn shot_section_3_math_mermaid_embeds() {
    let t = mount_section(100, 160);
    t.pump().await.ok();
    t.render_png(out("doc_s3_math.png"));
}

#[tokio::test]
async fn shot_section_4_commonmark_and_code() {
    let t = mount_section(160, 229);
    t.pump().await.ok();
    t.render_png(out("doc_s4_code.png"));
}

/// Measurement, not an assertion: prints the editor root's box against the
/// 960px window, to pin down the right-edge clipping seen in the PNGs.
#[tokio::test]
async fn measure_editor_root_against_window() {
    let t = mount_section(0, 40);
    t.pump().await.ok();
    let root = t.query(".editor-root").immediately().expect("root");
    let (w, h) = root.size();
    let c = root.center();
    let ul = root.upper_left();
    println!(
        "editor-root size={w}x{h} upper_left=({},{}) center=({},{})",
        ul.page().x,
        ul.page().y,
        c.page().x,
        c.page().y
    );
    for sel in [".cm-line", "table", "blockquote"] {
        if let Some(el) = t.query_all(sel).immediately().first() {
            let (ew, eh) = el.size();
            let eul = el.upper_left();
            println!(
                "{sel}: size={ew}x{eh} upper_left=({},{})",
                eul.page().x,
                eul.page().y
            );
        }
    }
}

#[tokio::test]
async fn shot_section_table_math_typst() {
    let t = mount_section(72, 100);
    t.pump().await.ok();
    t.render_png(out("doc_s2b_table_math.png"));
}

#[tokio::test]
async fn shot_section_code_fences() {
    let t = mount_section(206, 229);
    t.pump().await.ok();
    t.render_png(out("doc_s5_fences.png"));
}

/// The math in the formatting document must render on mount, with no typing.
/// The compile budget defers the fences past the first two; the view has to
/// run the pass again for them, or they stay as source.
#[tokio::test]
async fn math_renders_on_mount_without_typing() {
    let t = mount_section(84, 100);
    t.pump().await.ok();
    // Two inline formulas and one display block, none touching the caret.
    let rendered = t.query_all(".md-math-widget").immediately().len();
    assert_eq!(
        rendered, 3,
        "all three formulas should render on mount, got {rendered}"
    );
}

#[tokio::test]
async fn shot_full_document_caret_at_top() {
    let t = mount_tall(0);
    t.pump().await.ok();
    t.render_png(out("formatting_doc_top.png"));
}

#[tokio::test]
async fn shot_full_document_caret_inside_bold() {
    // Offset of the first `bold` word in the inline-styles paragraph.
    let caret = DOC.find("**bold**").map_or(0, |i| i + 4);
    let t = mount_tall(caret);
    t.pump().await.ok();
    t.render_png(out("formatting_doc_bold.png"));
}
