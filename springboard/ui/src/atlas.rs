//! Texture atlas: named rectangles inside one sheet image, so many sprites can share one texture. The model holds the
//! sheet size and the frame of each name in pixels (origin at the top-left, as image files are), and gives the same
//! frame as texture coordinates in 0 to 1 for the renderer.
//!
//! Pure: it loads no image and uploads nothing. Frames must lie inside the sheet and have a non-zero size. Names are
//! unique and not empty.

use std::collections::BTreeMap;

/// A rectangle inside the sheet, in pixels from the top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A frame as texture coordinates: `u0`, `v0` is the top-left corner, `u1`, `v1` the bottom-right, all in 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uv {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasError {
    /// The sheet has a zero width or height.
    EmptySheet,
    /// The name is empty.
    EmptyName,
    /// A frame with this name already exists.
    DuplicateName,
    /// The frame has a zero width or height.
    EmptyFrame,
    /// The frame does not lie inside the sheet.
    OutOfBounds,
}

/// A sheet image and the frames cut from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    width: u32,
    height: u32,
    frames: BTreeMap<String, Frame>,
}

impl Atlas {
    /// An empty atlas for a sheet of `width` by `height` pixels.
    pub fn new(width: u32, height: u32) -> Result<Self, AtlasError> {
        if width == 0 || height == 0 {
            return Err(AtlasError::EmptySheet);
        }
        Ok(Self {
            width,
            height,
            frames: BTreeMap::new(),
        })
    }

    /// Adds a named frame. The frame must be non-empty, inside the sheet, and its name must be new.
    pub fn add(&mut self, name: &str, frame: Frame) -> Result<(), AtlasError> {
        if name.is_empty() {
            return Err(AtlasError::EmptyName);
        }
        if frame.width == 0 || frame.height == 0 {
            return Err(AtlasError::EmptyFrame);
        }
        let right = u64::from(frame.x) + u64::from(frame.width);
        let bottom = u64::from(frame.y) + u64::from(frame.height);
        if right > u64::from(self.width) || bottom > u64::from(self.height) {
            return Err(AtlasError::OutOfBounds);
        }
        if self.frames.contains_key(name) {
            return Err(AtlasError::DuplicateName);
        }
        self.frames.insert(name.to_string(), frame);
        Ok(())
    }

    /// The pixel frame of a name.
    pub fn frame(&self, name: &str) -> Option<Frame> {
        self.frames.get(name).copied()
    }

    /// The texture coordinates of a name.
    pub fn uv(&self, name: &str) -> Option<Uv> {
        let f = self.frames.get(name)?;
        let w = self.width as f32;
        let h = self.height as f32;
        Some(Uv {
            u0: f.x as f32 / w,
            v0: f.y as f32 / h,
            u1: (f.x + f.width) as f32 / w,
            v1: (f.y + f.height) as f32 / h,
        })
    }

    /// Every frame name, sorted.
    pub fn names(&self) -> Vec<&str> {
        self.frames.keys().map(String::as_str).collect()
    }

    /// The number of frames.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether the atlas has no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: u32, y: u32, width: u32, height: u32) -> Frame {
        Frame { x, y, width, height }
    }

    #[test]
    fn a_zero_sized_sheet_is_refused() {
        assert_eq!(Atlas::new(0, 64), Err(AtlasError::EmptySheet));
        assert_eq!(Atlas::new(64, 0), Err(AtlasError::EmptySheet));
    }

    #[test]
    fn uv_coordinates_are_the_pixel_frame_over_the_sheet_size() {
        let mut a = Atlas::new(256, 128).unwrap();
        a.add("coin", frame(64, 32, 64, 32)).unwrap();
        let uv = a.uv("coin").unwrap();
        assert_eq!(uv.u0, 0.25);
        assert_eq!(uv.v0, 0.25);
        assert_eq!(uv.u1, 0.5);
        assert_eq!(uv.v1, 0.5);
    }

    #[test]
    fn frames_outside_the_sheet_are_refused() {
        let mut a = Atlas::new(100, 100).unwrap();
        assert_eq!(a.add("edge", frame(90, 0, 11, 10)), Err(AtlasError::OutOfBounds));
        assert_eq!(a.add("bottom", frame(0, 95, 10, 6)), Err(AtlasError::OutOfBounds));
        assert!(a.add("corner", frame(90, 90, 10, 10)).is_ok());
    }

    #[test]
    fn empty_names_frames_and_duplicates_are_refused() {
        let mut a = Atlas::new(10, 10).unwrap();
        assert_eq!(a.add("", frame(0, 0, 1, 1)), Err(AtlasError::EmptyName));
        assert_eq!(a.add("zero", frame(0, 0, 0, 1)), Err(AtlasError::EmptyFrame));
        a.add("one", frame(0, 0, 1, 1)).unwrap();
        assert_eq!(a.add("one", frame(1, 1, 1, 1)), Err(AtlasError::DuplicateName));
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn names_are_sorted_and_unknown_names_return_none() {
        let mut a = Atlas::new(10, 10).unwrap();
        a.add("walk-2", frame(0, 0, 1, 1)).unwrap();
        a.add("idle", frame(1, 0, 1, 1)).unwrap();
        assert_eq!(a.names(), vec!["idle", "walk-2"]);
        assert!(a.frame("jump").is_none());
        assert!(a.uv("jump").is_none());
        assert!(!a.is_empty());
    }
}
