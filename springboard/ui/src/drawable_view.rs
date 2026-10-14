//! A drawing view as a pure model (a reference-SDK GL view): it owns a drawable, runs the delegate's draw inside a display pass,
//! tracks whether the view needs a new pass, and reports a snapshot of the last pass. The drawable rules come from
//! `drawable.rs`. The model creates no context and no GPU object; a renderer runs the delegate and reads back the pixels.
//!
//! Rules: a display pass needs a valid drawable with a non-zero size. A snapshot needs at least one completed pass and
//! describes the size of that pass, not the current size, so a resize after the last pass does not change it.

use crate::drawable::{ColorFormat, Drawable, DrawableError};

/// Rows in a snapshot start on a multiple of this many bytes.
pub const ROW_ALIGNMENT: usize = 4;

/// Why a view cannot run a pass or report a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// The drawable configuration is invalid.
    Drawable(DrawableError),
    /// The drawable has no size yet.
    NoSize,
    /// No display pass has completed, so there is nothing to snapshot.
    NothingDrawn,
}

impl From<DrawableError> for ViewError {
    fn from(error: DrawableError) -> Self {
        ViewError::Drawable(error)
    }
}

/// The layout of a snapshot of the last completed pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    /// Bytes from the start of one row to the start of the next, a multiple of `ROW_ALIGNMENT`.
    pub bytes_per_row: usize,
    /// The bytes the snapshot takes in total.
    pub byte_len: usize,
}

/// The size and colour format of the drawable at the last completed pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PassSize {
    width: u32,
    height: u32,
    color: ColorFormat,
}

/// A view with a drawable and the bookkeeping of its display passes.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawableView {
    drawable: Drawable,
    needs_display: bool,
    passes: u64,
    last_pass: Option<PassSize>,
}

impl DrawableView {
    /// A new view. It needs a display pass before it has drawn anything.
    pub fn new(drawable: Drawable) -> Self {
        DrawableView { drawable, needs_display: true, passes: 0, last_pass: None }
    }

    pub fn drawable(&self) -> &Drawable {
        &self.drawable
    }

    /// Changes the drawable size and asks for a new pass.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.drawable.resize(width, height);
        self.needs_display = true;
    }

    /// Asks for a new display pass on the next update.
    pub fn set_needs_display(&mut self) {
        self.needs_display = true;
    }

    pub fn needs_display(&self) -> bool {
        self.needs_display
    }

    /// How many display passes have completed.
    pub fn passes(&self) -> u64 {
        self.passes
    }

    /// Runs one display pass: checks the drawable, hands it to `draw`, then records the pass. A failed check leaves the
    /// view unchanged and `draw` is not called. Returns the pass number.
    pub fn display_with(&mut self, draw: impl FnOnce(&Drawable)) -> Result<u64, ViewError> {
        self.drawable.validate()?;
        if self.drawable.width == 0 || self.drawable.height == 0 {
            return Err(ViewError::NoSize);
        }
        draw(&self.drawable);
        self.passes += 1;
        self.needs_display = false;
        self.last_pass = Some(PassSize {
            width: self.drawable.width,
            height: self.drawable.height,
            color: self.drawable.color,
        });
        Ok(self.passes)
    }

    /// The layout of the last completed pass as a snapshot. Rows are padded to `ROW_ALIGNMENT` bytes.
    pub fn snapshot(&self) -> Result<Snapshot, ViewError> {
        let pass = self.last_pass.ok_or(ViewError::NothingDrawn)?;
        if pass.color == ColorFormat::None {
            return Err(ViewError::Drawable(DrawableError::NoColorBuffer));
        }
        let unpadded = pass.width as usize * pass.color.bytes_per_sample() as usize;
        let bytes_per_row = unpadded.div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT;
        Ok(Snapshot {
            width: pass.width,
            height: pass.height,
            bytes_per_row,
            byte_len: bytes_per_row * pass.height as usize,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sized(color: ColorFormat, w: u32, h: u32) -> DrawableView {
        let mut d = Drawable::new(color);
        d.resize(w, h);
        DrawableView::new(d)
    }

    #[test]
    fn a_new_view_needs_a_pass_and_has_no_snapshot() {
        let v = sized(ColorFormat::Rgba8, 4, 4);
        assert!(v.needs_display());
        assert_eq!(v.snapshot(), Err(ViewError::NothingDrawn));
    }

    #[test]
    fn a_pass_calls_the_delegate_once_and_clears_the_flag() {
        let mut v = sized(ColorFormat::Rgba8, 4, 4);
        let mut calls = 0;
        let pass = v.display_with(|d| {
            calls += 1;
            assert_eq!(d.width, 4);
        });
        assert_eq!(pass, Ok(1));
        assert_eq!(calls, 1);
        assert!(!v.needs_display());
        v.set_needs_display();
        assert!(v.needs_display());
    }

    #[test]
    fn a_zero_size_view_refuses_a_pass_without_calling_the_delegate() {
        let mut v = DrawableView::new(Drawable::new(ColorFormat::Rgba8));
        let mut called = false;
        assert_eq!(v.display_with(|_| called = true), Err(ViewError::NoSize));
        assert!(!called);
        assert_eq!(v.passes(), 0);
    }

    #[test]
    fn an_invalid_drawable_is_refused() {
        let mut v = sized(ColorFormat::None, 4, 4);
        assert_eq!(v.display_with(|_| {}), Err(ViewError::Drawable(DrawableError::NoColorBuffer)));
    }

    #[test]
    fn snapshot_rows_are_padded_to_four_bytes() {
        // 16-bit colour, 3 pixels wide: 6 bytes, padded to 8. Two rows, so 16 bytes.
        let mut v = sized(ColorFormat::Rgb565, 3, 2);
        v.display_with(|_| {}).unwrap();
        let s = v.snapshot().unwrap();
        assert_eq!((s.bytes_per_row, s.byte_len), (8, 16));
    }

    #[test]
    fn a_resize_after_a_pass_does_not_change_the_snapshot() {
        let mut v = sized(ColorFormat::Rgba8, 2, 2);
        v.display_with(|_| {}).unwrap();
        v.resize(100, 100);
        let s = v.snapshot().unwrap();
        assert_eq!((s.width, s.height, s.bytes_per_row), (2, 2, 8));
        assert!(v.needs_display());
    }

    #[test]
    fn passes_are_counted() {
        let mut v = sized(ColorFormat::Bgra8, 1, 1);
        assert_eq!(v.display_with(|_| {}), Ok(1));
        assert_eq!(v.display_with(|_| {}), Ok(2));
        assert_eq!(v.passes(), 2);
    }
}
