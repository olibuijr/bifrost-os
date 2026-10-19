//! Texture atlas packing (SpriteKit's texture atlases) as a pure model: named image rectangles are placed into one sheet
//! with shelf packing. Images are placed tallest first, left to right along a shelf, and a new shelf starts below when
//! the next image does not fit the row. Each frame keeps a gap of `padding` pixels to its right and below, so filtered
//! sampling does not bleed one frame into the next.
//!
//! The atlas gives each frame its pixel rectangle and its texture coordinates. Uploading the sheet to the GPU and
//! sampling it is the renderer's work.

/// A frame placed on the sheet, in pixels. `x` and `y` are its top left corner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// A packed sheet. `width` and `height` are the bounds of the frames, not the maximum size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// The frames in the order they were given.
    pub frames: Vec<Frame>,
}

/// Why a set of images does not pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    /// The image has no area, or is wider or taller than the largest sheet side.
    TooLarge(String),
    /// Two images have the same name.
    Duplicate(String),
    /// The images do not fit in a sheet of the largest side.
    Overflow(String),
}

impl Atlas {
    /// The frame with `name`.
    pub fn frame(&self, name: &str) -> Option<&Frame> {
        self.frames.iter().find(|f| f.name == name)
    }

    /// The texture coordinates of the frame with `name` as `[u0, v0, u1, v1]`, in 0..=1 across the sheet.
    pub fn uv(&self, name: &str) -> Option<[f32; 4]> {
        let f = self.frame(name)?;
        let (w, h) = (self.width as f32, self.height as f32);
        Some([f.x as f32 / w, f.y as f32 / h, (f.x + f.w) as f32 / w, (f.y + f.h) as f32 / h])
    }
}

/// Packs the images `(name, width, height)` into one sheet no larger than `max_side` on either side. `padding` pixels
/// are left after each frame.
pub fn pack(images: &[(&str, u32, u32)], max_side: u32, padding: u32) -> Result<Atlas, PackError> {
    for &(name, w, h) in images {
        if w == 0 || h == 0 || w > max_side || h > max_side {
            return Err(PackError::TooLarge(name.to_string()));
        }
    }
    let mut names: Vec<&str> = images.iter().map(|i| i.0).collect();
    names.sort_unstable();
    if let Some(dup) = names.windows(2).find(|p| p[0] == p[1]) {
        return Err(PackError::Duplicate(dup[0].to_string()));
    }

    // tallest first, then widest, then by name, so the same set always packs the same way
    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by(|&a, &b| {
        let (ia, ib) = (images[a], images[b]);
        ib.2.cmp(&ia.2).then(ib.1.cmp(&ia.1)).then(ia.0.cmp(ib.0))
    });

    let (mut x, mut y, mut shelf_h) = (0u32, 0u32, 0u32);
    let mut placed: Vec<(usize, Frame)> = Vec::with_capacity(images.len());
    for i in order {
        let (name, w, h) = images[i];
        if x.saturating_add(w) > max_side {
            y = y.saturating_add(shelf_h).saturating_add(padding);
            x = 0;
            shelf_h = 0;
        }
        if y.saturating_add(h) > max_side {
            return Err(PackError::Overflow(name.to_string()));
        }
        placed.push((i, Frame { name: name.to_string(), x, y, w, h }));
        x = x.saturating_add(w).saturating_add(padding);
        shelf_h = shelf_h.max(h);
    }

    placed.sort_by_key(|p| p.0);
    let frames: Vec<Frame> = placed.into_iter().map(|p| p.1).collect();
    let width = frames.iter().map(|f| f.x + f.w).max().unwrap_or(0);
    let height = frames.iter().map(|f| f.y + f.h).max().unwrap_or(0);
    Ok(Atlas { width, height, frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_images_make_an_empty_sheet() {
        let a = pack(&[], 1024, 2).unwrap();
        assert_eq!((a.width, a.height), (0, 0));
        assert!(a.frames.is_empty());
    }

    #[test]
    fn small_images_share_one_shelf() {
        let a = pack(&[("a", 10, 10), ("b", 20, 10)], 100, 0).unwrap();
        assert_eq!(a.height, 10);
        assert_eq!(a.width, 30);
        let xs: Vec<u32> = a.frames.iter().map(|f| f.x).collect();
        assert_eq!(xs, vec![20, 0], "frames come back in the order given");
    }

    #[test]
    fn a_full_row_starts_a_new_shelf_below() {
        let a = pack(&[("a", 60, 10), ("b", 60, 10)], 100, 0).unwrap();
        let b = a.frame("b").unwrap();
        assert_eq!((b.x, b.y), (0, 10));
        assert_eq!(a.height, 20);
    }

    #[test]
    fn padding_separates_frames() {
        let a = pack(&[("a", 10, 10), ("b", 10, 10)], 100, 4).unwrap();
        let b = a.frame("b").unwrap();
        assert_eq!((b.x, b.y), (14, 0));
    }

    #[test]
    fn tallest_images_go_first_on_each_shelf() {
        let a = pack(&[("short", 10, 5), ("tall", 10, 20)], 100, 0).unwrap();
        assert_eq!((a.frame("tall").unwrap().x, a.frame("tall").unwrap().y), (0, 0));
        assert_eq!((a.frame("short").unwrap().x, a.frame("short").unwrap().y), (10, 0));
    }

    #[test]
    fn an_image_too_large_for_the_sheet_is_refused() {
        assert_eq!(pack(&[("big", 200, 10)], 100, 0), Err(PackError::TooLarge("big".into())));
        assert_eq!(pack(&[("empty", 0, 10)], 100, 0), Err(PackError::TooLarge("empty".into())));
    }

    #[test]
    fn duplicate_names_are_refused() {
        assert_eq!(pack(&[("a", 1, 1), ("a", 2, 2)], 100, 0), Err(PackError::Duplicate("a".into())));
    }

    #[test]
    fn images_that_do_not_fit_overflow() {
        let r = pack(&[("a", 100, 60), ("b", 100, 60)], 100, 0);
        assert_eq!(r, Err(PackError::Overflow("b".into())));
    }

    #[test]
    fn uv_coordinates_span_the_sheet() {
        let a = pack(&[("a", 10, 10), ("b", 10, 10)], 100, 0).unwrap();
        assert_eq!(a.uv("a"), Some([0.0, 0.0, 0.5, 1.0]));
        assert_eq!(a.uv("b"), Some([0.5, 0.0, 1.0, 1.0]));
        assert_eq!(a.uv("missing"), None);
    }
}
