//! JS → Rust bridge message dispatcher. The editor component
//! attaches a `document::eval` script to the DOM root; that
//! script forwards `input` / `selectionchange` / `beforeinput`
//! events as JSON messages here. Pulled out of `editor.rs` to
//! keep that file focused on the Dioxus component + render path.
//!
//! Conceptually the mirror of CM6's `DOMObserver` — translates
//! user-initiated DOM mutations back into `TransactionSpec`s the
//! authoritative state can absorb.
//!
//! The dispatcher routes on the `kind` field:
//!
//! - `input` / `before-input-*` / `composition-*` → user typing,
//!   delete, IME compose; diff against the visible-text mirror,
//!   translate to doc-space changes, push a transaction.
//! - `sel` → user moved the caret with mouse / arrows; push a
//!   selection-only transaction (with clamp-detection guards).
//! - `insert-bracket` / `enter-continue-list` → behaviors the
//!   keymap declares but that the browser's default would have
//!   handled, intercepted at `beforeinput` and re-applied here.
//! - `copy-range` → code-block / list-item copy buttons.
//! - `task-toggle` → clicking a task checkbox widget.
//! - `prop-set` / `prop-list-add` / `prop-list-remove` →
//!   frontmatter property cell edits; routes through
//!   `apply_property_change`.
//! - `prop-focus` → a typing cell gained focus; flip vim to
//!   Insert so the mode badge/painted caret agree with where
//!   keystrokes land (otherwise "Normal" j/k types into the
//!   cell).
//! - `prop-leave` → cell blurred via Esc/Enter; flip vim back
//!   to Normal and refocus the editor root.
//! - `focus-pos` → click on a math/typst/property widget;
//!   drops caret at an explicit byte offset inside the source
//!   so its Replace decoration goes away and editing is live.
//! - `link-clicked` → click on a link/wikilink; drop selection
//!   so the source markers stay collapsed.

use dioxus::prelude::*;
use editor_state::{
    Change, Changes, DecoratedRange, EditorState, Range, Selection, TextSlice, TransactionSpec,
};

use crate::editor::{DecorationSource, diff_text, push_selection};
use crate::event::{TransactionEvent, apply_tx};
use crate::tile::build::build_tiles;
use crate::tile::visible::VisibleText;

pub fn handle_bridge_msg(
    state: Signal<EditorState>,
    deco_source: Option<&DecorationSource>,
    vim: Option<Signal<editor_vim::VimState>>,
    widget_focus: Signal<bool>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
) {
    let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let sel = v.get("sel").and_then(|s| s.as_array());
    let (s_off, e_off) = match sel.map(Vec::as_slice) {
        Some([a0, a1]) => (
            usize::try_from(a0.as_u64().unwrap_or(0)).unwrap_or(0),
            usize::try_from(a1.as_u64().unwrap_or(0)).unwrap_or(0),
        ),
        _ => (0, 0),
    };

    match kind {
        "input" => handle_input(state, deco_source, sink, v, s_off, e_off),
        "sel" => {
            let cur = state.read().clone();
            push_selection(&state, &cur, deco_source, sink, s_off, e_off);
        }
        "before-input-insert" => {
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let cur = state.read().clone();
            let doc_len = cur.doc.len();
            let from = s_off.min(doc_len);
            let to = e_off.min(doc_len).max(from);
            tracing::debug!(
                from,
                to,
                inserted_len = text.len(),
                "editor.before_input.insert"
            );
            let changes = Changes::replace(from..to, text);
            let caret = from.saturating_add(text.len());
            let new_sel = Selection::single(Range::caret(caret));
            apply_tx(
                state,
                &cur,
                TransactionSpec::new()
                    .changes(changes)
                    .selection(new_sel)
                    .annotate("origin", "before-input"),
                sink,
            );
        }
        "before-input-delete-backward" => handle_delete_backward(state, sink, s_off, e_off),
        "before-input-delete-forward" => handle_delete_forward(state, sink, s_off, e_off),
        "composition-start" => {
            tracing::debug!("editor.composition.start");
        }
        "composition-end" => {
            tracing::debug!("editor.composition.end");
        }
        "insert-bracket" => handle_insert_bracket(state, sink, v, s_off, e_off),
        "enter-continue-list" => {
            let cur_clone = {
                let cur = state.read().clone();
                let doc_len = cur.doc.len();
                let from = s_off.min(doc_len);
                let to = e_off.min(doc_len).max(from);
                EditorState {
                    selection: Selection::single(Range::new(from, to)),
                    ..cur
                }
            };
            if let Some(spec) = editor_state::commands::enter_continue_list(&cur_clone) {
                apply_tx(state, &cur_clone, spec.annotate("origin", "enter"), sink);
            }
        }
        "copy-range" => handle_copy_range(state, v),
        "task-toggle" => handle_task_toggle(state, sink, v),
k if is_property_msg(k) => {
            handle_property_msg(k, state, vim, widget_focus, sink, v);
        }
        "focus-pos" => {
            let pos = usize::try_from(
                v.get("pos")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
            .unwrap_or(0);
            let cur = state.read().clone();
            let pos = pos.min(cur.doc.len());
            apply_tx(
                state,
                &cur,
                TransactionSpec::new()
                    .selection(Selection::caret(pos))
                    .annotate("origin", "focus-pos"),
                sink,
            );
        }
        "link-clicked" => {
            let cur = state.read().clone();
            apply_tx(
                state,
                &cur,
                TransactionSpec::new()
                    .selection(Selection::caret(0))
                    .annotate("origin", "link-clicked"),
                sink,
            );
        }
        _ => {}
    }
}

/// Resolve a `prop-*` bridge message into a `Changes` against
/// the frontmatter and dispatch the transaction. `mutate` is
/// given the existing `PropValue` and the raw payload, and
/// returns the new `PropValue`.
/// A text edit confirmed by the DOM, arriving as the browser's new visible text.
///
/// Rebuilds the visible-text mirror, diffs it against what the browser now
/// shows, and maps that back into document changes. Split out of
/// [`handle_bridge_msg`]'s dispatch, where it was by far the largest arm.
fn handle_input(
    state: Signal<EditorState>,
    deco_source: Option<&DecorationSource>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
    s_off: usize,
    e_off: usize,
) {
        let new_visible = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let cur = state.read().clone();
        // This pass only reconstructs the visible-text mirror to diff
        // against the DOM — it needs the structural (marker-hiding)
        // decorations, never the expensive analysis-derived ones. Pin
        // the phase so a source's heavy work is skipped on every
        // keystroke here regardless of the view's idle state.
        editor_state::set_deco_phase(editor_state::DecoPhase::Structural);
        let decorations: Vec<DecoratedRange> = deco_source.map_or_else(Vec::new, |src| {
            let mut v = src.run(&cur);
            v.sort_by_key(|d| d.from);
            v
        });
        let (arena, root) = build_tiles(&cur.doc.to_string(), &decorations);
        let old_visible = VisibleText::from_arena(&arena, root);
        if old_visible.text == new_visible {
            // Echo of our own re-render — selection on this
            // message is unreliable (text nodes may have
            // been recreated). Trust only the dedicated
            // `sel` channel for selection.
            return;
        }
        let vis_changes = diff_text(&old_visible.text, new_visible);
        if vis_changes.is_empty() {
            push_selection(&state, &cur, deco_source, sink, s_off, e_off);
            return;
        }
        let primary = cur.selection.primary();
        let vis_vec: Vec<Change> = vis_changes.iter().cloned().collect();
        let single_caret_insert = if primary.anchor == primary.head {
            match vis_vec.as_slice() {
                [c] if c.from == c.to => Some(c.inserted.clone()),
                _ => None,
            }
        } else {
            None
        };
        let mut doc_changes: Vec<Change> = Vec::new();
        if let Some(inserted) = single_caret_insert {
            let head = primary.head.min(cur.doc.len());
            doc_changes.push(Change {
                from: head,
                to: head,
                inserted,
            });
        } else {
            for c in &vis_vec {
                let doc_from = old_visible.visible_to_doc(c.from);
                let doc_to = old_visible.visible_to_doc(c.to);
                doc_changes.push(Change {
                    from: doc_from,
                    to: doc_to,
                    inserted: c.inserted.clone(),
                });
            }
        }
        let changes = Changes::from_sorted(doc_changes);
        let new_doc_len = isize::try_from(cur.doc.len()).unwrap_or(isize::MAX).saturating_add(
            changes
                .iter()
                .map(editor_state::Change::delta)
                .sum::<isize>(),
        );
        let new_doc_len = usize::try_from(new_doc_len.max(0)).unwrap_or(0);
        let intended_caret = changes
            .iter()
            .last()
            .map_or(s_off, |c| {
                let prior_delta: isize = changes
                    .iter()
                    .take_while(|x| x.from < c.from)
                    .map(editor_state::Change::delta)
                    .sum();
                let from_isize = isize::try_from(c.from).unwrap_or(isize::MAX);
                let inserted_isize = isize::try_from(c.inserted.len()).unwrap_or(isize::MAX);
                usize::try_from(
                    from_isize
                        .saturating_add(prior_delta)
                        .saturating_add(inserted_isize)
                        .max(0),
                )
                .unwrap_or(0)
            })
            .min(new_doc_len);
        tracing::debug!(
            old_visible_len = old_visible.text.len(),
            new_visible_len = new_visible.len(),
            new_doc_len,
            intended_caret,
            js_sel_start = s_off,
            js_sel_end = e_off,
            "editor.input"
        );
        let new_sel = Selection::caret(intended_caret);
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(new_sel)
                .annotate("origin", "input"),
            sink,
        );
    
}

/// `deleteContentBackward` — backspace, routed through state instead of
/// letting the DOM mutate itself.
///
/// Split out of [`handle_bridge_msg`]'s dispatch.
fn handle_delete_backward(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    s_off: usize,
    e_off: usize,
) {
        let cur = state.read().clone();
        let doc_len = cur.doc.len();
        let from = s_off.min(doc_len);
        let to = e_off.min(doc_len).max(from);
        let (del_from, del_to) = if from == to {
            if from == 0 {
                return;
            }
            let doc_str = cur.doc.to_string();
            let mut back = from.saturating_sub(1);
            while back > 0 && !doc_str.is_char_boundary(back) {
                back = back.saturating_sub(1);
            }
            (back, from)
        } else {
            (from, to)
        };
        tracing::debug!(del_from, del_to, "editor.before_input.delete_backward");
        let changes = Changes::delete(del_from..del_to);
        let new_sel = Selection::single(Range::caret(del_from));
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(new_sel)
                .annotate("origin", "before-input"),
            sink,
        );
    
}

/// `deleteContentForward` — the delete key. Mirror of
/// [`handle_delete_backward`].
///
/// Split out of [`handle_bridge_msg`]'s dispatch.
fn handle_delete_forward(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    s_off: usize,
    e_off: usize,
) {
        let cur = state.read().clone();
        let doc_len = cur.doc.len();
        let from = s_off.min(doc_len);
        let to = e_off.min(doc_len).max(from);
        let (del_from, del_to) = if from == to {
            if from >= doc_len {
                return;
            }
            let doc_str = cur.doc.to_string();
            let mut fwd = from.saturating_add(1);
            while fwd < doc_len && !doc_str.is_char_boundary(fwd) {
                fwd = fwd.saturating_add(1);
            }
            (from, fwd)
        } else {
            (from, to)
        };
        tracing::debug!(del_from, del_to, "editor.before_input.delete_forward");
        let changes = Changes::delete(del_from..del_to);
        let new_sel = Selection::single(Range::caret(del_from));
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(new_sel)
                .annotate("origin", "before-input"),
            sink,
        );
    
}


/// Kinds handled by [`handle_property_msg`] — the frontmatter property editor.
fn is_property_msg(kind: &str) -> bool {
    matches!(
        kind,
        "widget-focus"
            | "prop-focus"
            | "prop-leave"
            | "prop-set"
            | "prop-list-add"
            | "prop-add"
            | "prop-remove"
            | "prop-list-remove"
    )
}

/// The frontmatter property-editor messages.
///
/// Split out of [`handle_bridge_msg`]'s dispatch as a group: these eight arms
/// all edit the YAML property block and share nothing with the text-editing
/// path above them.
fn handle_property_msg(
    kind: &str,
    state: Signal<EditorState>,
    vim: Option<Signal<editor_vim::VimState>>,
    mut widget_focus: Signal<bool>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
) {
    match kind {
    "widget-focus" => {
        let focused = v
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        widget_focus.set(focused);
    }
    "prop-focus" => {
        if let Some(mut vs) = vim {
            let cur = vs.peek().clone();
            if cur.mode != editor_vim::Mode::Insert {
                let mut next = cur;
                next.mode = editor_vim::Mode::Insert;
                next.clear_pending();
                vs.set(next);
            }
        }
    }
    "prop-leave" => {
        if let Some(mut vs) = vim {
            let mut next = vs.peek().clone();
            next.mode = editor_vim::Mode::Normal;
            next.clear_pending();
            vs.set(next);
        }
        let script =
            r"(()=>{const el=document.querySelector('.editor-root');if(el)el.focus();})();";
        let _ = document::eval(script);
    }
    "prop-set" => {
        apply_property_change(state, sink, v, |old, ty, value| {
            use editor_state::markdown::PropValue;
            match ty {
                "text" => PropValue::Text(value.to_string()),
                "number" => value
                    .parse::<f64>()
                    .map_or_else(|_| PropValue::Text(value.to_string()), PropValue::Number),
                "date" => PropValue::Date(value.to_string()),
                "bool" => PropValue::Bool(value == "true"),
                _ => {
                    let _ = old;
                    PropValue::Text(value.to_string())
                }
            }
        });
    }
    "prop-list-add" => {
        apply_property_change(state, sink, v, |old, _ty, value| {
            use editor_state::markdown::PropValue;
            let mut items = match old {
                PropValue::List(v) => v.clone(),
                _ => Vec::new(),
            };
            let value = value.trim();
            if !value.is_empty() && !items.iter().any(|x| x == value) {
                items.push(value.to_string());
            }
            PropValue::List(items)
        });
    }
    "prop-add" => handle_prop_add(state, sink, v),
    "prop-remove" => handle_prop_remove(state, sink, v),
    "prop-list-remove" => {
        apply_property_change(state, sink, v, |old, _ty, value| {
            use editor_state::markdown::PropValue;
            let items = match old {
                PropValue::List(v) => {
                    v.iter().filter(|x| x.as_str() != value).cloned().collect()
                }
                _ => Vec::new(),
            };
            PropValue::List(items)
        });
    }
        _ => {}
    }
}

/// `insert-bracket` — auto-pairing for brackets and quotes.
///
/// Split out of [`handle_bridge_msg`]'s dispatch.
fn handle_insert_bracket(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
    s_off: usize,
    e_off: usize,
) {
        let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let cur_clone = {
            let cur = state.read().clone();
            let doc_len = cur.doc.len();
            let from = s_off.min(doc_len);
            let to = e_off.min(doc_len).max(from);
            EditorState {
                selection: Selection::single(Range::new(from, to)),
                ..cur
            }
        };
        if let Some(spec) = editor_state::commands::insert_bracket(&cur_clone, text) {
            apply_tx(
                state,
                &cur_clone,
                spec.annotate("origin", "insert-bracket"),
                sink,
            );
        } else {
            let p = cur_clone.selection.primary();
            let changes = editor_state::Changes::replace(p.from()..p.to(), text);
            let caret = p.from().saturating_add(text.len());
            apply_tx(
                state,
                &cur_clone,
                editor_state::TransactionSpec::new()
                    .changes(changes)
                    .selection(editor_state::Selection::caret(caret))
                    .annotate("origin", "insert-bracket-plain"),
                sink,
            );
        }
    
}

/// `task-toggle` — clicking a `- [ ]` checkbox in the rendered view.
///
/// Split out of [`handle_bridge_msg`]'s dispatch.
fn handle_task_toggle(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
) {
        let pos = usize::try_from(
            v.get("pos")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0);
        let cur = state.read().clone();
        let doc = cur.doc.to_string();
        let bracket_idx = pos.saturating_add(3);
        let next_char = doc.as_bytes().get(bracket_idx).copied();
        let new_char = match next_char {
            Some(b' ') => "x",
            Some(b'x' | b'X') => " ",
            _ => return,
        };
        let changes = Changes::replace(bracket_idx..bracket_idx.saturating_add(1), new_char);
        let prev_sel = cur.selection.clone();
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(prev_sel)
                .annotate("origin", "task-toggle"),
            sink,
        );
    
}

/// `copy-range` — hand the host the raw source text behind a rendered range.
///
/// Split out of [`handle_bridge_msg`]'s dispatch.
fn handle_copy_range(state: Signal<EditorState>, v: &serde_json::Value) {
        let from = usize::try_from(
            v.get("from")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0);
        let to = usize::try_from(
            v.get("to").and_then(serde_json::Value::as_u64).unwrap_or(0),
        )
        .unwrap_or(0);
        let cur = state.read();
        let doc = cur.doc.to_string();
        let from = from.min(doc.len());
        let to = to.min(doc.len()).max(from);
        let slice = doc.slice(from..to).to_string();
        let escaped = slice.replace('\\', "\\\\").replace('`', "\\`");
        let script =
            format!(r"navigator.clipboard && navigator.clipboard.writeText(`{escaped}`);");
        let _ = document::eval(&script);
    
}

/// `prop-add` — append a new key to the frontmatter block, creating the block
/// itself if the document has none.
///
/// Split out of [`handle_property_msg`].
fn handle_prop_add(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
) {
        // Append a new key with an empty value at the end
        // of the frontmatter block, just before the closing
        // `---`. The user is left focused on the
        // contenteditable add-row cell (cleared by JS); the
        // next live-preview pass renders the new row and
        // they can click into it to set a value.
        let Some(key) = v.get("key").and_then(|k| k.as_str()) else {
            return;
        };
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        let cur = state.read().clone();
        let doc = cur.doc.to_string();
        let Some(fm) = editor_state::markdown::parse_frontmatter(&doc) else {
            return;
        };
        // Reject duplicates so we don't end up with two
        // entries the parser would collapse.
        if fm.props.iter().any(|p| p.key == key) {
            return;
        }
        let insert_at = fm.closer.start;
        let new_text = editor_state::markdown::serialize_property(
            key,
            &editor_state::markdown::PropValue::Empty,
        );
        let changes = Changes::insert(insert_at, &new_text);
        let prev_sel = cur.selection.clone();
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(prev_sel)
                .annotate("origin", "prop-add"),
            sink,
        );
    
}

/// `prop-remove` — delete a key from the frontmatter block.
///
/// Split out of [`handle_property_msg`].
fn handle_prop_remove(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
) {
        // Delete the entire `prop.range` for the named key.
        let Some(key) = v.get("key").and_then(|k| k.as_str()) else {
            return;
        };
        let cur = state.read().clone();
        let doc = cur.doc.to_string();
        let Some(fm) = editor_state::markdown::parse_frontmatter(&doc) else {
            return;
        };
        let Some(prop) = fm.props.iter().find(|p| p.key == key) else {
            return;
        };
        let changes = Changes::delete(prop.range.clone());
        // Caret stays put — the deletion is below it in most
        // cases (closer is at the end of the FM range), and
        // map_through handles the rest.
        let prev_sel = cur.selection.clone();
        apply_tx(
            state,
            &cur,
            TransactionSpec::new()
                .changes(changes)
                .selection(prev_sel)
                .annotate("origin", "prop-remove"),
            sink,
        );
    
}

fn apply_property_change(
    state: Signal<EditorState>,
    sink: Option<Callback<TransactionEvent>>,
    v: &serde_json::Value,
    mutate: impl FnOnce(
        &editor_state::markdown::PropValue,
        &str,
        &str,
    ) -> editor_state::markdown::PropValue,
) {
    let Some(key) = v.get("key").and_then(|k| k.as_str()) else {
        return;
    };
    let value = v.get("value").and_then(|x| x.as_str()).unwrap_or("");
    let ty = v.get("ty").and_then(|x| x.as_str()).unwrap_or("text");
    let cur = state.read().clone();
    let doc = cur.doc.to_string();
    let Some(fm) = editor_state::markdown::parse_frontmatter(&doc) else {
        return;
    };
    let Some(prop) = fm.props.iter().find(|p| p.key == key) else {
        return;
    };
    let new_value = mutate(&prop.value, ty, value);
    if values_equal(&prop.value, &new_value) {
        return;
    }
    let new_text = editor_state::markdown::serialize_property(key, &new_value);
    let changes = Changes::replace(prop.range.clone(), new_text);
    let prev_sel = cur.selection.clone();
    apply_tx(
        state,
        &cur,
        TransactionSpec::new()
            .changes(changes)
            .selection(prev_sel)
            .annotate("origin", "prop-edit"),
        sink,
    );
}

fn values_equal(
    a: &editor_state::markdown::PropValue,
    b: &editor_state::markdown::PropValue,
) -> bool {
    use editor_state::markdown::PropValue::{Bool, Date, Empty, List, Number, Text};
    match (a, b) {
        (Text(x), Text(y)) | (Date(x), Date(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        // Exact equality is the question here (is this the same JSON value?),
        // not an approximate one; comparing bit patterns says that without
        // inviting an epsilon that would make 0.1+0.2 "equal" to 0.3.
        (Number(x), Number(y)) => x.to_bits() == y.to_bits(),
        (List(x), List(y)) => x == y,
        (Empty, Empty) => true,
        _ => false,
    }
}
