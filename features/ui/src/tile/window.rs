//! Which lines of a document the native renderer builds. The lines that
//! intersect the scroll viewport (plus an overscan margin on each side) are
//! rendered; the lines above and below are stood in for by spacers of their
//! total height, so the scroll container keeps its full height.
//!
//! Pure arithmetic over per-line heights, so it is tested on its own. The
//! heights are measured where the renderer can measure them and estimated
//! where it cannot yet.

use std::ops::Range;

/// The height of a line that has not been measured, in pixels. Close to one
/// body line at the editor's default size. An estimate only: a line's real
/// height replaces it as soon as the line has been rendered.
pub const ESTIMATED_LINE_PX: f64 = 28.0;

/// The lines to render, and the spacer heights either side of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// Index of the first rendered line.
    pub first: usize,
    /// One past the last rendered line.
    pub end: usize,
    /// Total height of the lines above `first`.
    pub top_pad: f64,
    /// Total height of the lines from `end` to the document's last line.
    pub bottom_pad: f64,
}

impl Window {
    /// The rendered lines.
    #[must_use]
    pub fn lines(&self) -> Range<usize> {
        self.first..self.end
    }
}

/// The window for a document whose lines have `heights`, scrolled to
/// `scroll_top` in a viewport `viewport` pixels tall. `overscan` pixels of
/// lines are kept on each side, so a small scroll shows no blank gap.
#[must_use]
pub fn window(heights: &[f64], scroll_top: f64, viewport: f64, overscan: f64) -> Window {
    let n = heights.len();
    if n == 0 {
        return Window {
            first: 0,
            end: 0,
            top_pad: 0.0,
            bottom_pad: 0.0,
        };
    }

    // Each line's top edge, and the document's total height.
    let mut tops = Vec::with_capacity(n + 1);
    let mut y = 0.0;
    for &h in heights {
        tops.push(y);
        y += h;
    }
    tops.push(y);

    let top = (scroll_top - overscan).max(0.0);
    let bottom = scroll_top + viewport + overscan;
    // First line whose bottom edge is below the top of the kept region, and
    // one past the last line whose top edge is above its bottom.
    let first = tops
        .partition_point(|&t| t <= top)
        .saturating_sub(1)
        .min(n - 1);
    let end = tops.partition_point(|&t| t < bottom).clamp(first + 1, n);

    Window {
        first,
        end,
        top_pad: tops[first],
        bottom_pad: y - tops[end],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(w: &Window, heights: &[f64]) -> f64 {
        w.top_pad + heights[w.lines()].iter().sum::<f64>() + w.bottom_pad
    }

    #[test]
    fn empty_document_has_no_lines() {
        let w = window(&[], 0.0, 500.0, 100.0);
        assert_eq!(w.lines(), 0..0);
        assert_eq!((w.top_pad, w.bottom_pad), (0.0, 0.0));
    }

    #[test]
    fn a_short_document_renders_every_line() {
        let h = vec![20.0; 5];
        let w = window(&h, 0.0, 500.0, 0.0);
        assert_eq!(w.lines(), 0..5);
        assert_eq!((w.top_pad, w.bottom_pad), (0.0, 0.0));
    }

    #[test]
    fn a_long_document_renders_only_the_viewport() {
        let h = vec![20.0; 1000]; // 20_000px
        let w = window(&h, 10_000.0, 400.0, 0.0);
        // Lines 500..520 cover 10_000..10_400.
        assert_eq!(w.lines(), 500..520);
        assert_eq!(w.top_pad, 10_000.0);
        assert_eq!(w.bottom_pad, 20_000.0 - 10_400.0);
    }

    #[test]
    fn spacers_keep_the_full_document_height() {
        let h: Vec<f64> = (0..300).map(|i| f64::from(i % 7) * 5.0 + 20.0).collect();
        let full: f64 = h.iter().sum();
        for scroll in [0.0, 333.0, 2_000.0, 9_000.0] {
            let w = window(&h, scroll, 600.0, 120.0);
            assert!((total(&w, &h) - full).abs() < 1e-9, "scroll {scroll}");
        }
    }

    #[test]
    fn overscan_keeps_lines_just_outside_the_viewport() {
        let h = vec![20.0; 1000];
        let tight = window(&h, 10_000.0, 400.0, 0.0);
        let loose = window(&h, 10_000.0, 400.0, 100.0);
        assert!(loose.first < tight.first);
        assert!(loose.end > tight.end);
    }

    #[test]
    fn scrolled_past_the_end_still_renders_the_last_line() {
        let h = vec![20.0; 10];
        let w = window(&h, 50_000.0, 400.0, 0.0);
        assert!(!w.lines().is_empty());
        assert_eq!(w.end, 10);
        assert!((total(&w, &h) - 200.0).abs() < 1e-9);
    }
}
