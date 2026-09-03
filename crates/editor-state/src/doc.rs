//! The document — a UTF-8 text buffer indexed by byte offsets.
//!
//! Wraps [`ropey::Rope`] so we get O(log n) inserts/deletes and
//! `String`-like accessors. We always use **byte offsets** as
//! positions, not char offsets — matches `&str` indexing and how
//! every other Rust string API works. (CM6 uses UTF-16 code unit
//! offsets to match the browser; we don't have that constraint
//! and bytes are friendlier to Rust code.)

use ropey::Rope;

/// Immutable view of the document's text. Cheap to clone — the
/// underlying rope is reference-counted so `.clone()` is O(1).
#[derive(Clone, Default)]
pub struct Doc {
    rope: Rope,
}

impl Doc {
    /// Create a doc from a string slice. Infallible.
    ///
    /// Named `new` rather than `from_str`: `Doc` also implements
    /// [`std::str::FromStr`] (`Err = Infallible`), and an inherent method
    /// sharing that name shadows the trait, which is both confusing and a
    /// `clippy::should_implement_trait` error. Use this for the always-Ok
    /// path; use `.parse()` if you want the trait.
    #[must_use]
    pub fn new(s: &str) -> Self {
        Self {
            rope: Rope::from_str(s),
        }
    }

    /// Total length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Borrow the underlying rope. Exposed for adapters that need
    /// rope-native indexing — e.g. `editor-crdt` translating byte
    /// offsets to unicode-scalar offsets via `byte_to_char` without
    /// materializing the document as a `String`.
    #[must_use]
    pub const fn rope(&self) -> &Rope {
        &self.rope
    }

    /// `true` if the doc is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Slice the doc as a `String` over a byte range. Panics on
    /// out-of-bounds or non-char-boundary indices — same contract
    /// as `&str[..]`.
    #[must_use]
    pub fn slice(&self, range: std::ops::Range<usize>) -> String {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }

    /// Internal: insert text at a byte offset. Returns a new doc
    /// (immutable API).
    pub(crate) fn insert(&self, byte_offset: usize, text: &str) -> Self {
        let mut new = self.rope.clone();
        let char_idx = new.byte_to_char(byte_offset);
        new.insert(char_idx, text);
        Self { rope: new }
    }

    /// Internal: delete a byte range. Returns a new doc.
    pub(crate) fn delete(&self, range: std::ops::Range<usize>) -> Self {
        let mut new = self.rope.clone();
        let start = new.byte_to_char(range.start);
        let end = new.byte_to_char(range.end);
        new.remove(start..end);
        Self { rope: new }
    }
}

impl std::str::FromStr for Doc {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<&str> for Doc {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Doc {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

/// The doc's full text.
///
/// This is what supplies `Doc::to_string()`, via the blanket [`ToString`] impl
/// — there used to be an inherent `to_string` here instead, which shadowed the
/// trait method (`clippy::inherent_to_string`). The rope streams itself out
/// chunk by chunk rather than materialising a second copy first.
impl std::fmt::Display for Doc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for chunk in self.rope.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for Doc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Doc")
            .field("len", &self.len())
            .field("text", &self.to_string())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_and_len() {
        let d = Doc::new("hello");
        assert_eq!(d.len(), 5);
        assert_eq!(d.to_string(), "hello");
    }

    #[test]
    fn slice_byte_range() {
        let d = Doc::new("hello world");
        assert_eq!(d.slice(6..11), "world");
    }

    #[test]
    fn insert_creates_new_doc_without_mutating_original() {
        let a = Doc::new("hello");
        let b = a.insert(5, " world");
        assert_eq!(a.to_string(), "hello");
        assert_eq!(b.to_string(), "hello world");
    }

    #[test]
    fn delete_range() {
        let d = Doc::new("hello world");
        let after = d.delete(5..6);
        assert_eq!(after.to_string(), "helloworld");
    }
}
