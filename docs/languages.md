# Languages and the code editor

How a host adds a language to the editor, and how `CodeEditor` uses it.
The editor depends on nothing above it, so a language comes from the
host: an app, or an extension that the app installs.

## A language: a grammar and a highlights query

`editor-syntax` highlights in two ways. Both produce the same tokens.

- `Lang`: the grammars compiled into the crate (Rust, TypeScript,
  JavaScript, Python, JSON, Bash, Markdown, each a cargo feature).
  `highlight(lang, src)`.
- `HostLanguage`: a grammar the host supplies plus the text of a
  highlights query. `HostLanguage::new(name, grammar, highlights)`
  compiles the query once and returns a `LanguageError` when the
  grammar's ABI is not one this tree-sitter runs, or when the query does
  not compile. `highlight_with(&lang, src)` runs it.

`Syntax` holds either kind (`Syntax::Builtin(Lang)` or
`Syntax::Host(Arc<HostLanguage>)`), and the rest of the editor takes a
`Syntax`.

A token's tag comes from the query's capture name through
`arborium_theme::tag_for_capture` (`@keyword` → `k`, `@string` → `s`), and
the view styles it as `md-tok-<tag>`. A query that uses the standard
capture names looks right in every theme without CSS of its own.

The grammar is a `Grammar` (tree-sitter's `Language`). Any grammar crate's
`LanguageFn` converts to one with `.into()`, for example
`tree_sitter_toml_ng::LANGUAGE` or `arborium_toml::language()`.
`Lang::grammar()` returns a built-in one, so a host can run a built-in
grammar with its own query.

### Not supported: grammars loaded from `.wasm`

A grammar is linked into the host binary; nothing here loads one from a
`.wasm` file at run time. Loading needs tree-sitter's `wasm` feature
(`WasmStore`, `Parser::set_wasm_store`, built on `wasmtime-c-api`).
`arborium-tree-sitter` 2.18 includes that module's source
(`binding_rust/wasm_language.rs`, behind `#[cfg(feature = "wasm")]`), but
its manifest declares no `wasm` feature and no `wasmtime-c-api`
dependency, so the module can never be compiled in. The upstream
`tree-sitter` crate's `wasm` feature uses a `Language` type that does not
interchange with arborium's. Its grammars also run in wasmtime, which
does not build for `wasm32-unknown-unknown`, and the editor runs there
too. A host that wants `grammar = "x.wasm"` either links that grammar in
or does without highlighting for it.

## Installing a language: `editor_state::languages`

A process-wide registry with the same shape as `fence_renderer`, plus
withdrawal, because a host disables extensions while it runs:

```rust
use editor_state::languages::{self, HostLanguage, LanguageEntry};

let toml = HostLanguage::new("toml", grammar, highlights)?;
languages::register_language(
    LanguageEntry::new("toml", toml).fences(["toml"]).extensions(["toml"]),
);
// later, when the extension is disabled:
languages::withdraw_language("toml");
```

- `language_for_fence(tag)`, `language_for_extension(ext)`,
  `language_for_path(path)` and `language_by_id(id)` look in the
  registry first and the built-in table second, so a registered language
  can also take over a built-in tag. When two registered languages claim
  the same tag, the one registered last wins.
- The markdown pass highlights a fence through `language_for_fence`.
  Its token cache is keyed by the `Syntax`'s identity, so tokens from a
  withdrawn language are never served again.
- `languages_revision()` changes on every register and withdraw.
  Decorations are computed when the editor renders. After a change, a
  host re-renders the markdown editors that show affected fences (for
  example by bumping their state signal). A `CodeEditor` re-renders when
  its `language` prop changes.

## `CodeEditor`

`editor_view::CodeEditor` (re-exported by the `editor` facade) edits a
source file. It is the same component as `Editor`, on the same
`Signal<EditorState>`, on Blitz and on the web. What it gets is different:

- decorations from `editor_state::code::decorations`: the whole buffer
  highlighted in its language, a line number per line (gutter markers,
  `code-ln`), the bracket pair at the caret (`md-bracket-match`), and
  diagnostics;
- `editor_state::code::keymap()`: Enter keeps the line's indent and adds
  one level after an opening bracket (`{|}` becomes three lines), Tab
  indents (with a selection, every line it touches), Shift-Tab outdents,
  plus undo, redo and select all;
- no markdown pass.

```rust
rsx! {
    CodeEditor {
        state,
        language: editor_state::languages::language_for_path("src/main.rs"),
        diagnostics: diagnostics(),   // Vec<Diagnostic>
    }
}
```

`language` and `diagnostics` are reactive: pass a new value and the
buffer is decorated again. `line_numbers` (default `true`) is read when
the component mounts. The host's `keymap` runs before the code keys. Its
`decorations` (an extension's marks, gutter markers and widgets, see
[extensions.md](extensions.md)) are drawn with the code ones. `history`,
`on_key`, `mounts`, `hover`, `completion`, `vim` and `on_transaction`
pass through to `Editor`. `TextEditor` is `Editor` under the name that
pairs with `CodeEditor`.

### Diagnostics

`editor_state::diagnostic::{Diagnostic, Severity}` are plain data: a byte
range, a severity, a message, and optionally a source and a code.
`editor-lsp` re-exports the same types, so what
`editor_lsp::diagnostics::resolve` returns can be handed straight to
`CodeEditor`. Each diagnostic is drawn as an underline
(`cm-lsp-diagnostic cm-lsp-<severity>`, with the message as its title),
and its line's number is marked with the worst severity on that line
(`code-ln-error`, `code-ln-warning`, …). With `line_numbers: false`,
only lines that have a diagnostic get a marker, a dot.

### Not built yet

- Folding. `EditorState::folds` and `commands::toggle_fold` exist, but
  the code editor has no fold ranges or gutter arrows.
- Find and replace. Vim's `/` search works when `vim` is passed. There
  is no find bar.
- Multiple cursors. `Selection` holds several ranges, but the view
  draws and edits the primary one only.
- Incremental parsing. Every edit parses the whole buffer again; the
  last result is cached per thread, so a pass over unchanged text does
  not parse.
- Completion, hover and go-to-definition from a language server. The
  `completion` and `hover` props are where a host plugs them in, and
  `editor-lsp` is the client. Starting and supervising servers is the
  host's job and is not wired here.
