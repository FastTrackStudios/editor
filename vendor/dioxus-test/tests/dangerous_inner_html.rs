//! Does dangerous_inner_html reach the blitz DOM? The editor's widget
//! decorations (block-ref chips, math, painted caret) depend on it.
use dioxus::prelude::*;
use dioxus_test::{
    matchers::{contains_substring, inner_html},
    render,
};

#[component]
fn Danger() -> Element {
    rsx! {
        div { class: "host", dangerous_inner_html: "<span class=\"inner\">payload</span>" }
    }
}

#[test]
fn dangerous_inner_html_renders() {
    let tester = render(Danger).build();
    tester
        .query(".host")
        .expect(inner_html(contains_substring("payload")))
        .immediately()
        .unwrap();
}
