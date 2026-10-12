//! Rendering a `Frame` into a 32-bit framebuffer: background, the BifrOSt mark (a vector shape scaled from the
//! panel's short side), the progress bar (geometry from the panel size) and each `Screen`. No allocation in `render`,
//! deterministic output (golden snapshot tests). Owner: task 02.

use crate::Frame;

/// The byte order of a 32-bit pixel in the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOrder {
    /// B, G, R, X in memory (UEFI GOP BlueGreenRedReserved, DRM XRGB8888 little endian).
    Bgrx,
    /// R, G, B, X in memory (UEFI GOP RedGreenBlueReserved, DRM XBGR8888).
    Rgbx,
}

/// Draws `frame` into `out` (`stride` pixels per row, at least `frame.width`, and `frame.height` rows).
pub fn render(frame: &Frame, out: &mut [u32], stride: usize, order: PixelOrder) {
    let _ = (frame, out, stride, order);
    todo!("boot-02")
}

/// The progress bar's rectangle (x, y, w, h) for a panel size, so stages can redraw only the bar.
pub fn bar_rect(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let _ = (width, height);
    todo!("boot-02")
}

/// Draws only the progress bar at `progress` (the incremental update each stage does per tick).
pub fn render_bar(width: u32, height: u32, progress: f32, appearance: crate::Appearance, out: &mut [u32], stride: usize, order: PixelOrder) {
    let _ = (width, height, progress, appearance, out, stride, order);
    todo!("boot-02")
}
