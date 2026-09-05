//! The fence plugin system.
//!
//! A fenced code block whose language has a plugin renders as the thing
//! it describes — a chart, a diagram, a formula — instead of as source.
//! Everything about that is the same for every language: look the plugin
//! up, spend from a per-pass compile budget, cache the result, wrap it in
//! a widget. Only the renderer itself differs.
//!
//! That was not how it worked. Three languages had three hand-written
//! modules in `markdown/` — `keyflow.rs`, `mermaid.rs`, `typst.rs` —
//! each with its own copy of the cache and the budget, and the fence
//! dispatch branched on `is_mermaid` with the widget class picked by an
//! `if`. Only keyflow went through a registry at all; the other two were
//! hard dependencies of this crate. Adding a fourth language meant
//! writing a fourth module and another branch.
//!
//! Now there is one registry and one trait. A language is a plugin, a
//! plugin is registered, and the dispatch does not know which languages
//! exist.
//!
//! ## Why the budget lives here
//!
//! Rendering happens on the decoration pass, which runs on every
//! keystroke. A note with a dozen fresh diagrams in it would compile a
//! dozen diagrams per character typed. Each plugin declares how many cold
//! compiles one pass may spend; past that, fences fall back to source
//! until the next pass, and the cache carries the ones already rendered.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A renderer for one fenced-code language.
pub trait FencePlugin: Send + Sync {
    /// Render the fence body, or `None` to fall back to showing source.
    ///
    /// `None` is not an error: a half-typed diagram is the normal case,
    /// and a fence that cannot render yet should look like what it is —
    /// the text you are still writing.
    fn render(&self, source: &str) -> Option<String>;

    /// The fence body, syntax-highlighted, for when it shows as source.
    fn highlight(&self, source: &str) -> String {
        crate::markdown::escape_html(source)
    }

    /// CSS class for the rendered widget — `md-mermaid-widget` and so on.
    fn widget_class(&self) -> &'static str;

    /// Cold compiles this plugin may spend on one decoration pass.
    ///
    /// One is right for anything expensive enough to notice. Typst uses
    /// two because inline math and block math both draw from it and a
    /// paragraph commonly gains both at once.
    fn budget_per_pass(&self) -> u8 {
        1
    }
}

fn registry() -> &'static RwLock<HashMap<String, Arc<dyn FencePlugin>>> {
    static REGISTRY: std::sync::OnceLock<RwLock<HashMap<String, Arc<dyn FencePlugin>>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register `plugin` for `language`. Registering twice replaces.
///
/// Language matching is case-insensitive, so ```` ```Mermaid ```` and
/// ```` ```mermaid ```` are the same fence.
pub fn register(language: &str, plugin: Arc<dyn FencePlugin>) {
    if let Ok(mut reg) = registry().write() {
        reg.insert(language.to_lowercase(), plugin);
    }
}

/// The plugin for `language`, if one is registered.
#[must_use]
pub fn get(language: &str) -> Option<Arc<dyn FencePlugin>> {
    registry()
        .read()
        .ok()?
        .get(&language.to_lowercase())
        .map(Arc::clone)
}

/// Every registered language, sorted. For diagnostics and tests.
#[must_use]
pub fn languages() -> Vec<String> {
    registry().read().map_or_else(
        |_| Vec::new(),
        |reg| {
            let mut v: Vec<String> = reg.keys().cloned().collect();
            v.sort();
            v
        },
    )
}

/// Register the languages this crate renders itself.
///
/// Called once, from the decoration pass. Mermaid and Typst live behind
/// the same trait as anything a host registers — the only thing that
/// makes them "built in" is that `editor-state` happens to depend on
/// their renderers, and the dispatch cannot tell the difference.
pub(crate) fn ensure_builtins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        register("mermaid", Arc::new(crate::markdown::mermaid::MermaidPlugin));
        register("typst", Arc::new(crate::markdown::typst::TypstPlugin));
    });
}

/// Adapts a [`FenceRenderer`](crate::fence_renderer::FenceRenderer) to
/// the plugin trait.
///
/// `fence_renderer` is the older, narrower seam and hosts outside this
/// repo register through it — keyflow's chart renderer is one. Rather
/// than break them, every such registration is wrapped and lands in this
/// registry too, so there is one place to look a language up even while
/// two ways to register survive.
struct RendererAdapter {
    inner: Arc<dyn crate::fence_renderer::FenceRenderer>,
    class: &'static str,
}

impl FencePlugin for RendererAdapter {
    fn render(&self, source: &str) -> Option<String> {
        self.inner.render_svg(source)
    }
    fn highlight(&self, source: &str) -> String {
        self.inner.highlight_html(source)
    }
    fn widget_class(&self) -> &'static str {
        self.class
    }
}

/// Mirror a `fence_renderer` registration into the plugin registry.
pub(crate) fn adopt_renderer(
    language: &str,
    renderer: Arc<dyn crate::fence_renderer::FenceRenderer>,
) {
    register(
        language,
        Arc::new(RendererAdapter {
            inner: renderer,
            // Languages registered through the old seam draw their own
            // widget markup at the dispatch; this class is only the
            // fallback wrapper for one that does not.
            class: "md-fence-widget",
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub;
    impl FencePlugin for Stub {
        fn render(&self, source: &str) -> Option<String> {
            (!source.is_empty()).then(|| format!("<svg>{source}</svg>"))
        }
        fn widget_class(&self) -> &'static str {
            "md-stub-widget"
        }
    }

    #[test]
    fn an_unregistered_language_is_none_not_a_panic() {
        assert!(get("definitely-not-registered").is_none());
    }

    #[test]
    fn registers_and_resolves_case_insensitively() {
        register("StubLang", Arc::new(Stub));
        assert!(get("stublang").is_some());
        assert!(get("STUBLANG").is_some());
    }

    #[test]
    fn declining_to_render_is_not_an_error() {
        register("declining", Arc::new(Stub));
        let p = get("declining").expect("just registered");
        assert!(p.render("").is_none());
        assert!(p.render("x").is_some());
    }

    #[test]
    fn the_default_budget_is_one_cold_compile() {
        assert_eq!(Stub.budget_per_pass(), 1);
    }

    #[test]
    fn highlight_falls_back_to_escaped_source() {
        // A plugin that renders but has nothing to say about syntax must
        // not emit raw markup into the page.
        assert_eq!(Stub.highlight("a < b"), "a &lt; b");
    }
}
