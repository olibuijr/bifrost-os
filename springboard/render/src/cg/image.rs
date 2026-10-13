//! Bitmap image objects: pixel geometry, component layout, colour space, decode array, masks, plus the data
//! providers and consumers that move bytes in and out of them.
//!
//! A `CgImage` is immutable and cheap to clone (the pixel bytes are shared). `to_premultiplied` converts any
//! supported layout into the renderer's packed premultiplied pixels (`gfx::Image`).

use crate::gfx::{pack, rgba, Image};
use std::sync::Arc;

/// Where the alpha channel sits in a pixel, and whether colour is already multiplied by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaInfo {
    None,
    PremultipliedLast,
    PremultipliedFirst,
    Last,
    First,
    NoneSkipLast,
    NoneSkipFirst,
    /// The image carries only alpha (a mask).
    Only,
}

impl AlphaInfo {
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::PremultipliedLast | Self::PremultipliedFirst | Self::Last | Self::First | Self::Only)
    }
    pub fn is_premultiplied(self) -> bool {
        matches!(self, Self::PremultipliedLast | Self::PremultipliedFirst)
    }
    /// True when the extra component comes before the colour components.
    pub fn is_first(self) -> bool {
        matches!(self, Self::PremultipliedFirst | Self::First | Self::NoneSkipFirst)
    }
    /// True when the pixel has a fourth, non-colour component (alpha or skipped).
    pub fn has_extra_component(self) -> bool {
        !matches!(self, Self::None | Self::Only)
    }
}

/// Byte order of multi-byte pixels or components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    Default,
    Little16,
    Little32,
    Big16,
    Big32,
}

impl ByteOrder {
    fn little(self) -> bool {
        matches!(self, Self::Little16 | Self::Little32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderingIntent {
    Default,
    AbsoluteColorimetric,
    RelativeColorimetric,
    Perceptual,
    Saturation,
}

/// The colour space an image's samples are in.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageColorSpace {
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
    Srgb,
    LinearSrgb,
    DisplayP3,
    GenericGray,
    /// A profile-based space with this many components.
    Icc { components: usize, profile: Arc<Vec<u8>> },
    /// Samples are indices into `palette` (packed `base` components, 8 bits each).
    Indexed { base: Box<ImageColorSpace>, palette: Arc<Vec<u8>> },
}

impl ImageColorSpace {
    /// Number of samples per pixel that belong to the colour (not alpha).
    pub fn components(&self) -> usize {
        match self {
            Self::DeviceGray | Self::GenericGray => 1,
            Self::DeviceRgb | Self::Srgb | Self::LinearSrgb | Self::DisplayP3 => 3,
            Self::DeviceCmyk => 4,
            Self::Icc { components, .. } => *components,
            Self::Indexed { .. } => 1,
        }
    }
}

/// Bytes handed to an image: in memory, or fetched on demand.
#[derive(Clone)]
pub enum DataProvider {
    Memory(Arc<[u8]>),
    /// `(total length, reader(offset, len) -> bytes)`; the reader may return fewer bytes at the end.
    Callback(usize, Arc<dyn Fn(usize, usize) -> Vec<u8> + Send + Sync>),
}

impl std::fmt::Debug for DataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DataProvider({} bytes)", self.len())
    }
}

impl PartialEq for DataProvider {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Memory(a), Self::Memory(b)) => Arc::ptr_eq(a, b) || a == b,
            (Self::Callback(n, a), Self::Callback(m, b)) => n == m && Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl DataProvider {
    pub fn from_vec(v: Vec<u8>) -> Self {
        Self::Memory(v.into())
    }
    pub fn from_slice(s: &[u8]) -> Self {
        Self::Memory(s.into())
    }
    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self::from_vec(std::fs::read(path)?))
    }
    pub fn from_callback(len: usize, f: impl Fn(usize, usize) -> Vec<u8> + Send + Sync + 'static) -> Self {
        Self::Callback(len, Arc::new(f))
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Memory(m) => m.len(),
            Self::Callback(n, _) => *n,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Reads up to `len` bytes starting at `offset`.
    pub fn bytes_at(&self, offset: usize, len: usize) -> Vec<u8> {
        match self {
            Self::Memory(m) => m.get(offset.min(m.len())..offset.saturating_add(len).min(m.len())).unwrap_or(&[]).to_vec(),
            Self::Callback(n, f) => f(offset, len.min(n.saturating_sub(offset))),
        }
    }
    /// All the bytes, shared when already in memory.
    pub fn data(&self) -> Arc<[u8]> {
        match self {
            Self::Memory(m) => m.clone(),
            Self::Callback(n, _) => self.bytes_at(0, *n).into(),
        }
    }
}

/// Where encoded bytes go: collected in memory and optionally written to a file when finished.
#[derive(Debug, Default)]
pub struct DataConsumer {
    buf: Vec<u8>,
    path: Option<std::path::PathBuf>,
}

impl DataConsumer {
    pub fn in_memory() -> Self {
        Self::default()
    }
    pub fn to_file(path: impl Into<std::path::PathBuf>) -> Self {
        DataConsumer { buf: Vec::new(), path: Some(path.into()) }
    }
    /// Appends bytes; returns how many were accepted.
    pub fn put_bytes(&mut self, b: &[u8]) -> usize {
        self.buf.extend_from_slice(b);
        b.len()
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    /// Writes the file when the consumer has a path; returns the collected bytes either way.
    pub fn finish(self) -> std::io::Result<Vec<u8>> {
        if let Some(p) = &self.path {
            std::fs::write(p, &self.buf)?;
        }
        Ok(self.buf)
    }
}

/// How an image is masked when drawn.
#[derive(Clone, Debug, PartialEq)]
enum Masking {
    /// Samples whose every component lies inside `[min, max]` (raw sample values) become transparent.
    ColorRanges(Vec<f32>),
    /// Another image: a mask or a grey image whose luminance is the alpha.
    Image(Box<CgImage>),
}

/// An immutable bitmap.
#[derive(Clone, Debug, PartialEq)]
pub struct CgImage {
    width: usize,
    height: usize,
    bits_per_component: usize,
    bits_per_pixel: usize,
    bytes_per_row: usize,
    color_space: Option<ImageColorSpace>,
    alpha: AlphaInfo,
    order: ByteOrder,
    float_components: bool,
    provider: DataProvider,
    decode: Option<Vec<f32>>,
    interpolate: bool,
    intent: RenderingIntent,
    is_mask: bool,
    masking: Option<Masking>,
    type_id: Option<String>,
}

fn row_bytes(width: usize, bpp: usize) -> Option<usize> {
    Some(width.checked_mul(bpp)?.div_ceil(8))
}

impl CgImage {
    /// Creates an image over `provider`. `None` when the geometry is inconsistent or the data is too short.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        color_space: ImageColorSpace,
        alpha: AlphaInfo,
        order: ByteOrder,
        provider: DataProvider,
        decode: Option<Vec<f32>>,
        interpolate: bool,
        intent: RenderingIntent,
    ) -> Option<Self> {
        if width == 0 || height == 0 || !matches!(bits_per_component, 1 | 2 | 4 | 5 | 8 | 16 | 32) {
            return None;
        }
        let ncomp = color_space.components() + alpha.has_extra_component() as usize;
        let packed555 = bits_per_component == 5 && bits_per_pixel == 16;
        if !packed555 && bits_per_pixel != bits_per_component * ncomp {
            return None;
        }
        let rb = row_bytes(width, bits_per_pixel)?;
        if bytes_per_row < rb {
            return None;
        }
        if provider.len() < bytes_per_row.checked_mul(height - 1)?.checked_add(rb)? {
            return None;
        }
        if let Some(d) = &decode {
            if d.len() != 2 * color_space.components() {
                return None;
            }
        }
        Some(CgImage {
            width,
            height,
            bits_per_component,
            bits_per_pixel,
            bytes_per_row,
            color_space: Some(color_space),
            alpha,
            order,
            float_components: bits_per_component == 32,
            provider,
            decode,
            interpolate,
            intent,
            is_mask: false,
            masking: None,
            type_id: None,
        })
    }

    /// An image mask: 1 to 8 bits of coverage per pixel, painted with the current fill colour.
    /// Unless `decode` is `[1, 0]`, a sample of 0 paints.
    pub fn new_mask(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        provider: DataProvider,
        decode: Option<Vec<f32>>,
        interpolate: bool,
    ) -> Option<Self> {
        if width == 0 || height == 0 || !matches!(bits_per_component, 1 | 2 | 4 | 8) {
            return None;
        }
        let rb = row_bytes(width, bits_per_component)?;
        if bytes_per_row < rb || provider.len() < bytes_per_row * (height - 1) + rb {
            return None;
        }
        Some(CgImage {
            width,
            height,
            bits_per_component,
            bits_per_pixel: bits_per_component,
            bytes_per_row,
            color_space: None,
            alpha: AlphaInfo::Only,
            order: ByteOrder::Default,
            float_components: false,
            provider,
            decode,
            interpolate,
            intent: RenderingIntent::Default,
            is_mask: true,
            masking: None,
            type_id: None,
        })
    }

    /// From straight (non-premultiplied) 8-bit RGBA bytes.
    pub fn from_rgba8(width: usize, height: usize, rgba: Vec<u8>) -> Option<Self> {
        Self::new(width, height, 8, 32, width * 4, ImageColorSpace::Srgb, AlphaInfo::Last, ByteOrder::Default, DataProvider::from_vec(rgba), None, true, RenderingIntent::Default)
    }

    /// From 16-bit-per-component straight RGBA (stored big-endian).
    pub fn from_rgba16(width: usize, height: usize, samples: &[u16]) -> Option<Self> {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_be_bytes()).collect();
        Self::new(width, height, 16, 64, width * 8, ImageColorSpace::Srgb, AlphaInfo::Last, ByteOrder::Big16, DataProvider::from_vec(bytes), None, true, RenderingIntent::Default)
    }

    /// From the renderer's premultiplied pixels.
    pub fn from_premultiplied(img: &Image) -> Option<Self> {
        let bytes: Vec<u8> = img.px.iter().flat_map(|p| p.to_le_bytes()).collect();
        Self::new(img.w, img.h, 8, 32, img.w * 4, ImageColorSpace::Srgb, AlphaInfo::PremultipliedLast, ByteOrder::Default, DataProvider::from_vec(bytes), None, true, RenderingIntent::Default)
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn bits_per_component(&self) -> usize {
        self.bits_per_component
    }
    pub fn bits_per_pixel(&self) -> usize {
        self.bits_per_pixel
    }
    pub fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }
    pub fn color_space(&self) -> Option<&ImageColorSpace> {
        self.color_space.as_ref()
    }
    pub fn alpha_info(&self) -> AlphaInfo {
        self.alpha
    }
    pub fn byte_order(&self) -> ByteOrder {
        self.order
    }
    pub fn uses_float_components(&self) -> bool {
        self.float_components
    }
    pub fn data_provider(&self) -> &DataProvider {
        &self.provider
    }
    pub fn decode_array(&self) -> Option<&[f32]> {
        self.decode.as_deref()
    }
    pub fn should_interpolate(&self) -> bool {
        self.interpolate
    }
    pub fn rendering_intent(&self) -> RenderingIntent {
        self.intent
    }
    pub fn is_mask(&self) -> bool {
        self.is_mask
    }
    /// The identifier of the format the image was decoded from, when it came from a decoder.
    pub fn type_identifier(&self) -> Option<&str> {
        self.type_id.as_deref()
    }
    pub fn with_type_identifier(mut self, id: &str) -> Self {
        self.type_id = Some(id.to_string());
        self
    }
    pub fn has_masking(&self) -> bool {
        self.masking.is_some()
    }

    /// A shallow copy (the pixel bytes are shared).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// The same pixels interpreted in another colour space with the same number of components.
    pub fn copy_with_color_space(&self, cs: ImageColorSpace) -> Option<Self> {
        let old = self.color_space.as_ref()?;
        if old.components() != cs.components() {
            return None;
        }
        let mut c = self.clone();
        c.color_space = Some(cs);
        Some(c)
    }

    /// The part of the image inside `(x, y, w, h)` (clipped to the bounds), as a new image.
    /// `None` when the clip is empty or a sub-byte pixel offset would split a byte.
    pub fn cropped(&self, x: usize, y: usize, w: usize, h: usize) -> Option<Self> {
        let x1 = x.saturating_add(w).min(self.width);
        let y1 = y.saturating_add(h).min(self.height);
        if x >= x1 || y >= y1 {
            return None;
        }
        let (nw, nh) = (x1 - x, y1 - y);
        let bit0 = x * self.bits_per_pixel;
        if !bit0.is_multiple_of(8) {
            return None;
        }
        let rb = row_bytes(nw, self.bits_per_pixel)?;
        let data = self.provider.data();
        let mut out = Vec::with_capacity(rb * nh);
        for row in y..y1 {
            let s = row * self.bytes_per_row + bit0 / 8;
            out.extend_from_slice(data.get(s..s + rb)?);
        }
        let mut c = self.clone();
        c.width = nw;
        c.height = nh;
        c.bytes_per_row = rb;
        c.provider = DataProvider::from_vec(out);
        if let Some(Masking::Image(m)) = &self.masking {
            // The mask follows the same relative rectangle.
            let mx = x * m.width / self.width;
            let my = y * m.height / self.height;
            let mw = (nw * m.width / self.width).max(1);
            let mh = (nh * m.height / self.height).max(1);
            c.masking = m.cropped(mx, my, mw, mh).map(|m| Masking::Image(Box::new(m)));
        }
        Some(c)
    }

    /// Masks with per-component ranges `[min0, max0, min1, max1, ...]` in raw sample values.
    pub fn with_masking_colors(&self, ranges: &[f32]) -> Option<Self> {
        let cs = self.color_space.as_ref()?;
        if self.is_mask || ranges.len() != 2 * cs.components() || ranges.chunks(2).any(|r| r[0] > r[1]) {
            return None;
        }
        let mut c = self.clone();
        c.masking = Some(Masking::ColorRanges(ranges.to_vec()));
        Some(c)
    }

    /// Masks with an image mask or a grey image (a grey image's value is the alpha).
    pub fn with_mask(&self, mask: &CgImage) -> Option<Self> {
        if !mask.is_mask && !matches!(mask.color_space, Some(ImageColorSpace::DeviceGray | ImageColorSpace::GenericGray)) {
            return None;
        }
        let mut c = self.clone();
        c.masking = Some(Masking::Image(Box::new(mask.clone())));
        Some(c)
    }

    /// One sample as a float in `0..=1` before the decode array; `comp` indexes the pixel's components.
    fn sample(&self, data: &[u8], x: usize, y: usize, comp: usize) -> f32 {
        let row = &data[y * self.bytes_per_row..];
        let bpc = self.bits_per_component;
        let le = self.order.little();
        match bpc {
            8 => {
                let n = self.bits_per_pixel / 8;
                let k = if self.order == ByteOrder::Little32 && n == 4 { 3 - comp } else { comp };
                row[x * n + k] as f32 / 255.0
            }
            16 => {
                let n = self.bits_per_pixel / 8;
                let i = x * n + comp * 2;
                let v = if le { u16::from_le_bytes([row[i], row[i + 1]]) } else { u16::from_be_bytes([row[i], row[i + 1]]) };
                v as f32 / 65535.0
            }
            32 => {
                let n = self.bits_per_pixel / 8;
                let i = x * n + comp * 4;
                let b = [row[i], row[i + 1], row[i + 2], row[i + 3]];
                f32::from_bits(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }).clamp(0.0, 1.0)
            }
            5 => {
                let i = x * 2;
                let v = if le { u16::from_le_bytes([row[i], row[i + 1]]) } else { u16::from_be_bytes([row[i], row[i + 1]]) };
                let s = [10, 5, 0][comp.min(2)];
                ((v >> s) & 31) as f32 / 31.0
            }
            _ => {
                let bit = x * self.bits_per_pixel + comp * bpc;
                let max = (1u16 << bpc) - 1;
                let v = (row[bit / 8] as u16 >> (8 - bpc - bit % 8)) & max;
                v as f32 / max as f32
            }
        }
    }

    /// The raw integer value of a sample (for indexed lookup and colour-range masking).
    fn raw_value(&self, data: &[u8], x: usize, y: usize, comp: usize) -> f32 {
        if self.bits_per_component == 32 {
            return self.sample(data, x, y, comp);
        }
        let max = ((1u64 << self.bits_per_component) - 1) as f32;
        (self.sample(data, x, y, comp) * max).round()
    }

    /// Converts to premultiplied RGBA pixels. A mask paints black with the mask's coverage as alpha.
    pub fn to_premultiplied(&self) -> Image {
        self.to_premultiplied_with(0xff00_0000)
    }

    /// As `to_premultiplied`, painting an image mask with `fill` (premultiplied packed pixel).
    pub fn to_premultiplied_with(&self, fill: u32) -> Image {
        let data = self.provider.data();
        let mut out = Image::new(self.width, self.height);
        if self.is_mask {
            let inv = self.decode.as_deref().is_some_and(|d| d.len() == 2 && d[0] > d[1]);
            let fa = fill >> 24;
            for y in 0..self.height {
                for x in 0..self.width {
                    let mut c = self.sample(&data, x, y, 0);
                    if !inv {
                        c = 1.0 - c;
                    }
                    let a = ((c * 255.0 + 0.5) as u32) * fa / 255;
                    out.px[y * self.width + x] = scale_px(fill, a, fa);
                }
            }
            return out;
        }
        let cs = self.color_space.clone().unwrap_or(ImageColorSpace::DeviceRgb);
        let nc = cs.components();
        let fast = self.bits_per_component == 8
            && self.decode.is_none()
            && self.masking.is_none()
            && self.order == ByteOrder::Default
            && matches!(cs, ImageColorSpace::DeviceRgb | ImageColorSpace::Srgb)
            && self.bits_per_pixel == 32;
        if fast {
            for y in 0..self.height {
                let row = &data[y * self.bytes_per_row..];
                for x in 0..self.width {
                    let p = &row[x * 4..x * 4 + 4];
                    out.px[y * self.width + x] = match self.alpha {
                        AlphaInfo::Last => rgba(p[0], p[1], p[2], p[3]),
                        AlphaInfo::PremultipliedLast => pack(p[0].min(p[3]), p[1].min(p[3]), p[2].min(p[3]), p[3]),
                        AlphaInfo::First => rgba(p[1], p[2], p[3], p[0]),
                        AlphaInfo::PremultipliedFirst => pack(p[1].min(p[0]), p[2].min(p[0]), p[3].min(p[0]), p[0]),
                        AlphaInfo::NoneSkipFirst => pack(p[1], p[2], p[3], 255),
                        _ => pack(p[0], p[1], p[2], 255),
                    };
                }
            }
            return out;
        }
        let dec: Vec<f32> = self.decode.clone().unwrap_or_else(|| (0..nc).flat_map(|_| [0.0, 1.0]).collect());
        let color_off = self.alpha.is_first() as usize;
        let alpha_idx = match self.alpha {
            AlphaInfo::Last | AlphaInfo::PremultipliedLast => Some(nc),
            AlphaInfo::First | AlphaInfo::PremultipliedFirst | AlphaInfo::Only => Some(0),
            _ => None,
        };
        let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        for y in 0..self.height {
            for x in 0..self.width {
                if let Some(Masking::ColorRanges(r)) = &self.masking {
                    let hit = (0..nc).all(|k| {
                        let v = self.raw_value(&data, x, y, color_off + k);
                        v >= r[2 * k] && v <= r[2 * k + 1]
                    });
                    if hit {
                        continue;
                    }
                }
                let mut comps = [0f32; 4];
                for k in 0..nc.min(4) {
                    comps[k] = dec[2 * k] + self.sample(&data, x, y, color_off + k) * (dec[2 * k + 1] - dec[2 * k]);
                }
                let col = match &cs {
                    ImageColorSpace::Indexed { base, palette } => {
                        let idx = self.raw_value(&data, x, y, color_off) as usize;
                        let i = idx * base.components();
                        let get = |k: usize| palette.get(i + k).copied().unwrap_or(0) as f32 / 255.0;
                        components_to_rgb(base, &[get(0), get(1), get(2), get(3)])
                    }
                    other => components_to_rgb(other, &comps),
                };
                let mut a = match alpha_idx {
                    Some(i) => self.sample(&data, x, y, i),
                    None => 1.0,
                };
                if let Some(Masking::Image(m)) = &self.masking {
                    let mx = x * m.width / self.width;
                    let my = y * m.height / self.height;
                    let mdata = m.provider.data();
                    let mut c = m.sample(&mdata, mx, my, 0);
                    if m.is_mask {
                        let inv = m.decode.as_deref().is_some_and(|d| d.len() == 2 && d[0] > d[1]);
                        if !inv {
                            c = 1.0 - c;
                        }
                    }
                    a *= c;
                }
                let ab = to8(a);
                out.px[y * self.width + x] = if self.alpha.is_premultiplied() {
                    pack(to8(col[0]).min(ab), to8(col[1]).min(ab), to8(col[2]).min(ab), ab)
                } else if ab == 255 {
                    pack(to8(col[0]), to8(col[1]), to8(col[2]), 255)
                } else {
                    rgba(to8(col[0]), to8(col[1]), to8(col[2]), ab)
                };
            }
        }
        out
    }

    /// Straight (non-premultiplied) 8-bit RGBA bytes.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let img = self.to_premultiplied();
        let mut out = Vec::with_capacity(img.px.len() * 4);
        for &p in &img.px {
            let [r, g, b, a] = p.to_le_bytes();
            if a == 0 || a == 255 {
                out.extend([r, g, b, a]);
            } else {
                let u = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                out.extend([u(r), u(g), u(b), a]);
            }
        }
        out
    }
}

fn scale_px(fill: u32, a: u32, fa: u32) -> u32 {
    if fa == 0 {
        return 0;
    }
    let [r, g, b, _] = fill.to_le_bytes();
    let s = |c: u8| (c as u32 * a / fa) as u8;
    pack(s(r), s(g), s(b), a as u8)
}

fn components_to_rgb(cs: &ImageColorSpace, c: &[f32; 4]) -> [f32; 3] {
    match cs {
        ImageColorSpace::DeviceGray | ImageColorSpace::GenericGray => [c[0]; 3],
        ImageColorSpace::DeviceCmyk => {
            let k = 1.0 - c[3];
            [(1.0 - c[0]) * k, (1.0 - c[1]) * k, (1.0 - c[2]) * k]
        }
        ImageColorSpace::Icc { components: 1, .. } => [c[0]; 3],
        ImageColorSpace::Icc { components: 4, .. } => components_to_rgb(&ImageColorSpace::DeviceCmyk, c),
        _ => [c[0], c[1], c[2]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_img() -> CgImage {
        CgImage::from_rgba8(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 255]).unwrap()
    }

    fn grey(w: usize, h: usize, bpc: usize, bpr: usize, data: Vec<u8>, decode: Option<Vec<f32>>) -> CgImage {
        CgImage::new(w, h, bpc, bpc, bpr, ImageColorSpace::DeviceGray, AlphaInfo::None, ByteOrder::Default, DataProvider::from_vec(data), decode, false, RenderingIntent::Default).unwrap()
    }

    #[test]
    fn geometry_is_validated() {
        let rgb = |bpp, bpr, data: Vec<u8>| CgImage::new(2, 2, 8, bpp, bpr, ImageColorSpace::DeviceRgb, AlphaInfo::Last, ByteOrder::Default, DataProvider::from_vec(data), None, false, RenderingIntent::Default);
        assert!(rgb(32, 8, vec![0; 16]).is_some());
        assert!(rgb(24, 8, vec![0; 16]).is_none(), "bpp mismatch");
        assert!(rgb(32, 8, vec![0; 8]).is_none(), "short data");
        assert!(rgb(32, 4, vec![0; 16]).is_none(), "row too narrow");
    }

    #[test]
    fn straight_alpha_is_premultiplied_on_conversion() {
        let px = rgba_img().to_premultiplied();
        assert_eq!(px.px[0], pack(255, 0, 0, 255));
        assert_eq!(px.px[1], rgba(0, 255, 0, 128));
        assert_eq!(px.px[2] >> 24, 0);
        let back = rgba_img().to_rgba8();
        assert_eq!(&back[..4], &[255, 0, 0, 255]);
        assert_eq!(back[5], 255);
    }

    #[test]
    fn crop_copies_the_rectangle() {
        let c = rgba_img().cropped(1, 0, 5, 1).unwrap();
        assert_eq!((c.width(), c.height()), (1, 1));
        assert_eq!(c.to_premultiplied().px[0], rgba(0, 255, 0, 128));
        assert!(rgba_img().cropped(2, 0, 1, 1).is_none());
    }

    #[test]
    fn grey_one_bit_and_decode_array() {
        let p = grey(8, 1, 1, 1, vec![0b1010_0000], None).to_premultiplied();
        assert_eq!(p.px[0], pack(255, 255, 255, 255));
        assert_eq!(p.px[1], pack(0, 0, 0, 255));
        let inv = grey(8, 1, 1, 1, vec![0b1010_0000], Some(vec![1.0, 0.0]));
        assert_eq!(inv.to_premultiplied().px[0], pack(0, 0, 0, 255));
    }

    #[test]
    fn image_mask_paints_where_sample_is_zero() {
        let m = CgImage::new_mask(2, 1, 1, 1, DataProvider::from_vec(vec![0b0100_0000]), None, false).unwrap();
        let p = m.to_premultiplied_with(pack(255, 0, 0, 255));
        assert_eq!(p.px[0], pack(255, 0, 0, 255));
        assert_eq!(p.px[1], 0);
    }

    #[test]
    fn colour_range_mask_hides_matching_pixels() {
        let img = rgba_img().with_masking_colors(&[250.0, 255.0, 0.0, 0.0, 0.0, 0.0]).unwrap();
        let p = img.to_premultiplied();
        assert_eq!(p.px[0] >> 24, 0, "red pixel masked out");
        assert_eq!(p.px[3], pack(10, 20, 30, 255));
        assert!(rgba_img().with_masking_colors(&[0.0, 1.0]).is_none());
    }

    #[test]
    fn image_mask_scales_alpha() {
        let src = || CgImage::from_rgba8(1, 1, vec![9, 9, 9, 255]).unwrap();
        let img = src().with_mask(&grey(1, 1, 8, 1, vec![255], None)).unwrap();
        assert_eq!(img.to_premultiplied().px[0] >> 24, 255);
        let img = src().with_mask(&grey(1, 1, 8, 1, vec![0], None)).unwrap();
        assert_eq!(img.to_premultiplied().px[0] >> 24, 0);
    }

    #[test]
    fn bgra_little_endian_order_and_16_bit() {
        let bgra = CgImage::new(1, 1, 8, 32, 4, ImageColorSpace::DeviceRgb, AlphaInfo::PremultipliedFirst, ByteOrder::Little32, DataProvider::from_vec(vec![30, 20, 10, 255]), None, false, RenderingIntent::Default).unwrap();
        assert_eq!(bgra.to_premultiplied().px[0], pack(10, 20, 30, 255));
        let wide = CgImage::from_rgba16(1, 1, &[0xffff, 0x8000, 0, 0xffff]).unwrap();
        let p = wide.to_premultiplied().px[0].to_le_bytes();
        assert_eq!((p[0], p[2], p[3]), (255, 0, 255));
        assert!((p[1] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn indexed_and_cmyk() {
        let cs = ImageColorSpace::Indexed { base: Box::new(ImageColorSpace::DeviceRgb), palette: Arc::new(vec![1, 2, 3, 200, 100, 50]) };
        let i = CgImage::new(2, 1, 8, 8, 2, cs, AlphaInfo::None, ByteOrder::Default, DataProvider::from_vec(vec![1, 0]), None, false, RenderingIntent::Default).unwrap();
        let p = i.to_premultiplied();
        assert_eq!(p.px[0], pack(200, 100, 50, 255));
        assert_eq!(p.px[1], pack(1, 2, 3, 255));
        let k = CgImage::new(1, 1, 8, 32, 4, ImageColorSpace::DeviceCmyk, AlphaInfo::None, ByteOrder::Default, DataProvider::from_vec(vec![0, 0, 0, 255]), None, false, RenderingIntent::Default).unwrap();
        assert_eq!(k.to_premultiplied().px[0], pack(0, 0, 0, 255));
    }

    #[test]
    fn provider_and_consumer() {
        let p = DataProvider::from_callback(10, |o, n| (o..o + n).map(|v| v as u8).collect());
        assert_eq!(p.bytes_at(8, 5), vec![8, 9]);
        assert_eq!(p.data().len(), 10);
        let m = DataProvider::from_slice(&[1, 2, 3]);
        assert_eq!(m.bytes_at(1, 9), vec![2, 3]);
        let mut c = DataConsumer::in_memory();
        assert_eq!(c.put_bytes(&[1, 2]), 2);
        assert_eq!(c.finish().unwrap(), vec![1, 2]);
    }

    #[test]
    fn copy_with_colour_space_needs_same_components() {
        assert!(rgba_img().copy_with_color_space(ImageColorSpace::DisplayP3).is_some());
        assert!(rgba_img().copy_with_color_space(ImageColorSpace::DeviceGray).is_none());
    }
}
