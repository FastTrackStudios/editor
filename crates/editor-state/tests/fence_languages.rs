//! A fence in a language the host installed is highlighted by the
//! markdown pass, with the token classes themes already style, and stops
//! being highlighted when the host withdraws the language.
//!
//! An integration test on purpose: it registers from OUTSIDE the crate,
//! as a host does (`editor_state::languages`).

use editor_state::languages::{self, HostLanguage, Lang, LanguageEntry};
use editor_state::{DecorationKind, EditorState, Selection};

fn toml() -> HostLanguage {
    HostLanguage::new(
        "toml",
        arborium_toml::language(),
        arborium_toml::HIGHLIGHTS_QUERY,
    )
    .expect("toml compiles")
}

/// The `md-tok-*` marks the live preview puts on `doc`, as
/// `(text, class)`.
fn token_marks(doc: &str) -> Vec<(String, String)> {
    let mut state = EditorState::new(doc);
    // The caret at the end, off the fence.
    state.selection = Selection::caret(doc.len());
    editor_state::markdown::live_preview(&state)
        .into_iter()
        .filter_map(|d| match d.kind {
            DecorationKind::Mark { class, .. } if class.starts_with("md-tok-") => {
                Some((doc.get(d.from..d.to).unwrap_or("").to_owned(), class))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_registered_languages_fence_is_highlighted_and_withdrawing_it_stops_that() {
    let doc = "# Config\n\n```tomlish\nname = \"editor\"\n```\n";
    assert!(token_marks(doc).is_empty(), "unknown before registering");

    languages::register_language(LanguageEntry::new("tomlish", toml()));
    let marks = token_marks(doc);
    assert!(
        marks.contains(&("\"editor\"".to_owned(), "md-tok-s".to_owned())),
        "the string is styled as one: {marks:?}"
    );

    assert!(languages::withdraw_language("tomlish"));
    assert!(
        token_marks(doc).is_empty(),
        "withdrawn: plain again, not served from the cache"
    );
}

#[test]
fn a_registered_language_takes_over_a_builtin_tag_until_withdrawn() {
    let doc = "```jsonc\n{\"a\": true}\n```\n";
    let builtin = token_marks(doc);
    assert!(
        !builtin.is_empty(),
        "the built-in JSON grammar highlights it"
    );

    // A host's own query over the built-in grammar: only `true`.
    let only_true = HostLanguage::new("jsonc-host", Lang::Json.grammar(), "(true) @constant")
        .expect("compiles");
    languages::register_language(LanguageEntry::new("jsonc-host", only_true).fences(["jsonc"]));
    assert_eq!(
        token_marks(doc),
        vec![("true".to_owned(), "md-tok-co".to_owned())],
        "the host's language answers the tag"
    );

    languages::withdraw_language("jsonc-host");
    assert_eq!(token_marks(doc), builtin, "the built-in one again");
}
