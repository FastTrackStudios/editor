//! Caret motions. Pure functions over `EditorState` returning a
//! new byte offset.
//!
//! vim ref: codemirror-vim/src/vim.js (`motions` table)
//! vim ref: helix/helix-core/src/movement.rs

use editor_state::ByteSlice;
use editor_state::{EditorState, TextSlice};

use crate::state::MotionInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,          // h
    Down,          // j
    Up,            // k
    Right,         // l
    WordForward,   // w
    WordBackward,  // b
    WordEnd,       // e
    WORDForward,   // W — whitespace-only delimited
    WORDBackward,  // B
    WORDEnd,       // E
    LineStart,     // 0
    LineEnd,       // $
    FirstNonblank, // ^
    DocStart,      // gg  (count = explicit line)
    DocEnd,        // G
    FindForward,   // f<c>
    FindBackward,  // F<c>
    TillForward,   // t<c>
    TillBackward,  // T<c>
    ParaForward,   // }
    ParaBackward,  // {
    SentForward,   // )
    SentBackward,  // (
    MatchBracket,  // %
    SearchNext,    // n
    SearchPrev,    // N
    EndPrevWord,   // ge
}

/// How a motion combines with an operator.
///
/// vim's `:help exclusive` — an exclusive motion's target char is
/// NOT part of the operated range, an inclusive one's is, and a
/// linewise motion snaps the range to whole lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

impl Motion {
    #[must_use]
    pub const fn from_char(ch: char) -> Option<Self> {
        Some(match ch {
            'h' => Self::Left,
            'j' => Self::Down,
            'k' => Self::Up,
            'l' => Self::Right,
            'w' => Self::WordForward,
            'b' => Self::WordBackward,
            'e' => Self::WordEnd,
            'W' => Self::WORDForward,
            'B' => Self::WORDBackward,
            'E' => Self::WORDEnd,
            '0' => Self::LineStart,
            '$' => Self::LineEnd,
            '^' => Self::FirstNonblank,
            'G' => Self::DocEnd,
            'f' => Self::FindForward,
            'F' => Self::FindBackward,
            't' => Self::TillForward,
            'T' => Self::TillBackward,
            '{' => Self::ParaBackward,
            '}' => Self::ParaForward,
            '(' => Self::SentBackward,
            ')' => Self::SentForward,
            '%' => Self::MatchBracket,
            'n' => Self::SearchNext,
            'N' => Self::SearchPrev,
            // `gg` and `ge` are two-char commands; the dispatcher
            // resolves them via `pending_g` and passes `DocStart`
            // / `EndPrevWord` directly.
            _ => return None,
        })
    }

    /// Operator-combination kind. vim ref: `:help inclusive`.
    #[must_use]
    pub const fn kind(self) -> MotionKind {
        match self {
            Self::WordEnd
            | Self::WORDEnd
            | Self::EndPrevWord
            | Self::FindForward
            | Self::TillForward
            | Self::MatchBracket => MotionKind::Inclusive,
            Self::Up | Self::Down | Self::DocStart | Self::DocEnd => MotionKind::Linewise,
            _ => MotionKind::Exclusive,
        }
    }
}

#[must_use]
pub fn apply(state: &EditorState, motion: Motion, count: usize) -> usize {
    let pos = state.selection.primary().head;
    let count = count.max(1);
    match motion {
        Motion::Left => left(state, pos, count),
        Motion::Right => right(state, pos, count),
        Motion::Up => vertical(state, pos, count, false, None).0,
        Motion::Down => vertical(state, pos, count, true, None).0,
        Motion::WordForward => word_forward(state, pos, count),
        Motion::WordBackward => word_backward(state, pos, count),
        Motion::WordEnd => word_end(state, pos, count),
        Motion::WORDForward => big_word_forward(state, pos, count),
        Motion::WORDBackward => big_word_backward(state, pos, count),
        Motion::WORDEnd => big_word_end(state, pos, count),
        Motion::LineStart => line_start(state, pos),
        Motion::LineEnd => line_end_n(state, pos, count),
        Motion::FirstNonblank => line_first_nonblank(state, pos),
        Motion::DocStart => nth_line_first_nonblank(state, 0),
        Motion::DocEnd => last_line_first_nonblank(state),
        // Sentences: treat as paragraph for v1 (cheap, good
        // enough for tests). vim ref: codemirror-vim sentence_().
        Motion::ParaForward | Motion::SentForward => para_forward(state, pos, count),
        Motion::ParaBackward | Motion::SentBackward => para_backward(state, pos, count),
        Motion::MatchBracket => match_bracket(state, pos).unwrap_or(pos),
        Motion::SearchNext | Motion::SearchPrev => pos, // v1: no search
        Motion::EndPrevWord => end_prev_word(state, pos, count),
        // f/F/t/T are handled via pending-input — never reached here.
        Motion::FindForward | Motion::FindBackward | Motion::TillForward | Motion::TillBackward => {
            pos
        }
    }
}

/// `f`/`F`/`t`/`T`. Line-local (vim never crosses lines here) and
/// char-correct — `ch` may be any Unicode scalar.
#[must_use]
pub fn find_char(
    state: &EditorState,
    pos: usize,
    ch: char,
    input: MotionInput,
    count: usize,
) -> Option<usize> {
    let doc = state.doc.to_string();
    let forward = matches!(input, MotionInput::FindForward | MotionInput::TillForward);
    let till = matches!(input, MotionInput::TillForward | MotionInput::TillBackward);
    let lo = line_start(state, pos);
    let hi = line_end(state, pos);
    let mut hits: usize = 0;
    if forward {
        let start = next_char_boundary(doc.as_bytes(), pos).min(hi);
        for (off, c) in doc.slice(start..hi).char_indices() {
            if c == ch {
                hits = hits.saturating_add(1);
                if hits == count {
                    let at = start.saturating_add(off);
                    return Some(if till {
                        prev_char_boundary(doc.as_bytes(), at).max(lo)
                    } else {
                        at
                    });
                }
            }
        }
    } else {
        let upto = pos.min(hi);
        for (off, c) in doc.slice(lo..upto).char_indices().rev() {
            if c == ch {
                hits = hits.saturating_add(1);
                if hits == count {
                    let at = lo.saturating_add(off);
                    return Some(if till {
                        at.saturating_add(c.len_utf8())
                    } else {
                        at
                    });
                }
            }
        }
    }
    None
}

// --- char boundary helpers --------------------------------------

/// Byte offset of the char boundary strictly before `p` (or 0).
#[must_use]
pub fn prev_char_boundary(bytes: &[u8], p: usize) -> usize {
    let mut q = p.min(bytes.len()).saturating_sub(1);
    while q > 0 && bytes.get(q).is_some_and(|b| (b & 0xC0) == 0x80) {
        q = q.saturating_sub(1);
    }
    q
}

/// Byte offset of the char boundary strictly after `p` (clamped
/// to `bytes.len()`).
#[must_use]
pub fn next_char_boundary(bytes: &[u8], p: usize) -> usize {
    let mut q = p.saturating_add(1).min(bytes.len());
    while q < bytes.len() && bytes.get(q).is_some_and(|b| (b & 0xC0) == 0x80) {
        q = q.saturating_add(1);
    }
    q
}

/// Clamp a normal-mode caret.
///
/// vim never parks the block cursor on the newline (or past EOF)
/// — it sits on the line's last char instead. Insert/visual modes
/// are exempt; callers apply this only where vim would (`:help
/// 'virtualedit'` default).
#[must_use]
pub fn clamp_normal(state: &EditorState, pos: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = pos.min(bytes.len());
    if p == bytes.len() || bytes.get(p) == Some(&b'\n') {
        let ls = line_start(state, p);
        if p > ls {
            p = prev_char_boundary(bytes, p).max(ls);
        }
    }
    p
}

// --- horizontal/vertical ---------------------------------------

fn left(state: &EditorState, pos: usize, n: usize) -> usize {
    let s = state.doc.to_string();
    let line_lo = line_start(state, pos);
    let bytes = s.as_bytes();
    let mut p = pos;
    for _ in 0..n {
        if p <= line_lo {
            break;
        }
        p = prev_char_boundary(bytes, p).max(line_lo);
    }
    p
}

fn right(state: &EditorState, pos: usize, n: usize) -> usize {
    let s = state.doc.to_string();
    let line_hi = line_end(state, pos);
    let bytes = s.as_bytes();
    let mut p = pos;
    for _ in 0..n {
        if p >= line_hi {
            break;
        }
        p = next_char_boundary(bytes, p).min(line_hi);
    }
    p
}

/// Vertical movement by logical lines, char-column based (UTF-8 safe).
///
/// `goal` is the sticky column (in chars) from a previous `j`/`k`
/// — vim keeps the column you started from even across shorter
/// lines. Returns `(new_pos, goal_col)` so the caller can stash
/// the column back into its state.
#[must_use]
pub fn vertical(
    state: &EditorState,
    pos: usize,
    n: usize,
    down: bool,
    goal: Option<usize>,
) -> (usize, usize) {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let ls = line_start(state, pos);
    let cur_col = s.slice(ls..pos).chars().count();
    let col = goal.unwrap_or(cur_col);
    let mut start = ls;
    if down {
        for _ in 0..n {
            let Some(nl) = next_newline(state, start) else {
                break;
            };
            if nl >= len {
                break;
            }
            start = nl.saturating_add(1);
        }
    } else {
        for _ in 0..n {
            if start == 0 {
                break;
            }
            let prev_nl = start.saturating_sub(1);
            let mut prev_start = prev_nl;
            while prev_start > 0
                && bytes
                    .get(prev_start.saturating_sub(1))
                    .is_some_and(|&b| b != b'\n')
            {
                prev_start = prev_start.saturating_sub(1);
            }
            start = prev_start;
        }
    }
    let line_hi = next_newline(state, start).unwrap_or(len);
    let mut p = start;
    for ch in s.slice(start..line_hi).chars().take(col) {
        p = p.saturating_add(ch.len_utf8());
    }
    (p, col)
}

// --- word motions ----------------------------------------------

const fn is_word(b: u8) -> bool {
    // Continuation bytes of multi-byte chars land in the `3`
    // (punct) class below, so a multi-byte run groups as one
    // "word" of punct class. ASCII-classing is v1-good-enough.
    b.is_ascii_alphanumeric() || b == b'_'
}

const fn classify(b: u8) -> u8 {
    if b == b'\n' {
        2
    } else if b.is_ascii_whitespace() {
        0
    } else if is_word(b) {
        1
    } else {
        3
    }
}

/// `W`-family classifier: everything non-whitespace is one class.
const fn classify_big(b: u8) -> u8 {
    match classify(b) {
        0 => 0,
        2 => 2,
        _ => 1,
    }
}

/// Skip whitespace forward, crossing newlines the way vim's word
/// motions do: an *empty* line is itself a word target, so stop
/// on it. Returns the new position.
fn skip_ws_forward(bytes: &[u8], mut p: usize) -> usize {
    while p < bytes.len() {
        match bytes.get(p) {
            Some(b' ' | b'\t' | b'\r') => p = p.saturating_add(1),
            Some(b'\n') => {
                // Peek: if the next line is empty, it's a stop.
                let next = p.saturating_add(1);
                if next < bytes.len() && bytes.get(next) == Some(&b'\n') {
                    return next;
                }
                p = p.saturating_add(1);
            }
            _ => break,
        }
    }
    p
}

fn word_forward_impl(bytes: &[u8], pos: usize, n: usize, class: fn(u8) -> u8) -> usize {
    let mut p = pos;
    for _ in 0..n {
        if p >= bytes.len() {
            break;
        }
        let Some(&b) = bytes.get(p) else { break };
        let c = class(b);
        if c == 2 {
            // On an (empty-line) newline: step over it.
            p = p.saturating_add(1);
        } else if c != 0 {
            // Skip the current word/punct run.
            while p < bytes.len() && bytes.get(p).is_some_and(|&b| class(b) == c) {
                p = p.saturating_add(1);
            }
        }
        p = skip_ws_forward(bytes, p);
    }
    p
}

fn word_backward_impl(bytes: &[u8], pos: usize, n: usize, class: fn(u8) -> u8) -> usize {
    let mut p = pos.min(bytes.len());
    for _ in 0..n {
        if p == 0 {
            break;
        }
        p = p.saturating_sub(1);
        // Skip whitespace going left, stopping on empty lines.
        while p > 0 {
            match bytes.get(p) {
                Some(b' ' | b'\t' | b'\r') => p = p.saturating_sub(1),
                Some(b'\n') => {
                    if bytes.get(p.saturating_sub(1)) == Some(&b'\n') {
                        break; // empty line is a word target
                    }
                    p = p.saturating_sub(1);
                }
                _ => break,
            }
        }
        let Some(&b) = bytes.get(p) else { continue };
        let c = class(b);
        if c == 2 || c == 0 {
            continue;
        }
        while p > 0
            && bytes
                .get(p.saturating_sub(1))
                .is_some_and(|&b| class(b) == c)
        {
            p = p.saturating_sub(1);
        }
    }
    p
}

fn word_end_impl(bytes: &[u8], pos: usize, n: usize, class: fn(u8) -> u8) -> usize {
    let mut p = pos;
    for _ in 0..n {
        if p.saturating_add(1) >= bytes.len() {
            break;
        }
        p = p.saturating_add(1);
        // Skip whitespace (newlines included — `e` sails past
        // empty lines, unlike `w`).
        while p < bytes.len() && bytes.get(p).is_some_and(|&b| matches!(class(b), 0 | 2)) {
            p = p.saturating_add(1);
        }
        if p >= bytes.len() {
            p = prev_char_boundary(bytes, bytes.len());
            break;
        }
        let Some(&b) = bytes.get(p) else { break };
        let c = class(b);
        while p.saturating_add(1) < bytes.len()
            && bytes
                .get(p.saturating_add(1))
                .is_some_and(|&b| class(b) == c)
        {
            p = p.saturating_add(1);
        }
    }
    let mut p = p.min(bytes.len().saturating_sub(1));
    // Classes are byte-wise, so a multi-byte char's run can end on
    // a continuation byte — snap back to the char's lead byte.
    while p > 0 && bytes.get(p).is_some_and(|b| (b & 0xC0) == 0x80) {
        p = p.saturating_sub(1);
    }
    p
}

fn word_forward(state: &EditorState, pos: usize, n: usize) -> usize {
    word_forward_impl(state.doc.to_string().as_bytes(), pos, n, classify)
}

fn word_backward(state: &EditorState, pos: usize, n: usize) -> usize {
    word_backward_impl(state.doc.to_string().as_bytes(), pos, n, classify)
}

fn word_end(state: &EditorState, pos: usize, n: usize) -> usize {
    word_end_impl(state.doc.to_string().as_bytes(), pos, n, classify)
}

fn big_word_forward(state: &EditorState, pos: usize, n: usize) -> usize {
    word_forward_impl(state.doc.to_string().as_bytes(), pos, n, classify_big)
}

fn big_word_backward(state: &EditorState, pos: usize, n: usize) -> usize {
    word_backward_impl(state.doc.to_string().as_bytes(), pos, n, classify_big)
}

fn big_word_end(state: &EditorState, pos: usize, n: usize) -> usize {
    word_end_impl(state.doc.to_string().as_bytes(), pos, n, classify_big)
}

/// `ge` — end of the previous word, inclusive.
fn end_prev_word(state: &EditorState, pos: usize, n: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = pos.min(bytes.len());
    for _ in 0..n {
        if p == 0 {
            break;
        }
        let started_on = bytes.get(p).map_or(0, |&b| classify(b));
        p = p.saturating_sub(1);
        // If we're still inside the word we started in, back out
        // of it first.
        if p > 0
            && bytes.get(p).is_some_and(|&b| classify(b) == started_on)
            && matches!(started_on, 1 | 3)
        {
            let c = started_on;
            while p > 0
                && bytes
                    .get(p.saturating_sub(1))
                    .is_some_and(|&b| classify(b) == c)
            {
                p = p.saturating_sub(1);
            }
            if p == 0 {
                break;
            }
            p = p.saturating_sub(1);
        }
        // Skip whitespace (incl. newlines) going left.
        while p > 0 && bytes.get(p).is_some_and(|&b| matches!(classify(b), 0 | 2)) {
            p = p.saturating_sub(1);
        }
    }
    p
}

// --- line helpers ----------------------------------------------

#[must_use]
pub fn line_start(state: &EditorState, pos: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = pos.min(bytes.len());
    while p > 0 && bytes.get(p.saturating_sub(1)).is_some_and(|&b| b != b'\n') {
        p = p.saturating_sub(1);
    }
    p
}

#[must_use]
pub fn line_end(state: &EditorState, pos: usize) -> usize {
    next_newline(state, pos).unwrap_or(state.doc.len())
}

#[must_use]
pub fn line_end_n(state: &EditorState, pos: usize, count: usize) -> usize {
    // `$` with count=1 → end of current line; with count=N → end
    // of line N-1 below.
    let mut p = pos;
    for _ in 0..count.saturating_sub(1) {
        let nl = next_newline(state, p).unwrap_or(state.doc.len());
        if nl == state.doc.len() {
            return nl;
        }
        p = nl.saturating_add(1);
    }
    next_newline(state, p).unwrap_or(state.doc.len())
}

/// Position at the first non-blank character of the `n`-th
/// line (0-indexed). Used by `gg` / `<N>G`. Clamps `n` to the
/// last line of the doc.
#[must_use]
pub fn nth_line_first_nonblank(state: &EditorState, n: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = 0usize;
    let mut line = 0usize;
    while line < n && p < bytes.len() {
        if let Some(off) = bytes.after(p).iter().position(|&b| b == b'\n') {
            p = p.saturating_add(off).saturating_add(1);
            line = line.saturating_add(1);
        } else {
            break;
        }
    }
    line_first_nonblank(state, p)
}

/// First non-blank of the last line — `G` without a count.
#[must_use]
pub fn last_line_first_nonblank(state: &EditorState) -> usize {
    let len = state.doc.len();
    line_first_nonblank(state, line_start(state, len))
}

#[must_use]
pub fn line_first_nonblank(state: &EditorState, pos: usize) -> usize {
    let start = line_start(state, pos);
    let end = line_end(state, pos);
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = start;
    while p < end && bytes.at(p).is_ascii_whitespace() {
        p = p.saturating_add(1);
    }
    p
}

fn next_newline(state: &EditorState, pos: usize) -> Option<usize> {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    bytes
        .iter()
        .skip(pos)
        .position(|&b| b == b'\n')
        .map(|i| pos.saturating_add(i))
}

// --- paragraph/bracket -----------------------------------------

fn para_forward(state: &EditorState, pos: usize, n: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = pos;
    for _ in 0..n {
        // advance to the next blank line.
        loop {
            let Some(nl) = next_newline(state, p) else {
                return bytes.len();
            };
            p = nl.saturating_add(1);
            let line_end_pos = next_newline(state, p).unwrap_or(bytes.len());
            if p == line_end_pos
                || bytes
                    .slice(p..line_end_pos)
                    .iter()
                    .all(u8::is_ascii_whitespace)
            {
                break;
            }
        }
    }
    p
}

fn para_backward(state: &EditorState, pos: usize, n: usize) -> usize {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut p = pos;
    for _ in 0..n {
        if p == 0 {
            break;
        }
        loop {
            let line_lo = line_start(state, p.saturating_sub(1));
            let line_hi = next_newline(state, line_lo).unwrap_or(bytes.len());
            p = line_lo;
            if line_lo == line_hi
                || bytes
                    .slice(line_lo..line_hi)
                    .iter()
                    .all(u8::is_ascii_whitespace)
            {
                break;
            }
            if p == 0 {
                break;
            }
        }
    }
    p
}

fn match_bracket(state: &EditorState, pos: usize) -> Option<usize> {
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    if pos >= bytes.len() {
        return None;
    }
    // vim `%`: if the caret isn't on a bracket, scan forward on
    // the current line for the first one.
    let line_hi = line_end(state, pos);
    let mut at = pos;
    while at < line_hi && !matches!(bytes.at(at), b'(' | b'[' | b'{' | b')' | b']' | b'}') {
        at = at.saturating_add(1);
    }
    if at >= line_hi {
        return None;
    }
    let (open, close, forward) = match bytes.at(at) {
        b'(' => (b'(', b')', true),
        b'[' => (b'[', b']', true),
        b'{' => (b'{', b'}', true),
        b')' => (b'(', b')', false),
        b']' => (b'[', b']', false),
        b'}' => (b'{', b'}', false),
        _ => return None,
    };
    let mut depth = 0i32;
    if forward {
        for i in at..bytes.len() {
            if bytes.at(i) == open {
                depth = depth.saturating_add(1);
            } else if bytes.at(i) == close {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    } else {
        for i in (0..=at).rev() {
            if bytes.at(i) == close {
                depth = depth.saturating_add(1);
            } else if bytes.at(i) == open {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    }
    None
}
