# Extraction record

The editor stack was split out of `FastTrackStudios/task` in August 2026,
following the pattern of the `architect` / `daw` / `keyflow` splits.

## Why

The editor is not a Task feature. Task, Keyflow and Session all embed it,
and a productivity app is a strange place to host a text editor that three
products depend on.

The concrete trigger: the Keyflow site wanted the editor in read-only mode
to render its guide. That would have made `keyflow → task`, while `task →
keyflow` already existed via `editor-keyflow`. Cargo would have tolerated
it — different packages, so no package cycle — but the graph would have
carried **two copies of `keyflow-text` and `engraver`** (one from the path
dep, one from the tag `task` pins), which is bloat today and a hard type
error the first time a `keyflow::Chart` crosses that boundary.

Moving the editor *below* keyflow removes the cycle rather than working
around it.

## Allocation

Computed over the workspace dependency graph:

```
UPWARD edges (editor -> rest of task):   0
DOWNWARD edges (task -> editor):         8, across 5 crates
```

Zero upward edges — the stack was already self-contained. Its only
non-crates.io dependencies are `dioxus`, `dioxus-native` and `dioxus-test`.

**Moved (11):** `editor`, `editor-state`, `editor-view`, `editor-vim`,
`editor-syntax`, `editor-crdt`, `editor-lsp`, `editor-typst`,
`editor-mermaid`, `mermaid-rs-renderer`, `playground`. Plus the docs and
the playwright suite.

**Did not move:** `editor-keyflow` and `editor-keyflow-lang`. They depend
on `keyflow-text` and `engraver`, so they belong in the keyflow repo —
`editor-keyflow` registers itself into this repo's fence registry, and
`editor-keyflow-lang` builds on `editor-state`. Both edges point *down*
from keyflow into editor, which is what keeps the graph acyclic.

## The pre-flight change

`editor-state` hard-depended on `editor-keyflow`. That single edge put the
whole stack above the notation domain and had to go first.

It was two call sites, both string-in string-out — `render_svg` and
`highlight_html` for the ```` ```kf ```` fence family — so the fix was a
seam, not a rewrite. See `editor_state::fence_renderer`, and the commit
`refactor(editor): render keyflow fences through a registry, not a
dependency` in task's history.

## Paths

`git filter-repo` carried history and then flattened the layout:

| was | is |
|---|---|
| `features/editor/<crate>` | `crates/<crate>` |
| `features/editor/playground` | `apps/playground` |
| `features/editor/tests` | `tests/browser` |
| `features/editor/docs` | `docs/` |

Only 7 commits survived the filter. That is not a mistake: `task` is
itself a recent split, and the editor arrived there as a bulk import on
2026-07-10. The deeper history lives in the repo it came from.

## What the scaffold had to fix

- **`playground`'s relative path dep.** `editor-lsp = { path =
  "../editor-lsp" }` resolved to `apps/editor-lsp` after the move. Now a
  workspace dep, which is what it should have been.
- **The blitz `[patch.crates-io]` entries.** Dropped on the first pass;
  `dioxus-test` needs `Node::outer_html_pretty`, a post-beta.1 API that
  only the FTS blitz fork has, so the snapshot tests would not compile.
  task's `styx-format` / `phon` / `phon-jit` / `facet-core` patches are
  deliberately *not* carried — none resolves into this graph.
- **Profile overrides that match nothing are an error**, so task's
  `phon` / `blake3` entries were dropped.

## Verification

From a clean clone, tagged deps only, no `[patch]` override:

```
cargo check --workspace     green
cargo fmt --all --check     green
cargo nextest run --workspace
    682 tests: 681 passed, 1 failed
```

The one failure —
`mermaid-rs-renderer … dense_flowchart_keeps_mid_span_edge_reasonably_direct`
— fails identically in `task` at the commit this was split from.
