//! Native line virtualization: a long document builds only the lines near
//! its viewport, and the caret's line is always among them.
#![cfg(feature = "native")]

mod common;
use common::*;
use std::time::Duration;

const DOC: &str = include_str!("../../../crates/editor-state/tests/playground_doc.md");

/// Let the viewport poll run and the view settle: the poll reads the scroll
/// container on a 16ms timer, so a few rounds of sleep and pump are needed.
async fn settle(t: &dioxus_test::DocumentTester) {
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.pump().await.ok();
    }
}

fn rendered_lines(t: &dioxus_test::DocumentTester) -> usize {
    t.query_all(".cm-line").immediately().len()
}

#[tokio::test]
async fn a_long_document_builds_only_the_lines_near_the_viewport() {
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).markdown());
    settle(&t).await;
    let built = rendered_lines(&t);
    assert!(
        built < 300,
        "a 2051-line document should build a viewport's worth of lines, built {built}"
    );
    assert!(built > 0, "some lines must render");
}

#[tokio::test]
async fn navigating_to_the_end_renders_the_last_line() {
    let text: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let last = text.lines().last().expect("a last line").to_string();
    let t = mount(Setup::text(text).caret(0).vim().markdown());
    settle(&t).await;
    press(&t, &["G"]);
    settle(&t).await;
    // A run of the last line's words that does not cross the caret: the caret
    // mark splits off the first letter, so the line is not one text run.
    let needle = "stay visible while your caret";
    let found = t
        .query_all(".cm-line")
        .immediately()
        .iter()
        .any(|l| l.inner_html().contains(needle));
    assert!(found, "the caret's line ({last:?}) should be built after G");
}

#[tokio::test]
async fn the_caret_line_stays_built_when_it_is_far_from_the_top() {
    let text: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(text).caret(text.len() - 1).markdown());
    settle(&t).await;
    let built = rendered_lines(&t);
    assert!(built > 0 && built < 300, "built {built}");
    let carets = t.query_all(".ed-native-caret").immediately().len();
    assert_eq!(carets, 1, "the caret must be painted on its (built) line");
}

/// The document position of the first built line (its `data-tile-pos`).
fn first_built_pos(t: &dioxus_test::DocumentTester) -> usize {
    t.query_all(".cm-line")
        .immediately()
        .first()
        .and_then(|l| l.attribute("data-tile-pos"))
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

/// Move the pointer over `(x, y)` and send a wheel of `dy` pixels there, the
/// way the window shell does.
fn wheel(t: &dioxus_test::DocumentTester, x: f32, y: f32, dy: f64) {
    t.pointer_move(f64::from(x), f64::from(y), false);
    t.send_ui_event(blitz_traits::events::UiEvent::Wheel(
        blitz_traits::events::BlitzWheelEvent {
            delta: blitz_traits::events::BlitzWheelDelta::Pixels(0.0, dy),
            coords: blitz_traits::events::PointerCoords {
                page_x: x,
                page_y: y,
                screen_x: x,
                screen_y: y,
                client_x: x,
                client_y: y,
            },
            buttons: blitz_traits::events::MouseEventButtons::empty(),
            mods: Modifiers::empty(),
            element: blitz_traits::events::Point { x: 0.0, y: 0.0 },
        },
    ));
}

#[tokio::test]
async fn wheel_scrolls_the_editor() {
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).markdown());
    settle(&t).await;
    let before = first_built_pos(&t);
    let root = t.query(".editor-root").immediately().expect("root");
    let c = root.center().page();
    t.pointer_move(f64::from(c.x as f32), f64::from(c.y as f32), false);
    wheel(&t, c.x as f32, c.y as f32, -2000.0);
    settle(&t).await;
    let after = first_built_pos(&t);
    assert!(
        after > before + 1000,
        "a 2000px wheel should move the built lines down the document: first built {before} -> {after}"
    );
}

/// Scroll down and back up many times, laying out after each step, as a user
/// scrolling does. The window moves each time, so lines and spacers come and
/// go; Blitz must lay out the changing tree without losing a node.
#[tokio::test]
async fn scrolling_back_and_forth_keeps_the_layout_intact() {
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).markdown());
    settle(&t).await;
    let root = t.query(".editor-root").immediately().expect("root");
    let c = root.center().page();
    for step in 0..30 {
        let dy = if step % 2 == 0 { -700.0 } else { 900.0 };
        wheel(&t, c.x as f32, c.y as f32, dy);
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            t.pump().await.ok();
            t.relayout();
        }
    }
    assert!(
        rendered_lines(&t) > 0,
        "the editor should still render lines"
    );
}

/// Scroll away from the caret, then move the caret back up through the
/// document one line at a time. Each move can scroll the caret's line into view
/// while the window is also changing; the layout must survive every step.
#[tokio::test]
async fn moving_the_caret_while_scrolled_keeps_the_layout_intact() {
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).vim().markdown());
    settle(&t).await;
    let root = t.query(".editor-root").immediately().expect("root");
    let c = root.center().page();
    wheel(&t, c.x as f32, c.y as f32, -4000.0);
    settle(&t).await;
    for _ in 0..60 {
        press(&t, &["k"]);
        for _ in 0..2 {
            tokio::time::sleep(Duration::from_millis(16)).await;
            t.pump().await.ok();
            t.relayout();
        }
    }
    assert!(
        rendered_lines(&t) > 0,
        "the editor should still render lines"
    );
}
