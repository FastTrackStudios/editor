//! Keystroke-to-paint on the native (Blitz) path: the time from a key (or
//! IME) event entering the document to a painted frame — handler, editor
//! transaction, re-render, DOM mutations, restyle, relayout and paint into
//! a recorded scene (no rasterization). A measurement, not an assertion:
//! `#[ignore]`d, run with
//!
//! ```text
//! cargo test --workspace --features editor-view/native --release \
//!     --test native_keystroke_bench -- --ignored --nocapture
//! ```
//!
//! Workload: `docs/design.md` (127 lines) with markdown live preview, a
//! 1000×800 window, the caret at the end of a mid-document paragraph.
#![cfg(feature = "native")]

mod common;
use common::*;
use dioxus_test::{DocumentTester, render};
use std::time::Instant;

const DOC: &str = include_str!("../../../docs/design.md");
const WARMUP: usize = 50;
const FRAMES: usize = 600;

fn caret() -> usize {
    DOC.get(DOC.len() / 2..)
        .and_then(|tail| tail.find("\n\n"))
        .map_or(0, |i| DOC.len() / 2 + i)
}

fn mount_bench() -> DocumentTester {
    let caret = caret();
    let t = render(Harness)
        .with_root_context(Setup::text(DOC).caret(caret).markdown())
        .with_window_size(1000, 800)
        .build();
    t.query(".editor-root").immediately().unwrap().focus();
    assert!(t.frame() > 0);
    t
}

/// Time `FRAMES` (after `WARMUP`) iterations of `input(i)` followed by a
/// frame, and print p50 / p99 / max in milliseconds.
fn bench(name: &str, t: &DocumentTester, mut input: impl FnMut(usize)) {
    let mut ms: Vec<f64> = (0..WARMUP + FRAMES)
        .map(|i| {
            let start = Instant::now();
            input(i);
            assert!(t.frame() > 0);
            start.elapsed().as_secs_f64() * 1e3
        })
        .skip(WARMUP)
        .collect();
    ms.sort_by(f64::total_cmp);
    let at = |q: usize| ms[(ms.len() - 1) * q / 100];
    println!(
        "BENCH {name}: frames={FRAMES} p50={:.3}ms p99={:.3}ms max={:.3}ms",
        at(50),
        at(99),
        at(100)
    );
}

#[tokio::test]
#[ignore = "benchmark; run with --ignored --nocapture"]
async fn keystroke_to_paint() {
    let t = mount_bench();
    // Type a character, then delete it: the doc returns to its start
    // every two keystrokes, so every sample sees the same document.
    bench("type+backspace", &t, |i| {
        press(&t, &[if i % 2 == 0 { "x" } else { "Backspace" }]);
    });
    bench("arrow-left/right", &t, |i| {
        press(
            &t,
            &[if i % 2 == 0 {
                "ArrowLeft"
            } else {
                "ArrowRight"
            }],
        );
    });
    bench("idle-frame", &t, |_| {});
    // Every pair of inputs is a round trip: the caret is back where it
    // started (the doc probe is HTML-escaped, so it is not compared).
    expect_probe(&t, "head", &caret().to_string()).await;
}

#[tokio::test]
#[ignore = "benchmark; run with --ignored --nocapture"]
async fn ime_to_paint() {
    let t = mount_bench();
    // A preedit growing and shrinking, as an IME composes.
    bench("ime-preedit", &t, |i| {
        let text = if i % 2 == 0 { "ni" } else { "nih" };
        t.ime(dioxus_test::BlitzImeEvent::Preedit(text.to_string(), None));
    });
    // Commit a glyph, then delete it with Backspace (as above).
    t.ime(dioxus_test::BlitzImeEvent::Preedit(String::new(), None));
    assert!(t.frame() > 0);
    bench("ime-commit+backspace", &t, |i| {
        if i % 2 == 0 {
            t.ime(dioxus_test::BlitzImeEvent::Preedit("ni".to_string(), None));
            t.ime(dioxus_test::BlitzImeEvent::Preedit(String::new(), None));
            t.ime(dioxus_test::BlitzImeEvent::Commit("你".to_string()));
        } else {
            press(&t, &["Backspace"]);
        }
    });
    // Every pair of inputs is a round trip: the caret is back where it
    // started (the doc probe is HTML-escaped, so it is not compared).
    expect_probe(&t, "head", &caret().to_string()).await;
}
