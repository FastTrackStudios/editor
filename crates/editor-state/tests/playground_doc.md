---
title: Editor playground
tags: [editor, demo, live-preview]
published: true
draft: false
author: cody
created: 2026-05-20
priority: 3
description: |
  Multi-line YAML scalar — used to demo block-
  scalar parsing and editing in the Properties
  panel above.
aliases:
  - playground
  - demo doc
---
# Welcome to the Editor playground

Setext-style heading too
========================

Subtitle via dashes
-------------------

## Inline styles

**bold**, *italic*, ***bold italic***, ~~strikethrough~~, \
                 ==highlight==, `inline code`, and an inline footnote^[click the marker to edit me].

Links: standard [Anthropic](https://anthropic.com), \
                 an autolink <https://obsidian.md>, \
                 wikilinks: [[Editor Roadmap]], [[Project README|the readme]], \
                 and to a header [[Editor Roadmap#Goals]]. Unresolved targets render red \
                 (no vault yet, so every wikilink is unresolved). \
                 Tags like #editor #live-preview #notes/howto, \
                 and a footnote ref [^1]. \
                 Block id at end of paragraph ^demo-block-id

## Block styles

> Blockquotes are just blockquotes.
> Multi-line works too.

- Unordered list item
- Another item

1. Ordered list
2. Stays numbered

- [ ] Click the checkbox to toggle
- [x] Done
- [/] In progress (custom Tasks-plugin status)
- [>] Forwarded
- [-] Cancelled

### Callouts (all 13 types)

> [!note] Note
> Callouts share the blockquote syntax.

> [!tip] Tip
> Press `/` anywhere to open the slash-command menu.

> [!warning]+ Collapsible warning
> The `+`/`-` on the type marker controls folded default.

> [!danger] Danger
> High-stakes call-out style.

> [!info] Info
> Use the slash menu `/callout` to insert any of the others — \
                 abstract, info, success, question, failure, bug, example, quote.

> [!example] Nested callouts
> The outer is an example callout.
> > [!warning] Two levels deep
> > The body inherits the inner kind.
> > > [!danger] Three levels
> > > Rare in practice but supported.
> Back to the outer level.

### Table

| Feature             | Status        | Notes                          |
|---------------------|---------------|--------------------------------|
| Headings (1-6)      | ✅ Mod-1..6   | Mod-0 strips                   |
| Tables              | ✅            | GFM pipe form                  |
| Math (inline+block) | ✅ Typst      | Compiled per-pass via cache    |
| Mermaid             | ✅ pure Rust  | mermaid-rs-renderer            |
| Frontmatter         | ✅ editable   | bool/number/date/list/text     |
| Vim                 | ✅ default-on | C/D/Y / gg / gu/gU/g~ / */#/n  |
| Slash menu          | ✅ `/` opens  | `/callout`, `/typst`, …        |

### Math

Inline math compiles via Typst: $E = m c^2$, and a longer one — \
                 $sum_(i=1)^n i = n(n+1)/2$.

$$ integral_0^1 x^2 d x = 1/3 $$

### Typst block

```typst
= Typst block heading

Full Typst documents render in-place.

$ A = mat(1, 2; 3, 4) $
```

### Mermaid diagram

```mermaid
flowchart LR
  A[Keystroke] --> B{Live preview}
  B -->|markdown| C[Decorations]
  B -->|math| D[Typst SVG]
  B -->|diagram| E[Mermaid SVG]
  C --> F[DOM patch]
  D --> F
  E --> F
```

### Editor commands

- **Mod-B** / **Mod-I** — bold / italic
- **Mod-K** — wrap as `[…](url)`
- **Mod-L** — cycle list marker (none → `-` → `1.` → `- [ ]`)
- **Mod-T** — toggle task on current line
- **Mod-1**..**Mod-6** — heading levels; **Mod-0** strips
- **Mod-E** — toggle reading mode
- **`/`** — open the slash-command palette

### Embeds

Wikilink embed: `![[diagram.png|320]]` (renders an `<img>` when the file resolves).

### Block IDs (Logseq-style references)

This is a referenceable block — press Mod-Shift-K on any block to give it an id.
id:: 5f9c1234-abcd-4ef0-8123-fedcba012345

You can reference it inline like this: ((5f9c1234-abcd-4ef0-8123-fedcba012345)).

Or embed the whole block as a card:

{{embed ((5f9c1234-abcd-4ef0-8123-fedcba012345))}}

### Page + section embeds (Obsidian-style)

Embed a whole page (placeholder until multi-file lookup):

![[Project README]]

Embed a section by heading — resolves intra-doc if the heading lives in this file:

![[#Math]]

Embed a block by Obsidian short-id (the `^demo-block-id` anchor near the top):

![[#^demo-block-id]]

### CommonMark basics

Markdown image: ![a kitten](https://placekitten.com/80/60 "A title")
and a sized one: ![kitten|60](https://placekitten.com/80/60)

Escapes stay literal: \*not emphasis\* and \`not code\`.

Reference links: [the spec][cm] and the shortcut form [cm].
An undefined one stays text: [nope][missing].

A hard break ends this line with two spaces,  
so this continues as a new line in the same paragraph.

Emoji shortcodes: :rocket: :musical_note: :+1: — but a time
range like 10:30 - 11:00 is left alone.

Nested lists:

- top level
  - one deep
    - two deep
1. ordered top
  1. ordered nested

A list inside a quote:

> - quoted item
> - another

An indented code block (four spaces):

    let x = 1;
    println!("{x}");

### Aligned table

| Left | Centre | Right |
|:-----|:------:|------:|
| a    | b      | c     |
| 1    | 2      | 3     |

### Collapsible callouts

> [!tip]+ Starts open
> The chevron folds it.

> [!danger]- Starts folded
> Hidden until you open it.

> [!question]
> A bare callout titles itself from its type.

[cm]: https://commonmark.org "CommonMark"

### Code fences (syntax highlighting)

```rust
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
```

```python
def greet(name):
    return f"Hello, {name}!"
```

```ts
const greet = (name: string) => `Hello, ${name}!`;
```

Comments like %% this %% hide on focus-away.

---

[^1]: Footnote definitions live at the bottom of the file.

Markers stay visible while your caret is on the span — move away and they fade out.