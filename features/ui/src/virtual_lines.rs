//! Line virtualization for the native (Blitz) renderer.
//!
//! The editor renders only the lines near its viewport, so an edit costs
//! what is on screen rather than what is in the document. This module owns
//! the state that makes that work: the scroll container, its measured
//! viewport, the measured heights of the lines, and the caret's line kept in
//! view.
//!
//! Blitz sends no `scroll` events, so the viewport is polled. Scroll offset
//! and client rect are read through `MountedData` on a short timer while the
//! editor is mounted.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;
use futures_util::StreamExt as _;
use editor_state::EditorState;

use crate::tile::window::{ESTIMATED_LINE_PX, Window, window};

/// Pixels of lines rendered above and below the viewport, so a small scroll
/// shows no gap while the next lines are built.
pub const OVERSCAN_PX: f64 = 600.0;

/// How often the viewport is read while the editor is mounted.
const POLL: Duration = Duration::from_millis(16);

/// The scroll container's position and height, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub top: f64,
    pub height: f64,
}

/// Handles to the virtualization state.
#[derive(Clone)]
pub struct VirtualLines {
    scroller: Signal<Option<Rc<MountedData>>>,
    viewport: Signal<Viewport>,
    heights: Signal<Vec<f64>>,
    handles: Signal<HashMap<usize, Rc<MountedData>>>,
    /// The lines the last render built. Handles to lines outside it are stale.
    built: Rc<std::cell::Cell<std::ops::Range<usize>>>,
    /// Document offset where each line tile starts, from the last render. A
    /// document line is not always a line tile (a block can span several), so
    /// the caret's tile is found through this, not through its line number.
    starts: Rc<RefCell<Vec<usize>>>,
}

/// Set up the virtualization state for `state`. Call once, unconditionally,
/// from the component body (it creates hooks).
pub fn use_virtual_lines(state: Signal<EditorState>) -> VirtualLines {
    let v = VirtualLines {
        scroller: use_signal(|| None),
        viewport: use_signal(Viewport::default),
        heights: use_signal(Vec::new),
        handles: use_signal(HashMap::new),
        starts: Rc::new(RefCell::new(Vec::new())),
        built: Rc::new(std::cell::Cell::new(0..0)),
    };

    // Poll the scroll container. Only writes when the viewport moved, so an
    // idle editor re-renders nothing.
    let scroller = v.scroller;
    let mut viewport = v.viewport;
    let handles = v.handles;
    let heights = v.heights;
    let built = v.built.clone();
    use_future(move || {
        let built = built.clone();
        async move {
            let mut ticks = ticker(POLL);
            while ticks.next().await.is_some() {
                let Some(el) = scroller.peek().clone() else {
                    continue;
                };
                let top = el.get_scroll_offset().await.map_or(0.0, |o| o.y);
                let height = el.get_client_rect().await.map_or(0.0, |r| r.size.height);
                let next = Viewport { top, height };
                if *viewport.peek() != next {
                    viewport.set(next);
                    // The scroll moved which lines are built. Measure the ones that
                    // are built now, once the layout has run for them.
                    let rendered = prune_and_snapshot(handles, &built);
                    for (index, el) in rendered {
                        measure(heights, index, &el).await;
                    }
                }
            }
        }
    });

    // Re-measure the rendered lines after every change to the document.
    let built_for_effect = v.built.clone();
    use_effect(move || {
        let _ = state.read();
        let rendered = prune_and_snapshot(handles, &built_for_effect);
        spawn(async move {
            for (index, el) in rendered {
                measure(heights, index, &el).await;
            }
        });
    });

    // Keep the caret's line on screen when editing or navigation moves it
    // out of the viewport.
    let scroller = v.scroller;
    let viewport = v.viewport;
    let starts = v.starts.clone();
    // The caret's head, and whether the viewport was known, at the last check.
    // The caret is scrolled into view when it moves, or once the viewport
    // first becomes known. Scrolling by hand never triggers it, so it does not
    // fight the user.
    // `use_hook`, not a plain `Rc::new`: a plain value is rebuilt on every
    // render, which made this guard forget the last check and scroll each time.
    let last = use_hook(|| Rc::new(std::cell::Cell::new((usize::MAX, false))));
    use_effect(move || {
        let s = state.read();
        let head = s.selection.primary().head.min(s.doc.len());
        let vp = *viewport.read();
        let known = vp.height > 0.0;
        let (last_head, last_known) = last.get();
        if head == last_head && known == last_known {
            return;
        }
        last.set((head, known));
        if !known {
            return;
        }
        // The caret's line tile: the last tile starting at or before the caret.
        let starts = starts.borrow();
        if starts.is_empty() {
            return;
        }
        let line = starts.partition_point(|&p| p <= head).saturating_sub(1);
        let hs = heights.peek();
        let top: f64 = (0..line).map(|i| height_of(&hs, i)).sum();
        let line_h = height_of(&hs, line);
        let target = if top < vp.top {
            Some(top)
        } else if top + line_h > vp.top + vp.height {
            Some(top + line_h - vp.height)
        } else {
            None
        };
        if let (Some(target), Some(el)) = (target, scroller.peek().clone()) {
            spawn(async move {
                let at = dioxus::html::geometry::PixelsVector2D::new(0.0, target);
                let _ = el.scroll(at, ScrollBehavior::Instant).await;
            });
        }
    });

    v
}

/// The height of line `i`, estimated when it has not been measured.
fn height_of(heights: &[f64], i: usize) -> f64 {
    heights.get(i).copied().unwrap_or(ESTIMATED_LINE_PX)
}

/// Forget the handles of lines the last render did not build (their nodes are
/// gone or going), and return the rest, for measuring. A handle to a removed
/// node must not be measured: it reads an element that is no longer in the tree.
fn prune_and_snapshot(
    mut handles: Signal<HashMap<usize, Rc<MountedData>>>,
    built: &Rc<std::cell::Cell<std::ops::Range<usize>>>,
) -> Vec<(usize, Rc<MountedData>)> {
    let range = built.take();
    built.set(range.clone());
    handles.with_mut(|h| h.retain(|i, _| range.contains(i)));
    handles
        .peek()
        .iter()
        .map(|(&i, el)| (i, el.clone()))
        .collect()
}

/// Read one rendered line's height and record it, if it changed.
async fn measure(mut heights: Signal<Vec<f64>>, index: usize, el: &MountedData) {
    let Ok(rect) = el.get_client_rect().await else {
        return;
    };
    let h = rect.size.height;
    // A line that has not been laid out yet reads as zero. Keep its estimate
    // rather than record a height that would collapse the document.
    if h <= 0.0 {
        return;
    }
    let current = heights.peek().get(index).copied();
    if current.is_some_and(|c| (c - h).abs() <= 0.5) {
        return;
    }
    heights.with_mut(|v| {
        if v.len() <= index {
            v.resize(index + 1, ESTIMATED_LINE_PX);
        }
        v[index] = h;
    });
}

impl VirtualLines {
    /// Attach the scroll container. Call from the container's `onmounted`.
    pub fn attach(&self, el: Rc<MountedData>) {
        let mut scroller = self.scroller;
        scroller.set(Some(el));
    }

    /// Record where each line tile starts in the document, from this render.
    /// Called during render: it only writes the table, so it re-renders nothing.
    pub fn record_line_starts(&self, starts: Vec<usize>) {
        *self.starts.borrow_mut() = starts;
    }

    /// Record a line's mounted element, and measure it. Call from each
    /// rendered line's `onmounted`.
    pub fn line_mounted(&self, index: usize, el: Rc<MountedData>) {
        let heights = self.heights;
        let mut handles = self.handles;
        handles.write().insert(index, el.clone());
        spawn(async move {
            measure(heights, index, &el).await;
        });
    }

    /// The lines to render for a document of `lines` lines. Subscribes the
    /// caller to the viewport and the heights, so it re-renders when either
    /// moves. The caret's line is brought into view by scrolling (see the
    /// caret effect), not by building the lines between it and the viewport.
    #[must_use]
    pub fn window(&self, lines: usize) -> Window {
        let heights_now = self.heights.read();
        let vp = *self.viewport.read();
        let heights: Vec<f64> = (0..lines).map(|i| height_of(&heights_now, i)).collect();
        let w = window(&heights, vp.top, vp.height, OVERSCAN_PX);
        self.built.set(w.lines());
        w
    }
}

/// A tick every `period`, for any executor.
///
/// The poll can't use `tokio::time`: a Blitz host such as architect's lab
/// runs the vdom on its own executor with no Tokio reactor, and
/// `tokio::time::sleep` panics there ("no reactor running"). One thread per
/// editor sleeps and sends; it ends when the editor (the receiver) goes,
/// because the next send fails. The channel holds one tick, so a busy UI
/// thread never queues a backlog of polls.
fn ticker(period: Duration) -> futures_channel::mpsc::Receiver<()> {
    let (mut tx, rx) = futures_channel::mpsc::channel(0);
    std::thread::Builder::new()
        .name("editor-viewport-poll".into())
        .spawn(move || {
            loop {
                std::thread::sleep(period);
                match tx.try_send(()) {
                    Ok(()) => {}
                    Err(e) if e.is_full() => {}
                    Err(_) => break,
                }
            }
        })
        .expect("spawn the editor's viewport poll thread");
    rx
}
