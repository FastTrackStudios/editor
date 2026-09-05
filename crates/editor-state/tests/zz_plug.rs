use editor_state::plugin::{self, FencePlugin};
use std::sync::Arc;

struct Fake;
impl FencePlugin for Fake {
    fn render(&self, s: &str) -> Option<String> {
        Some(format!("<svg id=\"fake\">{s}</svg>"))
    }
    fn widget_class(&self) -> &'static str {
        "md-fake-widget"
    }
}

#[test]
fn a_registered_language_renders_through_the_dispatch() {
    plugin::register("fakelang", Arc::new(Fake));
    let html = editor_state::html::render_markdown_html("```fakelang\nhello\n```\n");
    assert!(html.contains("md-fake-widget"), "{html}");
}

#[test]
fn builtin_languages_are_registered_by_the_pass() {
    let _ = editor_state::html::render_markdown_html("hi");
    let langs = plugin::languages();
    assert!(langs.iter().any(|l| l == "mermaid"), "{langs:?}");
    assert!(langs.iter().any(|l| l == "typst"), "{langs:?}");
}

#[test]
fn an_unregistered_fence_stays_a_code_block() {
    let html = editor_state::html::render_markdown_html("```nosuchlang\nbody\n```\n");
    assert!(!html.contains("widget"), "{html}");
}
