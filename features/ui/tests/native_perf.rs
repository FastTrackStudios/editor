//! Timing, not assertions. Prints how long the native path takes on the
//! formatting document: the decoration pass, mounting (layout included), and a
//! keystroke. Run in release for numbers that mean something:
//!
//! ```text
//! cargo test -p editor-view --no-default-features --features native \
//!   --release --test native_perf -- --nocapture --test-threads=1
//! ```
#![cfg(feature = "native")]

mod common;
use common::*;
use std::time::{Duration, Instant};

const DOC: &str = include_str!("../../../crates/editor-state/tests/playground_doc.md");

/// Median of `n` runs of `f`, with the first run dropped (cold caches).
fn median<F: FnMut()>(n: usize, mut f: F) -> Duration {
    f();
    let mut times: Vec<Duration> = (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    times[times.len() / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[tokio::test]
#[ignore = "timing: run in release with --ignored --nocapture"]
async fn decoration_pass_on_the_formatting_document() {
    use editor_state::{EditorState, markdown};
    let state = EditorState::new(DOC);
    // The first pass on a fresh thread compiles every formula and diagram.
    let cold_start = Instant::now();
    std::hint::black_box(markdown::live_preview(&state));
    println!(
        "live_preview, cold first pass: {:.2} ms",
        ms(cold_start.elapsed())
    );
    let one = median(15, || {
        std::hint::black_box(markdown::live_preview(&state));
    });
    println!(
        "live_preview, formatting doc ({} bytes): {:.2} ms (median of 15)",
        DOC.len(),
        ms(one)
    );

    // Scaling: the same document, ten times over.
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let big_state = EditorState::new(big);
    let ten = median(5, || {
        std::hint::black_box(markdown::live_preview(&big_state));
    });
    println!(
        "live_preview, 10x doc ({} bytes): {:.2} ms (median of 5)",
        big.len(),
        ms(ten)
    );
}

#[tokio::test]
#[ignore = "timing: run in release with --ignored --nocapture"]
async fn mount_and_keystroke_on_the_formatting_document() {
    let mount_t = median(5, || {
        let t = dioxus_test::render(Harness)
            .with_root_context(Setup::text(DOC).caret(0).markdown().vim())
            .with_window_size(960, 900)
            .build();
        std::hint::black_box(&t);
    });
    println!(
        "mount (build + layout), formatting doc: {:.2} ms (median of 5)",
        ms(mount_t)
    );

    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let big_mount = median(3, || {
        let t = dioxus_test::render(Harness)
            .with_root_context(Setup::text(big).caret(0).markdown().vim())
            .with_window_size(960, 900)
            .build();
        std::hint::black_box(&t);
    });
    println!(
        "mount (build + layout), 10x doc: {:.2} ms (median of 3)",
        ms(big_mount)
    );

    let t = mount(Setup::text(DOC).caret(0).markdown());
    t.pump().await.ok();
    // Each sample: one insert and one delete, each followed by a pump, which
    // is where Dioxus re-renders. Timing the dispatch alone measured nothing.
    let mut samples = Vec::new();
    for _ in 0..15 {
        let start = Instant::now();
        t.press_key(Key::Character("x".into()), Modifiers::empty());
        t.pump().await.ok();
        t.press_key(Key::Named(NamedKey::Backspace), Modifiers::empty());
        t.pump().await.ok();
        samples.push(start.elapsed());
    }
    samples.sort();
    let key = samples[samples.len() / 2];
    println!(
        "keystroke (insert, pump, delete, pump), formatting doc: {:.2} ms per key (median of 15)",
        ms(key) / 2.0
    );
}

#[tokio::test]
#[ignore = "timing: run in release with --ignored --nocapture"]
async fn keystroke_on_the_ten_times_document() {
    let big: &'static str = Box::leak(DOC.repeat(10).into_boxed_str());
    let t = mount(Setup::text(big).caret(0).markdown());
    t.pump().await.ok();
    let mut samples = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        t.press_key(Key::Character("x".into()), Modifiers::empty());
        t.pump().await.ok();
        t.press_key(Key::Named(NamedKey::Backspace), Modifiers::empty());
        t.pump().await.ok();
        samples.push(start.elapsed());
    }
    samples.sort();
    let key = samples[samples.len() / 2];
    println!(
        "keystroke, 10x doc: {:.2} ms per key (median of 10)",
        ms(key) / 2.0
    );
}
