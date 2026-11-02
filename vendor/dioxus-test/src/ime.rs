//! IME input for tests — FTS extension over the vendored upstream.
//!
//! [`DocumentTester::ime`] feeds a platform IME event through
//! `DioxusDocument::handle_ui_event`, the entry point the windowing shell
//! drives with winit's `Ime` events, so the focused element receives it
//! exactly as in a running app. The tester's shell records the IME
//! requests the document makes ([`DocumentTester::ime_enabled`]), which is
//! how a test sees whether focusing an element turned the IME on.

use std::sync::Mutex;

pub use blitz_traits::events::BlitzImeEvent;
use blitz_traits::events::UiEvent;
use blitz_traits::shell::ShellProvider;

use crate::DocumentTester;

/// The tester's shell: records the last IME enable/disable request.
#[derive(Default)]
pub(crate) struct RecordingShell {
    ime_enabled: Mutex<Option<bool>>,
}

impl ShellProvider for RecordingShell {
    fn set_ime_enabled(&self, is_enabled: bool) {
        if let Ok(mut slot) = self.ime_enabled.lock() {
            *slot = Some(is_enabled);
        }
    }
}

impl DocumentTester {
    /// Dispatches a platform IME event (preedit, commit, …) to the focused
    /// element.
    pub fn ime(&self, event: BlitzImeEvent) {
        self.send_ui_event(UiEvent::Ime(event));
    }

    /// The last IME request the document made of its shell: `Some(true)`
    /// once a focused element turned the IME on, `Some(false)` once a blur
    /// turned it off, `None` if it never asked.
    pub fn ime_enabled(&self) -> Option<bool> {
        self.shell.ime_enabled.lock().ok().and_then(|slot| *slot)
    }
}
