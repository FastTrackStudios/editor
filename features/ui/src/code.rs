//! `CodeEditor`: the editor for a source file, where [`Editor`] is the
//! one for prose and markdown.
//!
//! The same component underneath, on the same document model
//! (`Signal<EditorState>`), on Blitz and the web alike; what changes is
//! what it is given:
//!
//! - decorations from [`editor_state::code::decorations`]: the whole buffer
//!   highlighted in one language (built in, or one the host installed;
//!   `editor_state::languages`), line numbers in a gutter, the bracket
//!   pair at the caret, and the host's diagnostics as underlines with a
//!   mark on their line's number;
//! - the code keymap ([`editor_state::code::keymap`]): Enter keeps the
//!   indent (one more after an opening bracket), Tab and Shift-Tab indent
//!   and outdent, undo, redo, select all;
//! - no markdown pass: `#` is not a heading, `*` is not emphasis.
//!
//! The host's own keys, decorations, widgets and history (the
//! `editor-ext` surface) pass through as they do on [`Editor`]: its
//! `keymap` runs before the code keys, its `decorations` are drawn
//! with the code ones.
//!
//! [`Editor`]: crate::Editor

// The props derive compares the hover source's fn pointer, as `Editor`'s do
// (see the same expectation in `editor.rs`).
#![expect(
    unpredictable_function_pointer_comparisons,
    reason = "prop-diff equality on fn pointers; a false positive costs one redundant re-render"
)]

use dioxus::prelude::*;
use editor_state::code::{self, CodeView};
use editor_state::languages::Syntax;
use editor_state::{Diagnostic, EditorState, KeySpec, Keymap};

use crate::DecorationSource;

/// The prose and markdown editor, by the name a host that also uses
/// [`CodeEditor`] may prefer. The same component as [`crate::Editor`].
pub use crate::editor::Editor as TextEditor;

/// The class on the code editor's wrapper, which the stylesheet's code
/// rules hang from (monospace, no wrapping, the gutter's width).
pub const CODE_EDITOR_CLASS: &str = "editor-code";

/// An editor for a source file.
///
/// `language` and `diagnostics` are reactive: pass a new value and the
/// buffer is decorated again. The keymap and decorations props, and the
/// rest, behave as on [`crate::Editor`].
#[component]
pub fn CodeEditor(
    state: Signal<EditorState>,
    /// The buffer's language (`editor_state::languages::language_for_path`
    /// finds one by file name). `None`: plain text.
    #[props(default)]
    language: ReadSignal<Option<Syntax>>,
    /// The host's diagnostics, byte ranges into the buffer: an underline
    /// each (the message as its title) and a mark on the line's number.
    #[props(default)]
    diagnostics: ReadSignal<Vec<Diagnostic>>,
    /// A number before every line. Read when the editor mounts.
    #[props(default = true)]
    line_numbers: bool,
    /// The host's keys, tried before the code keymap.
    #[props(default)]
    keymap: Option<Keymap>,
    /// The host's decorations (an extension's marks, gutter markers and
    /// widgets), drawn with the code ones.
    #[props(default)]
    decorations: ReadSignal<Option<DecorationSource>>,
    #[props(default)] vim: Option<Signal<editor_vim::VimState>>,
    #[props(default)] hover: Option<editor_state::HoverSource>,
    #[props(default)] on_transaction: Option<Callback<crate::TransactionEvent>>,
    #[props(default)] completion: Option<crate::trigger::CompletionSource>,
    #[props(default = true)] editable: bool,
    #[props(default)] focused: Option<Signal<bool>>,
    #[props(default)] focus_request: Option<Signal<u64>>,
    #[props(default)] history: Option<Signal<editor_state::History>>,
    #[props(default)] on_key: Option<Callback<KeySpec, bool>>,
    #[props(default)] mounts: Option<crate::WidgetMounts>,
) -> Element {
    // One source for the editor's life. It reads the reactive props, so
    // whoever runs it (the native render, the web patch effect) runs
    // again when the language or the diagnostics change.
    let source = use_hook(|| {
        DecorationSource::new(move |s: &EditorState| {
            let language = language.read();
            let diagnostics = diagnostics.read();
            let mut out = code::decorations(
                s,
                CodeView {
                    language: language.as_ref(),
                    diagnostics: &diagnostics,
                    line_numbers,
                },
            );
            if let Some(extra) = decorations.read().as_ref() {
                out.extend(extra.run(s));
            }
            out
        })
    });
    let mut keys = keymap.unwrap_or_default();
    for b in code::keymap().bindings() {
        keys = keys.with(b.key.clone(), b.run);
    }
    let class = if line_numbers {
        format!("{CODE_EDITOR_CLASS} {CODE_EDITOR_CLASS}-numbered")
    } else {
        CODE_EDITOR_CLASS.to_owned()
    };
    let lang_name = language.read().as_ref().map(|l| l.name().to_owned());
    rsx! {
        div { class, "data-language": lang_name,
            crate::Editor {
                state,
                keymap: keys,
                decorations: source,
                vim,
                hover,
                on_transaction,
                completion,
                editable,
                focused,
                focus_request,
                history,
                on_key,
                mounts,
            }
        }
    }
}
