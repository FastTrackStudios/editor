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

Follows architect's shape: `crates/` is what an embedder reaches for,
`features/` is one capability per directory. A feature is a single crate
at `features/<name>`; it only nests a level deeper when it genuinely
needs more than one crate. Directory names are bare capabilities, package
names keep the `editor-` prefix — so `features/vim` builds `editor-vim`,
and downstream `use editor_vim::…` is unaffected by where it sits.

```
crates/
  editor              the facade — re-exports the common surface
  editor-state        document model, transactions, the markdown pass,
                      decorations, the fence registry
features/
  ui                  editor-view — the Dioxus component
  vim                 editor-vim — vim bindings
  syntax              editor-syntax — tree-sitter highlighting (arborium)
  crdt                editor-crdt — collaborative editing
  lsp                 editor-lsp — language-server client (native only)
  typst               editor-typst — Typst math fragments
  mermaid/            two crates, hence the extra level:
    mermaid           editor-mermaid — the fence adapter
    mermaid-rs-renderer  vendored layout/render (web_time for wasm)
apps/playground       the dogfooding app — the fastest way to see a change
tests/browser         playwright suite, driven against the playground
```

`crates/` never depends on an app; `features/` never depends on
`crates/`. `editor-state` is the one thing features may depend on
upward-looking — it is the model, and it lives in `crates/` because it is
half of what an embedder actually imports.

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
