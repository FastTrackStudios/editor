//! Dioxus render pass for the tile tree.
//!
//! The CM6 equivalent is `DocView.sync` (`view/src/docview.ts:80+`)
//! which imperatively patches the DOM. We don't patch — Dioxus does
//! that for us. Our job is to emit the right rsx for the current
//! tile tree and let the reconciler diff against the live DOM.
//!
//! This path drives the **native (Blitz) renderer**. The
//! web/desktop build fills a contenteditable via the JS patcher
//! instead (`tile::patch` + `editor::patcher_script`).
//!
//! Each emitted element carries `data-tile-id` / `data-tile-pos`
//! (the doc byte offset where the tile starts), and an
//! `onmousedown` that reports that offset to `on_click` — Blitz
//! has no contenteditable to place the caret, so click-to-
//! position is wired here at render time: clicking an element
//! drops the caret at the element's start. (Character-precise
//! within-element placement needs layout geometry — Blitz's
//! `find_text_position` or a non-stubbed `element_coordinates` —
//! and is a follow-up; this lands the caret at the clicked
//! span's start, which is word/mark-level precise.)

use dioxus::prelude::*;

use crate::tile::arena::{Arena, TileId};
use crate::tile::mark::mark_spec_of;
use crate::tile::text::text_of;
use crate::tile::{Tile, TileBody, TileKind};

/// Render a tile and its descendants. The root call passes the
/// `DocTile`; recursion walks into composites. `on_click` is
/// invoked with a doc byte offset when an element is pressed.
///
/// # Errors
/// Returns an error if the underlying Dioxus rsx! construction fails
/// to build the element tree.
pub fn render_tile(arena: &Arena, tile: TileId, on_click: Callback<usize>) -> Element {
    let t = arena.get(tile);
    let tid = tile.0;
    match t.kind {
        TileKind::Doc => rsx! {
            // The doc tile itself is rendered by the editor
            // component's outer <div>. We just emit children here.
            for &child in &t.children {
                {render_tile(arena, child, on_click)}
            }
        },
        TileKind::Text => {
            // Wrap in a span carrying `data-tile-pos` so geometry
            // and click-to-position can map back to a doc offset.
            let text = text_of(t);
            let pos = crate::tile::pos::pos_at_start(arena, tile);
            rsx! {
                span {
                    "data-tile-id": "{tid}",
                    "data-tile-pos": "{pos}",
                    onmousedown: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        on_click.call(pos);
                    },
                    "{text}"
                }
            }
        }
        TileKind::Mark => {
            // Reached only from the `TileKind::Mark` arm, so the spec is
            // present. A Mark tile carrying a non-Mark body is a build bug;
            // the default spec degrades it to an unstyled wrapper.
            let spec = mark_spec_of(t).cloned().unwrap_or_default();
            let class = spec.class;
            let pos = crate::tile::pos::pos_at_start(arena, tile);
            rsx! {
                span {
                    class: "{class}",
                    "data-tile-id": "{tid}",
                    "data-tile-pos": "{pos}",
                    onmousedown: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        on_click.call(pos);
                    },
                    for &child in &t.children {
                        {render_tile(arena, child, on_click)}
                    }
                }
            }
        }
        TileKind::WidgetBuffer => {
            let pos = crate::tile::pos::pos_at_start(arena, tile);
            rsx! {
                img {
                    class: "cm-widgetBuffer",
                    "aria-hidden": "true",
                    "data-tile-id": "{tid}",
                    "data-tile-pos": "{pos}",
                    "data-tile-len": "0",
                }
            }
        }
        TileKind::Widget => render_widget_el(arena, tile, t, tid, on_click),
        TileKind::Line => render_line_el(arena, tile, t, tid, on_click),
        TileKind::BlockWrapper => {
            let pos = crate::tile::pos::pos_at_start(arena, tile);
            rsx! {
                div {
                    "data-tile-id": "{tid}",
                    "data-tile-pos": "{pos}",
                    for &child in &t.children {
                        {render_tile(arena, child, on_click)}
                    }
                }
            }
        }
    }
}

/// The `<div class="cm-line">` element for one line tile.
///
/// Split out of [`render_tile`]'s `match` so that dispatcher stays under a
/// screenful; the body is unchanged.
fn render_line_el(
    arena: &Arena,
    tile: TileId,
    t: &Tile,
    tid: u32,
    on_click: Callback<usize>,
) -> Element {
    let pos = crate::tile::pos::pos_at_start(arena, tile);
    // Line-level decoration classes (md-h1, md-blockquote,
    // md-callout, md-list-item, …) come from `Decoration::line`
    // and live in the tile body; join them onto the base
    // `cm-line` so heading sizes, blockquote bars, callouts, and
    // list styling actually apply on the native renderer (the web
    // patcher does the same).
    let class = match &t.body {
        crate::tile::TileBody::Line { extra_classes } if !extra_classes.is_empty() => {
            format!("cm-line {}", extra_classes.join(" "))
        }
        _ => "cm-line".to_string(),
    };
    // Empty line: an empty div. `.cm-line { min-height: 1lh }`
    // gives it a full clickable row without any content — Blitz
    // honors `1lh`. (The web path emits a `<br>` here so the
    // browser caret can land in the empty contenteditable line;
    // Blitz needs no such placeholder, and a `<br>` there renders
    // several line-heights tall.) A click anywhere on the row
    // lands the caret at the line start.
    if t.children.is_empty() {
        rsx! {
            div {
                class: "{class}",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                onmousedown: move |_e: Event<MouseData>| on_click.call(pos),
            }
        }
    } else {
        rsx! {
            div {
                class: "{class}",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                onmousedown: move |_e: Event<MouseData>| on_click.call(pos),
                for &child in &t.children {
                    {render_tile(arena, child, on_click)}
                }
            }
        }
    }
    
}

/// The `<span class="editor-widget">` element for one widget tile.
///
/// Split out of [`render_tile`]'s `match`; the body is unchanged.
fn render_widget_el(
    arena: &Arena,
    tile: TileId,
    t: &Tile,
    tid: u32,
    on_click: Callback<usize>,
) -> Element {
    let html = match &t.body {
        TileBody::Widget { html } => html.clone(),
        _ => String::new(),
    };
    if html.is_empty() {
        rsx! {}
    } else {
        let pos = crate::tile::pos::pos_at_start(arena, tile);
        rsx! {
            span {
                class: "editor-widget",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                onmousedown: move |e: Event<MouseData>| {
                    e.stop_propagation();
                    on_click.call(pos);
                },
                dangerous_inner_html: "{html}"
            }
        }
    }
    
}
