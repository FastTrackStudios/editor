//! The extension surface on the native editor (`editor_state::ext`): a
//! host's key hook runs an extension's keymap by the node at the caret,
//! its command lands as one undo step in the history the host shares with
//! the editor, and its decorations draw — a mark, a gutter marker and a
//! widget that mounts the host's element.
#![cfg(feature = "native")]

mod common;
use common::*;
use dioxus::prelude::*;
use dioxus_test::{by_testid, render};
use editor_state::ext::{self, DecorationSpec, EditCommand, KeymapEntry, TextChange};
use editor_state::{EditorState, History, KeySpec, Keymap, Selection, TransactionSpec};
use editor_view::{DecorationSource, Editor, WidgetMounts};

/// What the person's Mod-z does: the history's undo.
fn undo(_: &EditorState) -> Option<TransactionSpec> {
    Some(TransactionSpec::new().user_event("undo"))
}

/// Tab in a list item: the extension indents the line two spaces.
fn indent(state: &EditorState) -> EditCommand {
    let snap = ext::snapshot(state, 0);
    EditCommand::changes(vec![TextChange::insert(snap.node.line_from, "  ")]).named("Indent")
}

#[component]
fn ExtHarness() -> Element {
    let mut state = use_signal(|| {
        let mut s = EditorState::new("- a\n- b");
        s.selection = Selection::caret(7);
        s
    });
    let history = use_signal(History::new);
    let keymap = Keymap::new().with("Mod-z", undo);
    let entries = vec![KeymapEntry {
        key: "Tab".into(),
        action: "OUTLINE_INDENT".into(),
        when: ext::Scope::Node(ext::NodeKind::ListItem),
    }];
    // The host's key hook: the extension's keymap, run here in-process.
    let on_key = move |press: KeySpec| {
        let cur = state.peek().clone();
        let Some(entry) = ext::resolve_key(&entries, &press, &cur) else {
            return false;
        };
        assert_eq!(entry.action, "OUTLINE_INDENT");
        let Ok(spec) = ext::transaction(&cur, &indent(&cur), 0, "lab.outline") else {
            return false;
        };
        editor_view::dispatch_with_history(state, history, spec, None);
        true
    };
    let decorations = use_hook(|| {
        DecorationSource::new(|s: &EditorState| {
            let specs = vec![
                DecorationSpec::Mark {
                    from: 2,
                    to: 3,
                    class: "hot".into(),
                    title: None,
                },
                DecorationSpec::Gutter {
                    at: 0,
                    text: "1".into(),
                    class: "count".into(),
                },
                DecorationSpec::Widget {
                    at: s.doc.len(),
                    view: "count".into(),
                    props: serde_json::Value::Null,
                    block: false,
                },
            ];
            ext::decorations(&specs, &s.doc.to_string())
        })
    });
    let mounts = use_hook(|| {
        WidgetMounts::new(|id: &str| {
            rsx! { b { "data-testid": "mounted", "{id}" } }
        })
    });
    let doc = state.read().doc.to_string();
    let undoable = history.read().can_undo();
    rsx! {
        Editor { state, keymap, decorations, history, on_key, mounts }
        div { "data-testid": "doc", "{doc}" }
        div { "data-testid": "undoable", "{undoable}" }
    }
}

fn mount_ext() -> dioxus_test::DocumentTester {
    let t = render(ExtHarness).build();
    t.query(".editor-root")
        .immediately()
        .expect("editor root should render")
        .focus();
    t
}

#[tokio::test]
async fn tab_in_a_list_item_runs_the_extensions_command_and_undoes_in_one_step() {
    let t = mount_ext();
    press(&t, &["Tab"]);
    expect_probe(&t, "doc", "- a\n  - b").await;
    expect_probe(&t, "undoable", "true").await;
    t.press_key(parse_key("z"), Modifiers::CONTROL);
    expect_probe(&t, "doc", "- a\n- b").await;
    expect_probe(&t, "undoable", "false").await;
}

#[tokio::test]
async fn the_extensions_decorations_draw_and_its_widget_mounts() {
    let t = mount_ext();
    t.query(".ext-mark.ext-hot")
        .await
        .unwrap_or_else(|e| panic!("mark: {e:?}"));
    t.query(".editor-gutter.ext-count")
        .await
        .unwrap_or_else(|e| panic!("gutter: {e:?}"));
    t.query(by_testid("mounted"))
        .expect(inner_html(eq("ext-2".to_string())))
        .await
        .unwrap_or_else(|e| panic!("mounted widget: {e:?}"));
    t.query("[data-editor-mount=\"ext-2\"]")
        .await
        .unwrap_or_else(|e| panic!("mount wrapper: {e:?}"));
}
