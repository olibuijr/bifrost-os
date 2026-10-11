//! Pixel buffer layout descriptors and the byte conversions between the renderer's premultiplied 8-bit pixels
//! (`u32`, red in the low byte, see `gfx::pack`) and the buffers that codecs, files and other programs use.
//!
//! A [`Layout`] names the channel depth, the alpha convention and the channel count. Buffers are interleaved,
//! row after row, with no padding. Converting to 16-bit or float buffers widens the 8-bit values; converting
//! back from them is not supported, because the renderer stores 8 bits per channel.

use crate::gfx::{pack, rgba};

/// Bits per colour channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// One byte per channel, values 0..=255.
    Bits8,
    /// Two bytes per channel, little-endian, values 0..=65535.
    Bits16,
    /// One little-endian `f32` per channel, values 0.0..=1.0.
    Float32,
}

impl Depth {
    /// Bytes used by one channel.
    pub fn bytes(self) -> usize {
        match self {
            Depth::Bits8 => 1,
            Depth::Bits16 => 2,
            Depth::Float32 => 4,
        }
    }
}

/// How the alpha channel is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    /// No alpha channel: three channels, RGB.
    None,
    /// Four channels, colour stored without the alpha factor.
    Straight,
    /// Four channels, colour already multiplied by alpha (the renderer's own layout).
    Premultiplied,
}

/// A complete interleaved pixel layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub depth: Depth,
    pub alpha: Alpha,
}

impl Layout {
    /// Channels per pixel: 3 without alpha, 4 with it.
    pub fn channels(self) -> usize {
        match self.alpha {
            Alpha::None => 3,
            _ => 4,
        }
    }

    /// Bytes per pixel.
    pub fn bytes_per_pixel(self) -> usize {
        self.channels() * self.depth.bytes()
    }

    /// Bytes in one row of `width` pixels, with no padding.
    pub fn row_bytes(self, width: usize) -> usize {
        width.saturating_mul(self.bytes_per_pixel())
    }

    /// Bytes in a `width` x `height` buffer.
    pub fn buffer_len(self, width: usize, height: usize) -> usize {
        self.row_bytes(width).saturating_mul(height)
    }

    /// True when `len` bytes is exactly the size of a `width` x `height` buffer in this layout.
    pub fn fits(self, len: usize, width: usize, height: usize) -> bool {
        len == self.buffer_len(width, height)
    }
}

/// Widens an 8-bit channel to 16 bits (255 becomes 65535).
pub fn widen(c: u8) -> u16 {
    c as u16 * 257
}

/// Narrows a 16-bit channel to 8 bits, rounding to the nearest value.
pub fn narrow(v: u16) -> u8 {
    ((v as u32 + 128) / 257) as u8
}

/// Divides by alpha to get the straight colour of a premultiplied pixel. Fully transparent pixels become zero.
pub fn unpremultiply(px: u32) -> [u8; 4] {
    let a = (px >> 24) as u8;
    if a == 0 {
        return [0, 0, 0, 0];
    }
    let un = |shift: u32| -> u8 {
        let c = (px >> shift) as u8 as u32;
        ((c * 255 + a as u32 / 2) / a as u32).min(255) as u8
    };
    [un(0), un(8), un(16), a]
}

/// Serialises pixels into a buffer in `layout`. Alpha-less layouts drop the alpha channel.
pub fn to_bytes(px: &[u32], layout: Layout) -> Vec<u8> {
    let mut out = Vec::with_capacity(layout.buffer_len(px.len(), 1));
    for &p in px {
        let c = match layout.alpha {
            Alpha::Straight => unpremultiply(p),
            Alpha::Premultiplied | Alpha::None => [p as u8, (p >> 8) as u8, (p >> 16) as u8, (p >> 24) as u8],
        };
        let n = layout.channels();
        for &v in &c[..n] {
            match layout.depth {
                Depth::Bits8 => out.push(v),
                Depth::Bits16 => out.extend_from_slice(&widen(v).to_le_bytes()),
                Depth::Float32 => out.extend_from_slice(&(v as f32 / 255.0).to_le_bytes()),
            }
        }
    }
    out
}

/// Reads an 8-bit buffer in `layout` into premultiplied pixels. Returns None when the length is not a whole
/// number of pixels or the depth is not 8 bits.
pub fn from_bytes_u8(bytes: &[u8], layout: Layout) -> Option<Vec<u32>> {
    if layout.depth != Depth::Bits8 {
        return None;
    }
    let n = layout.channels();
    if bytes.len() % n != 0 {
        return None;
    }
    let px = bytes
        .chunks_exact(n)
        .map(|c| match layout.alpha {
            Alpha::None => pack(c[0], c[1], c[2], 255),
            Alpha::Straight => rgba(c[0], c[1], c[2], c[3]),
            Alpha::Premultiplied => pack(c[0], c[1], c[2], c[3]),
        })
        .collect();
    Some(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_sizes_follow_depth_and_alpha() {
        let rgba8 = Layout { depth: Depth::Bits8, alpha: Alpha::Premultiplied };
        assert_eq!(rgba8.channels(), 4);
        assert_eq!(rgba8.buffer_len(2, 3), 24);
        assert!(rgba8.fits(24, 2, 3));
        assert!(!rgba8.fits(23, 2, 3));

        let rgb16 = Layout { depth: Depth::Bits16, alpha: Alpha::None };
        assert_eq!(rgb16.bytes_per_pixel(), 6);
        assert_eq!(rgb16.buffer_len(2, 3), 36);

        let f32a = Layout { depth: Depth::Float32, alpha: Alpha::Straight };
        assert_eq!(f32a.row_bytes(5), 80);
        assert_eq!(f32a.buffer_len(6, 1), 96);
    }

    #[test]
    fn widen_and_narrow_round_trip_every_8_bit_value() {
        assert_eq!(widen(0), 0);
        assert_eq!(widen(255), 65535);
        for c in 0..=255u8 {
            assert_eq!(narrow(widen(c)), c);
        }
        assert_eq!(narrow(32768), 128);
    }

    #[test]
    fn unpremultiply_recovers_the_straight_colour() {
        // A half-transparent pure red: the stored red is halved, unpremultiply restores it.
        let px = rgba(255, 0, 0, 128);
        assert_eq!(px & 0xff, 128);
        assert_eq!(unpremultiply(px), [255, 0, 0, 128]);
        // Fully transparent pixels carry no colour.
        assert_eq!(unpremultiply(rgba(200, 100, 50, 0)), [0, 0, 0, 0]);
        // Opaque pixels pass through unchanged.
        assert_eq!(unpremultiply(pack(10, 20, 30, 255)), [10, 20, 30, 255]);
    }

    #[test]
    fn premultiplied_bytes_keep_channel_order() {
        let layout = Layout { depth: Depth::Bits8, alpha: Alpha::Premultiplied };
        assert_eq!(to_bytes(&[pack(1, 2, 3, 4)], layout), vec![1, 2, 3, 4]);
    }

    #[test]
    fn sixteen_bit_output_is_little_endian_and_widened() {
        let layout = Layout { depth: Depth::Bits16, alpha: Alpha::None };
        // Alpha is dropped; 1 widens to 257 (0x0101), 2 to 514 (0x0202).
        let bytes = to_bytes(&[pack(1, 2, 3, 255)], layout);
        assert_eq!(bytes, vec![1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn float_output_scales_to_unit_range() {
        let layout = Layout { depth: Depth::Float32, alpha: Alpha::None };
        let bytes = to_bytes(&[pack(255, 0, 0, 255)], layout);
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0.0f32.to_le_bytes());
    }

    #[test]
    fn straight_bytes_round_trip_through_premultiplied_pixels() {
        let layout = Layout { depth: Depth::Bits8, alpha: Alpha::Straight };
        let bytes = vec![255, 0, 0, 128, 0, 0, 0, 0];
        let px = from_bytes_u8(&bytes, layout).expect("two whole pixels");
        assert_eq!(px[0], rgba(255, 0, 0, 128));
        assert_eq!(to_bytes(&px, layout), bytes);
    }

    #[test]
    fn reading_rejects_partial_pixels_and_wide_depths() {
        let rgba8 = Layout { depth: Depth::Bits8, alpha: Alpha::Premultiplied };
        assert!(from_bytes_u8(&[1, 2, 3], rgba8).is_none());
        let rgb8 = Layout { depth: Depth::Bits8, alpha: Alpha::None };
        assert_eq!(from_bytes_u8(&[1, 2, 3], rgb8), Some(vec![pack(1, 2, 3, 255)]));
        let wide = Layout { depth: Depth::Bits16, alpha: Alpha::None };
        assert!(from_bytes_u8(&[0; 6], wide).is_none());
    }
}
