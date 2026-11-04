//! IME composition on the native (Blitz) path, end to end: the platform
//! IME event goes in through the document's UI-event entry point (as
//! winit's `Ime` does in a running app), the FTS Blitz fork routes it to
//! the focused editor, dioxus-native-dom delivers it as
//! `compositionupdate` / `compositionend`, and the editor composes,
//! commits or cancels. Asserted on the rendered preedit and the doc and
//! selection probes.
#![cfg(feature = "native")]

mod common;
use common::*;
use dioxus_test::BlitzImeEvent;

fn preedit(text: &str) -> BlitzImeEvent {
    let end = text.len();
    BlitzImeEvent::Preedit(text.to_string(), Some((end, end)))
}

fn cleared() -> BlitzImeEvent {
    BlitzImeEvent::Preedit(String::new(), None)
}

fn commit(text: &str) -> BlitzImeEvent {
    BlitzImeEvent::Commit(text.to_string())
}

async fn expect_preedit(t: &dioxus_test::DocumentTester, text: &str) {
    t.query(".ed-preedit")
        .expect(inner_html(eq(text.to_string())))
        .await
        .unwrap_or_else(|e| panic!("preedit != {text}: {e:?}"));
}

async fn expect_no_preedit(t: &dioxus_test::DocumentTester) {
    t.query(".editor-root")
        .expect(inner_html(dioxus_test::matchers::not(contains_substring(
            "ed-preedit",
        ))))
        .await
        .unwrap_or_else(|e| panic!("preedit still drawn: {e:?}"));
}

#[tokio::test]
async fn focusing_the_editor_turns_the_ime_on() {
    let t = mount(Setup::text("ab"));
    assert_eq!(t.ime_enabled(), Some(true));
}

#[tokio::test]
async fn preedit_draws_at_caret_without_editing() {
    let t = mount(Setup::text("ab").caret(1));
    t.ime(BlitzImeEvent::Enabled);
    t.ime(preedit("n"));
    expect_preedit(&t, "n").await;
    t.ime(preedit("ni"));
    expect_preedit(&t, "ni").await;
    // Not in the document until it commits.
    expect_probe(&t, "doc", "ab").await;
    expect_probe(&t, "head", "1").await;
}

#[tokio::test]
async fn commit_inserts_composed_text_once() {
    let t = mount(Setup::text("ab").caret(1));
    t.ime(preedit("nihao"));
    expect_preedit(&t, "nihao").await;
    // winit's order: the preedit is cleared, then the text commits.
    t.ime(cleared());
    t.ime(commit("你好"));
    expect_probe(&t, "doc", "a你好b").await;
    expect_probe(&t, "head", &(1 + "你好".len()).to_string()).await;
    expect_no_preedit(&t).await;
}

#[tokio::test]
async fn cancel_drops_preedit_and_leaves_doc() {
    let t = mount(Setup::text("ab").caret(1));
    t.ime(preedit("ka"));
    expect_preedit(&t, "ka").await;
    t.ime(cleared());
    expect_probe(&t, "doc", "ab").await;
    expect_probe(&t, "head", "1").await;
    t.query(".editor-root")
        .expect(inner_html(contains_substring("ed-native-caret")))
        .await
        .unwrap();
    expect_no_preedit(&t).await;
}

#[tokio::test]
async fn dead_key_accent_composes_then_commits() {
    // macOS Option-e, e: a one-char preedit, then the composed glyph.
    let t = mount(Setup::text("caf").caret(3));
    t.ime(preedit("´"));
    expect_preedit(&t, "´").await;
    t.ime(cleared());
    t.ime(commit("é"));
    expect_probe(&t, "doc", "café").await;
}

#[tokio::test]
async fn composition_survives_a_rerender_and_follows_the_caret() {
    let t = mount(Setup::text("ab").caret(1).remote_edit());
    t.ime(preedit("ni"));
    expect_preedit(&t, "ni").await;
    // An edit from outside the editor re-renders it mid-composition and
    // shifts the caret right by one.
    t.query(dioxus_test::by_testid("remote-edit"))
        .immediately()
        .unwrap()
        .click();
    expect_probe(&t, "doc", "Xab").await;
    expect_probe(&t, "head", "2").await;
    expect_preedit(&t, "ni").await;
    t.ime(cleared());
    t.ime(commit("你"));
    expect_probe(&t, "doc", "Xa你b").await;
}

#[tokio::test]
async fn keys_are_ignored_mid_composition_and_typing_resumes_after() {
    let t = mount(Setup::text("ab").caret(1));
    t.ime(preedit("ni"));
    expect_preedit(&t, "ni").await;
    press(&t, &["x", "ArrowRight", "Backspace"]);
    expect_probe(&t, "doc", "ab").await;
    expect_probe(&t, "head", "1").await;
    t.ime(cleared());
    t.ime(commit("你"));
    expect_probe(&t, "doc", "a你b").await;
    // Plain typing is unchanged once the composition is over.
    t.type_text("z");
    expect_probe(&t, "doc", "a你zb").await;
}

#[tokio::test]
async fn composing_over_a_selection_replaces_it() {
    let t = mount(Setup::text("hello").caret(1));
    t.press_key(parse_key("ArrowRight"), Modifiers::SHIFT);
    t.press_key(parse_key("ArrowRight"), Modifiers::SHIFT);
    t.press_key(parse_key("ArrowRight"), Modifiers::SHIFT);
    expect_probe(&t, "head", "4").await;
    t.ime(preedit("k"));
    expect_probe(&t, "doc", "hello").await;
    expect_preedit(&t, "k").await;
    t.ime(cleared());
    t.ime(commit("か"));
    expect_probe(&t, "doc", "hかo").await;
}

#[tokio::test]
async fn cancelling_composition_over_a_selection_preserves_text() {
    let t = mount(Setup::text("hello").caret(1));
    for _ in 0..3 {
        t.press_key(parse_key("ArrowRight"), Modifiers::SHIFT);
    }
    t.ime(preedit("k"));
    expect_preedit(&t, "k").await;
    t.ime(cleared());
    expect_probe(&t, "doc", "hello").await;
    expect_probe(&t, "head", "4").await;
    expect_no_preedit(&t).await;
}

#[tokio::test]
async fn vim_normal_mode_ignores_the_ime() {
    let t = mount(Setup::text("ab").vim());
    t.ime(preedit("ni"));
    t.ime(cleared());
    t.ime(commit("你"));
    expect_probe(&t, "doc", "ab").await;
    expect_no_preedit(&t).await;
    // In insert mode it composes.
    press(&t, &["i"]);
    expect_probe(&t, "mode", "Insert").await;
    t.ime(preedit("ni"));
    expect_preedit(&t, "ni").await;
    t.ime(cleared());
    t.ime(commit("你"));
    expect_probe(&t, "doc", "你ab").await;
}
