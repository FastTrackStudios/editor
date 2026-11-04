//! Layout regressions on the native (Blitz) path: the editor must fit the
//! width its host gives it. `.editor-root` carries a 60px gutter as
//! padding (`editor.css`), and `size()` reports the content box, so the
//! border box is `x + GUTTER + width`. Under content-box sizing that
//! border box was 60px wider than the window, and wrapped text was clipped.
#![cfg(feature = "native")]

mod common;
use common::*;

const WINDOW_W: f64 = 960.0;
/// `.editor-root { padding-left }` in `crates/editor/assets/editor.css`.
const GUTTER: f64 = 60.0;

/// A line long enough to need wrapping at this width.
const LONG: &str = "This paragraph is long enough that it has to wrap onto a second line \
     at this window width, so its right edge is what the test measures.";

fn mount_at_width(text: &'static str) -> dioxus_test::DocumentTester {
    let tester = dioxus_test::render(Harness)
        .with_root_context(Setup::text(text).caret(0).markdown())
        .with_window_size(960, 900)
        .build();
    tester
        .query(".editor-root")
        .immediately()
        .expect("editor root should render")
        .focus();
    tester
}

/// Right edge of the root's border box, from its content-box `size()`.
fn root_border_right(root: &dioxus_test::ResolvedElement) -> f64 {
    let (w, _) = root.size();
    root.upper_left().page().x + GUTTER + f64::from(w)
}

#[tokio::test]
async fn editor_root_fits_inside_the_window() {
    let t = mount_at_width(LONG);
    t.pump().await.ok();
    let root = t.query(".editor-root").immediately().expect("root");
    let right = root_border_right(&root);
    assert!(
        right <= WINDOW_W + 0.5,
        "editor root right edge {right} overflows the {WINDOW_W}px window"
    );
}

#[tokio::test]
async fn every_line_fits_inside_the_root() {
    let t = mount_at_width(LONG);
    t.pump().await.ok();
    let root = t.query(".editor-root").immediately().expect("root");
    let root_right = root_border_right(&root);
    let lines = t.query_all(".cm-line").immediately();
    assert!(
        !lines.is_empty(),
        "the document should lay out at least one line"
    );
    for line in lines {
        let (lw, _) = line.size();
        let line_right = line.upper_left().page().x + f64::from(lw);
        assert!(
            line_right <= root_right + 0.5,
            "line right edge {line_right} overflows the root's right edge {root_right}"
        );
    }
}
