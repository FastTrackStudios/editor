#[test]
fn dump_playground_html() {
    let src = include_str!("playground_doc.md");
    let html = editor_state::html::render_markdown_html(src);
    std::fs::write("/tmp/editor_render.html", &html).unwrap();
    for pat in [
        "md-callout",
        "md-task",
        "md-table",
        "<table",
        "md-typst",
        "md-mermaid",
        "md-embed",
        "md-footnote",
        "md-tag",
        "md-block-ref",
        "md-highlight",
        "md-strike",
        "md-code",
        "md-wikilink",
    ] {
        println!("EDITOR {pat:16} {}", html.matches(pat).count());
    }
}
