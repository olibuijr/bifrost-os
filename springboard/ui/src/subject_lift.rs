//! Subject lifting: the model behind an image analysis interaction's subjects (the `subjects` and
//! `highlightedSubjects` of an image analysis interaction, and extracting a subject from an image).
//!
//! The caller's segmenter gives one 8-bit alpha mask per subject, in image pixels. This module owns the subject
//! list, which subject is under a pixel, the highlight set, and the cutout: the subject's pixels with the
//! background made transparent, cropped to the subject's bounds. Running a segmentation model is not done here.

/// A mask pixel at or above this alpha belongs to its subject.
pub const SUBJECT_THRESHOLD: u8 = 128;

/// A pixel rectangle in image coordinates, measured from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// An 8-bit alpha mask in image pixels, row-major, one byte per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl Mask {
    /// A mask of `width` x `height` pixels. `None` when a side is zero or `alpha` does not hold one byte per pixel.
    pub fn new(width: usize, height: usize, alpha: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 || alpha.len() != width.checked_mul(height)? {
            return None;
        }
        Some(Self { width, height, alpha })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The alpha at pixel `(x, y)`, or `None` outside the mask.
    pub fn alpha_at(&self, x: usize, y: usize) -> Option<u8> {
        if x < self.width && y < self.height {
            Some(self.alpha[y * self.width + x])
        } else {
            None
        }
    }

    /// The smallest rectangle holding every pixel at or above [`SUBJECT_THRESHOLD`]. `None` when there is none.
    pub fn bounds(&self) -> Option<PixelRect> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.alpha[y * self.width + x] >= SUBJECT_THRESHOLD {
                    any = true;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if !any {
            return None;
        }
        Some(PixelRect { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 })
    }
}

/// An RGBA image, row-major, four bytes per pixel (red, green, blue, alpha; not premultiplied).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

/// A subject lifted out of an image: its pixels with the background transparent.
pub type Cutout = Rgba;

impl Rgba {
    /// An image of `width` x `height` pixels. `None` when a side is zero or `pixels` is not four bytes per pixel.
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 || pixels.len() != width.checked_mul(height)?.checked_mul(4)? {
            return None;
        }
        Some(Self { width, height, pixels })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// One subject: an id the app keeps, its mask and the tight bounds of the mask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    id: u32,
    mask: Mask,
    bounds: PixelRect,
}

impl Subject {
    /// A subject with `id` and `mask`. `None` when the mask has no pixel at or above [`SUBJECT_THRESHOLD`].
    pub fn new(id: u32, mask: Mask) -> Option<Self> {
        let bounds = mask.bounds()?;
        Some(Self { id, mask, bounds })
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn mask(&self) -> &Mask {
        &self.mask
    }

    pub fn bounds(&self) -> PixelRect {
        self.bounds
    }

    /// Whether pixel `(x, y)` belongs to the subject.
    pub fn covers(&self, x: usize, y: usize) -> bool {
        self.mask.alpha_at(x, y).is_some_and(|a| a >= SUBJECT_THRESHOLD)
    }
}

/// The subjects found in one image and the current highlight, as the image analysis interaction keeps them.
#[derive(Debug, Default)]
pub struct Interaction {
    subjects: Vec<Subject>,
    highlighted: Vec<u32>,
}

impl Interaction {
    /// Subjects keep the order given, and a later subject is on top of an earlier one. A subject whose id was
    /// already given is dropped, so each id names one subject.
    pub fn new(subjects: Vec<Subject>) -> Self {
        let mut kept: Vec<Subject> = Vec::with_capacity(subjects.len());
        for s in subjects {
            if !kept.iter().any(|k| k.id == s.id) {
                kept.push(s);
            }
        }
        Self { subjects: kept, highlighted: Vec::new() }
    }

    pub fn subjects(&self) -> &[Subject] {
        &self.subjects
    }

    pub fn subject(&self, id: u32) -> Option<&Subject> {
        self.subjects.iter().find(|s| s.id == id)
    }

    /// The topmost subject covering pixel `(x, y)`, if any.
    pub fn subject_at(&self, x: usize, y: usize) -> Option<u32> {
        self.subjects.iter().rev().find(|s| s.covers(x, y)).map(|s| s.id)
    }

    /// Replaces the highlight set. Unknown ids are ignored. The highlight keeps subject order, without repeats.
    pub fn set_highlighted(&mut self, ids: &[u32]) {
        self.highlighted = self.subjects.iter().filter(|s| ids.contains(&s.id)).map(|s| s.id).collect();
    }

    pub fn highlighted(&self) -> &[u32] {
        &self.highlighted
    }

    pub fn is_highlighted(&self, id: u32) -> bool {
        self.highlighted.contains(&id)
    }

    /// Lifts subject `id` out of `image`: its bounds, with each pixel's alpha set to the mask alpha times the
    /// image alpha, so soft mask edges stay soft. `None` when the id is unknown or the image is not the size of
    /// the mask.
    pub fn lift(&self, id: u32, image: &Rgba) -> Option<Cutout> {
        let s = self.subject(id)?;
        if image.width != s.mask.width || image.height != s.mask.height {
            return None;
        }
        let b = s.bounds;
        let mut pixels = Vec::with_capacity(b.w * b.h * 4);
        for y in b.y..b.y + b.h {
            for x in b.x..b.x + b.w {
                let i = (y * image.width + x) * 4;
                let m = u32::from(s.mask.alpha[y * s.mask.width + x]);
                let src_a = u32::from(image.pixels[i + 3]);
                pixels.extend_from_slice(&image.pixels[i..i + 3]);
                pixels.push(((m * src_a + 127) / 255) as u8);
            }
        }
        Rgba::new(b.w, b.h, pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` x `h` mask with the given `(x, y, alpha)` pixels set and the rest zero.
    fn mask(w: usize, h: usize, on: &[(usize, usize, u8)]) -> Mask {
        let mut alpha = vec![0u8; w * h];
        for &(x, y, a) in on {
            alpha[y * w + x] = a;
        }
        Mask::new(w, h, alpha).expect("valid mask")
    }

    fn solid(w: usize, h: usize, rgba: [u8; 4]) -> Rgba {
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        Rgba::new(w, h, pixels).expect("valid image")
    }

    #[test]
    fn a_mask_needs_one_byte_per_pixel_and_a_size() {
        assert!(Mask::new(2, 2, vec![0; 3]).is_none());
        assert!(Mask::new(0, 2, Vec::new()).is_none());
        assert!(Mask::new(2, 2, vec![0; 4]).is_some());
        assert!(Rgba::new(2, 2, vec![0; 15]).is_none());
        assert!(Rgba::new(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn bounds_are_the_tight_box_of_pixels_at_the_threshold() {
        // (0, 0) is below the threshold, so it does not count.
        let m = mask(4, 3, &[(0, 0, SUBJECT_THRESHOLD - 1), (1, 1, 255), (2, 2, SUBJECT_THRESHOLD)]);
        assert_eq!(m.bounds(), Some(PixelRect { x: 1, y: 1, w: 2, h: 2 }));
        assert_eq!(mask(2, 2, &[]).bounds(), None);
    }

    #[test]
    fn a_subject_without_pixels_is_refused() {
        assert!(Subject::new(1, mask(2, 2, &[])).is_none());
        assert!(Subject::new(1, mask(2, 2, &[(0, 0, 200)])).is_some());
    }

    #[test]
    fn duplicate_ids_are_dropped_after_the_first() {
        let a = Subject::new(7, mask(2, 2, &[(0, 0, 255)])).unwrap();
        let b = Subject::new(7, mask(2, 2, &[(1, 1, 255)])).unwrap();
        let it = Interaction::new(vec![a.clone(), b]);
        assert_eq!(it.subjects().len(), 1);
        assert_eq!(it.subject(7), Some(&a));
    }

    #[test]
    fn the_topmost_subject_under_a_pixel_wins() {
        let below = Subject::new(1, mask(3, 3, &[(0, 0, 255), (1, 1, 255)])).unwrap();
        let above = Subject::new(2, mask(3, 3, &[(1, 1, 255)])).unwrap();
        let it = Interaction::new(vec![below, above]);
        assert_eq!(it.subject_at(1, 1), Some(2));
        assert_eq!(it.subject_at(0, 0), Some(1));
        assert_eq!(it.subject_at(2, 2), None);
        assert_eq!(it.subject_at(9, 9), None);
    }

    #[test]
    fn a_soft_edge_below_the_threshold_is_not_hit() {
        let s = Subject::new(1, mask(2, 2, &[(0, 0, 255), (1, 0, SUBJECT_THRESHOLD - 1)])).unwrap();
        let it = Interaction::new(vec![s]);
        assert_eq!(it.subject_at(1, 0), None);
    }

    #[test]
    fn highlight_ignores_unknown_ids_and_keeps_subject_order() {
        let a = Subject::new(1, mask(2, 2, &[(0, 0, 255)])).unwrap();
        let b = Subject::new(2, mask(2, 2, &[(1, 1, 255)])).unwrap();
        let mut it = Interaction::new(vec![a, b]);
        it.set_highlighted(&[9, 2, 1, 2]);
        assert_eq!(it.highlighted(), &[1, 2]);
        assert!(it.is_highlighted(2));
        it.set_highlighted(&[]);
        assert!(it.highlighted().is_empty());
    }

    #[test]
    fn lift_crops_to_the_subject_and_scales_alpha_by_the_mask() {
        // Mask: (1,1) full, (2,1) half (128), (2,2) full. Bounds are x 1..=2, y 1..=2, a 2 x 2 cutout.
        let s = Subject::new(5, mask(4, 3, &[(1, 1, 255), (2, 1, 128), (2, 2, 255)])).unwrap();
        let it = Interaction::new(vec![s]);
        let image = solid(4, 3, [10, 20, 30, 200]);
        let cut = it.lift(5, &image).expect("lifted");
        assert_eq!((cut.width(), cut.height()), (2, 2));
        let px = cut.pixels();
        // Pixels in reading order: (1,1), (2,1), (1,2), (2,2). Image alpha 200 scales the mask alpha.
        assert_eq!(&px[0..4], &[10, 20, 30, 200]);
        assert_eq!(px[7], 100, "half mask alpha gives half image alpha");
        assert_eq!(px[11], 0, "pixel outside the mask is transparent");
        assert_eq!(px[15], 200);
    }

    #[test]
    fn lift_refuses_unknown_ids_and_a_wrong_size_image() {
        let s = Subject::new(5, mask(2, 2, &[(0, 0, 255)])).unwrap();
        let it = Interaction::new(vec![s]);
        assert!(it.lift(6, &solid(2, 2, [0, 0, 0, 255])).is_none());
        assert!(it.lift(5, &solid(3, 2, [0, 0, 0, 255])).is_none());
        assert!(it.lift(5, &solid(2, 2, [0, 0, 0, 255])).is_some());
    }
}
