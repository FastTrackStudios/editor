//! Pointer input for tests — FTS extension over the vendored upstream.
//!
//! Upstream `dioxus-test` only synthesizes `click` directly on a target
//! element (no hit-testing, no coordinates). These helpers feed real
//! [`blitz_traits::events::UiEvent`] pointer events through
//! `DioxusDocument::handle_ui_event` — the same entry point a windowing
//! shell drives — so events are hit-tested against the resolved layout,
//! bubble through the DOM, and carry real element-relative coordinates.
//! A `pointer_down` → `pointer_move`(s) → `pointer_up` sequence is a DRAG,
//! exactly as a user performs one in the plugin editor window.
//!
//! Companion to the keyboard extension in `src/keyboard.rs`.

use blitz_dom::Document as _;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, Point, PointerCoords,
    UiEvent,
};
use keyboard_types::Modifiers;

use crate::{DocumentTester, ResolvedElement};

/// Builds a mouse pointer event at document coordinates `(x, y)`.
///
/// All coordinate spaces (page/screen/client) are set to the same value —
/// the headless document has no scrolling or window chrome, so they
/// coincide. The `element` field is left at zero: the blitz event driver
/// recomputes it per target during dispatch.
fn pointer_event(
    x: f64,
    y: f64,
    button: MouseEventButton,
    buttons: MouseEventButtons,
    mods: Modifiers,
) -> BlitzPointerEvent {
    let (x, y) = (x as f32, y as f32);
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
        button,
        buttons,
        mods,
        details: Default::default(),
        element: Point::default(),
        active_pointers: Default::default(),
    }
}

impl DocumentTester {
    /// Presses the primary mouse button at document coordinates `(x, y)`.
    ///
    /// The event is hit-tested: whichever element the layout places under
    /// that point receives it (respecting `pointer-events: none`), and it
    /// bubbles from there. Call [`DocumentTester::pump`] afterwards to run
    /// the Dioxus event handlers.
    pub fn pointer_down(&self, x: f64, y: f64) {
        self.send_ui_event(UiEvent::PointerDown(pointer_event(
            x,
            y,
            MouseEventButton::Main,
            MouseEventButtons::Primary,
            Modifiers::empty(),
        )));
    }

    /// Moves the mouse pointer to document coordinates `(x, y)`.
    ///
    /// `held` reports whether the primary button is currently pressed —
    /// pass `true` for the intermediate events of a drag (components
    /// commonly check `held_buttons()` to distinguish dragging from
    /// hovering).
    pub fn pointer_move(&self, x: f64, y: f64, held: bool) {
        let buttons = if held {
            MouseEventButtons::Primary
        } else {
            MouseEventButtons::None
        };
        self.send_ui_event(UiEvent::PointerMove(pointer_event(
            x,
            y,
            MouseEventButton::Main,
            buttons,
            Modifiers::empty(),
        )));
    }

    /// Releases the primary mouse button at document coordinates `(x, y)`.
    pub fn pointer_up(&self, x: f64, y: f64) {
        self.send_ui_event(UiEvent::PointerUp(pointer_event(
            x,
            y,
            MouseEventButton::Main,
            MouseEventButtons::None,
            Modifiers::empty(),
        )));
    }
}

impl ResolvedElement {
    /// Document-space coordinates of this element's top-left corner.
    ///
    /// Unlike [`ResolvedElement::upper_left`] (which reports the
    /// parent-relative taffy layout position), this walks the layout
    /// ancestry via `Node::absolute_position`, yielding the coordinates
    /// the pointer helpers above expect.
    pub fn document_origin(&self) -> (f64, f64) {
        let guard = self.document.borrow();
        let doc = guard.inner();
        let node = self.node_id.resolve(&doc);
        let p = node.absolute_position(0.0, 0.0);
        (p.x as f64, p.y as f64)
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;
    use test_that::prelude::*;

    use crate::{matchers::inner_html, render};

    #[tokio::test]
    async fn pointer_drag_drives_mouse_handlers_with_element_coordinates() {
        #[component]
        fn Draggable() -> Element {
            let mut state = use_signal(|| "idle".to_string());
            rsx! {
                div {
                    style: "width: 200px; height: 100px;",
                    "data-testid": "surface",
                    onmousedown: move |evt| {
                        let c = evt.element_coordinates();
                        state.set(format!("down {} {}", c.x, c.y));
                    },
                    onmousemove: move |evt| {
                        let c = evt.element_coordinates();
                        state.set(format!("move {} {}", c.x, c.y));
                    },
                    onmouseup: move |evt| {
                        let c = evt.element_coordinates();
                        state.set(format!("up {} {}", c.x, c.y));
                    },
                    "{state}"
                }
            }
        }

        let tester = render(Draggable).build();
        // The UA stylesheet gives <body> a margin, so resolve the surface's
        // document-space origin (this also exercises `document_origin`).
        let (ox, oy) = tester
            .query(crate::by_testid("surface"))
            .immediately()
            .unwrap()
            .document_origin();
        tester.pointer_down(ox + 10.0, oy + 20.0);
        tester
            .query(crate::by_testid("surface"))
            .expect(inner_html(contains_substring("down 10 20")))
            .await
            .unwrap();
        tester.pointer_move(ox + 50.0, oy + 60.0, true);
        tester
            .query(crate::by_testid("surface"))
            .expect(inner_html(contains_substring("move 50 60")))
            .await
            .unwrap();
        tester.pointer_up(ox + 50.0, oy + 60.0);
        tester
            .query(crate::by_testid("surface"))
            .expect(inner_html(contains_substring("up 50 60")))
            .await
            .unwrap();
    }
}
