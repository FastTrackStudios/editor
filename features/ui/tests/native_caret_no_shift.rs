//! Moving the caret must move only the caret. The text itself must not shift
//! sideways or change line height. Each case renders one document twice, with
//! the caret at two different positions, and compares the pictures pixel by
//! pixel. The caret's own paint (the caret colour, and a small margin around
//! it) is masked out; everything else must match exactly.
//!
//! Plain text only. Revealing markdown markers around a caret inside bold or
//! a link changes the text on purpose, so those cases are not compared here.
#![cfg(feature = "native")]

mod common;
use common::*;

const THEME: &str = "
:root { --background:#ffffff; --muted:#f2f4f7; --foreground:#1a1c20;
        --muted-foreground:#6b7280; --primary:#1d4ed8; }
.cm-line { font-size: 22px; line-height: 1.6; }
";

/// How far around a caret pixel to ignore, in pixels. Covers the caret's
/// anti-aliased edge, and nothing wider: a real shift of the next letter is
/// several pixels and must not hide here.
const MARGIN: usize = 2;

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

/// The editor's text region, in window pixels.
const TEXT_LEFT: usize = 60;
const TEXT_BOTTOM: usize = 150;

/// Is this pixel caret paint, including its anti-aliased edge? Caret paint is
/// blue (`#1d4ed8` blended with the background). Black and grey text is never
/// blue, so this does not mask text.
fn is_caret(px: &[u8]) -> bool {
    let (r, g, b) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]));
    b - r >= 25 && b > g
}

/// Render `text` with the caret at `caret`, in insert (bar) or normal (block)
/// mode, and return the picture.
async fn render(name: &str, text: &'static str, caret: usize, vim: bool) -> Rgba {
    let setup = if vim {
        Setup::text(text).caret(caret).vim()
    } else {
        Setup::text(text).caret(caret)
    };
    let t = mount(setup.theme(THEME));
    t.pump().await.ok();
    let dir = std::env::temp_dir().join("caret_no_shift");
    std::fs::create_dir_all(&dir).expect("output dir");
    let path = dir.join(format!("{name}.png"));
    t.render_png(&path);
    load_png(&path)
}

/// Count pixels that differ between two pictures, ignoring any pixel within
/// `MARGIN` of a caret pixel in either picture. Returns the count and the
/// first differing position, if any.
fn text_differences(a: &Rgba, b: &Rgba) -> (usize, Option<(usize, usize)>) {
    assert_eq!(
        (a.w, a.h),
        (b.w, b.h),
        "both pictures must be the same size"
    );
    let (w, h) = (a.w, a.h);

    // Mask: every pixel within MARGIN of a caret pixel in either picture.
    let mut masked = vec![false; w * h];
    for (img, _) in [(a, 0), (b, 1)] {
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                if !is_caret(&img.px[i..i + 4]) {
                    continue;
                }
                for my in y.saturating_sub(MARGIN)..(y + MARGIN + 1).min(h) {
                    for mx in x.saturating_sub(MARGIN)..(x + MARGIN + 1).min(w) {
                        masked[my * w + mx] = true;
                    }
                }
            }
        }
    }

    let mut count = 0;
    let mut first = None;
    // The editor's text: right of the gutter, above the debug probes that
    // print the caret offset under the editor (they change on purpose).
    for y in 0..h.min(TEXT_BOTTOM) {
        for x in TEXT_LEFT..w {
            if masked[y * w + x] {
                continue;
            }
            let i = (y * w + x) * 4;
            if a.px[i..i + 3] != b.px[i..i + 3] {
                count += 1;
                first.get_or_insert((x, y));
            }
        }
    }
    (count, first)
}

/// Assert that the text is identical with the caret at `from` and at `to`.
async fn assert_text_stays_put(name: &str, text: &'static str, from: usize, to: usize, vim: bool) {
    let a = render(&format!("{name}_from"), text, from, vim).await;
    let b = render(&format!("{name}_to"), text, to, vim).await;
    let (n, first) = text_differences(&a, &b);
    assert_eq!(
        n,
        0,
        "{name}: moving the caret {from} -> {to} changed {n} text pixels (first at {first:?}); \
         pictures in {}",
        std::env::temp_dir().join("caret_no_shift").display()
    );
}

#[tokio::test]
async fn insert_bar_moves_without_shifting_text() {
    assert_text_stays_put("insert_same_line", "hello world", 0, 5, false).await;
    assert_text_stays_put("insert_to_end", "hello world", 0, 11, false).await;
}

#[tokio::test]
async fn insert_bar_moves_between_lines_without_shifting_text() {
    assert_text_stays_put("insert_next_line", "hello\nworld", 0, 8, false).await;
}

#[tokio::test]
async fn block_caret_moves_without_shifting_text() {
    assert_text_stays_put("normal_on_char", "hello world", 0, 4, true).await;
}

#[tokio::test]
async fn block_caret_moves_to_end_of_line_without_shifting_text() {
    assert_text_stays_put("normal_end_of_line", "hello\nworld", 2, 5, true).await;
}

#[tokio::test]
async fn caret_on_blank_line_does_not_shift_text_below_it() {
    assert_text_stays_put("normal_blank_line", "a\n\nb", 0, 2, true).await;
}

/// Control: the same caret rendered twice must give the same pixels, or the
/// comparison above proves nothing.
#[tokio::test]
async fn same_caret_renders_identically() {
    let a = render("control_a", "hello world", 3, false).await;
    let b = render("control_b", "hello world", 3, false).await;
    let (n, first) = text_differences(&a, &b);
    assert_eq!(
        n, 0,
        "identical state rendered differently: {n} pixels, first at {first:?}"
    );
}
