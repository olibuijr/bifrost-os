//! Texture load options and mip chains: the device-free half of an image loader.
//!
//! Mirrors the option set of the reference texture loader (sRGB, mipmap allocation and generation, origin,
//! usage and cache hints). It owns no device: [`prepare`] returns a [`TextureDescriptor`] and the pixel levels,
//! and the caller uploads them. Pixels stay premultiplied RGBA, as in [`crate::gfx::Image`].

use crate::gfx::Image;
use crate::imgdec;

/// Largest side accepted, the same limit the image decoder uses.
pub const MAX_SIDE: usize = imgdec::MAX_SIDE;

/// Which way the pixel rows run. Decoded images are stored top row first, so only
/// `BottomLeft` reverses the rows; `Unchanged` and `TopLeft` keep them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Unchanged,
    TopLeft,
    BottomLeft,
}

impl Origin {
    pub fn flips(self) -> bool {
        self == Origin::BottomLeft
    }
}

/// What the texture is later bound for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub shader_read: bool,
    pub shader_write: bool,
    pub render_target: bool,
}

impl Default for Usage {
    fn default() -> Self {
        Usage { shader_read: true, shader_write: false, render_target: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheMode {
    #[default]
    Default,
    WriteCombined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Storage {
    #[default]
    Shared,
    Private,
}

/// Loading options. The default loads one level with no sRGB conversion.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LoadOptions {
    pub srgb: bool,
    /// Reserve the whole mip chain (its levels stay unfilled unless `generate_mipmaps` is set).
    pub allocate_mipmaps: bool,
    /// Fill every mip level by box-filtering the base image. Implies the chain is allocated.
    pub generate_mipmaps: bool,
    pub origin: Origin,
    pub usage: Usage,
    pub cpu_cache: CacheMode,
    pub storage: Storage,
}

impl LoadOptions {
    pub fn wants_mip_chain(&self) -> bool {
        self.allocate_mipmaps || self.generate_mipmaps
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureError {
    /// A zero width or height.
    Empty,
    TooLarge { width: usize, height: usize },
    /// The bytes are not a PNG or JPEG the decoder accepts, or the pixel count does not match the size.
    Undecodable,
}

/// The size, levels and hints a GPU texture object is created from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureDescriptor {
    pub width: usize,
    pub height: usize,
    pub mip_levels: usize,
    pub srgb: bool,
    pub usage: Usage,
    pub cpu_cache: CacheMode,
    pub storage: Storage,
    pub origin: Origin,
}

impl TextureDescriptor {
    pub fn new(width: usize, height: usize, options: &LoadOptions) -> Result<Self, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::Empty);
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(TextureError::TooLarge { width, height });
        }
        let mip_levels = if options.wants_mip_chain() { mip_count(width, height) } else { 1 };
        Ok(TextureDescriptor {
            width,
            height,
            mip_levels,
            srgb: options.srgb,
            usage: options.usage,
            cpu_cache: options.cpu_cache,
            storage: options.storage,
            origin: options.origin,
        })
    }

    /// Bytes per row of the base level (RGBA, 4 bytes a pixel).
    pub fn bytes_per_row(&self) -> usize {
        self.width * 4
    }

    pub fn mip_chain(&self) -> Vec<MipLevel> {
        mip_layout(self.width, self.height, self.mip_levels)
    }
}

/// Number of levels in a full chain: one more than the halvings from the longest side down to 1.
pub fn mip_count(width: usize, height: usize) -> usize {
    let longest = width.max(height).max(1);
    (usize::BITS - longest.leading_zeros()) as usize
}

/// Where one level sits in a tightly packed RGBA byte buffer of the whole chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: usize,
    pub height: usize,
    pub offset: usize,
    pub len: usize,
}

/// Lays `levels` halvings of a `width` x `height` base out back to back, each level at least 1x1.
pub fn mip_layout(width: usize, height: usize, levels: usize) -> Vec<MipLevel> {
    let (mut w, mut h, mut offset) = (width, height, 0);
    (0..levels)
        .map(|_| {
            let len = w * h * 4;
            let level = MipLevel { width: w, height: h, offset, len };
            offset += len;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            level
        })
        .collect()
}

/// Box-filters `base` down to 1x1. Index 0 is `base` itself; each level halves both sides (min 1).
pub fn generate_mips(base: &Image) -> Vec<Image> {
    let mut levels = vec![base.clone()];
    while let Some(next) = levels.last().filter(|l| l.w > 1 || l.h > 1).map(halve) {
        levels.push(next);
    }
    levels
}

fn halve(src: &Image) -> Image {
    let (w, h) = ((src.w / 2).max(1), (src.h / 2).max(1));
    let mut out = Image::new(w, h);
    for y in 0..h {
        let (y0, y1) = (2 * y, (2 * y + 1).min(src.h - 1));
        for x in 0..w {
            let (x0, x1) = (2 * x, (2 * x + 1).min(src.w - 1));
            let taps = [
                src.px[y0 * src.w + x0],
                src.px[y0 * src.w + x1],
                src.px[y1 * src.w + x0],
                src.px[y1 * src.w + x1],
            ];
            out.px[y * w + x] = average4(taps);
        }
    }
    out
}

/// Per-channel mean of four premultiplied pixels, rounded to nearest.
fn average4(taps: [u32; 4]) -> u32 {
    (0..4).fold(0, |acc, c| {
        let sum: u32 = taps.iter().map(|p| (p >> (c * 8)) & 255).sum();
        acc | (((sum + 2) / 4) << (c * 8))
    })
}

/// Reverses the rows of `img` when the origin asks for a bottom-left texture.
pub fn apply_origin(mut img: Image, origin: Origin) -> Image {
    if origin.flips() {
        let (w, h) = (img.w, img.h);
        for y in 0..h / 2 {
            for x in 0..w {
                img.px.swap(y * w + x, (h - 1 - y) * w + x);
            }
        }
    }
    img
}

/// A loaded image ready for upload: the descriptor and the levels that carry pixels.
/// `levels` has one entry per mip level when mipmaps are generated, otherwise only the base.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub descriptor: TextureDescriptor,
    pub levels: Vec<Image>,
}

pub fn prepare(img: Image, options: &LoadOptions) -> Result<Prepared, TextureError> {
    let descriptor = TextureDescriptor::new(img.w, img.h, options)?;
    if img.px.len() != img.w * img.h {
        return Err(TextureError::Undecodable);
    }
    let img = apply_origin(img, options.origin);
    let levels = if options.generate_mipmaps { generate_mips(&img) } else { vec![img] };
    Ok(Prepared { descriptor, levels })
}

/// Decodes PNG or JPEG bytes and prepares them.
pub fn load_bytes(bytes: &[u8], options: &LoadOptions) -> Result<Prepared, TextureError> {
    let img = imgdec::decode(bytes).ok_or(TextureError::Undecodable)?;
    prepare(img, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn image(w: usize, h: usize, px: &[u32]) -> Image {
        Image { w, h, px: px.to_vec() }
    }

    #[test]
    fn mip_count_follows_the_longest_side() {
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_count(4, 4), 3);
        assert_eq!(mip_count(256, 64), 9);
        assert_eq!(mip_count(300, 1), 9);
    }

    #[test]
    fn descriptor_rejects_empty_and_oversized_images() {
        let o = LoadOptions::default();
        assert_eq!(TextureDescriptor::new(0, 4, &o), Err(TextureError::Empty));
        assert_eq!(
            TextureDescriptor::new(MAX_SIDE + 1, 4, &o),
            Err(TextureError::TooLarge { width: MAX_SIDE + 1, height: 4 })
        );
        let d = TextureDescriptor::new(10, 6, &o).unwrap();
        assert_eq!((d.mip_levels, d.bytes_per_row(), d.usage.shader_read), (1, 40, true));
    }

    #[test]
    fn allocate_mipmaps_reserves_the_chain_without_filling_it() {
        let o = LoadOptions { allocate_mipmaps: true, ..LoadOptions::default() };
        let prepared = prepare(image(4, 4, &[0; 16]), &o).unwrap();
        assert_eq!(prepared.descriptor.mip_levels, 3);
        assert_eq!(prepared.levels.len(), 1, "only the base carries pixels");
    }

    #[test]
    fn mip_layout_packs_levels_back_to_back() {
        let chain = mip_layout(4, 2, 3);
        assert_eq!(
            chain,
            vec![
                MipLevel { width: 4, height: 2, offset: 0, len: 32 },
                MipLevel { width: 2, height: 1, offset: 32, len: 8 },
                MipLevel { width: 1, height: 1, offset: 40, len: 4 },
            ]
        );
    }

    #[test]
    fn generate_mips_averages_premultiplied_taps_per_channel() {
        let white = pack(255, 255, 255, 255);
        let base = image(2, 2, &[white, 0, 0, 0]);
        let levels = generate_mips(&base);
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[1].w, levels[1].h), (1, 1));
        assert_eq!(levels[1].px[0], pack(64, 64, 64, 64), "255 / 4 rounded to nearest");
    }

    #[test]
    fn generated_chain_matches_the_descriptor() {
        let o = LoadOptions { generate_mipmaps: true, ..LoadOptions::default() };
        let prepared = prepare(image(4, 2, &[pack(9, 9, 9, 255); 8]), &o).unwrap();
        assert_eq!(prepared.levels.len(), prepared.descriptor.mip_levels);
        assert_eq!(prepared.descriptor.mip_chain()[2].len, 4);
        assert_eq!((prepared.levels[2].w, prepared.levels[2].h), (1, 1));
        assert_eq!(prepared.levels[2].px[0], pack(9, 9, 9, 255));
    }

    #[test]
    fn bottom_left_origin_reverses_rows_and_top_left_keeps_them() {
        let (a, b) = (pack(1, 0, 0, 255), pack(0, 2, 0, 255));
        let flipped = prepare(image(1, 2, &[a, b]), &LoadOptions { origin: Origin::BottomLeft, ..Default::default() });
        assert_eq!(flipped.unwrap().levels[0].px, vec![b, a]);
        let kept = prepare(image(1, 2, &[a, b]), &LoadOptions { origin: Origin::TopLeft, ..Default::default() });
        assert_eq!(kept.unwrap().levels[0].px, vec![a, b]);
    }

    #[test]
    fn load_bytes_decodes_png_and_rejects_garbage() {
        let png = crate::imgdec::files_tests::png_bytes(4, 2, [200, 100, 0, 255]);
        let o = LoadOptions { srgb: true, ..Default::default() };
        let prepared = load_bytes(&png, &o).unwrap();
        assert_eq!((prepared.descriptor.width, prepared.descriptor.height), (4, 2));
        assert!(prepared.descriptor.srgb);
        assert_eq!(load_bytes(b"not an image", &o), Err(TextureError::Undecodable));
    }
}
