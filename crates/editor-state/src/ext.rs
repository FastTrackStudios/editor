//! The extension surface (`editor-ext`) on the live editor's state.
//!
//! `editor-ext` holds the values an extension sends and receives — a
//! [`Snapshot`], an [`EditCommand`], [`KeymapEntry`]s, [`DecorationSpec`]s
//! — with no editor in sight. This module joins them to [`EditorState`]:
//!
//! - [`snapshot`]: the state as an extension sees it;
//! - [`transaction`]: an [`EditCommand`] as **one** [`TransactionSpec`],
//!   checked, tagged `user_event = "extension"` and annotated with the
//!   extension that made it — so [`crate::History`] records it as one
//!   undo group that never coalesces with typing;
//! - [`resolve_key`]: the keymap entry a keystroke runs where the cursor is;
//! - [`decorations`] / [`mapped`]: decoration specs as the view's
//!   [`DecoratedRange`]s, and moved through an edit.
//!
//! ```
//! use editor_state::EditorState;
//! use editor_state::ext::{self, EditCommand, TextChange};
//!
//! let state = EditorState::new("- one\n");
//! let cmd = EditCommand::changes(vec![TextChange::insert(0, "  ")]).named("Indent");
//! let spec = ext::transaction(&state, &cmd, 0, "acme.outline").unwrap();
//! assert_eq!(state.update(spec).doc.to_string(), "  - one\n");
//! ```

pub use editor_ext::*;

use crate::change::{Change, Changes};
use crate::command::KeySpec;
use crate::decoration::DecoratedRange;
use crate::selection::{Range, Selection};
use crate::state::EditorState;
use crate::transaction::TransactionSpec;

/// The `user_event` of every transaction an extension makes.
pub const USER_EVENT: &str = "extension";
/// The annotation naming the extension that made a transaction.
pub const BY: &str = "extension";
/// The annotation carrying its undo step's name.
pub const STEP: &str = "step";

/// `state`'s selection as an extension's ranges.
#[must_use]
pub fn ranges(state: &EditorState) -> Vec<SelRange> {
    state
        .selection
        .ranges()
        .iter()
        .map(|r| SelRange {
            anchor: r.anchor,
            head: r.head,
        })
        .collect()
}

/// `state` as an extension sees it, at the host's `version`.
#[must_use]
pub fn snapshot(state: &EditorState, version: u64) -> Snapshot {
    Snapshot::of(
        &state.doc.to_string(),
        &ranges(state),
        state.selection.primary_index(),
        version,
    )
}

/// `cmd`, made by extension `by` against `state` at `version`, as one
/// transaction: one undo step.
///
/// # Errors
///
/// As [`EditCommand::checked`] and [`EditCommand::apply_to`]: a stale
/// command, a position outside the text or inside a character, changes
/// that overlap, a selection outside the text after.
pub fn transaction(
    state: &EditorState,
    cmd: &EditCommand,
    version: u64,
    by: &str,
) -> Result<TransactionSpec, ExtError> {
    let text = state.doc.to_string();
    let changes = cmd.checked(&text, version)?;
    let (_, selection) = cmd.apply_to(&text, &ranges(state), version)?;
    let changes = Changes::from_sorted(
        changes
            .into_iter()
            .map(|c| Change::replace(c.from..c.to, c.insert))
            .collect(),
    );
    let mut spec = TransactionSpec::new()
        .changes(changes)
        .user_event(USER_EVENT)
        .annotate("origin", USER_EVENT)
        .annotate(BY, by);
    if !selection.is_empty() {
        let primary = state
            .selection
            .primary_index()
            .min(selection.len().saturating_sub(1));
        let ranges = selection
            .into_iter()
            .map(|r| Range::new(r.anchor, r.head))
            .collect();
        spec = spec.selection(Selection::many(ranges, primary));
    }
    if let Some(name) = &cmd.name {
        spec = spec.annotate(STEP, name.clone());
    }
    Ok(spec)
}

/// The extension that made `spec`, when one did ([`transaction`]).
#[must_use]
pub fn made_by(spec: &TransactionSpec) -> Option<&str> {
    if spec.user_event.as_deref() != Some(USER_EVENT) {
        return None;
    }
    spec.annotations
        .iter()
        .find(|(k, _)| k == BY)
        .map(|(_, v)| v.as_str())
}

/// A keystroke as an extension's [`KeyChord`].
#[must_use]
pub fn chord(press: &KeySpec) -> KeyChord {
    KeyChord {
        key: press.key.clone(),
        ctrl: press.ctrl,
        alt: press.alt,
        shift: press.shift,
        meta: press.meta,
        r#mod: false,
    }
}

/// The entry of `entries` that `press` runs in `state`, by the node the
/// cursor is in ([`resolve`]).
#[must_use]
pub fn resolve_key<'a, T: AsRef<KeymapEntry>>(
    entries: &'a [T],
    press: &KeySpec,
    state: &EditorState,
) -> Option<&'a T> {
    if entries.is_empty() {
        return None;
    }
    let text = state.doc.to_string();
    let node = node_at(&text, state.selection.primary().head);
    resolve(entries, &chord(press), &node)
}

/// The id of the `i`th spec's mounted widget ([`DecoratedRange::mount`]).
#[must_use]
pub fn mount_id(i: usize) -> String {
    format!("ext-{i}")
}

/// The spec index a mounted widget's id names ([`mount_id`]).
#[must_use]
pub fn mount_index(id: &str) -> Option<usize> {
    id.strip_prefix("ext-")?.parse().ok()
}

/// `specs` as the view's decorations, over `text`. A spec that does not
/// fit `text` ([`DecorationSpec::checked`]) is left out. A widget's view
/// is mounted by the host under [`mount_id`] of its index.
#[must_use]
pub fn decorations(specs: &[DecorationSpec], text: &str) -> Vec<DecoratedRange> {
    let mut out = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        if spec.checked(text).is_err() {
            continue;
        }
        match spec {
            DecorationSpec::Mark {
                from,
                to,
                class,
                title,
            } => {
                let attrs = title
                    .iter()
                    .map(|t| ("title".to_owned(), t.clone()))
                    .collect();
                out.push(DecoratedRange::mark_with_attrs(
                    *from..*to,
                    format!("ext-mark {}", ext_classes(class)),
                    attrs,
                ));
            }
            DecorationSpec::Line { at, class } => {
                out.push(DecoratedRange::line(
                    *at,
                    format!("ext-line {}", ext_classes(class)),
                ));
            }
            DecorationSpec::Gutter {
                at,
                text: mark,
                class,
            } => {
                let line_start = text
                    .get(..*at)
                    .and_then(|t| t.rfind('\n'))
                    .map_or(0, |n| n.saturating_add(1));
                out.push(DecoratedRange::line(*at, "ext-gutter-line"));
                out.push(DecoratedRange::gutter(
                    line_start,
                    mark,
                    &ext_classes(class),
                ));
            }
            DecorationSpec::Widget { at, block, .. } => {
                out.push(DecoratedRange::mount(*at, &mount_id(i), *block));
            }
        }
    }
    out
}

/// `changes` as an extension's [`TextChange`]s.
#[must_use]
pub fn text_changes(changes: &Changes) -> Vec<TextChange> {
    changes
        .iter()
        .map(|c| TextChange::replace(c.from, c.to, c.inserted.clone()))
        .collect()
}

/// `specs` moved through `changes` (an edit of the text they were for);
/// a mark whose text was deleted goes.
#[must_use]
pub fn mapped(specs: &[DecorationSpec], changes: &Changes) -> Vec<DecorationSpec> {
    let changes = text_changes(changes);
    specs.iter().filter_map(|s| s.mapped(&changes)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::History;
    use crate::decoration::{DecorationKind, mount_of};

    /// What an extension's Tab does in a list item: indent the line two
    /// spaces, the caret kept on its text.
    fn indent(state: &EditorState) -> EditCommand {
        let snap = snapshot(state, 0);
        EditCommand::changes(vec![TextChange::insert(snap.node.line_from, "  ")])
            .named("Indent list item")
    }

    #[test]
    fn an_extensions_edit_is_one_undo_step_apart_from_typing() {
        let mut history = History::new();
        let mut state = EditorState::new("- a\n- b\n");
        // The person types "x" then "y" after "- b" (one typing group).
        for (at, ch) in [(7, "x"), (8, "y")] {
            let spec = TransactionSpec::new()
                .changes(Changes::insert(at, ch))
                .selection(Selection::caret(at + 1))
                .annotate("origin", "input");
            history.record(&state, &spec);
            state = state.update(spec);
        }
        assert_eq!(state.doc.to_string(), "- a\n- bxy\n");
        // The extension indents both lines in one command.
        let cmd = EditCommand::changes(vec![
            TextChange::insert(0, "  "),
            TextChange::insert(4, "  "),
        ])
        .named("Indent");
        let spec = transaction(&state, &cmd, 0, "lab.outline").unwrap();
        assert_eq!(made_by(&spec), Some("lab.outline"));
        history.record(&state, &spec);
        state = state.update(spec);
        assert_eq!(state.doc.to_string(), "  - a\n  - bxy\n");
        assert_eq!(
            state.selection.primary().head,
            13,
            "the caret moved with its text"
        );
        // One undo takes back both lines' indent, and nothing typed.
        let undo = history.undo(&state).unwrap();
        state = state.update(undo);
        assert_eq!(state.doc.to_string(), "- a\n- bxy\n");
        // The next takes back the typing.
        let undo = history.undo(&state).unwrap();
        state = state.update(undo);
        assert_eq!(state.doc.to_string(), "- a\n- b\n");
    }

    #[test]
    fn a_bad_command_is_refused_not_applied() {
        let state = EditorState::new("abc");
        let cmd = EditCommand::changes(vec![TextChange::delete(1, 9)]);
        assert!(transaction(&state, &cmd, 0, "x").is_err());
        let cmd = EditCommand::default().select(vec![SelRange::caret(99)]);
        assert!(transaction(&state, &cmd, 0, "x").is_err());
    }

    #[test]
    fn keys_resolve_by_the_node_at_the_cursor() {
        let entries = vec![
            KeymapEntry {
                key: "Tab".into(),
                action: "IN".into(),
                when: Scope::Node(NodeKind::ListItem),
            },
            KeymapEntry {
                key: "Tab".into(),
                action: "NEXT_CELL".into(),
                when: Scope::Node(NodeKind::Table),
            },
        ];
        let tab = KeySpec::from("Tab");
        let mut state = EditorState::new("- a\n| b | c |\nplain");
        state.selection = Selection::caret(2);
        assert_eq!(
            resolve_key(&entries, &tab, &state).map(|e| e.action.as_str()),
            Some("IN")
        );
        state.selection = Selection::caret(6);
        assert_eq!(
            resolve_key(&entries, &tab, &state).map(|e| e.action.as_str()),
            Some("NEXT_CELL")
        );
        state.selection = Selection::caret(16);
        assert!(resolve_key(&entries, &tab, &state).is_none());
        let state = EditorState::new("- a");
        let spec = transaction(&state, &indent(&state), 0, "x").unwrap();
        assert_eq!(state.update(spec).doc.to_string(), "  - a");
    }

    #[test]
    fn specs_become_decorations_and_widgets_mount() {
        let text = "todo: call\n";
        let specs = vec![
            DecorationSpec::Mark {
                from: 0,
                to: 4,
                class: "todo".into(),
                title: Some("A task".into()),
            },
            DecorationSpec::Gutter {
                at: 2,
                text: "<●>".into(),
                class: "dot".into(),
            },
            DecorationSpec::Widget {
                at: 10,
                view: "count".into(),
                props: serde_json::Value::Null,
                block: true,
            },
            // Past the end: left out.
            DecorationSpec::Line {
                at: 99,
                class: "x".into(),
            },
        ];
        let decos = decorations(&specs, text);
        assert_eq!(decos.len(), 4, "{decos:?}");
        assert!(matches!(
            &decos.first().unwrap().kind,
            DecorationKind::Mark { class, attrs } if class == "ext-mark ext-todo" && attrs.len() == 1
        ));
        let DecorationKind::Widget { html } = &decos.get(2).unwrap().kind else {
            panic!("{decos:?}")
        };
        assert!(
            html.contains("&lt;●&gt;") && html.contains("ext-dot"),
            "{html}"
        );
        let DecorationKind::Widget { html } = &decos.get(3).unwrap().kind else {
            panic!("{decos:?}")
        };
        assert_eq!(mount_of(html), Some(("ext-2", true)));
        assert_eq!(mount_index("ext-2"), Some(2));
        let moved = mapped(&specs, &Changes::insert(0, "- "));
        assert_eq!(moved.first().map(DecorationSpec::at), Some(2));
    }
}
