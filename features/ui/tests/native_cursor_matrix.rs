//! The caret, everywhere, on the native (Blitz) path.
//!
//! Each case mounts a document with the caret at one position, in one vim
//! mode, and checks two things:
//!
//! 1. The DOM has a caret element (a bar, a block, an underscore, or an
//!    empty-cell widget).
//! 2. The rendered pixels contain caret paint. This is the check that
//!    matters: a caret can exist in the DOM and still not show, because
//!    Blitz gives an inline span no layout box (issue #15).
//!
//! The pixel check is over the whole window, not a box. `DocumentTester`
//! reports an element's position relative to its nearest positioned
//! ancestor (the `.cm-line`), so a box taken from it is not in window
//! coordinates. A strict match on the caret colour is specific enough that
//! nothing else in these documents is counted.
//!
//! Every case writes its PNG to `$TMPDIR/caret/<name>.png`, so a failure can
//! be looked at, not just read.
#![cfg(feature = "native")]

mod common;
use common::*;

/// Sets `--primary`, the caret colour the editor reads (`#1d4ed8`).
const THEME: &str = "
:root { --background:#ffffff; --muted:#f2f4f7; --foreground:#1a1c20;
        --muted-foreground:#6b7280; --primary:#1d4ed8; }
.cm-line { font-size: 22px; line-height: 1.6; }
";

/// Every element a caret can be painted as.
const CARET_SELECTOR: &str =
    ".ed-native-caret, .ed-modal-caret-block, .ed-modal-caret-underscore, .ed-modal-caret-empty";

/// Caret paint must cover at least this many window pixels.
const MIN_CARET_PIXELS: usize = 6;

struct Case {
    name: &'static str,
    setup: Setup,
    /// Key presses after mount (vim mode changes, etc.).
    keys: Vec<&'static str>,
    /// Whether the pixel check applies. The underscore caret is a bottom
    /// edge (`box-shadow`); it is checked by DOM only.
    pixels: bool,
}

fn case(name: &'static str, setup: Setup, keys: &[&'static str], pixels: bool) -> Case {
    Case {
        name,
        setup,
        keys: keys.to_vec(),
        pixels,
    }
}

/// A paragraph long enough to wrap at the test window width (leaked once,
/// because `Setup` holds `&'static str`).
fn long_line() -> &'static str {
    Box::leak("word ".repeat(40).into_boxed_str())
}

/// Every position a user can end up with the caret.
fn cases() -> Vec<Case> {
    vec![
        // ── Plain editing (insert bar) ─────────────────────────────────
        case("insert_empty_doc", Setup::text("").caret(0), &[], true),
        case(
            "insert_start_of_doc",
            Setup::text("hello").caret(0),
            &[],
            true,
        ),
        case(
            "insert_end_of_doc",
            Setup::text("hello").caret(5),
            &[],
            true,
        ),
        case(
            "insert_blank_line_between_paragraphs",
            Setup::text("hello\n\nworld").caret(6),
            &[],
            true,
        ),
        case(
            "insert_trailing_empty_line",
            Setup::text("hello\n").caret(6),
            &[],
            true,
        ),
        case(
            "insert_whitespace_only_line",
            Setup::text("a\n   \nb").caret(4),
            &[],
            true,
        ),
        // ── Inside markdown spans (markers revealed around the caret) ──
        case(
            "insert_inside_bold",
            Setup::text("x **bold** y").caret(6).markdown(),
            &[],
            true,
        ),
        case(
            "insert_before_closing_bold_marker",
            Setup::text("x **bold** y").caret(8).markdown(),
            &[],
            true,
        ),
        case(
            "insert_inside_italic",
            Setup::text("*it* and").caret(2).markdown(),
            &[],
            true,
        ),
        case(
            "insert_inside_inline_code",
            Setup::text("`code` after").caret(2).markdown(),
            &[],
            true,
        ),
        case(
            "insert_inside_link_text",
            Setup::text("[link](https://x.y) z").caret(2).markdown(),
            &[],
            true,
        ),
        // ── Block contexts ─────────────────────────────────────────────
        case(
            "insert_heading_end",
            Setup::text("# Title").caret(7).markdown(),
            &[],
            true,
        ),
        case(
            "insert_list_item_end",
            Setup::text("- item").caret(6).markdown(),
            &[],
            true,
        ),
        case(
            "insert_task_line_end",
            Setup::text("- [ ] task").caret(10).markdown(),
            &[],
            true,
        ),
        case(
            "insert_callout_body_end",
            Setup::text("> [!tip] T\n> body").caret(17).markdown(),
            &[],
            true,
        ),
        case(
            "insert_code_fence_line_end",
            Setup::text("```rust\nfn x() {}\n```").caret(17).markdown(),
            &[],
            true,
        ),
        // ── Vim normal: block caret, or the empty-cell widget ──────────
        case(
            "normal_on_char",
            Setup::text("hello").caret(2).vim(),
            &[],
            true,
        ),
        case(
            "normal_end_of_line",
            Setup::text("hello\nworld").caret(5).vim(),
            &[],
            true,
        ),
        case(
            "normal_empty_line",
            Setup::text("a\n\nb").caret(2).vim(),
            &[],
            true,
        ),
        case(
            "normal_end_of_doc",
            Setup::text("hello").caret(5).vim(),
            &[],
            true,
        ),
        case(
            "normal_inside_bold",
            Setup::text("x **bold** y").caret(6).vim().markdown(),
            &[],
            true,
        ),
        // ── Widgets and inline syntax the caret can land in ────────────
        case(
            "insert_table_cell",
            Setup::text("| a | b |\n|---|---|\n| c | d |")
                .caret(27)
                .markdown(),
            &[],
            true,
        ),
        case(
            "insert_frontmatter_value",
            Setup::text("---\ntitle: x\n---\nbody").caret(9).markdown(),
            &[],
            true,
        ),
        case(
            "insert_task_text_start",
            Setup::text("- [ ] task").caret(6).markdown(),
            &[],
            true,
        ),
        case(
            "insert_done_task_end",
            Setup::text("- [x] done").caret(10).markdown(),
            &[],
            true,
        ),
        case(
            "insert_wikilink_inside",
            Setup::text("see [[Note]] now").caret(7).markdown(),
            &[],
            true,
        ),
        case(
            "insert_tag_inside",
            Setup::text("a #tag b").caret(4).markdown(),
            &[],
            true,
        ),
        case(
            "insert_callout_header_end",
            Setup::text("> [!note] Title").caret(14).markdown(),
            &[],
            true,
        ),
        case(
            "insert_math_source",
            Setup::text("$x^2$ and").caret(2).markdown(),
            &[],
            true,
        ),
        case(
            "insert_empty_line_in_fence",
            Setup::text("```\n\n```").caret(4).markdown(),
            &[],
            true,
        ),
        case(
            "insert_hard_break_second_line",
            Setup::text("a  \nb").caret(4).markdown(),
            &[],
            true,
        ),
        case(
            "insert_emoji_shortcode",
            Setup::text(":rocket:").caret(4).markdown(),
            &[],
            true,
        ),
        case(
            "insert_wrapped_line_end",
            Setup::text(long_line()).caret(200).markdown(),
            &[],
            true,
        ),
        case(
            "normal_table_cell",
            Setup::text("| a | b |\n|---|---|\n| c | d |")
                .caret(26)
                .vim()
                .markdown(),
            &[],
            true,
        ),
        // ── Vim visual and replace ─────────────────────────────────────
        case(
            "visual_char",
            Setup::text("hello").caret(0).vim(),
            &["v", "l"],
            true,
        ),
        case(
            "visual_line",
            Setup::text("hello\nworld").caret(0).vim(),
            &["V"],
            true,
        ),
        case(
            "replace_mode",
            Setup::text("hello").caret(0).vim(),
            &["R"],
            false,
        ),
    ]
}

/// Decoded RGBA8 image: width, height, pixels.
struct Rgba {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

fn load_png(path: &std::path::Path) -> Rgba {
    let file = std::fs::File::open(path).expect("open png");
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    Rgba {
        w: usize::try_from(info.width).expect("width"),
        h: usize::try_from(info.height).expect("height"),
        px: buf,
    }
}

/// Pixels within a tolerance of the caret colour `#1d4ed8`, anywhere in the
/// window. Text is near-black or grey and links are a lighter blue, so this
/// counts only caret paint, including the anti-aliased edge of a bar.
fn caret_pixels(img: &Rgba) -> usize {
    img.px
        .chunks_exact(4)
        .take(img.w * img.h)
        .filter(|p| {
            let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
            (r - 29).abs() <= 40 && (g - 78).abs() <= 40 && (b - 216).abs() <= 40
        })
        .count()
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[tokio::test]
async fn caret_is_painted_in_every_position() {
    let dir = std::env::temp_dir().join("caret");
    std::fs::create_dir_all(&dir).expect("caret output dir");

    let mut failures = Vec::new();
    for c in cases() {
        let name = c.name;
        let t = mount(c.setup.clone().theme(THEME));
        press(&t, &c.keys);
        t.pump().await.ok();

        // Render first, so a failing case still leaves its picture behind.
        let path = dir.join(format!("{}.png", slug(name)));
        t.render_png(&path);
        if t.query_all(CARET_SELECTOR).immediately().is_empty() {
            failures.push(format!("{name}: no caret element in the DOM"));
            continue;
        }
        if !c.pixels {
            continue;
        }
        let n = caret_pixels(&load_png(&path));
        println!("caret {name}: {n} px");
        if n < MIN_CARET_PIXELS {
            failures.push(format!(
                "{name}: {n} caret pixels painted (need {MIN_CARET_PIXELS})"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} caret case(s) failed; PNGs in {}:\n  {}",
        failures.len(),
        dir.display(),
        failures.join("\n  ")
    );
}
