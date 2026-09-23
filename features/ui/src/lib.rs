//! Dioxus view layer for the editor. Renders an
//! [`editor_state::EditorState`] into a contenteditable element
//! and translates user input into transactions.
//!
//! v1 scope: plain-text editing. Decorations come next.

pub use editor_state;

mod bridge;
mod editor;
mod event;
pub mod hover;
#[cfg(feature = "native")]
pub mod native;
pub mod palette;
pub mod tile;
pub mod trigger;

pub use editor::{DecorationSource, Editor, coarse_pointer};
pub use event::{TransactionEvent, dispatch_spec};
pub use hover::{HoverPopup, HoverTooltipView};
pub use trigger::{Candidate, CompletionKind, CompletionSource, CompletionState};

/// The editor's stylesheet, as text — for a host that injects styles
/// itself (a Blitz window, which does not load `<link>`ed sheets). The
/// umbrella crate serves the same file as an `asset!` for web and desktop.
pub const EDITOR_CSS: &str = include_str!("../../../crates/editor/assets/editor.css");
