//! Pixel formats: how the server packs a pixel into 8, 16 or 32 bits, and how a viewer turns it into 0x00RRGGBB.

use crate::Error;

/// A true-colour pixel format (RFB PIXEL_FORMAT). Colour channels are masked by `*_max` after a shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bpp: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// The format this viewer asks for: 32 bits, little-endian, 8 bits a channel, laid out as 0x00RRGGBB in memory
    /// order B, G, R, pad.
    pub const VIEWER: PixelFormat = PixelFormat {
        bpp: 32,
        depth: 24,
        big_endian: false,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 16,
        green_shift: 8,
        blue_shift: 0,
    };

    /// Reads the 16-byte PIXEL_FORMAT structure (the three bytes of padding at the end are skipped).
    pub fn parse(b: &[u8]) -> Result<PixelFormat, Error> {
        if b.len() < 16 {
            return Err(Error::Protocol("short pixel format".into()));
        }
        if b[3] == 0 {
            return Err(Error::Unsupported("colour-mapped pixel formats".into()));
        }
        let f = PixelFormat {
            bpp: b[0],
            depth: b[1],
            big_endian: b[2] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]),
            green_max: u16::from_be_bytes([b[6], b[7]]),
            blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10],
            green_shift: b[11],
            blue_shift: b[12],
        };
        f.validate()?;
        Ok(f)
    }

    /// The 16-byte PIXEL_FORMAT structure for a SetPixelFormat message.
    pub fn encode(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0] = self.bpp;
        out[1] = self.depth;
        out[2] = self.big_endian as u8;
        out[3] = 1;
        out[4..6].copy_from_slice(&self.red_max.to_be_bytes());
        out[6..8].copy_from_slice(&self.green_max.to_be_bytes());
        out[8..10].copy_from_slice(&self.blue_max.to_be_bytes());
        out[10] = self.red_shift;
        out[11] = self.green_shift;
        out[12] = self.blue_shift;
        out
    }

    /// Bytes one pixel takes on the wire.
    pub fn bytes_per_pixel(&self) -> usize {
        self.bpp as usize / 8
    }

    /// Only 8, 16 and 32 bit true-colour formats are handled; each channel must fit inside the pixel.
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.bpp, 8 | 16 | 32) {
            return Err(Error::Unsupported(format!("{} bits per pixel", self.bpp)));
        }
        let fits = |max: u16, shift: u8| max == 0 || shift as u32 + (16 - max.leading_zeros()) <= self.bpp as u32;
        if !(fits(self.red_max, self.red_shift) && fits(self.green_max, self.green_shift) && fits(self.blue_max, self.blue_shift)) {
            return Err(Error::Protocol("colour channel outside the pixel".into()));
        }
        Ok(())
    }

    /// Turns one raw pixel value into 0x00RRGGBB, scaling each channel to 8 bits.
    pub fn to_rgb(&self, raw: u32) -> u32 {
        let ch = |max: u16, shift: u8| -> u32 {
            let max = max as u32;
            if max == 0 {
                return 0;
            }
            let v = (raw >> shift) & max;
            if max == 255 {
                v
            } else {
                (v * 255 + max / 2) / max
            }
        };
        (ch(self.red_max, self.red_shift) << 16) | (ch(self.green_max, self.green_shift) << 8) | ch(self.blue_max, self.blue_shift)
    }

    /// Reads one pixel from the front of `b` and returns it as 0x00RRGGBB. `b` must hold `bytes_per_pixel` bytes.
    pub fn read_rgb(&self, b: &[u8]) -> u32 {
        let raw = match (self.bpp, self.big_endian) {
            (8, _) => b[0] as u32,
            (16, false) => u16::from_le_bytes([b[0], b[1]]) as u32,
            (16, true) => u16::from_be_bytes([b[0], b[1]]) as u32,
            (_, false) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (_, true) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        };
        self.to_rgb(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_format_round_trips() {
        let bytes = PixelFormat::VIEWER.encode();
        assert_eq!(bytes[3], 1, "true colour flag");
        assert_eq!(PixelFormat::parse(&bytes).unwrap(), PixelFormat::VIEWER);
    }

    #[test]
    fn thirty_two_bit_little_endian_pixel_is_bgrx_in_memory() {
        // Memory order B, G, R, pad: a red pixel is 00 00 ff 00.
        assert_eq!(PixelFormat::VIEWER.read_rgb(&[0x00, 0x00, 0xff, 0x00]), 0x00ff_0000);
        assert_eq!(PixelFormat::VIEWER.read_rgb(&[0xff, 0xff, 0xff, 0x00]), 0x00ff_ffff);
    }

    #[test]
    fn sixteen_bit_565_scales_to_eight_bits() {
        let f = PixelFormat { bpp: 16, depth: 16, big_endian: false, red_max: 31, green_max: 63, blue_max: 31, red_shift: 11, green_shift: 5, blue_shift: 0 };
        f.validate().unwrap();
        assert_eq!(f.to_rgb(0xf800), 0x00ff_0000);
        assert_eq!(f.to_rgb(0x07e0), 0x0000_ff00);
        assert_eq!(f.to_rgb(0x001f), 0x0000_00ff);
        assert_eq!(f.read_rgb(&[0x00, 0xf8]), 0x00ff_0000);
    }

    #[test]
    fn big_endian_32_bit_reads_big_endian() {
        let f = PixelFormat { big_endian: true, ..PixelFormat::VIEWER };
        assert_eq!(f.read_rgb(&[0x00, 0xff, 0x00, 0x00]), 0x00ff_0000);
    }

    #[test]
    fn eight_bit_three_three_two() {
        let f = PixelFormat { bpp: 8, depth: 8, big_endian: false, red_max: 7, green_max: 7, blue_max: 3, red_shift: 5, green_shift: 2, blue_shift: 0 };
        f.validate().unwrap();
        assert_eq!(f.to_rgb(0xe0), 0x00ff_0000);
        assert_eq!(f.to_rgb(0x03), 0x0000_00ff);
    }

    #[test]
    fn rejects_unsupported_formats() {
        let mut b = PixelFormat::VIEWER.encode();
        b[0] = 24;
        assert!(matches!(PixelFormat::parse(&b), Err(Error::Unsupported(_))));
        let mut b = PixelFormat::VIEWER.encode();
        b[3] = 0;
        assert!(matches!(PixelFormat::parse(&b), Err(Error::Unsupported(_))));
        let mut b = PixelFormat::VIEWER.encode();
        b[10] = 30;
        assert!(matches!(PixelFormat::parse(&b), Err(Error::Protocol(_))));
        assert!(PixelFormat::parse(&[0u8; 8]).is_err());
    }
}
