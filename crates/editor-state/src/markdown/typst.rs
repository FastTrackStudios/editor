//! Typst SVG rendering for the live-preview decorations.
//! Three kinds of fragment:
//!
//! - inline math `$x$` → small bare SVG
//! - block math `$$x$$` → display-mode SVG
//! - ```` ```typst ``` ```` fence body → full Typst doc SVG
//!
//! Compiles synchronously via `editor-typst::Compiler` but
//! defends against worst-case latency two ways:
//!
//! 1. **Per-pass compile budget.** `live_preview` calls
//!    [`reset_compile_budget`] at the start of each render
//!    pass. Cache misses count against the budget — once it's
//!    exhausted, further misses return `None` and the caller
//!    falls back to showing the source. The next live-preview
//!    pass picks up where this one left off (the cache is
//!    persistent across passes), so a doc with N fresh
//!    equations converges over ~⌈N/budget⌉ render cycles
//!    instead of blocking once for ~N×50ms.
//!
//! 2. **Thread-local LRU cache** keyed by `(kind, body)`. Cap
//!    is generous (128) so popular fragments stay hot through
//!    a long editing session — typing into a paragraph nearby
//!    doesn't evict the equation above.
//!
//! The SVG is post-processed to swap our sentinel fill color
//! (`#ff00fe`) for `currentColor`, so the rendered glyphs
//! inherit the editor pane's CSS `color:` and respond to theme
//! switches without a recompile.

use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypstKind {
    MathInline,
    MathBlock,
    /// ```` ```typst …``` ```` fence body, compiled as a full
    /// Typst document.
    Block,
}

const SENTINEL: &str = "#ff00fe";
const CACHE_CAP: usize = 128;
/// How many cold compiles we allow per `live_preview` pass.
/// Picked to keep the worst-case at ~2×50ms = ~100ms; tune if
/// profiling shows otherwise.
const COMPILE_BUDGET_PER_PASS: u8 = 2;

thread_local! {
    static COMPILE_BUDGET: Cell<u8> = const { Cell::new(COMPILE_BUDGET_PER_PASS) };
}

/// Re-arm the per-pass compile budget. Call once at the top of
/// every `live_preview` pass before any [`render_typst`] calls.
pub fn reset_compile_budget() {
    // A one-shot render has no next pass to converge on, so it gets an
    // unlimited budget — see [`crate::markdown::render_everything`].
    let limit = if crate::markdown::rendering_everything() {
        u8::MAX
    } else {
        COMPILE_BUDGET_PER_PASS
    };
    COMPILE_BUDGET.with(|c| c.set(limit));
}

/// Render a Typst source fragment to inline SVG. Returns `None`
/// when (a) compile fails, or (b) the per-pass budget is
/// exhausted and the body isn't cached. The caller shows the
/// raw source in either case — the user can keep editing.
/// Typst points per em of body text.
///
/// Calibrated, not derived. The arithmetic answer is 14pt × 4/3 = 18.67,
/// but typst sets math in New Computer Modern, whose x-height is small
/// enough that matching ems makes the maths look a size too small beside
/// the body face. 10pt/em reproduces the height the old fixed
/// `height: 0.95em` gave `E = mc^2`, which read correctly — the point of
/// the change is that a *taller* equation is now taller, not that simple
/// ones move.
const PT_PER_EM: f64 = 10.07;

/// Give the SVG an explicit `em` width and height taken from its own
/// viewBox, so it scales with the surrounding text instead of being
/// clamped to a fixed height.
///
/// A fixed `height` in CSS cannot work: it forces every equation to one
/// height whatever its structure, so a sum with limits and a fraction —
/// three times as tall as `E = mc^2` — had its glyphs shrunk to a third
/// of body size to fit.
fn size_to_body_text(svg: &str) -> String {
    let Some((w, h)) = view_box_size(svg) else {
        return svg.to_owned();
    };
    let (w, h) = (w / PT_PER_EM, h / PT_PER_EM);
    // Replace typst's own `pt` width/height, which are the same numbers
    // in a unit that does not track the reader's font size.
    let mut out = svg.to_owned();
    if let Some(start) = out.find("<svg") {
        let insert = start.saturating_add(4);
        out.insert_str(
            insert,
            &format!(r#" style="width:{w:.4}em;height:{h:.4}em""#),
        );
    }
    out
}

/// The width and height from an SVG's `viewBox`, in typst points.
fn view_box_size(svg: &str) -> Option<(f64, f64)> {
    let vb = svg.split_once("viewBox=\"")?.1.split_once('"')?.0;
    let mut parts = vb.split_whitespace().skip(2);
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

pub fn render_typst(kind: TypstKind, body: &str) -> Option<String> {
    if let Some(cached) = with_typst_cache(|c| c.get(kind, body)) {
        return Some(cached);
    }
    let budget = COMPILE_BUDGET.with(std::cell::Cell::get);
    if budget == 0 {
        return None;
    }
    COMPILE_BUDGET.with(|c| c.set(budget.saturating_sub(1)));

    // Wrap the fragment in a Typst preamble so each compiles
    // as a standalone document. `page(fill: none)` keeps the
    // SVG background transparent; the sentinel fill color is
    // replaced with `currentColor` after compile.
    let prelude = format!(
        "#set page(width: auto, height: auto, margin: 0pt, fill: none)\n\
         #set text(size: 14pt, fill: rgb(\"{SENTINEL}\"))\n"
    );
    let wrapped = match kind {
        // Both math kinds compile the same way — `$ x $`, with the spaces.
        // Not cosmetic: typst reports the *line* box for `$x$` set in a
        // paragraph (9.59pt for a sum whose ink is really 35.76pt), and
        // the browser then scales the whole equation down to fit that
        // box, so anything with limits or a fraction rendered at about
        // half the size of the surrounding text. The spaced form reports
        // the ink. Inline and block differ in CSS, not in the compile.
        TypstKind::MathInline | TypstKind::MathBlock => format!("{prelude}$ {body} $"),
        TypstKind::Block => format!("{prelude}{body}"),
    };
    let mut compiler = editor_typst::Compiler::new();
    compiler.set_source(wrapped);
    match compiler.compile_svg() {
        Ok(svg) => {
            // Typst emits hex literals lowercase but be
            // defensive about an uppercase variant if it ever
            // changes.
            // Trimmed: see the note in the keyflow renderer — a
            // newline around the SVG becomes a blank row under
            // `white-space: pre-wrap`.
            let themed = svg
                .replace("#ff00fe", "currentColor")
                .replace("#FF00FE", "currentColor")
                .trim()
                .to_owned();
            // Math only. A `typst` fence is a whole document laid out at
            // its own scale; sizing it to the body text shrank the block
            // to a caption.
            let themed = if matches!(kind, TypstKind::MathInline | TypstKind::MathBlock) {
                size_to_body_text(&themed)
            } else {
                themed
            };
            with_typst_cache(|c| c.put(kind, body.to_string(), themed.clone()));
            Some(themed)
        }
        Err(e) => {
            tracing::debug!(?e, body_len = body.len(), "typst compile failed");
            None
        }
    }
}

struct TypstCache {
    entries: Vec<(TypstKind, String, String)>,
    cap: usize,
}

impl TypstCache {
    fn new(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
            cap,
        }
    }
    fn get(&mut self, kind: TypstKind, body: &str) -> Option<String> {
        let i = self
            .entries
            .iter()
            .position(|(k, b, _)| *k == kind && b == body)?;
        let hit = self.entries.remove(i);
        let svg = hit.2.clone();
        self.entries.push(hit);
        Some(svg)
    }
    fn put(&mut self, kind: TypstKind, body: String, svg: String) {
        if self.entries.len() >= self.cap {
            self.entries.remove(0);
        }
        self.entries.push((kind, body, svg));
    }
}

fn with_typst_cache<R>(f: impl FnOnce(&mut TypstCache) -> R) -> R {
    thread_local! {
        static CACHE: std::cell::RefCell<TypstCache> =
            std::cell::RefCell::new(TypstCache::new(CACHE_CAP));
    }
    CACHE.with(|c| f(&mut c.borrow_mut()))
}

/// Typst as a fence plugin, for ```` ```typst ```` blocks.
///
/// Inline and display math reach the compiler by a different path —
/// they are spans inside a paragraph, not fences — so this covers the
/// block form only. The budget is shared with them, which is why it is
/// two rather than one.
pub struct TypstPlugin;

impl crate::plugin::FencePlugin for TypstPlugin {
    fn render(&self, source: &str) -> Option<String> {
        render_typst(TypstKind::Block, source)
    }

    fn widget_class(&self) -> &'static str {
        "md-typst-widget"
    }

    fn budget_per_pass(&self) -> u8 {
        COMPILE_BUDGET_PER_PASS
    }
}
