//! `CodeEditor` on the native renderer: a whole buffer highlighted in one
//! language (built in, or installed by the host), line numbers, the
//! bracket pair at the caret, Enter that keeps the indent, Tab and
//! Shift-Tab, diagnostics the host hands in (and takes back), and the
//! extension surface passing through.
#![cfg(feature = "native")]

mod common;
use common::*;
use dioxus::prelude::*;
use dioxus_test::{by_testid, render};
use editor_state::ext::{self, DecorationSpec};
use editor_state::languages::{self, HostLanguage, LanguageEntry, Syntax};
use editor_state::{Diagnostic, EditorState, Selection, Severity};
use editor_view::{CodeEditor, DecorationSource};

/// What a test mounts: the text, the caret, and the language.
#[derive(Clone)]
struct CodeSetup {
    text: &'static str,
    caret: usize,
    /// A file name; its language comes from `language_for_path`.
    path: &'static str,
}

#[component]
fn CodeHarness() -> Element {
    let setup = use_context::<CodeSetup>();
    let state = use_signal(|| {
        let mut s = EditorState::new(setup.text);
        s.selection = Selection::caret(setup.caret);
        s
    });
    let mut diagnostics = use_signal(Vec::<Diagnostic>::new);
    let language = languages::language_for_path(setup.path);
    // An extension's decoration, through the host's decorations prop.
    let extension = use_hook(|| {
        DecorationSource::new(|s: &EditorState| {
            let specs = vec![DecorationSpec::Mark {
                from: 7,
                to: 8,
                class: "hot".into(),
                title: None,
            }];
            ext::decorations(&specs, &s.doc.to_string())
        })
    });
    let s = state.read();
    let head = s.selection.primary().head;
    let doc = s.doc.to_string();
    drop(s);
    rsx! {
        style { dangerous_inner_html: EDITOR_CSS }
        CodeEditor {
            state,
            language,
            diagnostics: diagnostics(),
            decorations: Some(extension),
        }
        div { "data-testid": "head", "{head}" }
        div { "data-testid": "doc", "{doc}" }
        div {
            "data-testid": "diagnose",
            onclick: move |_| diagnostics.set(vec![Diagnostic::new(3..7, Severity::Error, "no main")]),
            "diagnose"
        }
        div {
            "data-testid": "clear",
            onclick: move |_| diagnostics.set(Vec::new()),
            "clear"
        }
    }
}

fn mount_code(setup: CodeSetup) -> dioxus_test::DocumentTester {
    let t = render(CodeHarness).with_root_context(setup).build();
    t.query(".editor-root")
        .immediately()
        .expect("editor root should render")
        .focus();
    t
}

fn rust_main() -> CodeSetup {
    CodeSetup {
        text: "fn main() {}",
        caret: 11,
        path: "src/main.rs",
    }
}

#[tokio::test]
async fn the_buffer_is_highlighted_numbered_and_its_brackets_matched() {
    let t = mount_code(rust_main());
    t.query(".editor-code[data-language=\"rust\"]")
        .await
        .unwrap_or_else(|e| panic!("wrapper: {e:?}"));
    t.query(".md-tok-k")
        .expect(inner_html(contains_substring("fn".to_string())))
        .await
        .unwrap_or_else(|e| panic!("`fn` is a keyword: {e:?}"));
    t.query(".editor-gutter.code-ln")
        .expect(inner_html(eq("1".to_string())))
        .await
        .unwrap_or_else(|e| panic!("line number: {e:?}"));
    t.query(".md-bracket-match")
        .await
        .unwrap_or_else(|e| panic!("bracket pair: {e:?}"));
    t.query(".ext-mark.ext-hot")
        .await
        .unwrap_or_else(|e| panic!("the extension's mark: {e:?}"));
}

#[tokio::test]
async fn enter_between_braces_opens_an_indented_line() {
    let t = mount_code(rust_main());
    press(&t, &["Enter"]);
    expect_probe(&t, "doc", "fn main() {\n  \n}").await;
    expect_probe(&t, "head", "14").await;
    assert_eq!(
        t.query_all(".editor-gutter.code-ln").immediately().len(),
        3,
        "a number per line"
    );
    // Typing on the new line goes where the caret is.
    press(&t, &["x"]);
    expect_probe(&t, "doc", "fn main() {\n  x\n}").await;
}

#[tokio::test]
async fn tab_indents_and_shift_tab_outdents() {
    let t = mount_code(CodeSetup {
        text: "x",
        caret: 0,
        path: "a.rs",
    });
    press(&t, &["Tab"]);
    expect_probe(&t, "doc", "  x").await;
    expect_probe(&t, "head", "2").await;
    t.press_key(parse_key("Tab"), Modifiers::SHIFT);
    expect_probe(&t, "doc", "x").await;
    // Undo, from the code keymap, through the shared history.
    t.press_key(parse_key("z"), Modifiers::CONTROL);
    expect_probe(&t, "doc", "  x").await;
}

#[tokio::test]
async fn diagnostics_the_host_hands_in_are_drawn_and_withdrawn() {
    let t = mount_code(rust_main());
    assert!(
        t.query_all(".cm-lsp-diagnostic").immediately().is_empty(),
        "none yet"
    );
    t.query(by_testid("diagnose"))
        .click()
        .await
        .unwrap_or_else(|e| panic!("click: {e:?}"));
    t.query(".cm-lsp-diagnostic.cm-lsp-error")
        .expect(inner_html(contains_substring("main".to_string())))
        .await
        .unwrap_or_else(|e| panic!("underline: {e:?}"));
    t.query(".editor-gutter.code-ln.code-ln-error")
        .await
        .unwrap_or_else(|e| panic!("gutter mark: {e:?}"));
    t.query(by_testid("clear"))
        .click()
        .await
        .unwrap_or_else(|e| panic!("click: {e:?}"));
    t.query(".editor-gutter.code-ln")
        .expect(dioxus_test::matchers::attribute(
            "class",
            eq(Some("editor-gutter code-ln".to_string())),
        ))
        .await
        .unwrap_or_else(|e| panic!("gutter unmarked: {e:?}"));
    assert!(t.query_all(".cm-lsp-diagnostic").immediately().is_empty());
}

#[tokio::test]
async fn a_language_the_host_installed_highlights_its_files() {
    let toml = HostLanguage::new(
        "toml",
        arborium_toml::language(),
        arborium_toml::HIGHLIGHTS_QUERY,
    )
    .expect("toml compiles");
    languages::register_language(LanguageEntry::new("toml", toml).extensions(["toml"]));
    assert!(matches!(
        languages::language_for_path("Cargo.toml"),
        Some(Syntax::Host(_))
    ));
    let t = mount_code(CodeSetup {
        text: "name = \"editor\"",
        caret: 0,
        path: "Cargo.toml",
    });
    t.query(".editor-code[data-language=\"toml\"]")
        .await
        .unwrap_or_else(|e| panic!("wrapper: {e:?}"));
    t.query(".md-tok-s")
        .expect(inner_html(contains_substring("editor".to_string())))
        .await
        .unwrap_or_else(|e| panic!("the string: {e:?}"));
    languages::withdraw_language("toml");
}
