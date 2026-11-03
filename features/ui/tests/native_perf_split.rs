//! Where a native keystroke's time goes, split by stage. Timing only.
//!
//!   cargo test -p editor-view --no-default-features --features native \
//!     --release --test native_perf_split -- --ignored --nocapture
#![cfg(feature = "native")]
mod common;

use std::time::{Duration, Instant};

const DOC: &str = include_str!("../../../crates/editor-state/tests/playground_doc.md");

fn median<F: FnMut()>(n: usize, mut f: F) -> Duration {
    f();
    let mut v: Vec<Duration> = (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    v.sort();
    v[v.len() / 2]
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn native_keystroke_stages() {
    use editor_state::{EditorState, markdown};
    for (label, text) in [("1x", DOC.to_string()), ("10x", DOC.repeat(10))] {
        let leaked: &'static str = Box::leak(text.into_boxed_str());
        let state = EditorState::new(leaked);
        let to_string = median(15, || {
            std::hint::black_box(state.doc.to_string());
        });
        let decos = markdown::live_preview(&state);
        let decorate = median(15, || {
            std::hint::black_box(markdown::live_preview(&state));
        });
        let text_owned = state.doc.to_string();
        let build = median(15, || {
            let (arena, root) = editor_view::tile::build::build_tiles(&text_owned, &decos);
            std::hint::black_box((&arena, root));
        });
        println!(
            "{label} ({} bytes): doc->String {:.2} ms, decorations {:.2} ms, build_tiles {:.2} ms",
            leaked.len(),
            to_string.as_secs_f64() * 1000.0,
            decorate.as_secs_f64() * 1000.0,
            build.as_secs_f64() * 1000.0,
        );
    }
}

#[tokio::test]
#[ignore = "timing: run in release with --ignored --nocapture"]
async fn render_stage_split_by_document_size() {
    for copies in [1, 4, 10] {
        render_stages(copies).await;
    }
}

async fn render_stages(copies: usize) {
    use common::*;
    let big: &'static str = Box::leak(DOC.repeat(copies).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).markdown());
    t.pump().await.ok();
    let nodes = t.query_all("*").immediately().len();
    let lines = t.query_all(".cm-line").immediately().len();
    // One keystroke, in three parts. `pump` waits for the vdom and applies
    // the mutations, but does not lay out; `relayout` is the layout.
    let (mut dispatch, mut pump, mut lay) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..10 {
        let s0 = Instant::now();
        t.press_key(Key::Character("x".into()), Modifiers::empty());
        let s1 = Instant::now();
        t.pump().await.ok();
        let s2 = Instant::now();
        t.relayout();
        let s3 = Instant::now();
        dispatch.push(s1 - s0);
        pump.push(s2 - s1);
        lay.push(s3 - s2);
        t.press_key(Key::Named(NamedKey::Backspace), Modifiers::empty());
        t.pump().await.ok();
        t.relayout();
    }
    let med = |mut v: Vec<Duration>| {
        v.sort();
        v[v.len() / 2].as_secs_f64() * 1000.0
    };
    println!(
        "{copies:>2}x doc: {nodes:>6} elements, {lines:>5} lines. Keystroke: dispatch {:.2} ms, pump {:.2} ms, layout {:.2} ms",
        med(dispatch),
        med(pump),
        med(lay),
    );
}
