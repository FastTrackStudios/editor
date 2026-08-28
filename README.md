# Editor

A Rust-native text editor for Dioxus, designed around the architectural ideas
of [CodeMirror 6](https://codemirror.net/) — without being a verbatim port.

## What this is

A small library of crates that provide:

- A pure-Rust document + transaction model (`editor-state`)
- A Dioxus `<Editor>` component backed by `contenteditable` with a Rust-owned
  selection and decoration system (`editor-view`)
- An umbrella crate (`editor`) that re-exports the common surface

## Where this sits

```
        architect
            │
        ▶ editor ◀        this repo — dioxus + crates.io, nothing else
            │
         keyflow
            │
    task / session / signal
```

Split out of [task](https://github.com/FastTrackStudios/task) in August
2026. It is its own repo because it is **embeddable**: the whole stack
depends on `dioxus` and crates.io and nothing more — no framework, no RPC
layer, no product. Task, Keyflow and Session all embed it, and a
dependency on any one of them would make it useless to the others.

That constraint is the repo's reason for existing, so it is worth stating
plainly: **do not add a dependency on anything above this layer.**

### Fence languages are plugged in, not linked

The markdown pass renders fenced blocks it understands. `editor-typst` and
`editor-mermaid` are ordinary dependencies — they sit at this layer and
travel with the editor.

Anything *above* this layer registers itself instead, through
`editor_state::fence_renderer`:

```rust
editor_state::fence_renderer::register_fence_renderer(
    "kf",
    std::sync::Arc::new(editor_keyflow::Fences),
);
```

`editor-keyflow` (in the [keyflow](https://github.com/FastTrackStudios/keyflow)
repo) is the reference implementation — it renders ```` ```kf ```` chart
fences via the Keyflow parser and Engraver. Nothing registered is not an
error: an unrenderable fence falls back to showing its source, which is
what the editor already does for any language it does not know.

## Build

```bash
nix develop        # or direnv: `use flake`
just check
just rust-test
just ci            # what CI runs, in CI's order
just play          # the dogfooding playground app
just test          # the playwright browser suite
```

One test fails on a clean clone —
`mermaid-rs-renderer … dense_flowchart_keeps_mid_span_edge_reasonably_direct`
— a layout-quality assertion that also fails in `task` at the commit this
was split from. It is not split damage; fix the layout, not the assertion.


## Why CodeMirror 6 as a reference

CM6 is the editor under Obsidian, Replit, JupyterLab, Marimo, and many others.
Its architecture has held up for a reason:

- **Plain-text document, decorations laid on top.** Markdown stays markdown —
  styling, hidden markers, widgets are all overlays, not changes to the
  underlying string. This fits a markdown-file-backed app exactly.
- **Transactions over mutation.** Edits are values you produce and apply, not
  imperative DOM calls. The view is a function of state.
- **Composable extensions.** Decorations, keymaps, and behaviors are
  independent units the user composes; the editor doesn't know markdown or
  vim itself.
- **Position anchors.** Cursors and ranges survive concurrent edits because
  they're tracked through transformation, not snapshotted.

We're not porting CM6. We're taking the same ideas and writing them the
Rust-native way — typed enums where TS uses string tags, ownership where TS
uses shared references, signals where TS uses observers.

## What it's not

- Not a code editor with syntax highlighting (yet)
- Not a port of `lezer` (the incremental parser) — markdown parsing is
  one-shot per block until that becomes a bottleneck
- Not aimed at gigabyte files — block-sized content (≤ a few KB per block)
  is the sweet spot
- Not aimed at non-DOM renderers — `editor-view` targets `dioxus-web` and
  `dioxus-desktop` (webview). `dioxus-native`/blitz would need a different
  view layer.

## Crates

| Crate | What it does |
|---|---|
| `editor-state` | Doc, transactions, selections, decorations, extensions. No DOM. Pure logic + tests. |
| `editor-view`  | `<Editor>` Dioxus component. Renders the doc to a contenteditable, bridges DOM events back to transactions. |
| `editor`       | Umbrella re-export. What downstream apps depend on. |

## CRDT story (planned, not in v1)

The architecture is built so that a CRDT integration can sit *between* the
canonical document and the view. Per-block `LoroText` for content, a tree
CRDT for block structure. The view consumes transactions; whether those
transactions came from local input or a remote peer doesn't matter to it.

## License

MIT.
