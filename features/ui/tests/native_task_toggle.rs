//! A task's checkbox, pressed on the native (Blitz) renderer, toggles the
//! task (the web renderer does it through its JS bridge's `task-toggle`).
#![cfg(feature = "native")]

mod common;
use common::*;

/// Where line `i`'s checkbox is drawn: the start of its line, past the
/// editor's 60px gutter. (The checkbox is a widget's inner HTML, which
/// the tester gives no page box, so it is pressed by its line's.)
fn checkbox(t: &dioxus_test::DocumentTester, i: usize) -> (f32, f32) {
    let lines = t.query_all(".cm-line").immediately();
    let c = lines[i].center();
    (73.0, c.page().y as f32)
}

// One press per test: a second press on the re-rendered tree trips an
// unwrap in blitz-dom's inline hit test (`node.rs`), in the tester only.
#[tokio::test]
async fn pressing_an_open_tasks_checkbox_checks_it() {
    let t = mount(
        Setup::text("intro\n\n- [ ] first\n- [x] second")
            .caret(0)
            .markdown(),
    );
    t.pump().await.ok();
    let (x, y) = checkbox(&t, 2);
    t.click_at(x, y);
    t.pump().await.ok();
    expect_probe(&t, "doc", "intro\n\n- [x] first\n- [x] second").await;
    // A press on the box toggles; it does not move the caret.
    expect_probe(&t, "head", "0").await;
}

#[tokio::test]
async fn pressing_a_done_tasks_checkbox_opens_it() {
    let t = mount(
        Setup::text("intro\n\n- [ ] first\n- [x] second")
            .caret(0)
            .markdown(),
    );
    t.pump().await.ok();
    let (x, y) = checkbox(&t, 3);
    t.click_at(x, y);
    t.pump().await.ok();
    expect_probe(&t, "doc", "intro\n\n- [ ] first\n- [ ] second").await;
}

#[tokio::test]
async fn a_press_on_the_tasks_text_places_the_caret_and_toggles_nothing() {
    let t = mount(Setup::text("intro\n\n- [ ] first").caret(0).markdown());
    t.pump().await.ok();
    let (_, y) = checkbox(&t, 2);
    t.click_at(120.0, y);
    t.pump().await.ok();
    expect_probe(&t, "doc", "intro\n\n- [ ] first").await;
}
