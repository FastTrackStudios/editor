//! Languages the host installs: the seam between a host's language
//! registry and the highlighter.
//!
//! The editor knows the grammars compiled into `editor-syntax`
//! ([`Lang`]). A host that installs more (an extension shipping a
//! tree-sitter grammar and a highlights query) registers each here, with
//! the fence tags and file extensions it answers to. Every lookup asks
//! this registry first and the built-in table second, so a registered
//! language highlights a ```` ```tag ```` fence in the markdown pass
//! and a whole buffer in the code editor, and may take over a built-in
//! tag.
//!
//! Same shape as [`crate::fence_renderer`]: process-wide, because the
//! markdown pass that asks has no business carrying a table. Unlike
//! fence renderers, languages come and go while the app runs (a host
//! disables an extension), so [`withdraw_language`] removes one and
//! [`languages_revision`] counts changes.
//!
//! # Wiring
//!
//! ```ignore
//! use editor_state::languages::{self, HostLanguage, LanguageEntry};
//! let toml = HostLanguage::new("toml", tree_sitter_toml::LANGUAGE, HIGHLIGHTS)?;
//! languages::register_language(
//!     LanguageEntry::new("toml", toml).fences(["toml"]).extensions(["toml"]),
//! );
//! // The extension is disabled:
//! languages::withdraw_language("toml");
//! ```
//!
//! Decorations are computed when the editor renders, so after a change
//! here a host re-renders the editors that show affected text (the
//! [`CodeEditor`]'s `language` prop changes by itself; a markdown editor
//! re-renders on its next state change, or when the host bumps its
//! state signal). Token caches are keyed by the [`Syntax`]'s identity,
//! so a withdrawn language's tokens are never served again.
//!
//! [`CodeEditor`]: ../../editor_view/fn.CodeEditor.html

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

pub use editor_syntax::{
    Grammar, HostLanguage, Lang, LanguageError, LanguageFn, Syntax, Token, highlight_with,
};

/// One installed language: its id, the fence tags and file extensions it
/// answers to, and the [`Syntax`] that highlights it.
#[derive(Debug, Clone)]
pub struct LanguageEntry {
    /// Unique among registered languages; registering the same id again
    /// replaces the entry.
    pub id: String,
    /// Fence info tags (` ```kf `), matched case-insensitively. The id is
    /// always one.
    pub fences: Vec<String>,
    /// File extensions without the dot (`kf`), matched case-insensitively.
    pub extensions: Vec<String>,
    pub syntax: Syntax,
}

impl LanguageEntry {
    /// An entry answering to its `id` as a fence tag, with no file
    /// extensions yet.
    #[must_use]
    pub fn new(id: impl Into<String>, syntax: impl Into<Syntax>) -> Self {
        Self {
            id: id.into(),
            fences: Vec::new(),
            extensions: Vec::new(),
            syntax: syntax.into(),
        }
    }

    /// Fence tags it also answers to.
    #[must_use]
    pub fn fences<S: Into<String>>(mut self, tags: impl IntoIterator<Item = S>) -> Self {
        self.fences.extend(tags.into_iter().map(Into::into));
        self
    }

    /// File extensions (no dot) it answers to.
    #[must_use]
    pub fn extensions<S: Into<String>>(mut self, exts: impl IntoIterator<Item = S>) -> Self {
        self.extensions.extend(
            exts.into_iter()
                .map(|e| e.into().trim_start_matches('.').to_owned()),
        );
        self
    }

    fn answers_fence(&self, tag: &str) -> bool {
        self.id.eq_ignore_ascii_case(tag) || self.fences.iter().any(|f| f.eq_ignore_ascii_case(tag))
    }

    fn answers_extension(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

fn registry() -> &'static RwLock<Vec<LanguageEntry>> {
    static REGISTRY: OnceLock<RwLock<Vec<LanguageEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

static REVISION: AtomicU64 = AtomicU64::new(0);

fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

/// Install `entry`, replacing a registered language with the same id.
///
/// When two registered languages claim the same fence tag or extension,
/// the one registered last wins; withdrawing it uncovers the other.
pub fn register_language(entry: LanguageEntry) {
    if let Ok(mut entries) = registry().write() {
        entries.retain(|e| !e.id.eq_ignore_ascii_case(&entry.id));
        entries.push(entry);
    }
    bump();
}

/// Remove the registered language `id`. Returns whether one was there.
/// Its fences and files fall back to the built-in table (or to plain
/// text).
pub fn withdraw_language(id: &str) -> bool {
    let removed = registry().write().is_ok_and(|mut entries| {
        let before = entries.len();
        entries.retain(|e| !e.id.eq_ignore_ascii_case(id));
        entries.len() != before
    });
    if removed {
        bump();
    }
    removed
}

/// The ids of the registered languages, in registration order. The
/// built-in ones are not listed ([`Lang::ALL`]).
#[must_use]
pub fn registered_languages() -> Vec<String> {
    registry()
        .read()
        .map(|entries| entries.iter().map(|e| e.id.clone()).collect())
        .unwrap_or_default()
}

/// A counter that moves on every register and withdraw, so a host can
/// tell whether decorations it computed are out of date.
#[must_use]
pub fn languages_revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

fn find(pred: impl Fn(&LanguageEntry) -> bool) -> Option<Syntax> {
    let entries = registry().read().ok()?;
    entries
        .iter()
        .rev()
        .find(|e| pred(e))
        .map(|e| e.syntax.clone())
}

/// The language for a fence info tag: a registered one, else a built-in
/// one ([`Lang::from_fence_tag`]).
#[must_use]
pub fn language_for_fence(tag: &str) -> Option<Syntax> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    find(|e| e.answers_fence(tag)).or_else(|| Lang::from_fence_tag(tag).map(Syntax::Builtin))
}

/// The language for a file extension (`rs`, `.kf`): a registered one,
/// else a built-in one ([`Lang::from_extension`]).
#[must_use]
pub fn language_for_extension(ext: &str) -> Option<Syntax> {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() {
        return None;
    }
    find(|e| e.answers_extension(ext)).or_else(|| Lang::from_extension(ext).map(Syntax::Builtin))
}

/// The language for a file path, by its extension.
#[must_use]
pub fn language_for_path(path: &str) -> Option<Syntax> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    language_for_extension(ext)
}

/// The language with this id: a registered one, else a built-in one by
/// its name or fence tag.
#[must_use]
pub fn language_by_id(id: &str) -> Option<Syntax> {
    find(|e| e.id.eq_ignore_ascii_case(id))
        .or_else(|| Lang::from_fence_tag(id).map(Syntax::Builtin))
}

/// A registered language as a shared handle, for a host that keeps its
/// own table: `Some` only for host-supplied languages.
#[must_use]
pub fn host_language(id: &str) -> Option<Arc<HostLanguage>> {
    match find(|e| e.id.eq_ignore_ascii_case(id))? {
        Syntax::Host(lang) => Some(lang),
        Syntax::Builtin(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml(name: &str) -> HostLanguage {
        HostLanguage::new(
            name,
            arborium_toml::language(),
            arborium_toml::HIGHLIGHTS_QUERY,
        )
        .expect("toml compiles")
    }

    #[test]
    fn a_registered_language_answers_its_fences_extensions_and_paths() {
        register_language(
            LanguageEntry::new("t-reg", toml("t-reg"))
                .fences(["t-reg-fence"])
                .extensions([".treg"]),
        );
        for found in [
            language_for_fence("t-reg"),
            language_for_fence("T-REG-FENCE"),
            language_for_extension("treg"),
            language_for_path("/a/b/Cargo.TREG"),
            language_by_id("t-reg"),
        ] {
            assert_eq!(found.as_ref().map(Syntax::name), Some("t-reg"));
        }
        assert!(host_language("t-reg").is_some());
        assert!(registered_languages().contains(&"t-reg".to_owned()));
    }

    #[test]
    fn withdrawing_removes_it_and_moves_the_revision() {
        register_language(LanguageEntry::new("t-wd", toml("t-wd")));
        assert!(language_for_fence("t-wd").is_some());
        let before = languages_revision();
        assert!(withdraw_language("t-wd"));
        assert!(languages_revision() > before);
        assert!(language_for_fence("t-wd").is_none());
        assert!(!withdraw_language("t-wd"), "only once");
    }

    #[test]
    fn the_last_registered_wins_and_withdrawing_it_uncovers_the_other() {
        register_language(LanguageEntry::new("t-a", toml("t-a")).fences(["t-shared"]));
        register_language(LanguageEntry::new("t-b", toml("t-b")).fences(["t-shared"]));
        assert_eq!(language_for_fence("t-shared").unwrap().name(), "t-b");
        withdraw_language("t-b");
        assert_eq!(language_for_fence("t-shared").unwrap().name(), "t-a");
        withdraw_language("t-a");
        assert!(language_for_fence("t-shared").is_none());
    }

    #[test]
    fn registering_the_same_id_replaces_it() {
        let first = Arc::new(toml("t-same"));
        register_language(LanguageEntry::new("t-same", Arc::clone(&first)));
        register_language(LanguageEntry::new("t-same", toml("t-same")));
        assert_ne!(
            language_for_fence("t-same"),
            Some(Syntax::Host(first)),
            "the new one, not the old"
        );
        assert_eq!(
            registered_languages()
                .iter()
                .filter(|id| *id == "t-same")
                .count(),
            1
        );
        withdraw_language("t-same");
    }

    #[test]
    fn builtins_answer_when_nothing_is_registered() {
        assert_eq!(language_for_fence("rs"), Some(Syntax::Builtin(Lang::Rust)));
        assert_eq!(
            language_for_path("src/main.rs"),
            Some(Syntax::Builtin(Lang::Rust))
        );
        assert!(language_for_fence("").is_none());
        assert!(language_for_path("Makefile").is_none());
    }
}
