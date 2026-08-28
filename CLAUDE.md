# editor — Repo Instructions

**This repo is an embeddable text/markdown editor.** It was split out of
`FastTrackStudios/task` in August 2026.

```
architect → editor → keyflow → { task, session, signal }
```

| repo | relationship |
|---|---|
| **editor** (here) | the editor stack: state, view, vim, syntax, crdt, lsp, typst, mermaid |
| [keyflow](https://github.com/FastTrackStudios/keyflow) | consumes this; owns `editor-keyflow`, which registers the chart fence renderer |
| [task](https://github.com/FastTrackStudios/task) | consumes this (notes, the vault) |
| [session](https://github.com/FastTrackStudios/session) | consumes this |

## The rule that matters

**Nothing here may depend on anything above this layer.** The whole stack
depends on `dioxus` and crates.io and nothing else — no framework, no RPC
layer, no product, no music engraver. That is not incidental; it is why
three different products can embed it.

Before adding a dependency, ask whether it is *below* the editor. If it is
a capability the editor renders but does not own — a diagram language, a
chart format, a spreadsheet — it goes through a registry instead.

### The fence-renderer seam

`editor_state::fence_renderer` is the mechanism. `editor-state` declares
what the markdown pass needs from a fence language (render to SVG,
highlight to HTML); the host registers an implementation at startup.

`editor-typst` and `editor-mermaid` are ordinary dependencies — they are
at this layer. Keyflow is not: `editor-keyflow` needs `keyflow-text` and
`engraver`, so it lives in the keyflow repo and registers itself. That one
edge is what this split removed; do not put it back.

Testing across the seam: `editor-state` tests the plumbing against a stub
renderer (see `markdown::tests::StubCharts`) — it cannot reach a real
chart engine. The end-to-end test lives with the renderer, in the keyflow
repo.

## Layout

```
crates/
  editor              the facade — re-exports the common surface
  editor-state        document model, transactions, the markdown pass,
                      decorations, the fence registry
  editor-view         the Dioxus component
  editor-vim          vim bindings
  editor-syntax       tree-sitter highlighting (arborium)
  editor-crdt         collaborative editing
  editor-lsp          language-server client (native only — spawns a child)
  editor-typst        Typst math fragments
  editor-mermaid      Mermaid diagrams
  mermaid-rs-renderer vendored mermaid layout/render (web_time for wasm)
apps/playground       the dogfooding app — the fastest way to see a change
tests/browser         playwright suite, driven against the playground
```

## Build

```bash
nix develop        # or direnv: `use flake`
just check
just rust-test
just ci
just play          # the playground
just test          # playwright
```

### Known-failing test on a clean clone

`mermaid-rs-renderer … dense_flowchart_keeps_mid_span_edge_reasonably_direct`
is a layout-quality assertion that also fails in `task` at the commit this
was split from. It is not split damage. Fix the layout, not the assertion.

## Licence

MIT OR Apache-2.0, inherited from task. Deliberately permissive: this is
the one part of the fleet most plausibly published to crates.io, and GPL
would make it unusable to most of that audience.
