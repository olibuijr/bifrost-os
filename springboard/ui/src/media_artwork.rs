//! Artwork for a media item as a pure model: `Artwork` (an identifier the system caches by, and the image sizes the item
//! can supply), `ArtworkSize`, and `ArtworkError`. [`Artwork::choose`] picks the size to load for a target,
//! [`aspect_fit`] gives the drawn size inside a target, and [`request_pixels`] turns points and a display scale into
//! the pixel size a provider is asked for (the size argument of an artwork provider). No image bytes are decoded or
//! drawn here.

/// The largest side a representation may have, in pixels.
pub const MAX_SIDE: u32 = 16_384;

/// The pixel size of one image representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtworkSize {
    pub width: u32,
    pub height: u32,
}

impl ArtworkSize {
    /// Width times height, in pixels.
    pub fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// True when this size is at least the target on both sides.
    pub fn covers(self, width: u32, height: u32) -> bool {
        self.width >= width && self.height >= height
    }
}

/// Why an artwork value was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtworkError {
    /// The identifier is empty after trimming.
    EmptyId,
    /// A side is zero or larger than [`MAX_SIDE`].
    InvalidSize,
}

/// `Artwork`: an identifier and the representations the item can supply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artwork {
    id: String,
    sizes: Vec<ArtworkSize>,
}

impl Artwork {
    /// `init(id:artworkProvider:)` without the provider: the identifier is trimmed and must not be empty.
    pub fn new(id: &str) -> Result<Self, ArtworkError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(ArtworkError::EmptyId);
        }
        Ok(Artwork { id: id.to_string(), sizes: Vec::new() })
    }

    /// `id`: the identifier the system caches artwork under.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The representations added so far, in the order they were added.
    pub fn sizes(&self) -> &[ArtworkSize] {
        &self.sizes
    }

    /// Adds a representation. A size that is already present is not added twice.
    pub fn add_representation(&mut self, width: u32, height: u32) -> Result<(), ArtworkError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(ArtworkError::InvalidSize);
        }
        let size = ArtworkSize { width, height };
        if !self.sizes.contains(&size) {
            self.sizes.push(size);
        }
        Ok(())
    }

    /// The representation to load for a target of `width` by `height` pixels: the smallest one that covers the target
    /// on both sides (the first of equal areas). When none covers it, the largest one. `None` when there are none.
    pub fn choose(&self, width: u32, height: u32) -> Option<ArtworkSize> {
        let covering = self
            .sizes
            .iter()
            .copied()
            .filter(|s| s.covers(width, height))
            .min_by_key(|s| s.area());
        covering.or_else(|| self.sizes.iter().copied().max_by_key(|s| s.area()))
    }
}

/// The size of `source` drawn inside a `width` by `height` box, keeping its aspect ratio. Uses whole pixels, rounded
/// down, and fills the box along the side that limits it. `None` when the box has a zero side.
pub fn aspect_fit(source: ArtworkSize, width: u32, height: u32) -> Option<ArtworkSize> {
    if width == 0 || height == 0 || source.width == 0 || source.height == 0 {
        return None;
    }
    let (sw, sh, w, h) = (source.width as u64, source.height as u64, width as u64, height as u64);
    if sw * h <= sh * w {
        // The source is at least as tall as the box's proportions: height fills the box.
        Some(ArtworkSize { width: (sw * h / sh) as u32, height })
    } else {
        Some(ArtworkSize { width, height: (sh * w / sw) as u32 })
    }
}

/// The pixel size to ask a provider for: `points` times `scale_percent` (200 for a 2x display), rounded up. `None` when
/// either value is zero or the result is larger than [`MAX_SIDE`].
pub fn request_pixels(points: u32, scale_percent: u32) -> Option<u32> {
    if points == 0 || scale_percent == 0 {
        return None;
    }
    let pixels = (points as u64 * scale_percent as u64 + 99) / 100;
    if pixels > MAX_SIDE as u64 {
        None
    } else {
        Some(pixels as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> ArtworkSize {
        ArtworkSize { width, height }
    }

    #[test]
    fn an_identifier_is_trimmed_and_must_not_be_empty() {
        assert_eq!(Artwork::new("   "), Err(ArtworkError::EmptyId));
        assert_eq!(Artwork::new(" cover-7 ").unwrap().id(), "cover-7");
    }

    #[test]
    fn a_representation_needs_sides_from_one_to_the_maximum_and_is_not_added_twice() {
        let mut a = Artwork::new("cover").unwrap();
        assert_eq!(a.add_representation(0, 300), Err(ArtworkError::InvalidSize));
        assert_eq!(a.add_representation(300, MAX_SIDE + 1), Err(ArtworkError::InvalidSize));
        assert_eq!(a.add_representation(MAX_SIDE, 1), Ok(()));
        a.add_representation(300, 300).unwrap();
        a.add_representation(300, 300).unwrap();
        assert_eq!(a.sizes(), &[size(MAX_SIDE, 1), size(300, 300)]);
    }

    #[test]
    fn choose_takes_the_smallest_size_that_covers_the_target() {
        let mut a = Artwork::new("cover").unwrap();
        for side in [300, 600, 1200] {
            a.add_representation(side, side).unwrap();
        }
        assert_eq!(a.choose(500, 500), Some(size(600, 600)));
        assert_eq!(a.choose(600, 600), Some(size(600, 600)), "an exact match covers");
        assert_eq!(a.choose(601, 600), Some(size(1200, 1200)), "one short side is not enough");
    }

    #[test]
    fn choose_falls_back_to_the_largest_size_when_none_covers() {
        let mut a = Artwork::new("wide").unwrap();
        a.add_representation(300, 300).unwrap();
        a.add_representation(1200, 400).unwrap();
        // 2000x100 is covered by neither: 1200x400 has the larger area (480000 against 90000).
        assert_eq!(a.choose(2000, 100), Some(size(1200, 400)));
        assert_eq!(Artwork::new("empty").unwrap().choose(10, 10), None);
    }

    #[test]
    fn aspect_fit_fills_the_box_along_the_limiting_side() {
        // Wider than the box: width fills 300, height is 800 * 300 / 1200 = 200.
        assert_eq!(aspect_fit(size(1200, 800), 300, 300), Some(size(300, 200)));
        // Taller than the box: height fills 300, width is 800 * 300 / 1200 = 200.
        assert_eq!(aspect_fit(size(800, 1200), 300, 300), Some(size(200, 300)));
        // Same proportions as the box: both sides fill it.
        assert_eq!(aspect_fit(size(4, 3), 400, 300), Some(size(400, 300)));
        assert_eq!(aspect_fit(size(4, 3), 0, 300), None);
    }

    #[test]
    fn aspect_fit_rounds_down_to_whole_pixels() {
        // 1000 by 300 into 100 by 100: width fills 100, height is 300 * 100 / 1000 = 30.
        assert_eq!(aspect_fit(size(1000, 300), 100, 100), Some(size(100, 30)));
        // 300 by 1000 into 100 by 100: height fills 100, width is 300 * 100 / 1000 = 30.
        assert_eq!(aspect_fit(size(300, 1000), 100, 100), Some(size(30, 100)));
        // 2 by 3 into 10 by 10: height 10, width 2 * 10 / 3 = 6.66, rounded down to 6.
        assert_eq!(aspect_fit(size(2, 3), 10, 10), Some(size(6, 10)));
    }

    #[test]
    fn request_pixels_scales_points_and_rounds_up() {
        assert_eq!(request_pixels(300, 200), Some(600), "300 points on a 2x display");
        assert_eq!(request_pixels(1, 150), Some(2), "1.5 rounds up to 2");
        assert_eq!(request_pixels(100, 100), Some(100), "1x is unchanged");
    }

    #[test]
    fn request_pixels_refuses_zero_and_oversized_requests() {
        assert_eq!(request_pixels(0, 200), None);
        assert_eq!(request_pixels(300, 0), None);
        assert_eq!(request_pixels(20_000, 100), None, "above the maximum side");
    }
}
