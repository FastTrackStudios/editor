//! Screenshot extension — rasterize the headless document to a PNG.
//!
//! dioxus-test lays a component out through blitz-dom but never paints
//! pixels. This adds a CPU rasterizer (the same `anyrender_vello_cpu`
//! path blitz's own screenshot tests use) so a test can dump exactly
//! what the component looks like — invaluable for visual bugs (caret
//! styling, decoration layout) that HTML assertions can't catch.

use std::path::Path;

use anyrender::{PaintScene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::Document as _;
use blitz_paint::paint_scene;
use peniko::{Fill, color::palette, kurbo::Rect};

use crate::DocumentTester;

impl DocumentTester {
    /// Render the current document state to an RGBA8 PNG at `path`,
    /// sized to the tester's window. Paints on an opaque white
    /// background so anti-aliased text reads correctly.
    pub fn render_png(&self, path: impl AsRef<Path>) {
        let mut doc = self.document.borrow_mut();
        // Apply any pending vdom mutations and re-resolve layout so the PNG
        // reflects the CURRENT state — e.g. after key events changed the
        // doc or the caret decoration since `build()`.
        while doc.poll(None) {}
        doc.inner_mut().resolve(1.0);
        let (w, h, scale) = {
            let inner = doc.inner();
            let vp = inner.viewport();
            (vp.window_size.0, vp.window_size.1, vp.scale_f64())
        };

        let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene: &mut <VelloCpuImageRenderer as anyrender::ImageRenderer>::ScenePainter<'_>| {
                scene.fill(
                    Fill::NonZero,
                    Default::default(),
                    palette::css::WHITE,
                    Default::default(),
                    &Rect::new(0.0, 0.0, w as f64, h as f64),
                );
                let mut inner = doc.inner_mut();
                paint_scene(scene, &mut inner, scale, w, h, 0, 0);
            },
            w,
            h,
        );

        let file = std::fs::File::create(path.as_ref())
            .unwrap_or_else(|e| panic!("create png {}: {e}", path.as_ref().display()));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("png header")
            .write_image_data(&buffer)
            .expect("png data");
    }
}
