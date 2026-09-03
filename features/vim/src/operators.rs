//! Operators: `d`, `c`, `y` and friends. An operator takes a
//! byte range (built from a motion or text object) and returns a
//! [`TransactionSpec`] that performs the edit + updates the
//! relevant register.
//!
//! vim ref: codemirror-vim/src/vim.js (`operators` table)
//! vim ref: zed/crates/vim/src/normal/delete.rs

use editor_state::{Changes, EditorState, Selection, TransactionSpec};

use crate::motions;
use crate::state::VimState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,  // d
    Change,  // c
    Yank,    // y
    Indent,  // >
    Outdent, // <
}

impl Operator {
    #[must_use]
    pub const fn from_char(ch: char) -> Option<Self> {
        Some(match ch {
            'd' => Self::Delete,
            'c' => Self::Change,
            'y' => Self::Yank,
            '>' => Self::Indent,
            '<' => Self::Outdent,
            _ => return None,
        })
    }
}

/// Apply an operator to a half-open byte range `[from, to)`.
/// Charwise — does **not** snap to line bounds.
pub fn apply_range(
    state: &EditorState,
    vim: &mut VimState,
    op: Operator,
    from: usize,
    to: usize,
) -> TransactionSpec {
    let (lo, hi) = (from.min(to), from.max(to));
    let text = if hi > lo {
        state.doc.slice(lo..hi)
    } else {
        String::new()
    };
    let reg = vim.pending_register.take();
    vim.registers.write(reg, &text);
    vim.clear_pending();
    match op {
        Operator::Yank => {
            // No edit, caret returns to start of range.
            TransactionSpec::new().selection(Selection::caret(lo))
        }
        Operator::Delete => {
            // Post-delete the caret sits at `lo`; if that's now
            // past the line's last char, clamp back one (vim
            // never parks a normal-mode caret on the newline).
            let s = state.doc.to_string();
            let bytes = s.as_bytes();
            let at_eol_after = hi >= bytes.len() || bytes.get(hi) == Some(&b'\n');
            let ls = motions::line_start(state, lo);
            let caret = if at_eol_after && lo > ls {
                motions::prev_char_boundary(bytes, lo).max(ls)
            } else {
                lo
            };
            TransactionSpec::new()
                .changes(Changes::delete(lo..hi))
                .selection(Selection::caret(caret))
        }
        Operator::Change => {
            vim.mode = crate::state::Mode::Insert;
            TransactionSpec::new()
                .changes(Changes::delete(lo..hi))
                .selection(Selection::caret(lo))
        }
        Operator::Indent => indent_range(state, lo, hi, /*outdent=*/ false),
        Operator::Outdent => indent_range(state, lo, hi, /*outdent=*/ true),
    }
}

/// Linewise operator.
///
/// The dispatcher passes a range that already
/// includes the trailing newline (when one exists). For yank/del
/// the register payload is suffixed with `\n` so paste reinserts
/// the line correctly.
pub fn apply_linewise(
    state: &EditorState,
    vim: &mut VimState,
    op: Operator,
    from: usize,
    to: usize,
) -> TransactionSpec {
    let (lo, hi) = (from.min(to), from.max(to));
    let mut text = if hi > lo {
        state.doc.slice(lo..hi)
    } else {
        String::new()
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let reg = vim.pending_register.take();
    vim.registers.write(reg, &text);
    vim.clear_pending();
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    match op {
        Operator::Yank => TransactionSpec::new().selection(Selection::caret(lo)),
        Operator::Delete => {
            // Caret lands on the first non-blank of the line that
            // moves up into the hole (or the previous line when
            // the deleted lines were the last ones).
            let caret = if hi < bytes.len() {
                let next_line_nb = motions::line_first_nonblank(state, hi);
                lo.saturating_add(next_line_nb.saturating_sub(hi))
            } else if lo > 0 {
                // Deleted through EOF — also eat the newline that
                // used to precede the deleted block, so the doc
                // doesn't keep a dangling blank last line.
                motions::line_first_nonblank(state, lo.saturating_sub(1))
            } else {
                0
            };
            let (lo, hi) = if hi >= bytes.len() && lo > 0 {
                (lo.saturating_sub(1), hi)
            } else {
                (lo, hi)
            };
            TransactionSpec::new()
                .changes(Changes::delete(lo..hi))
                .selection(Selection::caret(caret))
        }
        Operator::Change => {
            // `cc`/`S`: clear the line's content but KEEP the
            // trailing newline (vim doesn't merge lines here) and
            // the leading indent (autoindent behavior).
            vim.mode = crate::state::Mode::Insert;
            let content_hi = if hi > lo && bytes.get(hi.saturating_sub(1)) == Some(&b'\n') {
                hi.saturating_sub(1)
            } else {
                hi
            };
            let mut ind = lo;
            while ind < content_hi
                && (bytes.get(ind) == Some(&b' ') || bytes.get(ind) == Some(&b'\t'))
            {
                ind = ind.saturating_add(1);
            }
            TransactionSpec::new()
                .changes(Changes::delete(ind..content_hi))
                .selection(Selection::caret(ind))
        }
        Operator::Indent => indent_range(state, lo, hi, false),
        Operator::Outdent => indent_range(state, lo, hi, true),
    }
}

fn indent_range(state: &EditorState, lo: usize, hi: usize, outdent: bool) -> TransactionSpec {
    // Walk the lines in [lo, hi]; for each, either prepend "    "
    // or strip up to four leading spaces / one tab.
    let s = state.doc.to_string();
    let bytes = s.as_bytes();
    let mut changes = Vec::new();
    let mut p = crate::motions::line_start(state, lo);
    // A linewise range's `hi` includes the trailing newline —
    // don't let that pull the *next* line into the indent.
    let limit = if hi > lo { hi.saturating_sub(1) } else { hi };
    while p <= limit && p < bytes.len() {
        if outdent {
            let mut strip: usize = 0;
            for i in 0..4 {
                let idx = p.saturating_add(i);
                if idx < bytes.len() && bytes.get(idx) == Some(&b' ') {
                    strip = strip.saturating_add(1);
                } else if i == 0 && p < bytes.len() && bytes.get(p) == Some(&b'\t') {
                    strip = 1;
                    break;
                } else {
                    break;
                }
            }
            if strip > 0 {
                changes.push(editor_state::Change::delete(p..p.saturating_add(strip)));
            }
        } else {
            changes.push(editor_state::Change::insert(p, "    "));
        }
        // jump to next line
        match bytes
            .get(p..)
            .unwrap_or(&[])
            .iter()
            .position(|&b| b == b'\n')
        {
            Some(i) => p = p.saturating_add(i).saturating_add(1),
            None => break,
        }
    }
    TransactionSpec::new()
        .changes(Changes::from_sorted(changes))
        .selection(Selection::caret(crate::motions::line_first_nonblank(
            state, lo,
        )))
}
