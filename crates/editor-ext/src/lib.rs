//! The editor's extension surface, as data.
//!
//! `editor-state`'s [`Keymap`], `Transaction` and decorations are Rust
//! values and fn pointers: only code compiled into the editor can use
//! them. An extension that runs in another process, in a sandbox (Luau,
//! a WASI component, a page) or across a socket needs the same powers as
//! plain values it can send and receive. This crate is those values:
//!
//! - a [`Snapshot`] of the live editor: its text, selection, cursor, and
//!   the markdown node the cursor is in ([`NodeAt`]: a list item, a table
//!   cell, a heading…);
//! - an [`EditCommand`]: changes and a selection, applied as **one
//!   transaction** and so one undo step;
//! - a [`KeymapEntry`]: a key that names an action, scoped to the editor
//!   or to a node kind ([`Scope`]);
//! - a [`DecorationSpec`]: a mark, a line or gutter decoration, or a
//!   widget that mounts a view;
//! - a [`SlashEntry`]: a row of the slash menu that names an action.
//!
//! Positions are **byte offsets** into the UTF-8 text, from 0 — the
//! editor's own unit. Every change and decoration is checked against the
//! text it is for ([`EditCommand::checked`], [`DecorationSpec::checked`]),
//! so a sandboxed extension cannot panic the editor with a bad offset.
//!
//! `editor-state` turns these into its own types (`editor_state::ext`);
//! a host that does not run the editor (a test, a CRDT-backed document)
//! can apply an [`EditCommand`] to a string with [`EditCommand::apply_to`].
//!
//! [`Keymap`]: https://docs.rs/editor-state/latest/editor_state/command/struct.Keymap.html

use serde::{Deserialize, Serialize};

/// One selected range: `anchor` stays put, `head` moves. Equal for a
/// caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct SelRange {
    pub anchor: usize,
    pub head: usize,
}

impl SelRange {
    /// A caret at `pos`.
    #[must_use]
    pub const fn caret(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// Its lower end.
    #[must_use]
    pub const fn from(self) -> usize {
        if self.anchor < self.head {
            self.anchor
        } else {
            self.head
        }
    }

    /// Its upper end.
    #[must_use]
    pub const fn to(self) -> usize {
        if self.anchor < self.head {
            self.head
        } else {
            self.anchor
        }
    }
}

/// What kind of markdown block a position is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// Prose, or anything not below.
    #[default]
    Paragraph,
    /// An empty (or whitespace-only) line.
    Blank,
    /// `# …` to `###### …`.
    Heading,
    /// `- …`, `* …`, `+ …`, `1. …`, `1) …`, with or without a task box.
    ListItem,
    /// A row of a pipe table (`| a | b |`).
    Table,
    /// Inside a fenced code block, its fences included.
    Fence,
    /// `> …`.
    Quote,
}

impl NodeKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Paragraph,
        Self::Blank,
        Self::Heading,
        Self::ListItem,
        Self::Table,
        Self::Fence,
        Self::Quote,
    ];

    /// Its name in a [`Scope`] (`list-item`, `table`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Blank => "blank",
            Self::Heading => "heading",
            Self::ListItem => "list-item",
            Self::Table => "table",
            Self::Fence => "fence",
            Self::Quote => "quote",
        }
    }

    /// The kind named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// One cell of a pipe table: its row (the header is 0; the delimiter row
/// is not counted), its column, and the byte range of its content, spaces
/// trimmed (empty content: a caret position inside the cell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub from: usize,
    pub to: usize,
}

/// The block a position is in, from a line-by-line reading of the
/// markdown (the editor's live-preview pass agrees on these kinds).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct NodeAt {
    pub kind: NodeKind,
    /// The line's start, a byte offset.
    pub line_from: usize,
    /// The line's end, before its newline.
    pub line_to: usize,
    /// The line's index, from 0.
    pub line: usize,
    /// The line's leading whitespace, in bytes (a list item's indent).
    pub indent: usize,
    /// A list item's marker (`-`, `1.`), a heading's `#`s, a quote's `>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Where the line's content starts, past its indent and marker.
    pub content_from: usize,
    /// In a table: the cell the position is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<Cell>,
    /// In a table: every cell of it, row by row (the delimiter row left
    /// out), for moving between them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
}

/// One line of `text`: its start, its end (before the newline), its index.
#[derive(Clone, Copy, Debug)]
struct Line {
    from: usize,
    to: usize,
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut from = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(Line { from, to: i });
            from = i.saturating_add(1);
        }
    }
    out.push(Line {
        from,
        to: text.len(),
    });
    out
}

fn line_text(text: &str, l: Line) -> &str {
    text.get(l.from..l.to).unwrap_or("")
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    line.len().saturating_sub(t.len()) <= 3 && (t.starts_with("```") || t.starts_with("~~~"))
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|')
        && t.contains('-')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// The list marker at the start of `rest` (after the indent) and its
/// length with the space after it and any task box.
fn list_marker(rest: &str) -> Option<(&str, usize)> {
    let bytes = rest.as_bytes();
    let first = *bytes.first()?;
    let marker_len = if matches!(first, b'-' | b'*' | b'+') {
        1
    } else {
        let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        match bytes.get(digits) {
            Some(b'.' | b')') => digits.saturating_add(1),
            _ => return None,
        }
    };
    let after = rest.get(marker_len..)?;
    let spaces = after
        .len()
        .saturating_sub(after.trim_start_matches(' ').len());
    if spaces == 0 && !after.is_empty() {
        return None;
    }
    let mut len = marker_len.saturating_add(spaces);
    let tail = rest.get(len..).unwrap_or("");
    if tail.len() >= 3
        && (tail.starts_with("[ ]") || tail.starts_with("[x]") || tail.starts_with("[X]"))
    {
        len = len.saturating_add(3);
        let more = rest.get(len..).unwrap_or("");
        len = len.saturating_add(
            more.len()
                .saturating_sub(more.trim_start_matches(' ').len()),
        );
    }
    Some((rest.get(..marker_len)?, len))
}

/// The cells of one table row starting at byte `from`.
fn row_cells(line: &str, from: usize, row: usize) -> Vec<Cell> {
    let mut pipes = Vec::new();
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '|' => pipes.push(i),
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut col = 0usize;
    for w in pipes.windows(2) {
        let (Some(&a), Some(&b)) = (w.first(), w.get(1)) else {
            continue;
        };
        let start = a.saturating_add(1);
        let raw = line.get(start..b).unwrap_or("");
        let lead = raw.len().saturating_sub(raw.trim_start().len());
        let content = raw.trim();
        let (cf, ct) = if content.is_empty() {
            // An empty cell: a caret one space in, or at its start.
            let at = start.saturating_add(usize::from(!raw.is_empty()));
            (at, at)
        } else {
            let cf = start.saturating_add(lead);
            (cf, cf.saturating_add(content.len()))
        };
        out.push(Cell {
            row,
            col,
            from: from.saturating_add(cf),
            to: from.saturating_add(ct),
        });
        col = col.saturating_add(1);
    }
    out
}

/// The block `pos` is in. `pos` past the end reads as the end.
#[must_use]
pub fn node_at(text: &str, pos: usize) -> NodeAt {
    let pos = pos.min(text.len());
    let all = lines(text);
    let idx = all
        .iter()
        .position(|l| pos >= l.from && pos <= l.to)
        .unwrap_or_else(|| all.len().saturating_sub(1));
    let Some(&line) = all.get(idx) else {
        return NodeAt::default();
    };
    let lt = line_text(text, line);
    let indent = lt.len().saturating_sub(lt.trim_start().len());
    let mut node = NodeAt {
        kind: NodeKind::Paragraph,
        line_from: line.from,
        line_to: line.to,
        line: idx,
        indent,
        marker: None,
        content_from: line.from.saturating_add(indent),
        cell: None,
        cells: Vec::new(),
    };
    // Fences first: whatever a fenced line looks like, it is code.
    let mut open = false;
    for (i, l) in all.iter().enumerate() {
        let fence = is_fence(line_text(text, *l));
        if i == idx {
            if open || fence {
                node.kind = NodeKind::Fence;
                return node;
            }
            break;
        }
        if fence {
            open = !open;
        }
    }
    let rest = lt.get(indent..).unwrap_or("");
    if rest.is_empty() {
        node.kind = NodeKind::Blank;
    } else if is_table_row(lt) {
        node.kind = NodeKind::Table;
        let mut first = idx;
        while first > 0
            && all
                .get(first.saturating_sub(1))
                .is_some_and(|l| is_table_row(line_text(text, *l)))
        {
            first = first.saturating_sub(1);
        }
        let mut row = 0usize;
        let mut i = first;
        while let Some(l) = all.get(i) {
            let t = line_text(text, *l);
            if !is_table_row(t) {
                break;
            }
            if !is_delimiter_row(t) {
                node.cells.extend(row_cells(t, l.from, row));
                row = row.saturating_add(1);
            }
            i = i.saturating_add(1);
        }
        if !is_delimiter_row(lt) {
            let this_row: Vec<Cell> = row_cells(lt, line.from, 0);
            let here = this_row
                .iter()
                .position(|c| pos <= c.to.max(c.from))
                .or_else(|| this_row.len().checked_sub(1));
            if let Some(col) = here {
                node.cell = node
                    .cells
                    .iter()
                    .find(|c| c.from >= line.from && c.to <= line.to && c.col == col)
                    .copied();
            }
        }
    } else if let Some(hashes) = Some(rest.bytes().take_while(|b| *b == b'#').count())
        && (1..=6).contains(&hashes)
        && rest
            .get(hashes..)
            .is_some_and(|r| r.is_empty() || r.starts_with(' '))
    {
        node.kind = NodeKind::Heading;
        node.marker = rest.get(..hashes).map(str::to_owned);
        let after = rest.get(hashes..).unwrap_or("");
        let gap = after.len().saturating_sub(after.trim_start().len());
        node.content_from = node.content_from.saturating_add(hashes).saturating_add(gap);
    } else if let Some((marker, len)) = list_marker(rest) {
        node.kind = NodeKind::ListItem;
        node.marker = Some(marker.to_owned());
        node.content_from = node.content_from.saturating_add(len);
    } else if rest.starts_with('>') {
        node.kind = NodeKind::Quote;
        node.marker = Some(">".to_owned());
        let after = rest.get(1..).unwrap_or("");
        let gap = after.len().saturating_sub(after.trim_start().len());
        node.content_from = node.content_from.saturating_add(1).saturating_add(gap);
    }
    node
}

/// The live editor, as an extension sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Snapshot {
    /// The whole document.
    pub text: String,
    /// Every selected range; one, a caret, most of the time.
    pub selection: Vec<SelRange>,
    /// Which of them is the primary one.
    #[serde(default)]
    pub primary: usize,
    /// The primary range's head.
    pub cursor: usize,
    /// The block the cursor is in.
    pub node: NodeAt,
    /// The document's version, as its host counts: an [`EditCommand`]
    /// whose `base` is not it is refused as stale.
    #[serde(default)]
    pub version: u64,
}

impl Snapshot {
    /// `text` with `selection` (`primary` its primary), at `version`.
    #[must_use]
    pub fn of(text: &str, selection: &[SelRange], primary: usize, version: u64) -> Self {
        let primary = primary.min(selection.len().saturating_sub(1));
        let cursor = selection.get(primary).map_or(0, |r| r.head);
        Self {
            text: text.to_owned(),
            selection: selection.to_vec(),
            primary,
            cursor,
            node: node_at(text, cursor),
            version,
        }
    }
}

/// One change of the text: bytes `from..to` replaced by `insert` (an
/// insertion when they are equal, a deletion when `insert` is empty).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct TextChange {
    pub from: usize,
    pub to: usize,
    #[serde(default)]
    pub insert: String,
}

impl TextChange {
    /// `text` inserted at `at`.
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self {
            from: at,
            to: at,
            insert: text.into(),
        }
    }

    /// Bytes `from..to` deleted.
    #[must_use]
    pub const fn delete(from: usize, to: usize) -> Self {
        Self {
            from,
            to,
            insert: String::new(),
        }
    }

    /// Bytes `from..to` replaced by `text`.
    pub fn replace(from: usize, to: usize, text: impl Into<String>) -> Self {
        Self {
            from,
            to,
            insert: text.into(),
        }
    }
}

/// What an extension does to the text, as one undo step.
///
/// Its changes, all against the text it read, are applied together as
/// **one transaction**. Then the selection: `selection` when given (positions in
/// the text after), else the one there was, moved by the changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct EditCommand {
    #[serde(default)]
    pub changes: Vec<TextChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Vec<SelRange>>,
    /// The undo step's name (`Indent list item`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The [`Snapshot::version`] it was made against: refused when the
    /// document has moved on since. `None`: not checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<u64>,
}

/// Why an [`EditCommand`] or a [`DecorationSpec`] was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtError {
    /// A position past the end of the text.
    OutOfBounds { at: usize, len: usize },
    /// A position inside a character.
    NotCharBoundary { at: usize },
    /// A change whose end is before its start.
    Reversed { from: usize, to: usize },
    /// Two changes over the same bytes.
    Overlap { at: usize },
    /// Made against an older version of the document.
    Stale { base: u64, now: u64 },
    /// A class with something other than letters, digits, `-` and `_`.
    BadClass(String),
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfBounds { at, len } => {
                write!(f, "position {at} is past the end of the text ({len} bytes)")
            }
            Self::NotCharBoundary { at } => write!(f, "position {at} is inside a character"),
            Self::Reversed { from, to } => write!(f, "change {from}..{to} ends before it starts"),
            Self::Overlap { at } => write!(f, "two changes overlap at {at}"),
            Self::Stale { base, now } => write!(
                f,
                "made against version {base}, but the document is at {now}: read it again"
            ),
            Self::BadClass(c) => write!(
                f,
                "class {c:?}: only letters, digits, '-' and '_' (and spaces between classes)"
            ),
        }
    }
}

impl std::error::Error for ExtError {}

const fn check_pos(text: &str, at: usize) -> Result<(), ExtError> {
    if at > text.len() {
        Err(ExtError::OutOfBounds {
            at,
            len: text.len(),
        })
    } else if !text.is_char_boundary(at) {
        Err(ExtError::NotCharBoundary { at })
    } else {
        Ok(())
    }
}

/// Which side a position at a change's edge stays on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    /// Before text inserted at it.
    Before,
    /// After text inserted at it.
    After,
}

/// `pos` in the text after `changes` (sorted, as [`EditCommand::checked`]
/// gives them, all against the same text).
#[must_use]
pub fn map_pos(changes: &[TextChange], pos: usize, assoc: Assoc) -> usize {
    let mut grow = 0usize;
    let mut shrink = 0usize;
    for c in changes {
        if pos < c.from {
            break;
        }
        if pos <= c.to {
            let start = c.from.saturating_add(grow).saturating_sub(shrink);
            let end = start.saturating_add(c.insert.len());
            // The end of a replaced range is after the replacement; an
            // insertion's point, or a point inside, goes by `assoc`.
            return if (pos == c.to && c.from < c.to) || assoc == Assoc::After {
                end
            } else {
                start
            };
        }
        grow = grow.saturating_add(c.insert.len());
        shrink = shrink.saturating_add(c.to.saturating_sub(c.from));
    }
    pos.saturating_add(grow).saturating_sub(shrink)
}

impl EditCommand {
    /// Changes and nothing else.
    #[must_use]
    pub const fn changes(changes: Vec<TextChange>) -> Self {
        Self {
            changes,
            selection: None,
            name: None,
            base: None,
        }
    }

    /// The selection after it.
    #[must_use]
    pub fn select(mut self, selection: Vec<SelRange>) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Its undo step's name.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Its changes, checked against `text` (at `version`) and sorted.
    ///
    /// # Errors
    ///
    /// A stale `base`, a position outside `text` or inside a character, a
    /// reversed change, two changes over the same bytes.
    pub fn checked(&self, text: &str, version: u64) -> Result<Vec<TextChange>, ExtError> {
        if let Some(base) = self.base
            && base != version
        {
            return Err(ExtError::Stale { base, now: version });
        }
        let mut changes = self.changes.clone();
        for c in &changes {
            if c.to < c.from {
                return Err(ExtError::Reversed {
                    from: c.from,
                    to: c.to,
                });
            }
            check_pos(text, c.from)?;
            check_pos(text, c.to)?;
        }
        changes.sort_by_key(|c| (c.from, c.to));
        for w in changes.windows(2) {
            if let (Some(a), Some(b)) = (w.first(), w.get(1))
                && b.from < a.to
            {
                return Err(ExtError::Overlap { at: b.from });
            }
        }
        Ok(changes)
    }

    /// The text after it, and the selection after it, from `text` with
    /// `selection` at `version`: for a host with no editor of its own.
    ///
    /// # Errors
    ///
    /// As [`Self::checked`]; or a selection outside the text after.
    pub fn apply_to(
        &self,
        text: &str,
        selection: &[SelRange],
        version: u64,
    ) -> Result<(String, Vec<SelRange>), ExtError> {
        let changes = self.checked(text, version)?;
        let mut out = String::with_capacity(text.len());
        let mut at = 0usize;
        for c in &changes {
            out.push_str(text.get(at..c.from).unwrap_or(""));
            out.push_str(&c.insert);
            at = c.to;
        }
        out.push_str(text.get(at..).unwrap_or(""));
        let selection = match &self.selection {
            Some(s) => {
                for r in s {
                    check_pos(&out, r.anchor)?;
                    check_pos(&out, r.head)?;
                }
                s.clone()
            }
            None => selection
                .iter()
                .map(|r| SelRange {
                    anchor: map_pos(&changes, r.anchor, Assoc::After),
                    head: map_pos(&changes, r.head, Assoc::After),
                })
                .collect(),
        };
        Ok((out, selection))
    }
}

/// A keystroke, or the shortcut a keymap names: `Tab`, `Shift-Tab`,
/// `Mod-Enter` (`Mod` is Cmd on a Mac, Ctrl elsewhere), `Ctrl-Alt-k`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub r#mod: bool,
}

impl KeyChord {
    /// Reads `Shift-Tab`: modifiers then the key, joined by `-`. Unknown
    /// modifiers are ignored.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let parts: Vec<&str> = text.split('-').collect();
        let last = parts.len().saturating_sub(1);
        for (i, part) in parts.iter().enumerate() {
            if i == last {
                (*part).clone_into(&mut out.key);
            } else {
                match *part {
                    "Mod" => out.r#mod = true,
                    "Ctrl" => out.ctrl = true,
                    "Alt" | "Option" => out.alt = true,
                    "Shift" => out.shift = true,
                    "Cmd" | "Meta" => out.meta = true,
                    _ => {}
                }
            }
        }
        // `Mod--`: the key is `-`.
        if out.key.is_empty() && text.ends_with("--") {
            "-".clone_into(&mut out.key);
        }
        out
    }

    /// Whether `press` (a keystroke, `mod` unset) is this shortcut. A
    /// single character matches either case; `Mod` matches Ctrl or Cmd.
    #[must_use]
    pub fn matches(&self, press: &Self) -> bool {
        let key = if self.key.chars().count() == 1 && press.key.chars().count() == 1 {
            self.key.eq_ignore_ascii_case(&press.key)
        } else {
            self.key == press.key
        };
        let mods = if self.r#mod {
            press.ctrl || press.meta
        } else {
            self.ctrl == press.ctrl && self.meta == press.meta
        };
        key && mods && self.alt == press.alt && self.shift == press.shift
    }
}

/// Where a keymap entry is in force.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Anywhere in the editor.
    #[default]
    Editor,
    /// While the cursor is in a block of this kind.
    Node(NodeKind),
}

impl Scope {
    /// Reads `editor` or `node:<kind>` (`node:list-item`, `node:table`).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "" | "editor" => Some(Self::Editor),
            other => other
                .strip_prefix("node:")
                .and_then(NodeKind::parse)
                .map(Self::Node),
        }
    }

    /// Whether it is in force at `node`.
    #[must_use]
    pub fn holds(self, node: &NodeAt) -> bool {
        match self {
            Self::Editor => true,
            Self::Node(k) => node.kind == k,
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Editor => f.write_str("editor"),
            Self::Node(k) => write!(f, "node:{}", k.name()),
        }
    }
}

impl Serialize for Scope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "scope {text:?}: \"editor\" or \"node:<kind>\" (kind one of {})",
                NodeKind::ALL.map(NodeKind::name).join(", ")
            ))
        })
    }
}

/// A key that runs an action while the editor has the focus: `Tab` in a
/// list item runs `ACME_INDENT`. The action reads the editor and edits it
/// as an extension would ([`Snapshot`], [`EditCommand`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct KeymapEntry {
    /// The shortcut ([`KeyChord::parse`]).
    pub key: String,
    /// The action it runs.
    pub action: String,
    /// Where it is in force (`editor`, `node:list-item`).
    #[serde(default)]
    pub when: Scope,
}

/// The entry `press` runs at `node`: of the entries whose key it is and
/// whose scope holds, a node's before the editor's, then the first.
#[must_use]
pub fn resolve<'a, T: AsRef<KeymapEntry>>(
    entries: &'a [T],
    press: &KeyChord,
    node: &NodeAt,
) -> Option<&'a T> {
    let hits = || {
        entries.iter().filter(|e| {
            let e = e.as_ref();
            e.when.holds(node) && KeyChord::parse(&e.key).matches(press)
        })
    };
    hits()
        .find(|e| matches!(e.as_ref().when, Scope::Node(_)))
        .or_else(|| hits().next())
}

impl AsRef<Self> for KeymapEntry {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// A decoration an extension adds: drawn over the text, never in it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecorationSpec {
    /// Bytes `from..to` drawn with class `ext-<class>` (and `title`, as a
    /// tooltip).
    Mark {
        from: usize,
        to: usize,
        class: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// The line holding `at` gets class `ext-<class>`.
    Line { at: usize, class: String },
    /// A marker in the gutter of the line holding `at`: a short `text`
    /// (`●`, `3`), class `ext-<class>`.
    Gutter {
        at: usize,
        text: String,
        #[serde(default)]
        class: String,
    },
    /// A view mounted at `at` (its own line, when `block`): the
    /// extension's view `view`, given `props`.
    Widget {
        at: usize,
        view: String,
        #[serde(default)]
        props: serde_json::Value,
        #[serde(default)]
        block: bool,
    },
}

fn check_class(class: &str) -> Result<(), ExtError> {
    if class
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '))
    {
        Ok(())
    } else {
        Err(ExtError::BadClass(class.to_owned()))
    }
}

/// `class`'s words, each `ext-`-prefixed, so an extension's classes never
/// reach the editor's own.
#[must_use]
pub fn ext_classes(class: &str) -> String {
    class
        .split_whitespace()
        .map(|c| format!("ext-{c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl DecorationSpec {
    /// Its start.
    #[must_use]
    pub const fn at(&self) -> usize {
        match self {
            Self::Mark { from, .. } => *from,
            Self::Line { at, .. } | Self::Gutter { at, .. } | Self::Widget { at, .. } => *at,
        }
    }

    /// It, checked against `text`.
    ///
    /// # Errors
    ///
    /// A position outside `text` or inside a character, a reversed mark, a
    /// class with other than letters, digits, `-`, `_`.
    pub fn checked(&self, text: &str) -> Result<(), ExtError> {
        match self {
            Self::Mark {
                from, to, class, ..
            } => {
                if to < from {
                    return Err(ExtError::Reversed {
                        from: *from,
                        to: *to,
                    });
                }
                check_pos(text, *from)?;
                check_pos(text, *to)?;
                check_class(class)
            }
            Self::Line { at, class } | Self::Gutter { at, class, .. } => {
                check_pos(text, *at)?;
                check_class(class)
            }
            Self::Widget { at, .. } => check_pos(text, *at),
        }
    }

    /// It in the text after `changes` (sorted, against the text it was
    /// for); `None` for a mark whose text was all deleted.
    #[must_use]
    pub fn mapped(&self, changes: &[TextChange]) -> Option<Self> {
        let mut out = self.clone();
        match &mut out {
            Self::Mark { from, to, .. } => {
                *from = map_pos(changes, *from, Assoc::After);
                *to = map_pos(changes, *to, Assoc::Before);
                if to <= from {
                    return None;
                }
            }
            Self::Line { at, .. } | Self::Gutter { at, .. } | Self::Widget { at, .. } => {
                *at = map_pos(changes, *at, Assoc::Before);
            }
        }
        Some(out)
    }
}

/// A row of the editor's slash menu that runs an action.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct SlashEntry {
    /// What the row says (`Table`).
    pub label: String,
    /// The action it runs, the slash and query typed taken out first.
    pub action: String,
    /// The header it is under (`Insert`); the extension's name if empty.
    #[serde(default)]
    pub group: String,
    /// A line under the label.
    #[serde(default)]
    pub desc: String,
    /// A short badge left of it (`|-|`).
    #[serde(default)]
    pub icon: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_item_and_its_marker() {
        let text = "# Title\n\n- one\n  - [ ] two\n3. three\n> quote\n";
        let n = node_at(text, text.find("two").unwrap());
        assert_eq!(n.kind, NodeKind::ListItem);
        assert_eq!((n.indent, n.marker.as_deref()), (2, Some("-")));
        assert_eq!(n.content_from, text.find("two").unwrap());
        assert_eq!(node_at(text, 2).kind, NodeKind::Heading);
        assert_eq!(node_at(text, 8).kind, NodeKind::Blank);
        let n = node_at(text, text.find("three").unwrap());
        assert_eq!(
            (n.kind, n.marker.as_deref()),
            (NodeKind::ListItem, Some("3."))
        );
        assert_eq!(
            node_at(text, text.find("quote").unwrap()).kind,
            NodeKind::Quote
        );
        // `-x` is not a list item.
        assert_eq!(node_at("-x", 1).kind, NodeKind::Paragraph);
    }

    #[test]
    fn a_table_cell_and_every_cell() {
        let text = "| a | bb |\n|---|----|\n| c |    |\n";
        let at = text.find("bb").unwrap();
        let n = node_at(text, at);
        assert_eq!(n.kind, NodeKind::Table);
        let cell = n.cell.unwrap();
        assert_eq!((cell.row, cell.col, cell.from, cell.to), (0, 1, at, at + 2));
        assert_eq!(n.cells.len(), 4, "{:?}", n.cells);
        let empty = n.cells.get(3).unwrap();
        assert_eq!((empty.row, empty.col, empty.from, empty.to), (1, 1, 28, 28));
        assert_eq!(text.get(empty.from - 1..empty.from), Some(" "));
        let c = node_at(text, text.find('c').unwrap()).cell.unwrap();
        assert_eq!((c.row, c.col), (1, 0));
    }

    #[test]
    fn a_fence_is_code_whatever_it_holds() {
        let text = "```\n- not a list\n```\n- a list\n";
        assert_eq!(node_at(text, 6).kind, NodeKind::Fence);
        assert_eq!(node_at(text, 0).kind, NodeKind::Fence);
        assert_eq!(
            node_at(text, text.rfind("a list").unwrap()).kind,
            NodeKind::ListItem
        );
    }

    #[test]
    fn an_edit_is_checked_then_applied_in_one() {
        let text = "- one\n- two\n";
        let cmd = EditCommand::changes(vec![
            TextChange::insert(6, "  "),
            TextChange::insert(0, "  "),
        ]);
        let (out, sel) = cmd.apply_to(text, &[SelRange::caret(8)], 0).unwrap();
        assert_eq!(out, "  - one\n  - two\n");
        assert_eq!(sel, vec![SelRange::caret(12)]);
        let bad = EditCommand::changes(vec![TextChange::delete(2, 40)]);
        assert!(matches!(
            bad.checked(text, 0),
            Err(ExtError::OutOfBounds { .. })
        ));
        let overlap =
            EditCommand::changes(vec![TextChange::delete(0, 3), TextChange::delete(2, 4)]);
        assert_eq!(overlap.checked(text, 0), Err(ExtError::Overlap { at: 2 }));
        assert!(matches!(
            EditCommand::changes(vec![TextChange::insert(1, "x")]).checked("é", 0),
            Err(ExtError::NotCharBoundary { at: 1 })
        ));
        let stale = EditCommand {
            base: Some(3),
            ..EditCommand::default()
        };
        assert_eq!(
            stale.checked(text, 4),
            Err(ExtError::Stale { base: 3, now: 4 })
        );
    }

    #[test]
    fn keymaps_by_key_and_scope() {
        let entries: Vec<KeymapEntry> = serde_json::from_str(
            r#"[{"key":"Tab","action":"ANY_TAB"},
                {"key":"Tab","action":"LIST_IN","when":"node:list-item"},
                {"key":"Shift-Tab","action":"LIST_OUT","when":"node:list-item"},
                {"key":"Tab","action":"CELL_NEXT","when":"node:table"}]"#,
        )
        .unwrap();
        let tab = KeyChord::parse("Tab");
        let back = KeyChord {
            shift: true,
            ..KeyChord::parse("Tab")
        };
        let list = node_at("- a", 2);
        let table = node_at("| a |", 2);
        let prose = node_at("a", 0);
        let hit = |p: &KeyChord, n: &NodeAt| resolve(&entries, p, n).map(|e| e.action.as_str());
        assert_eq!(hit(&tab, &list), Some("LIST_IN"));
        assert_eq!(hit(&back, &list), Some("LIST_OUT"));
        assert_eq!(hit(&tab, &table), Some("CELL_NEXT"));
        assert_eq!(hit(&tab, &prose), Some("ANY_TAB"));
        assert_eq!(hit(&back, &prose), None);
        assert!(serde_json::from_str::<Scope>(r#""node:nope""#).is_err());
        assert_eq!(
            serde_json::to_string(&Scope::Node(NodeKind::ListItem)).unwrap(),
            r#""node:list-item""#
        );
        let m = KeyChord::parse("Mod-k");
        assert!(m.matches(&KeyChord {
            ctrl: true,
            ..KeyChord::parse("K")
        }));
    }

    #[test]
    fn decorations_are_checked_and_follow_edits() {
        let text = "hello world";
        let mark: DecorationSpec =
            serde_json::from_str(r#"{"kind":"mark","from":6,"to":11,"class":"todo"}"#).unwrap();
        assert!(mark.checked(text).is_ok());
        let bad = DecorationSpec::Line {
            at: 0,
            class: "x\" onclick=".to_owned(),
        };
        assert!(matches!(bad.checked(text), Err(ExtError::BadClass(_))));
        let moved = mark.mapped(&[TextChange::insert(0, ">> ")]).unwrap();
        assert_eq!(moved.at(), 9);
        assert!(mark.mapped(&[TextChange::delete(5, 11)]).is_none());
        let widget: DecorationSpec = serde_json::from_str(
            r#"{"kind":"widget","at":5,"view":"count","props":{"n":2},"block":true}"#,
        )
        .unwrap();
        assert!(widget.checked(text).is_ok());
        assert_eq!(ext_classes("todo hot"), "ext-todo ext-hot");
    }

    #[test]
    fn positions_map_through_changes() {
        let c = [TextChange::replace(2, 4, "xyz")];
        assert_eq!(map_pos(&c, 1, Assoc::After), 1);
        assert_eq!(map_pos(&c, 3, Assoc::After), 5);
        assert_eq!(map_pos(&c, 3, Assoc::Before), 2);
        assert_eq!(map_pos(&c, 6, Assoc::After), 7);
        let ins = [TextChange::insert(2, "ab")];
        assert_eq!(map_pos(&ins, 2, Assoc::Before), 2);
        assert_eq!(map_pos(&ins, 2, Assoc::After), 4);
    }
}
