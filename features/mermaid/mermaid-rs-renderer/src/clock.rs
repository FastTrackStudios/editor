//! A stopwatch that works in a browser.
//!
//! The layout pipeline records per-stage timings, and they are
//! diagnostics — nothing about the diagram depends on them. But
//! `Instant::now()` traps on `wasm32-unknown-unknown`, and a trap in a
//! diagnostic takes the whole editor down with it: rendering one mermaid
//! fence panicked the wasm module, and the editor's pane went blank with
//! no diagram, no error and no rest of the document.
//!
//! `web-time` is supposed to cover this, and this crate depends on it,
//! but the symbol that panicked was `std::time::Instant::now` called
//! from `build_routed_edges` — so on this target its browser backend was
//! not the one compiled in. Rather than depend on that resolving the way
//! it is documented to, the timing is simply not taken on wasm. A
//! diagnostic is the wrong thing to risk a blank page for.

/// Elapsed microseconds since [`Stopwatch::start`], or zero on wasm.
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
}

impl Stopwatch {
    #[must_use]
    pub fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
        }
    }

    /// Microseconds elapsed. Always `0` on wasm, where no clock is read.
    #[must_use]
    pub fn elapsed_us(&self) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_micros()
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }
}
