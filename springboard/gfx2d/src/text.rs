//! Text: a glyph atlas (`R8Unorm`, 2048x2048, shelf packing, page reset when full) filled by the existing font
//! rasteriser (`bifrost_render::font::Font::render_var` on one character, at the run's size and a quarter-pixel
//! subpixel offset), and the layout of a `TextRun` into glyph quads (`shapes::Kind::Mask` instances) with advances
//! from `bifrost_render::font`.
//!
//! Matching the CPU path: `Font::render` puts the pen at column `PAD` of its image, advances each glyph by
//! `Font::measure_var` and applies no kerning (the font API exposes no kern table, and the CPU path does not use
//! one), so the layout does the same. A glyph's bitmap is its CPU image; the quarter-pixel offset is applied to the
//! 1x bitmap by linear interpolation, because the font API has no pen offset.
//!
//! Atlas encoding for `shapes::Instance` (Mask): `rect` = quad x, y, w, h in the run's device pixels;
//! `params[2]` = `Kind::Mask` as a number; `color` = the straight RGBA colour 0..1; `color2` = the glyph's rect in
//! the atlas, x, y, w, h in texels (the shader divides by `ATLAS_SIZE`); `xform` = identity; `clip` = none.
//!
//! Owner: wave G1 agent `gfx2d-text`.

use std::collections::HashMap;

use bifrost_gpu::{Device, Error as GpuError, PixelFormat, Region, Result, Texture, TextureDescriptor, TextureUsage};
use bifrost_render::canvas::TextRun;
use bifrost_render::font::{Font, Style, Var};

use crate::shapes::Instance;
#[cfg(test)]
use crate::shapes::Kind;

/// The atlas is square, in texels.
pub const ATLAS_SIZE: u32 = 2048;
/// Columns (and rows) `Font::render_var` leaves around a glyph: `2 + embolden + shadow` with no emboldening or shadow.
pub const PAD: usize = 2;
/// Horizontal subpixel positions per pixel.
pub const SUBPIXELS: u8 = 4;

/// A glyph's place in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face: u64,
    pub glyph: u16,
    /// Size in 1/64 px.
    pub size_64: u32,
    /// Subpixel x offset in quarter pixels (0..3).
    pub subpixel: u8,
    /// Hash of the variation values.
    pub vars: u64,
}

impl GlyphKey {
    /// The key of glyph `glyph` of `face` at `px` pixels with the variation values `vars`.
    pub fn new(face: u64, glyph: u16, px: f32, subpixel: u8, vars: &[Var]) -> GlyphKey {
        GlyphKey { face, glyph, size_64: (px * 64.0).round().max(0.0) as u32, subpixel, vars: vars_hash(vars) }
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over the axis tags and the values' bit patterns, in order. Deterministic across runs.
pub fn vars_hash(vars: &[Var]) -> u64 {
    let mut h = FNV_OFFSET;
    for (tag, value) in vars {
        for b in tag.iter().copied().chain(value.to_bits().to_le_bytes()) {
            h = (h ^ b as u64).wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// The key of one character's glyph. A character with no outline in the face (glyph 0: an emoji, or a character the
/// face lacks) renders as its own bitmap, so the code point is folded into the variation hash to keep such
/// characters apart.
pub fn glyph_key(face: u64, glyph: u16, ch: char, px: f32, vars: &[Var]) -> GlyphKey {
    let mut key = GlyphKey::new(face, glyph, px, 0, vars);
    if glyph == 0 {
        key.vars ^= (ch as u64).wrapping_add(1).wrapping_mul(FNV_PRIME);
    }
    key
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasEntry {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset of the bitmap from the pen position.
    pub left: f32,
    pub top: f32,
}

/// Shelf packer: rectangles are placed left to right on shelves (rows) of the height of their first rectangle; a
/// rectangle goes on the shortest existing shelf that is tall enough and has room, else a new shelf opens below.
/// Pure: no GPU, no allocation beyond the shelf list.
#[derive(Debug, Clone)]
pub struct ShelfPacker {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    h: u32,
    used: u32,
}

impl ShelfPacker {
    pub fn new(width: u32, height: u32) -> ShelfPacker {
        ShelfPacker { width, height, shelves: Vec::new(), next_y: 0 }
    }

    /// The top-left corner of a `w` x `h` rectangle, or None when it does not fit (or is empty or larger than the
    /// whole packer).
    pub fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        let mut best: Option<usize> = None;
        for (i, s) in self.shelves.iter().enumerate() {
            let fits = s.h >= h && self.width - s.used >= w;
            if fits && best.map_or(true, |b| s.h < self.shelves[b].h) {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            let s = &mut self.shelves[i];
            let at = (s.used, s.y);
            s.used += w;
            return Some(at);
        }
        if self.next_y + h > self.height {
            return None;
        }
        let at = (0, self.next_y);
        self.shelves.push(Shelf { y: self.next_y, h, used: w });
        self.next_y += h;
        Some(at)
    }

    /// Forgets every rectangle.
    pub fn clear(&mut self) {
        self.shelves.clear();
        self.next_y = 0;
    }

    /// Rows opened so far.
    pub fn used_height(&self) -> u32 {
        self.next_y
    }

    pub fn shelf_count(&self) -> usize {
        self.shelves.len()
    }
}

/// Splits a pen position into the whole pixel and the quarter-pixel offset. The offset rounds to the nearest quarter,
/// and a result that rounds up to a whole pixel carries into the pixel.
pub fn split_pen(x: f32) -> (i32, u8) {
    let whole = x.floor();
    let mut q = ((x - whole) * SUBPIXELS as f32).round() as u32;
    let mut whole = whole as i32;
    if q >= SUBPIXELS as u32 {
        whole += 1;
        q = 0;
    }
    (whole, q as u8)
}

/// Shifts a `w` x `h` alpha bitmap right by `q` quarter pixels. The result is one column wider (the shifted ink can
/// spill into it): `out[x] = ((4 - q) * a[x] + q * a[x - 1] + 2) / 4`, with the columns outside the bitmap read as 0.
pub fn shift_alpha(w: usize, h: usize, a: &[u8], q: u8) -> Vec<u8> {
    let f = q.min(SUBPIXELS - 1) as u32;
    let g = SUBPIXELS as u32 - f;
    let ow = w + 1;
    let mut out = vec![0u8; ow * h];
    for y in 0..h {
        for x in 0..ow {
            let cur = if x < w { a[y * w + x] as u32 } else { 0 };
            let prev = if x >= 1 && x - 1 < w { a[y * w + x - 1] as u32 } else { 0 };
            out[y * ow + x] = ((g * cur + f * prev + 2) / 4) as u8;
        }
    }
    out
}

/// A rasterised glyph: alpha coverage, `w` x `h`, with the pen at column `PAD` of the bitmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Bitmap {
    pub fn is_blank(&self) -> bool {
        self.a.iter().all(|&a| a == 0)
    }
}

/// Rasterises one character at `px` with `vars`, shifted by `subpixel` quarter pixels. The bitmap is the CPU image
/// `Font::render_var` draws (its alpha is the coverage), so it matches the CPU path pixel for pixel at offset 0.
pub fn rasterize(font: &Font<'_>, ch: char, px: f32, vars: &[Var], subpixel: u8) -> Bitmap {
    let mut buf = [0u8; 4];
    let style = Style { px, embolden: 0.0, color: bifrost_render::gfx::pack(255, 255, 255, 255), shadow: false };
    let img = font.render_var(ch.encode_utf8(&mut buf), &style, vars);
    let alpha: Vec<u8> = img.px.iter().map(|p| (p >> 24) as u8).collect();
    let a = shift_alpha(img.w, img.h, &alpha, subpixel);
    Bitmap { w: img.w + 1, h: img.h, a }
}

/// One character of a run, placed: its key, the pen position (device px, before the glyph's own offset) and the
/// quad's top-left corner in device px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub ch: char,
    pub key: GlyphKey,
    pub pen: f32,
    pub subpixel: u8,
    pub x: i32,
    pub y: i32,
}

/// Lays out `run` as the CPU path does: the origin is (`run.x`, `run.y`) rounded to whole pixels (the CPU image is
/// blitted at a whole origin), the pen starts at `PAD` inside it and advances by each character's
/// `Font::measure_var`. No kerning.
pub fn placements(font: &Font<'_>, run: &TextRun) -> Vec<Placement> {
    let ox = run.x.round() as i32;
    let oy = run.y.round() as i32;
    let mut pen = ox as f32 + PAD as f32;
    let mut out = Vec::with_capacity(run.text.len());
    let mut buf = [0u8; 4];
    for ch in run.text.chars() {
        let gid = font.glyph(ch);
        let key = glyph_key(run.face, gid, ch, run.px, &run.vars);
        let (whole, q) = split_pen(pen);
        out.push(Placement { ch, key, pen, subpixel: q, x: whole - PAD as i32, y: oy });
        pen += font.measure_var(ch.encode_utf8(&mut buf), run.px, &run.vars);
    }
    out
}


/// The quad for one placed glyph whose atlas rect is `entry`, in the run's coordinates. The uv rect is normalised to
/// the atlas size, as the shape shader samples it.
pub fn mask_instance(x: i32, y: i32, entry: &AtlasEntry, color: u32) -> Instance {
    let a = ATLAS_SIZE as f32;
    let uv = [entry.x as f32 / a, entry.y as f32 / a, (entry.x + entry.w) as f32 / a, (entry.y + entry.h) as f32 / a];
    let dst = bifrost_render::canvas::RectF::new(x as f32, y as f32, entry.w as f32, entry.h as f32);
    crate::shapes::mask(dst, 0.0, uv, color, bifrost_render::canvas::Transform2D::IDENTITY, None)
}

/// The glyph atlas: one `R8Unorm` texture, the packer, and the glyph entries. A glyph is rasterised and uploaded the
/// first time a run needs it. When the atlas is full it is reset (every glyph forgotten, then re-uploaded as needed).
pub struct GlyphAtlas {
    pub(crate) texture: Texture,
    packer: ShelfPacker,
    entries: HashMap<GlyphKey, AtlasEntry>,
}

impl GlyphAtlas {
    pub fn new(device: &Device) -> Result<GlyphAtlas> {
        let texture = device.new_texture(&TextureDescriptor {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            format: PixelFormat::R8Unorm,
            usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST,
            exportable: false,
            modifiers: Vec::new(),
        })?;
        Ok(GlyphAtlas { texture, packer: ShelfPacker::new(ATLAS_SIZE, ATLAS_SIZE), entries: HashMap::new() })
    }

    /// The atlas texture, bound by the renderer for Mask instances.
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    /// The atlas entry of a glyph already in the atlas.
    pub fn entry(&self, key: &GlyphKey) -> Option<AtlasEntry> {
        self.entries.get(key).copied()
    }

    /// How full the atlas shelves are, 0..1 of its height.
    pub fn fill_ratio(&self) -> f32 {
        self.packer.used_height() as f32 / ATLAS_SIZE as f32
    }

    /// Glyphs currently in the atlas.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forgets every glyph. The texture keeps its old pixels until glyphs are uploaded again.
    pub fn reset(&mut self) {
        self.packer.clear();
        self.entries.clear();
    }

    /// Lays out `run` (drawn with `font`, the face `run.face` names) into Mask instances appended to `out`, in
    /// `run`'s coordinates. Missing glyphs are rasterised and uploaded first. Returns how many glyphs were uploaded.
    /// If the atlas fills up part way, it is reset and the run is laid out again from the start; a run that cannot
    /// fit in an empty atlas is an error.
    pub fn layout(&mut self, font: &Font<'_>, run: &TextRun, color: u32, out: &mut Vec<Instance>) -> Result<u32> {
        let start = out.len();
        let places = placements(font, run);
        for attempt in 0..2 {
            out.truncate(start);
            let mut uploaded = 0;
            let mut full = false;
            for p in &places {
                let entry = match self.entries.get(&p.key) {
                    Some(e) => *e,
                    None => {
                        let bitmap = rasterize(font, p.ch, run.px, &run.vars, p.subpixel);
                        match self.insert(p.key, &bitmap)? {
                            Some(e) => {
                                uploaded += 1;
                                e
                            }
                            None => {
                                full = true;
                                break;
                            }
                        }
                    }
                };
                if entry.w == 0 {
                    continue;
                }
                out.push(mask_instance(p.x, p.y, &entry, color));
            }
            if !full {
                return Ok(uploaded);
            }
            if attempt == 0 {
                self.reset();
            }
        }
        out.truncate(start);
        Err(GpuError::Invalid(format!("text run of {} glyphs does not fit the glyph atlas", places.len())))
    }

    /// Places and uploads one bitmap. `Ok(None)` when the atlas has no room left.
    fn insert(&mut self, key: GlyphKey, bitmap: &Bitmap) -> Result<Option<AtlasEntry>> {
        if bitmap.is_blank() {
            let entry = AtlasEntry { x: 0, y: 0, w: 0, h: 0, left: -(PAD as f32), top: 0.0 };
            self.entries.insert(key, entry);
            return Ok(Some(entry));
        }
        let (w, h) = (bitmap.w as u32, bitmap.h as u32);
        if w > ATLAS_SIZE || h > ATLAS_SIZE {
            return Err(GpuError::Invalid(format!("glyph bitmap {w}x{h} is larger than the atlas")));
        }
        let Some((x, y)) = self.packer.allocate(w, h) else {
            return Ok(None);
        };
        self.texture.replace_region(Region { x, y, width: w, height: h }, &bitmap.a, bitmap.w)?;
        let entry = AtlasEntry { x, y, w, h, left: -(PAD as f32), top: 0.0 };
        self.entries.insert(key, entry);
        Ok(Some(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn sf() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf (run tools/pack.py)")
    }

    fn run(text: &str, px: f32, x: f32, y: f32) -> TextRun {
        TextRun { face: 7, text: text.to_string(), px, vars: Vec::new(), x, y }
    }

    fn hash_of<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn packer_places_left_to_right_then_opens_a_shelf() {
        let mut p = ShelfPacker::new(10, 10);
        assert_eq!(p.allocate(4, 3), Some((0, 0)));
        assert_eq!(p.allocate(4, 3), Some((4, 0)));
        // 8 + 4 > 10 on shelf 0, so a new shelf opens at y = 3
        assert_eq!(p.allocate(4, 3), Some((0, 3)));
        assert_eq!(p.allocate(2, 2), Some((8, 0)), "shelf 0 is 3 tall and has 2 columns left");
        assert_eq!(p.shelf_count(), 2);
        assert_eq!(p.used_height(), 6);
    }

    #[test]
    fn packer_reuses_a_tall_shelf_for_a_short_rectangle() {
        let mut p = ShelfPacker::new(10, 20);
        assert_eq!(p.allocate(1, 5), Some((0, 0)));
        assert_eq!(p.allocate(1, 2), Some((1, 0)), "a 2-high item fits the 5-high shelf");
        assert_eq!(p.allocate(9, 1), Some((0, 5)), "9 columns do not fit beside the first two");
        // both shelves fit a 1x1; the 1-high shelf is the shortest that fits
        assert_eq!(p.allocate(1, 1), Some((9, 5)));
        assert_eq!(p.allocate(1, 1), Some((2, 0)), "then the 5-high shelf, which has room");
        assert_eq!(p.shelf_count(), 2);
    }

    #[test]
    fn packer_refuses_empty_and_oversize_rectangles() {
        let mut p = ShelfPacker::new(10, 20);
        assert_eq!(p.allocate(0, 1), None);
        assert_eq!(p.allocate(1, 0), None);
        assert_eq!(p.allocate(11, 1), None, "wider than the packer");
        assert_eq!(p.allocate(1, 21), None, "taller than the packer");
        assert_eq!(p.shelf_count(), 0);
    }

    #[test]
    fn packer_reports_full_then_clears() {
        let mut p = ShelfPacker::new(4, 4);
        assert_eq!(p.allocate(4, 2), Some((0, 0)));
        assert_eq!(p.allocate(4, 2), Some((0, 2)));
        assert_eq!(p.allocate(1, 1), None, "no room left on any shelf or below");
        p.clear();
        assert_eq!(p.used_height(), 0);
        assert_eq!(p.allocate(4, 4), Some((0, 0)));
    }

    #[test]
    fn packer_never_overlaps_rectangles() {
        let mut p = ShelfPacker::new(32, 32);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for i in 0..40u32 {
            let (w, h) = (1 + (i * 7) % 9, 1 + (i * 5) % 6);
            if let Some((x, y)) = p.allocate(w, h) {
                assert!(x + w <= 32 && y + h <= 32);
                for &(px, py, pw, ph) in &placed {
                    let apart = x + w <= px || px + pw <= x || y + h <= py || py + ph <= y;
                    assert!(apart, "({x},{y},{w},{h}) overlaps ({px},{py},{pw},{ph})");
                }
                placed.push((x, y, w, h));
            }
        }
        assert!(placed.len() > 10);
    }

    #[test]
    fn split_pen_rounds_to_quarters_and_carries() {
        assert_eq!(split_pen(3.0), (3, 0));
        assert_eq!(split_pen(3.1), (3, 0), "0.4 of a pixel rounds to 0 quarters");
        assert_eq!(split_pen(3.2), (3, 1), "0.2 px is 0.8 quarters, which rounds to 1");
        assert_eq!(split_pen(3.9), (4, 0), "3.6 quarters rounds up and carries");
        assert_eq!(split_pen(-0.5), (-1, 2));
        assert_eq!(split_pen(0.0), (0, 0));
    }

    #[test]
    fn shift_at_offset_zero_only_adds_an_empty_column() {
        let a = [10u8, 20, 30, 40];
        assert_eq!(shift_alpha(2, 2, &a, 0), vec![10, 20, 0, 30, 40, 0]);
    }

    #[test]
    fn shift_interpolates_between_neighbours() {
        // [0, 255] shifted by half a pixel: 2*0 + 2*0 -> 0 ; 2*255 + 2*0 + 2 -> 128 ; 0 + 2*255 + 2 -> 128
        assert_eq!(shift_alpha(2, 1, &[0, 255], 2), vec![0, 128, 128]);
        // [255] shifted by three quarters: 64 and 191, which sum to the original 255
        assert_eq!(shift_alpha(1, 1, &[255], 3), vec![64, 191]);
    }

    #[test]
    fn shift_keeps_rows_apart() {
        // two rows of width 1: the second row must not leak into the first
        assert_eq!(shift_alpha(1, 2, &[200, 40], 1), vec![150, 50, 30, 10]);
    }

    #[test]
    fn glyph_key_hash_follows_every_field() {
        let base = GlyphKey::new(9, 36, 16.0, 0, &[]);
        assert_eq!(hash_of(&base), hash_of(&GlyphKey::new(9, 36, 16.0, 0, &[])));
        assert_ne!(base, GlyphKey { subpixel: 1, ..base });
        assert_ne!(base, GlyphKey { face: 8, ..base });
        assert_ne!(base, GlyphKey { glyph: 37, ..base });
        assert_ne!(base, GlyphKey::new(9, 36, 16.5, 0, &[]), "16.5 px is a different size");
        assert_eq!(GlyphKey::new(9, 36, 16.0, 0, &[]).size_64, 1024);
        assert_eq!(GlyphKey::new(9, 36, 13.5, 0, &[]).size_64, 864);
    }

    #[test]
    fn vars_hash_is_order_and_value_sensitive() {
        assert_eq!(vars_hash(&[]), FNV_OFFSET);
        let a = vars_hash(&[(*b"wght", 590.0)]);
        assert_eq!(a, vars_hash(&[(*b"wght", 590.0)]));
        assert_ne!(a, vars_hash(&[(*b"wght", 591.0)]));
        assert_ne!(a, vars_hash(&[(*b"opsz", 590.0)]));
        let ab = vars_hash(&[(*b"wght", 590.0), (*b"opsz", 17.0)]);
        let ba = vars_hash(&[(*b"opsz", 17.0), (*b"wght", 590.0)]);
        assert_ne!(ab, ba);
    }

    #[test]
    fn glyphs_without_outline_keep_their_characters_apart() {
        let a = glyph_key(1, 0, '\u{1F600}', 16.0, &[]);
        let b = glyph_key(1, 0, '\u{1F601}', 16.0, &[]);
        assert_ne!(a, b, "two emoji must not share the notdef entry");
        assert_eq!(glyph_key(1, 36, 'A', 16.0, &[]), glyph_key(1, 36, 'B', 16.0, &[]), "a real glyph ignores the character");
    }

    #[test]
    fn mask_instance_encodes_rect_colour_and_atlas_rect() {
        let e = AtlasEntry { x: 100, y: 40, w: 9, h: 14, left: -2.0, top: 0.0 };
        let i = mask_instance(5, 7, &e, bifrost_render::gfx::pack(255, 0, 0, 255));
        assert_eq!(i.rect, [5.0, 7.0, 9.0, 14.0]);
        assert_eq!(i.params[2], Kind::Mask as u32 as f32);
        assert_eq!(i.color, [1.0, 0.0, 0.0, 1.0]);
        // normalised to the 2048 px atlas: (100, 40) to (109, 54)
        assert_eq!(i.color2, [100.0 / 2048.0, 40.0 / 2048.0, 109.0 / 2048.0, 54.0 / 2048.0]);
        assert_eq!(i.xform, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(i.clip, [0.0; 4]);
    }

    #[test]
    fn mask_instance_unpremultiplies_the_colour() {
        // half-covered red, premultiplied: r = 128 of a = 128 is full red; alpha 128/255
        let i = mask_instance(0, 0, &AtlasEntry { x: 0, y: 0, w: 1, h: 1, left: 0.0, top: 0.0 }, bifrost_render::gfx::pack(128, 0, 0, 128));
        assert_eq!(i.color[0], 1.0);
        assert!((i.color[3] - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn placements_follow_the_cpu_pen_and_measure() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let text = "Hello, World!";
        let r = run(text, 16.0, 10.0, 20.0);
        let ps = placements(&font, &r);
        assert_eq!(ps.len(), text.chars().count());
        for (i, p) in ps.iter().enumerate() {
            let prefix = &text[..i];
            let want = 10.0 + PAD as f32 + font.measure(prefix, 16.0);
            assert!((p.pen - want).abs() < 1e-3, "glyph {i}: pen {} want {want}", p.pen);
        }
        // the whole run's advance is the CPU measure
        let last = ps.last().unwrap();
        let end = last.pen + font.measure("!", 16.0);
        assert!((end - (10.0 + PAD as f32 + font.measure(text, 16.0))).abs() < 1e-3);
    }

    #[test]
    fn placements_quad_sits_on_the_pen_whole_pixel() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let ps = placements(&font, &run("ilW", 16.0, 0.0, 0.0));
        for p in &ps {
            let (whole, q) = split_pen(p.pen);
            assert_eq!(p.x, whole - PAD as i32);
            assert_eq!(p.subpixel, q);
            assert_eq!(p.y, 0);
        }
        assert!(ps[1].pen > ps[0].pen && ps[2].pen > ps[1].pen, "pen advances");
    }

    #[test]
    fn placements_fold_a_fractional_origin_into_whole_pixels() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let a = placements(&font, &run("A", 16.0, 10.4, 3.6));
        let b = placements(&font, &run("A", 16.0, 10.0, 4.0));
        assert_eq!(a[0].pen, b[0].pen);
        assert_eq!(a[0].y, 4);
    }

    #[test]
    fn unshifted_bitmap_matches_the_cpu_image() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let st = Style { px: 24.0, embolden: 0.0, color: bifrost_render::gfx::pack(255, 255, 255, 255), shadow: false };
        let cpu = font.render("H", &st);
        let bmp = rasterize(&font, 'H', 24.0, &[], 0);
        assert_eq!(bmp.w, cpu.w + 1, "one spare column for subpixel shifts");
        assert_eq!(bmp.h, cpu.h);
        for y in 0..cpu.h {
            for x in 0..cpu.w {
                assert_eq!(bmp.a[y * bmp.w + x], (cpu.px[y * cpu.w + x] >> 24) as u8, "pixel {x},{y}");
            }
        }
        assert!(!bmp.is_blank());
    }

    #[test]
    fn subpixel_bitmap_is_the_shifted_cpu_image() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let st = Style { px: 24.0, embolden: 0.0, color: bifrost_render::gfx::pack(255, 255, 255, 255), shadow: false };
        let cpu = font.render("l", &st);
        let alpha: Vec<u8> = cpu.px.iter().map(|p| (p >> 24) as u8).collect();
        let want = shift_alpha(cpu.w, cpu.h, &alpha, 2);
        assert_eq!(rasterize(&font, 'l', 24.0, &[], 2).a, want);
    }

    #[test]
    fn space_has_no_ink_and_is_skipped_in_key_terms() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        assert!(rasterize(&font, ' ', 16.0, &[], 0).is_blank());
        let ps = placements(&font, &run(" a", 16.0, 0.0, 0.0));
        assert_ne!(ps[0].key, ps[1].key);
    }

    #[test]
    fn same_text_gives_same_keys() {
        let data = sf();
        let font = Font::parse(&data).unwrap();
        let a = placements(&font, &run("abc", 16.0, 0.0, 0.0));
        let b = placements(&font, &run("abc", 16.0, 0.0, 0.0));
        assert_eq!(a.iter().map(|p| p.key).collect::<Vec<_>>(), b.iter().map(|p| p.key).collect::<Vec<_>>());
        assert_eq!(a[0].key.face, 7);
    }
}
