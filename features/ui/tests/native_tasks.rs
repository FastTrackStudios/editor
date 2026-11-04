//! Task checkbox statuses on the native (Blitz) path. Obsidian Tasks uses
//! more than `[ ]` and `[x]`: `[/]` in progress, `[>]` forwarded, `[-]`
//! cancelled. Each must draw its own mark, not an empty box.
#![cfg(feature = "native")]

mod common;
use common::*;

/// Caret on the last line, so the earlier task lines show their widgets.
const TASKS: &str = "- [/] in progress\n- [>] forwarded\n- [-] cancelled\n- [ ] todo";

#[tokio::test]
async fn in_progress_status_draws_its_own_mark() {
    let t = mount(Setup::text(TASKS).caret(TASKS.len()).markdown());
    t.query(".editor-root")
        .expect(inner_html(contains_substring("data-status=\"/\"")))
        .await
        .unwrap();
    t.query(".editor-root")
        .expect(inner_html(contains_substring("◐")))
        .await
        .unwrap();
}

#[tokio::test]
async fn forwarded_and_cancelled_statuses_are_distinct() {
    let t = mount(Setup::text(TASKS).caret(TASKS.len()).markdown());
    // Blitz serializes `>` in attribute values as `&gt;`, so assert the
    // glyph the status draws, not the escaped attribute.
    t.query(".editor-root")
        .expect(inner_html(contains_substring("→")))
        .await
        .unwrap();
    t.query(".editor-root")
        .expect(inner_html(contains_substring("data-status=\"-\"")))
        .await
        .unwrap();
}

#[tokio::test]
async fn done_task_still_shows_the_check_and_checked_class() {
    let t = mount(Setup::text("- [x] done\n- [ ] todo").caret(18).markdown());
    t.query(".editor-root")
        .expect(inner_html(contains_substring("md-task-checkbox checked")))
        .await
        .unwrap();
}
