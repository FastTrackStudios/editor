//! Non-panicking byte-range access into `str` and slices.
//!
//! The editor works in byte offsets: parser spans, decoration ranges,
//! cursor positions. Almost all of them are correct almost all of the
//! time — but "almost" is the problem. A stale span surviving one edit
//! too long, or a range that lands mid-codepoint because the document
//! holds text rather than ASCII, turns into `panic!` inside whatever
//! app embedded the editor. Editors do not get to crash their host.
//!
//! So indexing is banned repo-wide (see the root `Cargo.toml`) and every
//! offset-driven read goes through the helpers here instead. They clamp
//! rather than panic: an out-of-range end becomes the end of the text, a
//! reversed range becomes empty, and an offset inside a multi-byte
//! character moves outward to the nearest boundary. For every offset
//! that was already valid — which is to say the overwhelming majority —
//! the result is byte-identical to what `&text[range]` returned.
//!
//! Clamping is the right failure mode here, not `Option`: a decoration
//! drawn one character short is a cosmetic glitch the next keystroke
//! repairs, whereas propagating `None` through the markdown pass would
//! silently drop whole spans and give callers a second way to be wrong.
//! Where a caller genuinely needs to distinguish "empty" from "invalid",
//! it should check the range itself before slicing.

use std::ops::Range;

/// Byte-offset reads that clamp instead of panicking.
///
/// Implemented for `str`; see the module docs for the clamping rules.
pub trait TextSlice {
    /// The substring covered by `range`, clamped to the text's bounds and
    /// widened to the nearest character boundaries. Empty if the range is
    /// reversed or starts past the end.
    fn slice(&self, range: Range<usize>) -> &str;

    /// Everything before `end`, clamped as in [`TextSlice::slice`].
    fn before(&self, end: usize) -> &str;

    /// Everything from `start` onward, clamped as in [`TextSlice::slice`].
    fn after(&self, start: usize) -> &str;
}

impl TextSlice for str {
    fn slice(&self, range: Range<usize>) -> &str {
        let start = floor_boundary(self, range.start);
        let end = ceil_boundary(self, range.end);
        // `get` rather than `&self[..]`: the boundary helpers already
        // guarantee this succeeds, and this way a future bug in them
        // degrades to an empty span instead of a panic.
        if start > end {
            return "";
        }
        self.get(start..end).unwrap_or_default()
    }

    fn before(&self, end: usize) -> &str {
        self.get(..ceil_boundary(self, end)).unwrap_or_default()
    }

    fn after(&self, start: usize) -> &str {
        self.get(floor_boundary(self, start)..).unwrap_or_default()
    }
}

/// Largest character boundary at or before `offset`, clamped to `s.len()`.
///
/// Moving a start offset *backwards* keeps the character it landed inside
/// whole rather than lopping off its leading byte.
fn floor_boundary(s: &str, offset: usize) -> usize {
    let mut i = offset.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

/// Smallest character boundary at or after `offset`, clamped to `s.len()`.
///
/// Moving an end offset *forwards* is the mirror of [`floor_boundary`]:
/// together they widen a mid-character range to the characters it touches
/// instead of splitting one.
fn ceil_boundary(s: &str, offset: usize) -> usize {
    let mut i = offset.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i = i.saturating_add(1);
    }
    i
}

/// Byte-offset reads into a `[u8]` that clamp instead of panicking.
///
/// The `str` counterpart of this is [`TextSlice`]; this one exists for the
/// byte-level scanners in the markdown pass, which walk `as_bytes()` looking
/// for ASCII markers (`#`, `>`, `-`, backticks). Those loops carry their own
/// bounds checks, but the checks and the reads are separate expressions, so
/// nothing but care keeps them in agreement — and care is what this lint set
/// exists to stop relying on.
pub trait ByteSlice {
    /// The byte at `i`, or `0` if `i` is out of range.
    ///
    /// `0` is the right sentinel for these callers specifically: every one of
    /// them compares against a non-zero ASCII marker or asks an `is_ascii_*`
    /// question, so an out-of-range read answers "no" exactly as a bounds
    /// check would have. It is NOT a general-purpose accessor — if a caller
    /// ever needs to tell a real NUL from the end of input, it wants
    /// [`slice::get`] instead.
    fn at(&self, i: usize) -> u8;

    /// The bytes covered by `range`, clamped to the slice's bounds. Empty if
    /// the range is reversed or starts past the end.
    fn slice(&self, range: Range<usize>) -> &[u8];

    /// Everything from `start` onward, clamped. Empty past the end.
    fn after(&self, start: usize) -> &[u8];
}

impl ByteSlice for [u8] {
    fn at(&self, i: usize) -> u8 {
        self.get(i).copied().unwrap_or(0)
    }

    fn slice(&self, range: Range<usize>) -> &[u8] {
        let start = range.start.min(self.len());
        let end = range.end.min(self.len());
        if start > end {
            return &[];
        }
        self.get(start..end).unwrap_or_default()
    }

    fn after(&self, start: usize) -> &[u8] {
        self.get(start.min(self.len())..).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteSlice, TextSlice};

    #[test]
    fn valid_ranges_match_plain_indexing() {
        let s = "hello world";
        assert_eq!(s.slice(0..5), "hello");
        assert_eq!(s.slice(6..11), "world");
        assert_eq!(s.before(5), "hello");
        assert_eq!(s.after(6), "world");
    }

    #[test]
    fn out_of_range_clamps_instead_of_panicking() {
        let s = "abc";
        assert_eq!(s.slice(0..99), "abc");
        assert_eq!(s.slice(99..120), "");
        assert_eq!(s.before(99), "abc");
        assert_eq!(s.after(99), "");
    }

    #[test]
    fn reversed_range_is_empty() {
        // Bound rather than written inline: a literal `4..2` is a
        // `reversed_empty_ranges` error, and reversing it is the point.
        let (from, to) = (4, 2);
        assert_eq!("abcdef".slice(from..to), "");
    }

    #[test]
    fn mid_character_offsets_widen_to_boundaries() {
        // 'é' is two bytes at 1..3, so 2 is not a boundary.
        let s = "aéb";
        assert_eq!(s.len(), 4);
        assert_eq!(s.slice(1..2), "é");
        assert_eq!(s.slice(2..4), "éb");
        // `before` widens its end exactly as `slice` does, so an offset
        // inside 'é' keeps the whole character rather than splitting it.
        assert_eq!(s.before(2), "aé");
        assert_eq!(s.after(2), "éb");
    }

    #[test]
    fn bytes_read_out_of_range_as_zero() {
        let b: &[u8] = b"ab";
        assert_eq!(b.at(0), b'a');
        assert_eq!(b.at(1), b'b');
        assert_eq!(b.at(2), 0);
        assert_eq!(b.at(usize::MAX), 0);
        // The sentinel answers marker questions the way a bounds check would.
        assert!(b.at(9) != b'#');
        assert!(!b.at(9).is_ascii_digit());
    }

    #[test]
    fn byte_ranges_clamp() {
        let b: &[u8] = b"abcde";
        assert_eq!(b.slice(1..3), b"bc");
        assert_eq!(b.slice(3..99), b"de");
        assert_eq!(b.slice(9..12), b"");
        let (from, to) = (4, 1);
        assert_eq!(b.slice(from..to), b"");
        assert_eq!(b.after(3), b"de");
        assert_eq!(b.after(99), b"");
    }

    #[test]
    fn empty_text_is_always_empty() {
        assert_eq!("".slice(0..0), "");
        assert_eq!("".slice(3..9), "");
        assert_eq!("".before(4), "");
        assert_eq!("".after(4), "");
    }
}
