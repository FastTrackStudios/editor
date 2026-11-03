//! A wrapped callout body must hang from the same left edge on every row.
//!
//! The `> ` marker is kept as transparent text, and before the fix it took
//! its width on the first row only, so the first row of a wrapped callout
//! started to the right of the rows below it. This test measures the painted
//! text, row by row, so it sees what a reader sees.
#![cfg(feature = "native")]

mod common;
use common::*;

const THEME: &str = "
:root { --background:#ffffff; --muted:#f2f4f7; --foreground:#1a1c20;
        --muted-foreground:#6b7280; --primary:#1d4ed8; }
.cm-line { font-size: 22px; line-height: 1.6; }
";

/// A callout whose body wraps at the test window width.
fn callout_text() -> &'static str {
    Box::leak(
        "> [!info] Info\n> Use the slash menu to insert any of the others, and this body is long enough to wrap onto several rows so the indent of each row can be compared."
            .to_string()
            .into_boxed_str(),
    )
}

struct Rgba {
    w: usize,
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
        px: buf,
    }
}

/// A window coordinate as a pixel index, clamped at zero.
fn px(v: f64) -> usize {
    usize::try_from(v.max(0.0).floor() as i64).unwrap_or(0)
}

/// For each text row in the band `[top, bottom)`, the leftmost dark pixel
/// at or right of `from_x`. Rows with no ink are skipped.
fn row_left_edges(img: &Rgba, from_x: usize, top: usize, bottom: usize) -> Vec<usize> {
    let mut edges = Vec::new();
    let mut in_ink_run = false;
    let mut row_left: Option<usize> = None;
    for y in top..bottom {
        let mut left = None;
        for x in from_x..img.w {
            let i = (y * img.w + x) * 4;
            let p = &img.px[i..i + 3];
            // Ink is dark text; the callout's tint and bar are not.
            if u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]) < 300 {
                left = Some(x);
                break;
            }
        }
        match (left, in_ink_run) {
            (Some(x), false) => {
                in_ink_run = true;
                row_left = Some(x);
            }
            (Some(x), true) => {
                row_left = Some(row_left.map_or(x, |r| r.min(x)));
            }
            (None, true) => {
                edges.push(row_left.unwrap_or(from_x));
                in_ink_run = false;
                row_left = None;
            }
            (None, false) => {}
        }
    }
    if in_ink_run {
        edges.push(row_left.unwrap_or(from_x));
    }
    edges
}

#[tokio::test]
async fn wrapped_callout_body_rows_share_one_left_edge() {
    let t = dioxus_test::render(Harness)
        .with_root_context(Setup::text(callout_text()).caret(0).markdown().theme(THEME))
        .with_window_size(960, 600)
        .build();
    t.pump().await.ok();

    // The body is the cm-line after the header. Its box bounds the rows.
    let lines = t.query_all(".cm-line.md-callout").immediately();
    let body = lines
        .iter()
        .find(|l| {
            !l.attribute("class")
                .unwrap_or_default()
                .contains("md-callout-header")
        })
        .expect("a callout body line");
    let (_, bh) = body.size();
    let bx = body.upper_left().page().x;
    let by = body.upper_left().page().y;
    assert!(
        bh > 30.0,
        "the body should wrap to several rows (height {bh})"
    );

    let dir = std::env::temp_dir().join("callout_wrap");
    std::fs::create_dir_all(&dir).expect("output dir");
    let path = dir.join("callout_body.png");
    t.render_png(&path);
    let img = load_png(&path);

    // Measure only the body's box, starting past the bar and its padding.
    let from_x = px(bx + 20.0);
    let top = px(by);
    let bottom = top + px(f64::from(bh));
    let edges = row_left_edges(&img, from_x, top, bottom);

    assert!(
        edges.len() >= 2,
        "expected at least two text rows, got {edges:?}"
    );
    let first = edges[0];
    for (row, e) in edges.iter().enumerate() {
        assert!(
            e.abs_diff(first) <= 2,
            "row {row} starts at x={e}, but the first row starts at x={first} (rows {edges:?}); see {}",
            path.display()
        );
    }
}
