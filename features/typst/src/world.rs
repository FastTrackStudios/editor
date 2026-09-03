//! In-memory `typst::World` impl. One main source, bundled
//! fonts, no filesystem, no `@preview` package downloads. Good
//! enough for inline-math + small block snippets in the editor;
//! file imports + the package registry can come later.
//!
//! `typst` expects `World: Sync` so it can in principle cache
//! results across threads. We're single-threaded (browser),
//! so we wrap interior mutable state in `parking_lot::Mutex`
//! and `send_wrapper::SendWrapper` — same trick the
//! obsidian-typst plugin uses to get the API past Rust's
//! type checker for wasm builds.

use std::collections::HashMap;

use parking_lot::Mutex;
use typst::Library;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, Span};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;

const MAIN_VIRTUAL_PATH: &str = "/main.typ";

/// Fallback compile date, used only if the host clock reports a date typst
/// will not accept. Any fixed date works; typst only reads it for `datetime`.
const EPOCH_YEAR: i32 = 2026;

pub struct World {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_id: FileId,
    /// Source files keyed by `FileId`. `set_main_source` rewrites
    /// the entry under `main_id`. Future package / import
    /// resolution can populate additional entries.
    sources: Mutex<HashMap<FileId, Source>>,
    /// Binary file contents (images etc.). Empty for v1.
    files: Mutex<HashMap<FileId, Bytes>>,
    /// Tracked clock — typst calls `today()` during eval. Pin to the build
    /// time so successive compiles are deterministic. `None` if the host
    /// clock reported a date typst rejects; typst reads that as "unknown".
    today: Option<Datetime>,
}

impl World {
    /// Build a world preloaded with the typst-assets bundled
    /// font set (`typst-assets/fonts` feature).
    #[must_use]
    pub fn with_bundled_fonts() -> Self {
        use chrono::Datelike;

        let mut fonts = Vec::new();
        for data in typst_assets::fonts() {
            let bytes = Bytes::new(data);
            for font in Font::iter(bytes) {
                fonts.push(font);
            }
        }
        let book = FontBook::from_fonts(&fonts);
        let main_id = FileId::new(None, typst::syntax::VirtualPath::new(MAIN_VIRTUAL_PATH));
        let mut sources = HashMap::new();
        sources.insert(main_id, Source::new(main_id, String::new()));
        // `chrono` with `clock` + `wasmbind` routes through
        // `Date.now()` on `wasm32-unknown-unknown`; without
        // `wasmbind` `Local::now()` panics with "time not
        // implemented on this platform".
        let now = chrono::Local::now();
        // `month()`/`day()` are 1..=12 and 1..=31, so the `u8` conversions
        // cannot fail; `and_then` keeps that a fact rather than an assertion,
        // and any surprise falls through to the same epoch fallback as an
        // out-of-range date.
        let today = u8::try_from(now.month())
            .ok()
            .zip(u8::try_from(now.day()).ok())
            .and_then(|(month, day)| Datetime::from_ymd(now.year(), month, day))
            .or_else(|| Datetime::from_ymd(EPOCH_YEAR, 1, 1));
        Self {
            library: LazyHash::new(Library::builder().build()),
            book: LazyHash::new(book),
            fonts,
            main_id,
            sources: Mutex::new(sources),
            files: Mutex::new(HashMap::new()),
            today,
        }
    }

    pub fn set_main_source(&mut self, text: String) {
        let id = self.main_id;
        let mut sources = self.sources.lock();
        if let Some(src) = sources.get_mut(&id) {
            if src.text() != text.as_str() {
                src.replace(&text);
            }
        } else {
            sources.insert(id, Source::new(id, text));
        }
    }

    /// Resolve a typst span back to a human-readable
    /// `path:line:col` string for diagnostics.
    pub fn lookup_span(&self, span: Span) -> Option<String> {
        let id = span.id()?;
        // Resolve under the lock, then release it before formatting — the
        // guard must not be live across the `?` or the `format!`.
        let (line, col) = {
            let sources = self.sources.lock();
            sources.get(&id).and_then(|src| {
                let range = src.range(span)?;
                Some((
                    src.byte_to_line(range.start)?.saturating_add(1),
                    src.byte_to_column(range.start)?.saturating_add(1),
                ))
            })
        }?;
        Some(format!(
            "{}:{}:{}",
            id.vpath().as_rooted_path().display(),
            line,
            col
        ))
    }
}

impl typst::World for World {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.sources
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(id.vpath().as_rooted_path().to_path_buf()))
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(id.vpath().as_rooted_path().to_path_buf()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        self.today
    }
}
