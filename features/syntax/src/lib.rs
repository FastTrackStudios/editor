//! Token-level syntax highlighting via Arborium tree-sitter.
//!
//! Two ways in, one tokenizer:
//!
//! - [`Lang`]: the grammars compiled into this crate (cargo features),
//!   asked for by fence tag ([`Lang::from_fence_tag`]) or file extension
//!   ([`Lang::from_extension`]); [`highlight`] runs one.
//! - [`HostLanguage`]: a language the *host* supplies at run time, a
//!   tree-sitter [`Grammar`] plus the text of a highlights query.
//!   [`HostLanguage::new`] compiles the query once; [`highlight_with`]
//!   runs it. The grammar is any tree-sitter `Language` value linked into
//!   the host binary, from a grammar crate's `LanguageFn`
//!   (`tree_sitter_toml::LANGUAGE`, `arborium_toml::language()`, …).
//!
//! [`Syntax`] holds either, so a caller that does not care which (the
//! markdown pass, the code editor) highlights through one value.
//!
//! Both produce the same [`Token`] tags: a capture name such as
//! `@keyword.function` maps through `arborium_theme::tag_for_capture` to a
//! short tag (`k`), which the view styles as `md-tok-k`. A host query that
//! uses the standard capture names is styled by every theme with no CSS of
//! its own.
//!
//! # Grammars from `.wasm` files: not supported
//!
//! A grammar loaded from a `.wasm` file at run time needs tree-sitter's
//! `wasm` feature (`WasmStore`, `Parser::set_wasm_store`, backed by
//! `wasmtime-c-api`). `arborium-tree-sitter` (2.18) carries the source of
//! that module (`binding_rust/wasm_language.rs`, behind
//! `#[cfg(feature = "wasm")]`) but its manifest declares no `wasm` feature
//! and no `wasmtime-c-api` dependency, so the module can never be compiled
//! in. The upstream `tree-sitter` crate's `wasm` feature has a different
//! `Language` type (not interchangeable with this one) and runs grammars in
//! wasmtime, which is native only: it does not build for
//! `wasm32-unknown-unknown`, where this crate must also run. So a host
//! language's grammar is linked into the host binary; there is no loader
//! here, and none is faked.

use std::sync::{Arc, OnceLock};

use arborium_tree_sitter::StreamingIterator;

/// A tree-sitter grammar: the parser tables a host links in.
///
/// Build one from a grammar crate's `LanguageFn` with `.into()`
/// (`Grammar::from(arborium_toml::language())`). Cheap to clone (a
/// pointer to static tables).
pub use arborium_tree_sitter::Language as Grammar;
/// The constructor a tree-sitter grammar crate exports
/// (`tree_sitter_<lang>::LANGUAGE`, `arborium_<lang>::language()`).
pub use arborium_tree_sitter::LanguageFn;
/// The grammar ABI versions this highlighter can run
/// (`MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION`). A grammar
/// generated for another ABI is refused by [`HostLanguage::new`].
pub use arborium_tree_sitter::{LANGUAGE_VERSION, MIN_COMPATIBLE_LANGUAGE_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    #[cfg(feature = "lang-rust")]
    Rust,
    #[cfg(feature = "lang-typescript")]
    TypeScript,
    #[cfg(feature = "lang-javascript")]
    JavaScript,
    #[cfg(feature = "lang-python")]
    Python,
    #[cfg(feature = "lang-json")]
    Json,
    #[cfg(feature = "lang-bash")]
    Bash,
    #[cfg(feature = "lang-markdown")]
    Markdown,
}

impl Lang {
    /// Every grammar compiled into this build.
    pub const ALL: &'static [Self] = &[
        #[cfg(feature = "lang-rust")]
        Self::Rust,
        #[cfg(feature = "lang-typescript")]
        Self::TypeScript,
        #[cfg(feature = "lang-javascript")]
        Self::JavaScript,
        #[cfg(feature = "lang-python")]
        Self::Python,
        #[cfg(feature = "lang-json")]
        Self::Json,
        #[cfg(feature = "lang-bash")]
        Self::Bash,
        #[cfg(feature = "lang-markdown")]
        Self::Markdown,
    ];

    /// Map a fenced-code-block info tag (e.g. `rs`, `rust`, `ts`) to a [`Lang`].
    #[must_use]
    pub fn from_fence_tag(tag: &str) -> Option<Self> {
        let normalized = tag.trim().to_ascii_lowercase();
        match normalized.as_str() {
            #[cfg(feature = "lang-rust")]
            "rs" | "rust" => Some(Self::Rust),
            #[cfg(feature = "lang-typescript")]
            "ts" | "typescript" | "mts" | "cts" => Some(Self::TypeScript),
            #[cfg(feature = "lang-javascript")]
            "js" | "javascript" | "mjs" | "cjs" | "node" => Some(Self::JavaScript),
            #[cfg(feature = "lang-python")]
            "py" | "python" | "python3" => Some(Self::Python),
            #[cfg(feature = "lang-json")]
            "json" | "jsonc" => Some(Self::Json),
            #[cfg(feature = "lang-bash")]
            "sh" | "bash" | "shell" | "zsh" => Some(Self::Bash),
            #[cfg(feature = "lang-markdown")]
            "md" | "markdown" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Map a file extension (`rs`, `.tsx`; no dot needed, any case) to a
    /// [`Lang`].
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        let normalized = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        match normalized.as_str() {
            #[cfg(feature = "lang-rust")]
            "rs" => Some(Self::Rust),
            #[cfg(feature = "lang-typescript")]
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            #[cfg(feature = "lang-javascript")]
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            #[cfg(feature = "lang-python")]
            "py" | "pyi" => Some(Self::Python),
            #[cfg(feature = "lang-json")]
            "json" | "jsonc" => Some(Self::Json),
            #[cfg(feature = "lang-bash")]
            "sh" | "bash" | "zsh" => Some(Self::Bash),
            #[cfg(feature = "lang-markdown")]
            "md" | "markdown" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// The language's id: `rust`, `typescript`, `javascript`, `python`,
    /// `json`, `bash`, `markdown`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "lang-rust")]
            Self::Rust => "rust",
            #[cfg(feature = "lang-typescript")]
            Self::TypeScript => "typescript",
            #[cfg(feature = "lang-javascript")]
            Self::JavaScript => "javascript",
            #[cfg(feature = "lang-python")]
            Self::Python => "python",
            #[cfg(feature = "lang-json")]
            Self::Json => "json",
            #[cfg(feature = "lang-bash")]
            Self::Bash => "bash",
            #[cfg(feature = "lang-markdown")]
            Self::Markdown => "markdown",
        }
    }

    /// The compiled-in grammar, for a host that wants to run it with a
    /// query of its own ([`HostLanguage::new`]).
    #[must_use]
    pub fn grammar(self) -> Grammar {
        grammar_for(self).0.into()
    }

    /// The highlights query this crate ships for the language.
    #[must_use]
    pub fn highlights_query(self) -> &'static str {
        grammar_for(self).1
    }

    /// The grammar and its shipped query, compiled once per process.
    fn compiled(self) -> Option<Arc<HostLanguage>> {
        static CACHE: OnceLock<std::sync::Mutex<Vec<(Lang, Arc<HostLanguage>)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let mut entries = cache.lock().ok()?;
        if let Some((_, hit)) = entries.iter().find(|(l, _)| *l == self) {
            return Some(Arc::clone(hit));
        }
        let compiled =
            Arc::new(HostLanguage::new(self.name(), self.grammar(), self.highlights_query()).ok()?);
        entries.push((self, Arc::clone(&compiled)));
        Some(compiled)
    }
}

/// A highlighted token: a non-overlapping byte range with an Arborium scope tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub tag: &'static str,
}

fn grammar_for(lang: Lang) -> (LanguageFn, &'static str) {
    match lang {
        #[cfg(feature = "lang-rust")]
        Lang::Rust => (
            arborium::lang_rust::language(),
            arborium::lang_rust::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-typescript")]
        Lang::TypeScript => (
            arborium::lang_typescript::language(),
            &arborium::lang_typescript::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-javascript")]
        Lang::JavaScript => (
            arborium::lang_javascript::language(),
            arborium::lang_javascript::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-python")]
        Lang::Python => (
            arborium::lang_python::language(),
            arborium::lang_python::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-json")]
        Lang::Json => (
            arborium::lang_json::language(),
            arborium::lang_json::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-bash")]
        Lang::Bash => (
            arborium::lang_bash::language(),
            arborium::lang_bash::HIGHLIGHTS_QUERY,
        ),
        #[cfg(feature = "lang-markdown")]
        Lang::Markdown => (
            arborium::lang_markdown::language(),
            arborium::lang_markdown::HIGHLIGHTS_QUERY,
        ),
    }
}

/// Why a host's grammar and query could not be made into a
/// [`HostLanguage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageError {
    /// The grammar was generated for a tree-sitter ABI this highlighter
    /// cannot run (outside
    /// `MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION`).
    Abi { version: usize },
    /// The highlights query does not compile against the grammar: a syntax
    /// error, or a node, field or capture the grammar does not have.
    Query {
        row: usize,
        column: usize,
        message: String,
    },
}

impl std::fmt::Display for LanguageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abi { version } => write!(
                f,
                "grammar ABI {version} is not in {MIN_COMPATIBLE_LANGUAGE_VERSION}..={LANGUAGE_VERSION}"
            ),
            Self::Query {
                row,
                column,
                message,
            } => write!(
                f,
                "highlights query, line {}, column {}: {message}",
                row.saturating_add(1),
                column.saturating_add(1)
            ),
        }
    }
}

impl std::error::Error for LanguageError {}

/// A language the host supplies: a grammar and its compiled highlights
/// query.
///
/// Build once (compiling the query is the expensive step) and share it
/// (`Arc<HostLanguage>`); it is `Send + Sync`.
pub struct HostLanguage {
    name: String,
    grammar: Grammar,
    query: arborium_tree_sitter::Query,
}

impl HostLanguage {
    /// Compile `highlights` (a tree-sitter highlights query, the text of a
    /// `highlights.scm`) against `grammar`.
    ///
    /// # Errors
    ///
    /// [`LanguageError::Abi`] when the grammar's ABI is not one this
    /// highlighter runs; [`LanguageError::Query`] when the query does not
    /// compile against the grammar.
    pub fn new(
        name: impl Into<String>,
        grammar: impl Into<Grammar>,
        highlights: &str,
    ) -> Result<Self, LanguageError> {
        let grammar: Grammar = grammar.into();
        let version = grammar.abi_version();
        if !(MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION).contains(&version) {
            return Err(LanguageError::Abi { version });
        }
        let query = arborium_tree_sitter::Query::new(&grammar, highlights).map_err(|e| {
            LanguageError::Query {
                row: e.row,
                column: e.column,
                message: e.message,
            }
        })?;
        Ok(Self {
            name: name.into(),
            grammar,
            query,
        })
    }

    /// The name it was built with.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The grammar it parses with.
    #[must_use]
    pub const fn grammar(&self) -> &Grammar {
        &self.grammar
    }
}

impl std::fmt::Debug for HostLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLanguage")
            .field("name", &self.name)
            .field("captures", &self.query.capture_names().len())
            .finish_non_exhaustive()
    }
}

/// A language to highlight with: compiled in, or supplied by the host.
///
/// Equality is by identity for a host language (the same `Arc`), so a
/// language withdrawn and registered again is a different `Syntax` and
/// a cache keyed by it does not serve the old one's tokens.
#[derive(Debug, Clone)]
pub enum Syntax {
    Builtin(Lang),
    Host(Arc<HostLanguage>),
}

impl Syntax {
    /// Highlight `src`; see [`highlight`].
    #[must_use]
    pub fn highlight(&self, src: &str) -> Vec<Token> {
        match self {
            Self::Builtin(lang) => highlight(*lang, src),
            Self::Host(lang) => highlight_with(lang, src),
        }
    }

    /// The language's name (a [`Lang::name`] or the host's name).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin(lang) => lang.name(),
            Self::Host(lang) => lang.name(),
        }
    }
}

impl PartialEq for Syntax {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Builtin(a), Self::Builtin(b)) => a == b,
            (Self::Host(a), Self::Host(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for Syntax {}

impl From<Lang> for Syntax {
    fn from(lang: Lang) -> Self {
        Self::Builtin(lang)
    }
}

impl From<Arc<HostLanguage>> for Syntax {
    fn from(lang: Arc<HostLanguage>) -> Self {
        Self::Host(lang)
    }
}

impl From<HostLanguage> for Syntax {
    fn from(lang: HostLanguage) -> Self {
        Self::Host(Arc::new(lang))
    }
}

/// Parse `src` with `lang`'s grammar and return non-overlapping byte-range tokens
/// sorted by start. Overlapping captures are resolved by preferring the innermost
/// (most recently opened) one.
#[must_use]
pub fn highlight(lang: Lang, src: &str) -> Vec<Token> {
    lang.compiled()
        .map_or_else(Vec::new, |compiled| highlight_with(&compiled, src))
}

/// Parse `src` with a host language and return its tokens, as
/// [`highlight`] does for a compiled-in one.
#[must_use]
pub fn highlight_with(lang: &HostLanguage, src: &str) -> Vec<Token> {
    let mut parser = arborium_tree_sitter::Parser::new();
    if parser.set_language(&lang.grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let query = &lang.query;

    let bytes = src.as_bytes();
    let capture_names = query.capture_names();
    let mut raw: Vec<RawSpan> = Vec::new();

    let mut cursor = arborium_tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let Some(name) = usize::try_from(capture.index)
                .ok()
                .and_then(|i| capture_names.get(i).copied())
            else {
                continue;
            };
            if name.starts_with('_') || name.starts_with("injection.") {
                continue;
            }
            let Some(tag) = arborium_theme::tag_for_capture(name) else {
                continue;
            };
            // A source larger than u32::MAX is not something this highlighter
            // serves; drop the capture rather than truncate its range.
            let (Ok(start), Ok(end)) = (
                u32::try_from(capture.node.start_byte()),
                u32::try_from(capture.node.end_byte()),
            ) else {
                continue;
            };
            raw.push(RawSpan { start, end, tag });
        }
    }

    flatten(raw, src.len())
}

#[derive(Debug, Clone, Copy)]
struct RawSpan {
    start: u32,
    end: u32,
    tag: &'static str,
}

/// Resolve overlaps by sweeping events with a stack and emitting the innermost
/// active tag for each non-overlapping segment. Mirrors dioxus-code's segment
/// algorithm but yields only tagged ranges and coalesces adjacent same-tag
/// outputs.
fn flatten(mut raw: Vec<RawSpan>, src_len: usize) -> Vec<Token> {
    if raw.is_empty() {
        return Vec::new();
    }

    // Outer-first ordering means inner spans get pushed onto the stack later
    // and therefore win at any overlapping position.
    raw.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));

    let mut events: Vec<(u32, bool, usize)> = Vec::with_capacity(raw.len().saturating_mul(2));
    for (i, span) in raw.iter().enumerate() {
        if span.start >= span.end {
            continue;
        }
        events.push((span.start, true, i));
        events.push((span.end, false, i));
    }
    // Process ends before starts at the same position to close before opening.
    events.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut tokens: Vec<Token> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut last_pos: u32 = 0;

    for (pos, is_start, idx) in events {
        if pos > last_pos {
            if let Some(tag) = stack.last().and_then(|&top| raw.get(top)).map(|s| s.tag) {
                // u32 -> usize is lossless on every target this builds for;
                // saturate rather than panic on a hypothetical 16-bit one.
                let (Ok(from), Ok(to)) = (usize::try_from(last_pos), usize::try_from(pos)) else {
                    continue;
                };
                push_token(&mut tokens, from, to, tag);
            }
            last_pos = pos;
        }

        if is_start {
            stack.push(idx);
        } else if let Some(i) = stack.iter().rposition(|&j| j == idx) {
            stack.remove(i);
        }
    }

    // Clip any trailing overrun to the source bounds.
    if let Some(last) = tokens.last_mut() {
        if last.end > src_len {
            last.end = src_len;
        }
        if last.start >= last.end {
            tokens.pop();
        }
    }

    tokens
}

fn push_token(tokens: &mut Vec<Token>, start: usize, end: usize, tag: &'static str) {
    if start >= end {
        return;
    }
    if let Some(last) = tokens.last_mut()
        && last.end == start
        && last.tag == tag
    {
        last.end = end;
        return;
    }
    tokens.push(Token { start, end, tag });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "lang-rust")]
    #[test]
    fn highlights_rust_fn_keyword() {
        let tokens = highlight(Lang::Rust, "fn main() {}");
        assert!(
            tokens
                .iter()
                .any(|t| t.tag == "k" && t.start == 0 && t.end == 2),
            "expected a keyword token for `fn` at 0..2, got {tokens:?}",
        );
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn tokens_are_sorted_and_non_overlapping() {
        let tokens = highlight(Lang::Rust, "fn main() { let x = 1; }");
        for pair in tokens.windows(2) {
            assert!(pair[0].end <= pair[1].start, "overlap: {pair:?}");
        }
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn fence_tag_rust_aliases() {
        assert_eq!(Lang::from_fence_tag("rust"), Some(Lang::Rust));
        assert_eq!(Lang::from_fence_tag("rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_fence_tag("RUST"), Some(Lang::Rust));
    }

    #[test]
    fn fence_tag_unknown_returns_none() {
        assert_eq!(Lang::from_fence_tag("nope-not-a-lang"), None);
    }

    /// The tags of `src`'s tokens, as `(text, tag)`.
    fn tagged<'a>(src: &'a str, tokens: &[Token]) -> Vec<(&'a str, &'static str)> {
        tokens
            .iter()
            .map(|t| (src.get(t.start..t.end).unwrap_or(""), t.tag))
            .collect()
    }

    #[test]
    fn a_host_grammar_and_its_query_highlight_like_a_builtin() {
        let toml = HostLanguage::new(
            "toml",
            arborium_toml::language(),
            arborium_toml::HIGHLIGHTS_QUERY,
        )
        .expect("the toml grammar and its query compile");
        assert_eq!(toml.name(), "toml");
        let src = "[package]\nname = \"editor\"\n";
        let tokens = highlight_with(&toml, src);
        let tags = tagged(src, &tokens);
        assert!(
            tags.iter().any(|(text, _)| *text == "\"editor\""),
            "the string is a token: {tags:?}"
        );
        // The same tag table as the compiled-in grammars: a string is `s`.
        assert!(
            tags.contains(&("\"editor\"", "s")),
            "a string is tagged `s`: {tags:?}"
        );
        for pair in tokens.windows(2) {
            assert!(pair[0].end <= pair[1].start, "overlap: {pair:?}");
        }
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn a_builtin_grammar_runs_with_the_hosts_own_query() {
        let only_fn = HostLanguage::new("rust-fn", Lang::Rust.grammar(), "\"fn\" @keyword")
            .expect("a one-line query compiles");
        let src = "fn main() { let x = 1; }";
        let tags = tagged(src, &highlight_with(&only_fn, src));
        assert_eq!(tags, vec![("fn", "k")], "only what the query captures");
    }

    #[test]
    fn a_query_that_does_not_compile_is_an_error_not_a_panic() {
        let err = HostLanguage::new("toml", arborium_toml::language(), "(no_such_node) @x")
            .expect_err("an unknown node is refused");
        assert!(matches!(err, LanguageError::Query { .. }), "{err:?}");
        assert!(err.to_string().contains("highlights query"), "{err}");
    }

    #[test]
    fn syntax_equality_is_identity_for_host_languages() {
        let make = || {
            Arc::new(
                HostLanguage::new(
                    "toml",
                    arborium_toml::language(),
                    arborium_toml::HIGHLIGHTS_QUERY,
                )
                .expect("compiles"),
            )
        };
        let a = make();
        let b = make();
        assert_eq!(Syntax::Host(Arc::clone(&a)), Syntax::Host(Arc::clone(&a)));
        assert_ne!(Syntax::Host(a), Syntax::Host(b));
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn syntax_builtin_matches_highlight() {
        let src = "fn main() {}";
        assert_eq!(
            Syntax::from(Lang::Rust).highlight(src),
            highlight(Lang::Rust, src)
        );
        assert_eq!(Syntax::from(Lang::Rust).name(), "rust");
    }

    #[test]
    fn every_builtin_compiles_and_is_found_by_its_name_and_extension() {
        for &lang in Lang::ALL {
            assert!(lang.compiled().is_some(), "{lang:?} compiles");
            assert_eq!(Lang::from_fence_tag(lang.name()), Some(lang));
        }
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn extensions_resolve_without_the_dot_and_in_any_case() {
        assert_eq!(Lang::from_extension("rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension(".RS"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension("toml"), None);
    }
}
