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
//! drops the caret at the element's start, and clicking a line past
//! its text drops it at the line's end. (Character-precise
//! within-element placement needs layout geometry — Blitz's
//! `find_text_position` or a non-stubbed `element_coordinates` —
//! and is a follow-up; this lands the caret at the clicked
//! span's start, which is word/mark-level precise.)

use std::rc::Rc;

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
            // A link (`data-href`: a link's URL, a wikilink's body): a
            // press anywhere in it follows it, as the web renderer's
            // `link-clicked` does; the arrow keys walk into it to edit.
            let href = spec
                .attrs
                .iter()
                .find(|(k, _)| k == "data-href")
                .map(|(_, v)| v.clone());
            let follow = href.and_then(|href| {
                let LinkClick(go) = try_consume_context::<Option<LinkClick>>().flatten()?;
                Some(Callback::new(move |_: usize| go.call(href.clone())))
            });
            let press = follow.unwrap_or(on_click);
            rsx! {
                span {
                    class: "{class}",
                    "data-tile-id": "{tid}",
                    "data-tile-pos": "{pos}",
                    onmousedown: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        press.call(pos);
                    },
                    for &child in &t.children {
                        {render_tile(arena, child, press)}
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
        TileKind::Line => render_line_el(arena, tile, t, tid, on_click, None),
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
    mount: Option<LineMount>,
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
    // several line-heights tall.)
    //
    // The line's own press is one no text took (each span stops its
    // own): past the end of the text, or anywhere on an empty row. It
    // lands the caret at the line's end, as a text editor does.
    let end = crate::tile::pos::pos_at_end(arena, tile);
    let on_mounted = move |e: Event<MountedData>| {
        if let Some((index, report)) = mount {
            report.call((index, e.data()));
        }
    };
    if t.children.is_empty() {
        rsx! {
            div {
                class: "{class}",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                onmousedown: move |_e: Event<MouseData>| on_click.call(end),
                onmounted: on_mounted,
            }
        }
    } else {
        rsx! {
            div {
                class: "{class}",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                onmousedown: move |_e: Event<MouseData>| on_click.call(end),
                onmounted: on_mounted,
                for &child in &t.children {
                    {render_tile(arena, child, on_click)}
                }
            }
        }
    }
}

/// Reports a rendered line's handle to the view: the line's index in the
/// document, and a callback that receives its mounted element (for measuring).
pub type LineMount = (usize, Callback<(usize, std::rc::Rc<MountedData>)>);

/// Render only the lines of `win`.
///
/// Spacers of the lines' total height sit
/// above and below, so the scroll container keeps the whole document's
/// height. Each rendered line reports its mounted element through
/// `on_mounted`, so the view can measure it.
///
/// `win` comes from [`crate::tile::window::window`] over the same document
/// `root` was built from.
///
/// # Errors
///
/// None of its own: an `Element` is Dioxus' render result, an error only
/// when a component under it fails to render.
pub fn render_window(
    arena: &Arena,
    root: TileId,
    win: &crate::tile::window::Window,
    on_click: Callback<usize>,
    on_mounted: Callback<(usize, std::rc::Rc<MountedData>)>,
) -> Element {
    let children = arena.get(root).children.clone();
    let range = win.lines();
    let lines = range.filter_map(|i| children.get(i).map(|&child| (i, child)));
    let top = win.top_pad;
    let bottom = win.bottom_pad;
    // Both spacers are always present, even at zero height: adding and removing
    // them as the window moves churns the tree around the lines, which the
    // layout must follow on every scroll step.
    rsx! {
        div { class: "ed-spacer", style: "height: {top}px;", "aria-hidden": "true" }
        for (index, child) in lines {
            Fragment {
                key: "{index}",
                {
                    let t = arena.get(child);
                    render_line_el(arena, child, t, child.0, on_click, Some((index, on_mounted)))
                }
            }
        }
        div { class: "ed-spacer", style: "height: {bottom}px;", "aria-hidden": "true" }
    }
}

/// What a host mounts into the editor's mounted widgets.
///
/// A mounted widget (`editor_state::DecoratedRange::mount`) is drawn as
/// the element for its id (an extension's view,
/// `editor_state::ext::mount_id`). Give it to the
/// `Editor`'s `mounts` once, capturing signals rather than values; it is
/// compared by identity.
#[derive(Clone)]
pub struct WidgetMounts(Rc<dyn Fn(&str) -> Element>);

impl WidgetMounts {
    /// Mounts what `f` gives for each id.
    pub fn new(f: impl Fn(&str) -> Element + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// The element for `id`.
    ///
    /// # Errors
    ///
    /// The host's element failed to render.
    #[must_use]
    pub fn mount(&self, id: &str) -> Element {
        (self.0)(id)
    }
}

impl PartialEq for WidgetMounts {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for WidgetMounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WidgetMounts(..)")
    }
}

/// What pressing a link does: the host's `on_link_click`, given the link's
/// `data-href` (the editor provides it when the host set one).
#[derive(Clone, Copy)]
pub struct LinkClick(pub Callback<String>);

/// What pressing a task's checkbox does: toggle the task whose line starts
/// at the offset (the editor provides it). The web renderer's checkbox
/// sends `task-toggle` through its JS bridge instead.
#[derive(Clone, Copy)]
pub struct TaskToggle(pub Callback<usize>);

/// A task checkbox widget's parts (`editor_state::markdown`'s
/// `<span class="md-task-checkbox…" data-task-pos=… data-status="x">✓</span>`).
struct TaskBox {
    class: String,
    status: String,
    glyph: String,
}

fn task_box_of(html: &str) -> Option<TaskBox> {
    let rest = html.strip_prefix("<span class=\"")?;
    let (class, rest) = rest.split_once('"')?;
    if !class.split(' ').any(|c| c == "md-task-checkbox") {
        return None;
    }
    let status = rest
        .split_once("data-status=\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map_or("", |(s, _)| s);
    let (_, inner) = rest.split_once('>')?;
    let glyph = inner.strip_suffix("</span>")?;
    // The glyph is plain text; anything else is not this shape.
    if glyph.contains('<') {
        return None;
    }
    Some(TaskBox {
        class: class.to_owned(),
        status: status.replace("&quot;", "\""),
        glyph: glyph.to_owned(),
    })
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
    } else if let Some((id, block)) = editor_state::decoration::mount_of(&html)
        && let Some(mounts) = try_consume_context::<Option<WidgetMounts>>().flatten()
    {
        // A mounted widget: the host's view, not HTML. Its events are
        // its own (a press does not move the caret).
        let pos = crate::tile::pos::pos_at_start(arena, tile);
        let id = id.to_owned();
        let class = if block {
            "editor-widget editor-mount editor-mount-block"
        } else {
            "editor-widget editor-mount"
        };
        rsx! {
            span {
                class,
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                "data-editor-mount": "{id}",
                contenteditable: "false",
                onmousedown: move |e: Event<MouseData>| e.stop_propagation(),
                {mounts.mount(&id)}
            }
        }
    } else if let Some((class, text)) = editor_state::decoration::gutter_of(&html) {
        // A gutter marker, drawn here: Blitz may have no HTML parser.
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
                span { class: "{class}", "aria-hidden": "true", "{text}" }
            }
        }
    } else if let Some(task) = task_box_of(&html)
        && let Some(TaskToggle(toggle)) = try_consume_context::<TaskToggle>()
    {
        // A task's checkbox, drawn here rather than as HTML, its press
        // toggling the task (the web renderer's goes through the bridge).
        let pos = crate::tile::pos::pos_at_start(arena, tile);
        rsx! {
            span {
                class: "editor-widget",
                "data-tile-id": "{tid}",
                "data-tile-pos": "{pos}",
                span {
                    class: "{task.class}",
                    style: "position: relative;",
                    "data-task-pos": "{pos}",
                    "data-status": "{task.status}",
                    onmousedown: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        toggle.call(pos);
                    },
                    // Done: a tick drawn, not the `✓` character. Blitz's
                    // inline hit test gives a press on anything in the
                    // box's inline flow to the line (the caret went to its
                    // end): what the box holds is placed out of it.
                    if task.status.eq_ignore_ascii_case("x") {
                        svg {
                            style: "position: absolute; left: 15%; top: 15%; width: 70%; height: 70%;",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "3.5",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            path { d: "M20 6 9 17l-5-5" }
                        }
                    } else if !task.glyph.is_empty() {
                        // Out of the inline flow, like the tick, so a press
                        // on it is the box's.
                        span {
                            style: "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;",
                            "{task.glyph}"
                        }
                    }
                }
            }
        }
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
