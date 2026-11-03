//! Keyboard input for tests — FTS extension over the vendored upstream.
//!
//! Upstream `dioxus-test` only synthesizes `click` (and lets you hand-build
//! `PlatformEventData` yourself, which is impossible for keyboard events
//! because `dioxus-native-dom`'s `BlitzKeyboardData` is crate-private). These
//! helpers instead feed real [`blitz_traits::events::UiEvent`]s through
//! `DioxusDocument::handle_ui_event`, the same entry point the windowing
//! shell uses — so focus routing, `prevent_default`, and the dioxus event
//! conversion all behave exactly as they do in a running app.

use blitz_dom::Document as _;
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
    MouseEventButtons, Point, PointerCoords, PointerDetails, UiEvent,
};
use keyboard_types::{Code, Key, Location, Modifiers, NamedKey};
use smol_str::SmolStr;

use crate::{DocumentTester, ResolvedElement};

impl ResolvedElement {
    /// Gives this element document focus, as a click or tab-navigation
    /// would. Subsequent key events sent via
    /// [`DocumentTester::press_key`] / [`DocumentTester::type_text`] are
    /// routed here.
    pub fn focus(&self) {
        let mut doc = self.document.borrow_mut();
        let id = {
            let guard = doc.inner();
            self.node_id.resolve(&guard).id
        };
        doc.inner_mut().set_focus_to(id);
    }
}

fn key_event(key: Key, modifiers: Modifiers, state: KeyState) -> BlitzKeyEvent {
    let text = match (&key, state) {
        (Key::Character(s), KeyState::Pressed) => Some(SmolStr::new(s)),
        _ => None,
    };
    BlitzKeyEvent {
        key,
        code: Code::Unidentified,
        modifiers,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state,
        text,
    }
}

impl DocumentTester {
    /// Dispatches a raw Blitz [`UiEvent`] to the document.
    ///
    /// Key events are routed to the currently focused node; pointer events
    /// are hit-tested. This is the same path a real windowing shell drives,
    /// so it exercises focus handling and default actions — unlike
    /// [`crate::ResolvedElement::send_event`], which targets an element
    /// directly.
    pub fn send_ui_event(&self, event: UiEvent) {
        self.document.borrow_mut().handle_ui_event(event);
    }

    /// Presses and releases `key` with `modifiers` on the focused element.
    pub fn press_key(&self, key: Key, modifiers: Modifiers) {
        self.send_ui_event(UiEvent::KeyDown(key_event(
            key.clone(),
            modifiers,
            KeyState::Pressed,
        )));
        self.send_ui_event(UiEvent::KeyUp(key_event(
            key,
            modifiers,
            KeyState::Released,
        )));
    }

    /// Types `text` into the focused element, one key press per character.
    /// `'\n'` is sent as `Enter`.
    pub fn type_text(&self, text: &str) {
        for ch in text.chars() {
            let key = if ch == '\n' {
                Key::Named(NamedKey::Enter)
            } else {
                Key::Character(ch.to_string())
            };
            self.press_key(key, Modifiers::empty());
        }
    }

    /// The blitz-dom node currently under the pointer (test diagnostics). Wheel
    /// events scroll from this node, so a test checks it first.
    pub fn hover_node_id(&self) -> Option<blitz_dom::NodeId> {
        self.document.borrow().inner().get_hover_node_id()
    }

    /// The blitz-dom node id that currently holds focus (test diagnostics).
    pub fn blitz_focus(&self) -> Option<blitz_dom::NodeId> {
        self.document
            .borrow_mut()
            .inner_mut()
            .get_focussed_node_id()
    }

    /// Presses and releases the primary mouse button at page coordinates
    /// `(x, y)`. Unlike [`crate::ResolvedElement::click`] this goes through
    /// Blitz's real hit-testing and focus handling (focus follows
    /// pointerdown), so overlays, z-order, and per-tile mousedown handlers
    /// all behave as they would in a running app.
    pub fn click_at(&self, x: f32, y: f32) {
        self.send_ui_event(UiEvent::PointerDown(pointer_event(
            x,
            y,
            MouseEventButtons::Primary,
        )));
        self.send_ui_event(UiEvent::PointerUp(pointer_event(
            x,
            y,
            MouseEventButtons::empty(),
        )));
    }
}

fn pointer_event(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons,
        mods: Modifiers::empty(),
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: Default::default(),
    }
}
