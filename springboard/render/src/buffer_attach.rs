//! Colour metadata that travels with an image buffer: the primaries, the transfer function and the YCbCr matrix,
//! plus the clean aperture (the part of the buffer that holds picture rather than padding). The attachments are
//! typed fields, not string keys. Nothing is inferred from pixels: a field is set by whoever made the buffer.

use crate::colorspace::Space;

/// Colour primaries of the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primaries {
    /// ITU-R BT.709 (the sRGB primaries).
    Bt709,
    /// SMPTE 170M, the standard-definition primaries.
    Smpte170m,
    /// ITU-R BT.2020, the wide gamut of UHD video.
    Bt2020,
    /// DCI P3 with the D65 white point (the Display P3 primaries).
    DisplayP3,
}

/// Transfer function (the curve that encodes the light levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Srgb,
    Bt709,
    Linear,
    /// SMPTE ST 2084, the perceptual quantizer used for HDR10.
    Pq,
    /// ARIB STD-B67, hybrid log-gamma.
    Hlg,
}

/// The YCbCr matrix: the luma weights used to turn RGB into Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YcbcrMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl YcbcrMatrix {
    /// The weights (Kr, Kg, Kb) with Y = Kr*R + Kg*G + Kb*B. They sum to 1.
    pub fn luma_weights(self) -> [f64; 3] {
        let (kr, kb) = match self {
            YcbcrMatrix::Bt601 => (0.299, 0.114),
            YcbcrMatrix::Bt709 => (0.2126, 0.0722),
            YcbcrMatrix::Bt2020 => (0.2627, 0.0593),
        };
        [kr, 1.0 - kr - kb, kb]
    }
}

/// A rectangle inside a buffer, in pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// The colour metadata of one buffer. An unset field means the buffer does not say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attachments {
    pub primaries: Option<Primaries>,
    pub transfer: Option<Transfer>,
    pub matrix: Option<YcbcrMatrix>,
    /// The clean aperture, as stored; `clean_rect` clips it to the buffer.
    pub clean: Option<Rect>,
}

impl Attachments {
    /// True when no field is set.
    pub fn is_empty(&self) -> bool {
        *self == Attachments::default()
    }

    /// The working space these attachments name, when the renderer has that space: BT.709 primaries or Display P3
    /// primaries with the sRGB transfer function. None for any other combination or a missing field.
    pub fn color_space(&self) -> Option<Space> {
        match (self.primaries?, self.transfer?) {
            (Primaries::Bt709, Transfer::Srgb) => Some(Space::Srgb),
            (Primaries::DisplayP3, Transfer::Srgb) => Some(Space::DisplayP3),
            _ => None,
        }
    }

    /// The clean aperture clipped to a buffer of `width` by `height`. Without one, the whole buffer. An aperture
    /// that starts outside the buffer is empty.
    pub fn clean_rect(&self, width: usize, height: usize) -> Rect {
        let full = Rect { x: 0, y: 0, w: width, h: height };
        match self.clean {
            None => full,
            Some(c) if c.x >= width || c.y >= height => Rect { x: 0, y: 0, w: 0, h: 0 },
            Some(c) => Rect {
                x: c.x,
                y: c.y,
                w: c.w.min(width - c.x),
                h: c.h.min(height - c.y),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_and_display_p3_map_to_working_spaces() {
        let srgb = Attachments {
            primaries: Some(Primaries::Bt709),
            transfer: Some(Transfer::Srgb),
            ..Attachments::default()
        };
        assert_eq!(srgb.color_space(), Some(Space::Srgb));
        let p3 = Attachments { primaries: Some(Primaries::DisplayP3), ..srgb };
        assert_eq!(p3.color_space(), Some(Space::DisplayP3));
    }

    #[test]
    fn unknown_pairs_and_missing_fields_have_no_space() {
        let hdr = Attachments {
            primaries: Some(Primaries::Bt2020),
            transfer: Some(Transfer::Pq),
            ..Attachments::default()
        };
        assert_eq!(hdr.color_space(), None);
        let no_transfer = Attachments { primaries: Some(Primaries::Bt709), ..Attachments::default() };
        assert_eq!(no_transfer.color_space(), None);
    }

    #[test]
    fn clean_rect_is_the_whole_buffer_without_an_aperture() {
        let a = Attachments::default();
        assert!(a.is_empty());
        assert_eq!(a.clean_rect(1920, 1080), Rect { x: 0, y: 0, w: 1920, h: 1080 });
    }

    #[test]
    fn clean_rect_is_clipped_to_the_buffer() {
        // 12 x 8 buffer, aperture at (4, 2) of size 10 x 10: width min(10, 12-4)=8, height min(10, 8-2)=6.
        let a = Attachments { clean: Some(Rect { x: 4, y: 2, w: 10, h: 10 }), ..Attachments::default() };
        assert!(!a.is_empty());
        assert_eq!(a.clean_rect(12, 8), Rect { x: 4, y: 2, w: 8, h: 6 });
    }

    #[test]
    fn clean_rect_starting_outside_is_empty() {
        let a = Attachments { clean: Some(Rect { x: 20, y: 0, w: 4, h: 4 }), ..Attachments::default() };
        assert_eq!(a.clean_rect(12, 8), Rect { x: 0, y: 0, w: 0, h: 0 });
    }

    #[test]
    fn luma_weights_follow_the_matrix_constants() {
        // BT.601 green: 1 - 0.299 - 0.114 = 0.587.
        let g601 = YcbcrMatrix::Bt601.luma_weights()[1];
        assert!((g601 - 0.587).abs() < 1e-9);
        // BT.2020 green: 1 - 0.2627 - 0.0593 = 0.678.
        let g2020 = YcbcrMatrix::Bt2020.luma_weights()[1];
        assert!((g2020 - 0.678).abs() < 1e-9);
        for m in [YcbcrMatrix::Bt601, YcbcrMatrix::Bt709, YcbcrMatrix::Bt2020] {
            let sum: f64 = m.luma_weights().iter().sum();
            assert!((sum - 1.0).abs() < 1e-9);
        }
    }
}
