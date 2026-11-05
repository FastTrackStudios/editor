# Extensions: the editor's surface as data

`editor-state`'s `Keymap`, `Transaction` and decorations are Rust values
and fn pointers, usable only by code compiled into the editor. An
extension that runs elsewhere (another process, a Luau or WASI sandbox, a
page, over a socket) needs the same powers as plain values it can send
and receive. Those values are `crates/editor-ext` (package `editor-ext`,
serde and nothing else, so a host that only talks to extensions does not
pull in the document model). Architect's extension host is one consumer
(`architect_extensions::editor`, FastTrackStudios/architect#312).

## The values (`editor-ext`)

| type | what it is |
|---|---|
| `Snapshot` | the live editor: `text`, `selection` (`SelRange`s), `primary`, `cursor`, `node`, `version` |
| `NodeAt` (`node_at`) | the markdown block a position is in: `kind` (`NodeKind`: paragraph, blank, heading, list-item, table, fence, quote), its line, `indent`, `marker`, `content_from`; in a table, the `cell` and every `cells` (the delimiter row left out) |
| `EditCommand` | `changes` (`TextChange {from, to, insert}`, all against the text read, any order, none overlapping), then `selection`, a `name`, a `base` version: **one transaction, one undo step** |
| `KeymapEntry` | a `key` (`KeyChord::parse`: `Tab`, `Shift-Tab`, `Mod-Enter`) naming an `action`, `when` a `Scope` holds (`editor`, `node:list-item`, `node:table`, …); `resolve` picks a node's entry before an editor-wide one |
| `DecorationSpec` | `mark {from, to, class, title?}`, `line {at, class}`, `gutter {at, text, class}`, `widget {at, view, props, block}` |
| `SlashEntry` | a slash-menu row: `label`, `action`, `group`, `desc`, `icon` |

Positions are **byte offsets** into the UTF-8 text, from 0. Every command
and decoration is checked against the text it is for
(`EditCommand::checked`, `DecorationSpec::checked`): a position past the
end or inside a character, a reversed or overlapping change, a stale
`base`, or a class other than letters, digits, `-` and `_` is refused
(`ExtError`), never applied and never a panic. Extensions' classes are
drawn `ext-<class>` (`ext_classes`), so they never reach the editor's own.
`EditCommand::apply_to` applies one to a plain string, for a host with no
editor (tests, a CRDT-backed document).

## On the editor's state (`editor_state::ext`)

`crates/editor-state/src/ext.rs` re-exports `editor-ext` and joins it to
`EditorState`:

- `snapshot(state, version)`;
- `transaction(state, cmd, version, by)`: the command as one
  `TransactionSpec`, tagged `user_event = "extension"` and annotated with
  the extension (`made_by`) and its step's name. `History` records it as
  one group that never coalesces with typing, so one undo takes it back
  whole;
- `resolve_key(entries, press, state)`: the entry a keystroke runs, by
  the node at the cursor;
- `decorations(specs, text)`: the view's `DecoratedRange`s. A gutter
  marker is `DecoratedRange::gutter`; a widget is `DecoratedRange::mount`
  under `mount_id(i)` (its index in `specs`, read back with
  `mount_index`), which the host fills with a view of its own;
- `mapped(specs, changes)`: decorations moved through an edit.

## On the view (`editor-view`)

The `Editor` component takes, for a host that runs extensions:

- `history`: the undo history, the host's. A transaction the host
  applies itself goes through `dispatch_with_history`, so it undoes in
  the same stack as the person's typing;
- `on_key`: asked with every keystroke the editor would handle, after an
  open menu's own keys and before vim and the keymap; `true` takes it.
  Resolve an extension's keymap here;
- `mounts` (`WidgetMounts`): the element for each mounted widget's id,
  drawn by the native renderer (the web renderer draws the empty
  placeholder);
- `on_action`: called with an extension's slash-row action once it is
  picked (`palette::register_actions`, `CommandKind::Action`; pass the
  same to `CommandPalette`).

A Blitz document may have no HTML parser (a headless one has none), so
the native renderer draws gutter markers (`decoration::gutter_of`) and
mounted widgets (`decoration::mount_of`) itself, never from HTML.

## Tests

`crates/editor-ext/src/lib.rs` (nodes, cells, checks, keymaps,
decorations, position mapping), `crates/editor-state/src/ext.rs` (an
extension's edit is one undo step apart from typing; a bad command is
refused; keys by node; specs as decorations and mounts),
`features/ui/tests/native_ext.rs` (on the native editor: Tab in a list
item runs the host's command, Ctrl-Z undoes it in one step; a mark, a
gutter marker and a mounted widget drawn), and `palette`'s
`an_extensions_slash_row_runs_its_action`.
