//! The plain (non-vim) caret is painted wherever it can be: before a
//! character, at the end of a line, on an empty line and at the end of the
//! document. A host without vim (Keyflow's editor in architect's lab) only
//! ever shows this caret, so a position where it is not painted leaves the
//! person typing blind. Each case renders the editor and counts caret-blue
//! pixels in the picture.
#![cfg(feature = "native")]

mod common;
use common::*;

const THEME: &str = "
:root { --background:#ffffff; --muted:#f2f4f7; --foreground:#1a1c20;
        --muted-foreground:#6b7280; --primary:#1d4ed8; }
.cm-line { font-size: 22px; line-height: 1.6; }
";

fn caret_pixels(path: &std::path::Path) -> usize {
    let file = std::fs::File::open(path).expect("open png");
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let len = usize::try_from(info.buffer_size()).expect("size");
    buf[..len]
        .chunks_exact(4)
        .filter(|px| {
            let (r, g, b) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]));
            b - r >= 60 && b > g + 30
        })
        .count()
}

async fn painted(name: &str, text: &'static str, caret: usize) -> usize {
    let t = mount(Setup::text(text).caret(caret).theme(THEME));
    t.pump().await.ok();
    let dir = std::env::temp_dir().join("caret_paints");
    std::fs::create_dir_all(&dir).expect("output dir");
    let path = dir.join(format!("{name}.png"));
    t.render_png(&path);
    caret_pixels(&path)
}

#[tokio::test(flavor = "current_thread")]
async fn plain_caret_paints_everywhere() {
    let cases: [(&str, &'static str, usize); 4] = [
        ("before_a_character", "C F G C\nAm\n", 2),
        ("end_of_a_line", "C F G C\nAm\n", 7),
        ("empty_line", "C F G C\n\nAm\n", 8),
        ("end_of_document", "C F G C", 7),
    ];
    let mut blind = Vec::new();
    for (name, text, caret) in cases {
        let n = painted(name, text, caret).await;
        // A 1.5px bar a line tall is a few dozen pixels.
        if n < 10 {
            blind.push(format!("{name}: {n} caret pixels"));
        }
    }
    assert!(blind.is_empty(), "caret not painted: {blind:?}");
}
