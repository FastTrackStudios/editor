//! A link pressed on the native (Blitz) renderer is given to the host's
//! `on_link_click` (the web renderer's goes through its JS bridge's
//! `link-clicked`): a wikilink's body, a link's URL.
#![cfg(feature = "native")]

mod common;
use common::*;
use dioxus::prelude::*;
use dioxus_test::render;
use editor_state::{EditorState, Selection};
use editor_view::{DecorationSource, Editor};

#[component]
fn LinkHarness() -> Element {
    let state = use_signal(|| {
        let mut s = EditorState::new("intro\n\nSee [[Garden|the garden]] today");
        s.selection = Selection::caret(0);
        s
    });
    let mut followed = use_signal(String::new);
    let decorations = DecorationSource::ptr(editor_state::markdown::live_preview);
    let head = state.read().selection.primary().head;
    rsx! {
        style { dangerous_inner_html: EDITOR_CSS }
        Editor {
            state,
            decorations,
            on_link_click: move |href: String| followed.set(href),
        }
        div { "data-testid": "followed", "{followed}" }
        div { "data-testid": "head", "{head}" }
    }
}

/// The tester's hit test reads a stale layout after a press re-renders
/// the line, so each press is on a fresh editor.
async fn press_at(x: f32) -> (String, String) {
    let t = render(LinkHarness).build();
    t.query(".editor-root")
        .immediately()
        .expect("editor")
        .focus();
    t.pump().await.ok();
    let line = t
        .query_all(".cm-line")
        .immediately()
        .into_iter()
        .nth(2)
        .expect("the link's line");
    let y = line.center().page().y as f32;
    t.click_at(x, y);
    t.pump().await.ok();
    let probe = |id: &str| {
        t.query(dioxus_test::by_testid(id))
            .immediately()
            .expect("probe")
            .inner_html()
            .trim()
            .to_owned()
    };
    (probe("followed"), probe("head"))
}

#[tokio::test]
async fn pressing_a_wikilink_follows_it_and_leaves_the_caret() {
    // "See " sits past the 60px gutter; the link's text follows it.
    let (followed, head) = press_at(120.0).await;
    assert_eq!(followed, "Garden|the garden");
    assert_eq!(head, "0", "a press on a link does not move the caret");
}

#[tokio::test]
async fn pressing_the_text_before_it_places_the_caret() {
    let (followed, head) = press_at(72.0).await;
    assert_eq!(followed, "");
    assert_ne!(head, "0");
}
