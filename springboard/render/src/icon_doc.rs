//! Layered app icons: the shell's icon model and renderer (the IconFoundation and IconRendering roles).
//!
//! An app package ships an `icon.json` next to its layer images (PNG). The document has a background (a solid
//! colour or a top-to-bottom gradient) and up to [`MAX_LAYERS`] foreground layers. Each layer is placed by its
//! centre and width as fractions of the icon body, with a blend mode and an opacity. [`render`] composites the
//! layers, masks the result into the continuous-corner rounded square (a superellipse with exponent 5), adds a
//! subtle specular edge and a drop shadow, and returns a premultiplied image at any pixel size.
//!
//! A legacy flat `icon.png` goes through the same path: it becomes a document with one full-body layer on a
//! neutral plate, so every icon is masked the same way. Three appearance variants recolour the layers:
//! `Default` (as drawn), `Dark` (dark background, lightened foreground) and `Tinted` (the foreground's luma
//! mapped onto the person's tint colour over a neutral plate). [`app_icon`] is the one entry point the Dock,
//! the launcher, the switcher and Files use; its results are cached by (app, size, scale, variant).
//!
//! Not yet: SVG layers (the render crate has no SVG rasteriser; layers must be PNG), and the Settings row that
//! sets the style (the shell publishes it with [`set_style`]).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::gfx::{pack, Image};

/// The body's side as a fraction of the icon canvas. The rest is the room the drop shadow needs.
pub const BODY_FRAC: f32 = 0.8;
/// Foreground layers one document may carry.
pub const MAX_LAYERS: usize = 4;
/// The neutral plate behind a legacy flat icon (straight RGB).
pub const PLATE_RGB: [u8; 3] = [236, 236, 238];
/// Strength of the drop shadow (black, alpha at its densest).
const SHADOW_ALPHA: f32 = 0.35;
/// How far the shadow falls below the body, as a fraction of the canvas.
const SHADOW_DROP: f32 = 0.025;
/// Supersampling grid per axis for the body mask (16 samples per pixel).
const SS: usize = 4;
/// Icons kept in the process cache before it is emptied.
const CACHE_CAP: usize = 256;
/// The tint colour the Tinted variant starts with (systemBlue).
pub const DEFAULT_TINT: [u8; 3] = [10, 132, 255];
/// Layer name a legacy icon uses in its synthetic document.
const LEGACY_LAYER: &str = "icon.png";

/// How the icon is drawn for the person's appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Variant {
    Default = 0,
    Dark = 1,
    Tinted = 2,
}

impl Variant {
    /// The variant named by the `icon style` setting (`default`, `dark`, `tinted`); anything else is Default.
    pub fn parse(s: &str) -> Variant {
        match s.trim() {
            "dark" => Variant::Dark,
            "tinted" => Variant::Tinted,
            _ => Variant::Default,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Variant::Default => "default",
            Variant::Dark => "dark",
            Variant::Tinted => "tinted",
        }
    }

    /// The variant with this index (its `as u8`); an unknown index is Default.
    pub fn from_u8(v: u8) -> Variant {
        match v {
            1 => Variant::Dark,
            2 => Variant::Tinted,
            _ => Variant::Default,
        }
    }
}

/// How a layer is composited onto what is below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
}

impl Blend {
    fn parse(s: &str) -> Result<Blend, String> {
        match s {
            "normal" => Ok(Blend::Normal),
            "multiply" => Ok(Blend::Multiply),
            "screen" => Ok(Blend::Screen),
            other => Err(format!("unknown blend mode {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Solid([u8; 4]),
    /// Straight RGBA, `top` at the top edge and `bottom` at the bottom edge.
    Gradient { top: [u8; 4], bottom: [u8; 4] },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Path of the layer image inside the package (relative, no `..`).
    pub image: String,
    /// Centre of the layer, as fractions of the icon body (0.5, 0.5 is the middle).
    pub x: f32,
    pub y: f32,
    /// Width of the layer as a fraction of the icon body (1.0 fills it).
    pub scale: f32,
    pub blend: Blend,
    pub opacity: f32,
}

impl Layer {
    fn full(image: &str) -> Layer {
        Layer { image: image.to_string(), x: 0.5, y: 0.5, scale: 1.0, blend: Blend::Normal, opacity: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconDocument {
    pub background: Background,
    pub layers: Vec<Layer>,
}

impl IconDocument {
    /// Parses an `icon.json`. Missing keys take defaults (a plate background, centred full-size normal layers).
    pub fn parse(text: &str) -> Result<IconDocument, String> {
        let root = json::parse(text).ok_or("icon.json is not valid JSON")?;
        let fields = match &root {
            json::Json::Obj(f) => f,
            _ => return Err("icon.json must be an object".into()),
        };
        let background = match get(fields, "background") {
            None => Background::Solid([PLATE_RGB[0], PLATE_RGB[1], PLATE_RGB[2], 255]),
            Some(json::Json::Obj(b)) => parse_background(b)?,
            Some(_) => return Err("background must be an object".into()),
        };
        let mut layers = Vec::new();
        match get(fields, "layers") {
            None => {}
            Some(json::Json::Arr(items)) => {
                if items.len() > MAX_LAYERS {
                    return Err(format!("an icon has at most {MAX_LAYERS} layers"));
                }
                for item in items {
                    layers.push(parse_layer(item)?);
                }
            }
            Some(_) => return Err("layers must be an array".into()),
        }
        Ok(IconDocument { background, layers })
    }
}

fn get<'a>(fields: &'a [(String, json::Json)], key: &str) -> Option<&'a json::Json> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn parse_background(b: &[(String, json::Json)]) -> Result<Background, String> {
    if let Some(json::Json::Str(s)) = get(b, "solid") {
        return Ok(Background::Solid(parse_hex(s).ok_or("solid must be #rrggbb or #rrggbbaa")?));
    }
    if let Some(json::Json::Arr(a)) = get(b, "gradient") {
        if let [json::Json::Str(top), json::Json::Str(bottom)] = a.as_slice() {
            let top = parse_hex(top).ok_or("gradient colours must be #rrggbb or #rrggbbaa")?;
            let bottom = parse_hex(bottom).ok_or("gradient colours must be #rrggbb or #rrggbbaa")?;
            return Ok(Background::Gradient { top, bottom });
        }
        return Err("gradient must hold exactly two colours".into());
    }
    Err("background needs a solid or a gradient".into())
}

fn parse_layer(j: &json::Json) -> Result<Layer, String> {
    let f = match j {
        json::Json::Obj(f) => f,
        _ => return Err("a layer must be an object".into()),
    };
    let image = match get(f, "image") {
        Some(json::Json::Str(s)) if !s.is_empty() => s.clone(),
        _ => return Err("a layer needs an image path".into()),
    };
    let num = |k: &str, d: f32| match get(f, k) {
        Some(json::Json::Num(v)) => *v as f32,
        _ => d,
    };
    let blend = match get(f, "blend") {
        None => Blend::Normal,
        Some(json::Json::Str(s)) => Blend::parse(s)?,
        Some(_) => return Err("blend must be a string".into()),
    };
    Ok(Layer {
        image,
        x: num("x", 0.5).clamp(-1.0, 2.0),
        y: num("y", 0.5).clamp(-1.0, 2.0),
        scale: num("scale", 1.0).clamp(0.0, 1.5),
        blend,
        opacity: num("opacity", 1.0).clamp(0.0, 1.0),
    })
}

/// `#rrggbb` or `#rrggbbaa`.
fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.strip_prefix('#')?;
    if !h.is_ascii() || !(h.len() == 6 || h.len() == 8) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let a = if h.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, a])
}

/// A parsed icon with its layer images decoded (layer `i` is `doc.layers[i]`).
#[derive(Clone, Debug, PartialEq)]
pub struct IconSource {
    pub doc: IconDocument,
    pub layers: Vec<Image>,
}

impl IconSource {
    /// A legacy flat icon: the picture fills the body on the neutral plate.
    pub fn legacy(img: Image) -> IconSource {
        let background = Background::Solid([PLATE_RGB[0], PLATE_RGB[1], PLATE_RGB[2], 255]);
        IconSource { doc: IconDocument { background, layers: vec![Layer::full(LEGACY_LAYER)] }, layers: vec![img] }
    }

    pub fn render(&self, px: usize, variant: Variant, tint: [u8; 3]) -> Image {
        render(&self.doc, &self.layers, px, variant, tint)
    }
}

/// Reads a package's `icon.json` and its layer PNGs. Layer paths must stay inside the package.
pub fn load_dir(dir: &Path) -> Result<IconSource, String> {
    let text = std::fs::read_to_string(dir.join("icon.json")).map_err(|e| e.to_string())?;
    let doc = IconDocument::parse(&text)?;
    let mut layers = Vec::with_capacity(doc.layers.len());
    for l in &doc.layers {
        let rel = Path::new(&l.image);
        if !rel.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            return Err(format!("layer path {} must stay inside the package", l.image));
        }
        let bytes = std::fs::read(dir.join(rel)).map_err(|e| e.to_string())?;
        layers.push(crate::cg::imageio::decode(&bytes).ok_or_else(|| format!("layer {} is not a PNG", l.image))?);
    }
    Ok(IconSource { doc, layers })
}

/// Loads what `path` names: an `icon.json` (its package directory is read) or a flat image (a legacy icon).
pub fn load_path(path: &Path) -> Option<IconSource> {
    if path.file_name().and_then(|n| n.to_str()) == Some("icon.json") {
        return load_dir(path.parent()?).ok();
    }
    let bytes = std::fs::read(path).ok()?;
    Some(IconSource::legacy(crate::cg::imageio::decode(&bytes)?))
}

/// Coverage (0 to 1, 4x4 supersampled) of pixel (x, y) by the superellipse body whose square starts at (ox, oy)
/// and is `side` px wide. Exponent 5: |u|^5 + |v|^5 <= 1 with u, v in -1..1 from the square's centre.
pub fn body_coverage(x: usize, y: usize, ox: f32, oy: f32, side: f32) -> f32 {
    if side <= 0.0 {
        return 0.0;
    }
    let half = side / 2.0;
    let (cx, cy) = (ox + half, oy + half);
    let mut hits = 0u32;
    for sy in 0..SS {
        for sx in 0..SS {
            let u = (x as f32 + (sx as f32 + 0.5) / SS as f32 - cx) / half;
            let v = (y as f32 + (sy as f32 + 0.5) / SS as f32 - cy) / half;
            if superellipse_inside(u, v) {
                hits += 1;
            }
        }
    }
    hits as f32 / (SS * SS) as f32
}

fn superellipse_inside(u: f32, v: f32) -> bool {
    let (a, b) = (u.abs(), v.abs());
    let (a2, b2) = (a * a, b * b);
    a2 * a2 * a + b2 * b2 * b <= 1.0
}

/// Porter-Duff "over" with a blend mode, on premultiplied RGBA in 0..1.
/// out_c = (1 - sa) dc + (1 - da) sc + sa da B(cb, cs), with straight colours cb, cs and the separable B.
pub fn composite(dst: [f32; 4], src: [f32; 4], blend: Blend) -> [f32; 4] {
    let (sa, da) = (unit(src[3]), unit(dst[3]));
    if sa <= 0.0 {
        return dst;
    }
    let oa = sa + da - sa * da;
    let mut out = [0.0f32; 4];
    out[3] = oa;
    for c in 0..3 {
        let cs = src[c] / sa;
        let cb = if da > 0.0 { dst[c] / da } else { 0.0 };
        let b = match blend {
            Blend::Normal => cs,
            Blend::Multiply => cb * cs,
            Blend::Screen => cb + cs - cb * cs,
        };
        out[c] = (1.0 - sa) * dst[c] + (1.0 - da) * src[c] + sa * da * b;
    }
    out
}

/// The recolour of a background pixel (straight RGB in 0..1) for a variant.
pub fn map_background(rgb: [f32; 3], variant: Variant) -> [f32; 3] {
    match variant {
        Variant::Default => rgb,
        // a near-black plate that keeps a trace of the original hue
        Variant::Dark => mix3(rgb, [28.0 / 255.0, 28.0 / 255.0, 30.0 / 255.0], 0.85),
        // a neutral grey plate, a little lighter where the original is lighter
        Variant::Tinted => {
            let g = 0.10 + 0.18 * luma(rgb);
            [g, g, g]
        }
    }
}

/// The recolour of a foreground pixel (straight RGB in 0..1) for a variant. `tint` is 8-bit RGB.
pub fn map_foreground(rgb: [f32; 3], variant: Variant, tint: [u8; 3]) -> [f32; 3] {
    match variant {
        Variant::Default => rgb,
        Variant::Dark => mix3(rgb, [1.0, 1.0, 1.0], 0.25),
        // grayscale foreground mapped onto the tint: white takes the tint, black stays black
        Variant::Tinted => {
            let l = luma(rgb);
            let t = [tint[0] as f32 / 255.0, tint[1] as f32 / 255.0, tint[2] as f32 / 255.0];
            [t[0] * l, t[1] * l, t[2] * l]
        }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn unit(v: f32) -> f32 {
    v.max(0.0).min(1.0)
}

fn straight_f(c: [u8; 4]) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]
}

fn background_at(bg: &Background, t: f32) -> [f32; 4] {
    match bg {
        Background::Solid(c) => straight_f(*c),
        Background::Gradient { top, bottom } => {
            let (a, b) = (straight_f(*top), straight_f(*bottom));
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        }
    }
}

fn premul(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

fn unpremul(p: [f32; 4]) -> [f32; 4] {
    if p[3] <= 0.0 {
        return [0.0; 4];
    }
    [p[0] / p[3], p[1] / p[3], p[2] / p[3], p[3]]
}

fn unpack(p: u32) -> [f32; 4] {
    [(p & 255) as f32 / 255.0, ((p >> 8) & 255) as f32 / 255.0, ((p >> 16) & 255) as f32 / 255.0, (p >> 24) as f32 / 255.0]
}

fn pack_premul(p: [f32; 4]) -> u32 {
    let a = unit(p[3]);
    let c = |v: f32| (unit(v).min(a) * 255.0).round() as u8;
    pack(c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8)
}

/// Separable box blur, `r` px each side, clamped at the edges. Two passes of this approximate a Gaussian.
fn box_blur(src: &[f32], n: usize, r: usize) -> Vec<f32> {
    let k = 1.0 / (2 * r + 1) as f32;
    let at = |v: isize| v.clamp(0, n as isize - 1) as usize;
    let mut tmp = vec![0.0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            let s: f32 = (0..=2 * r).map(|d| src[y * n + at(x as isize + d as isize - r as isize)]).sum();
            tmp[y * n + x] = s * k;
        }
    }
    let mut out = vec![0.0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            let s: f32 = (0..=2 * r).map(|d| tmp[at(y as isize + d as isize - r as isize) * n + x]).sum();
            out[y * n + x] = s * k;
        }
    }
    out
}

/// Composites one layer onto the canvas (premultiplied, `n` px square). The layer's centre and width are
/// fractions of the body, which starts at `origin` and is `body` px wide. Samples are nearest-neighbour.
fn draw_layer(canvas: &mut [[f32; 4]], n: usize, body: f32, origin: f32, layer: &Layer, img: &Image, variant: Variant, tint: [u8; 3]) {
    if img.w == 0 || img.h == 0 || img.px.len() < img.w * img.h || layer.scale <= 0.0 {
        return;
    }
    let lw = layer.scale * body;
    let lh = lw * img.h as f32 / img.w as f32;
    let left = origin + layer.x * body - lw / 2.0;
    let top = origin + layer.y * body - lh / 2.0;
    let x0 = left.floor().max(0.0) as usize;
    let y0 = top.floor().max(0.0) as usize;
    let x1 = ((left + lw).ceil().max(0.0) as usize).min(n);
    let y1 = ((top + lh).ceil().max(0.0) as usize).min(n);
    for y in y0..y1 {
        for x in x0..x1 {
            let sx = (((x as f32 + 0.5 - left) / lw * img.w as f32) as isize).clamp(0, img.w as isize - 1) as usize;
            let sy = (((y as f32 + 0.5 - top) / lh * img.h as f32) as isize).clamp(0, img.h as isize - 1) as usize;
            let s = unpremul(unpack(img.px[sy * img.w + sx]));
            if s[3] <= 0.0 {
                continue;
            }
            let m = map_foreground([s[0], s[1], s[2]], variant, tint);
            let src = premul([m[0], m[1], m[2], unit(s[3] * layer.opacity)]);
            let i = y * n + x;
            canvas[i] = composite(canvas[i], src, layer.blend);
        }
    }
}

/// Renders a document at `px` x `px`: background, layers, the body mask, the specular edge and the drop shadow.
/// The body fills [`BODY_FRAC`] of the canvas, centred, so the shadow has room. Output is premultiplied.
pub fn render(doc: &IconDocument, layers: &[Image], px: usize, variant: Variant, tint: [u8; 3]) -> Image {
    if px == 0 {
        return Image { w: 0, h: 0, px: Vec::new() };
    }
    let n = px;
    let nf = n as f32;
    let body = nf * BODY_FRAC;
    let origin = (nf - body) / 2.0;

    let mut canvas = vec![[0.0f32; 4]; n * n];
    for y in 0..n {
        let bg = background_at(&doc.background, (y as f32 + 0.5) / nf);
        let m = map_background([bg[0], bg[1], bg[2]], variant);
        let row = premul([m[0], m[1], m[2], bg[3]]);
        canvas[y * n..(y + 1) * n].iter_mut().for_each(|p| *p = row);
    }
    for (layer, img) in doc.layers.iter().zip(layers).take(MAX_LAYERS) {
        draw_layer(&mut canvas, n, body, origin, layer, img, variant, tint);
    }

    // the body mask
    let cov: Vec<f32> = (0..n * n).map(|i| body_coverage(i % n, i / n, origin, origin, body)).collect();
    for (p, c) in canvas.iter_mut().zip(&cov) {
        p.iter_mut().for_each(|ch| *ch *= c);
    }
    // a subtle specular edge: the ring one px inside the body edge, brighter at the top
    let inner_side = (body - 2.0).max(1.0);
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let edge = (cov[i] - body_coverage(x, y, origin + 1.0, origin + 1.0, inner_side)).max(0.0);
            if edge > 0.0 {
                let a = edge * 0.3 * (1.0 - 0.5 * (y as f32 + 0.5) / nf);
                let p = canvas[i];
                canvas[i] = [a + p[0] * (1.0 - a), a + p[1] * (1.0 - a), a + p[2] * (1.0 - a), a + p[3] * (1.0 - a)];
            }
        }
    }
    // the drop shadow: the body's coverage moved down, blurred, under the icon
    let drop = nf * SHADOW_DROP;
    let raw: Vec<f32> = (0..n * n).map(|i| body_coverage(i % n, i / n, origin, origin + drop, body)).collect();
    let r = (n / 40).max(1);
    let shadow = box_blur(&box_blur(&raw, n, r), n, r);
    let px_out = canvas
        .iter()
        .zip(&shadow)
        .map(|(p, sh)| {
            let sa = SHADOW_ALPHA * sh * (1.0 - unit(p[3]));
            pack_premul([p[0], p[1], p[2], p[3] + sa])
        })
        .collect();
    Image { w: n, h: n, px: px_out }
}

/// What an icon is drawn for: the app, its size in points, the scale, and the appearance variant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconKey {
    pub app: String,
    /// Size in points times 10 (so 29.0 pt is 290).
    pub size_pt10: u32,
    /// Scale times 1000 (so 2x is 2000).
    pub scale_milli: u32,
    pub variant: Variant,
}

impl IconKey {
    pub fn new(app: &str, size_pt: f32, scale: f32, variant: Variant) -> IconKey {
        IconKey {
            app: app.to_string(),
            size_pt10: (size_pt * 10.0).round().max(0.0) as u32,
            scale_milli: (scale * 1000.0).round().max(0.0) as u32,
            variant,
        }
    }

    /// The canvas side in device pixels.
    pub fn px(&self) -> usize {
        ((self.size_pt10 as f32 / 10.0) * (self.scale_milli as f32 / 1000.0)).round() as usize
    }
}

/// Rendered icons, at most `cap` of them. A full cache is emptied rather than evicted entry by entry.
pub struct IconCache {
    map: HashMap<IconKey, Arc<Image>>,
    cap: usize,
}

impl IconCache {
    pub fn new(cap: usize) -> IconCache {
        IconCache { map: HashMap::new(), cap: cap.max(1) }
    }

    /// The cached icon for `key`, or the result of `make` (not cached when it returns None).
    pub fn get_or_try_insert_with(&mut self, key: IconKey, make: impl FnOnce() -> Option<Image>) -> Option<Arc<Image>> {
        if let Some(hit) = self.map.get(&key) {
            return Some(Arc::clone(hit));
        }
        let img = Arc::new(make()?);
        if self.map.len() >= self.cap {
            self.map.clear();
        }
        self.map.insert(key, Arc::clone(&img));
        Some(img)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Drops every size, scale and variant of `app`'s icon (after it was installed or updated).
    pub fn invalidate_app(&mut self, app: &str) {
        self.map.retain(|k, _| k.app != app);
    }
}

static VARIANT: AtomicU8 = AtomicU8::new(Variant::Default as u8);
static TINT: AtomicU32 = AtomicU32::new(0);
/// Counts the style changes, so the shell can tell that its icons are stale without comparing them.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();

fn cache() -> &'static Mutex<IconCache> {
    CACHE.get_or_init(|| Mutex::new(IconCache::new(CACHE_CAP)))
}

fn pack_tint(t: [u8; 3]) -> u32 {
    u32::from_le_bytes([t[0], t[1], t[2], 0])
}

/// The appearance the icons are drawn in: the variant and the tint colour (8-bit RGB).
pub fn current_style() -> (Variant, [u8; 3]) {
    let t = TINT.load(Ordering::Relaxed).to_le_bytes();
    (Variant::from_u8(VARIANT.load(Ordering::Relaxed)), [t[0], t[1], t[2]])
}

/// Publishes the style the shell chose in Settings. A change empties the icon cache, so the next draw is new.
pub fn set_style(variant: Variant, tint: [u8; 3]) {
    let v_changed = VARIANT.swap(variant as u8, Ordering::Relaxed) != variant as u8;
    let t_changed = TINT.swap(pack_tint(tint), Ordering::Relaxed) != pack_tint(tint);
    if v_changed || t_changed {
        GENERATION.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut c) = cache().lock() {
            c.clear();
        }
    }
}

/// How many times the style has changed since start (see [`set_style`]).
pub fn style_generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Forgets every cached icon. A process that sees its apps change (the Dock client) calls this.
pub fn clear_cache() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

/// Forgets the cached icons of `app` (its id as the shell names it). Called when the app is installed or updated.
pub fn invalidate_app(app: &str) {
    if let Ok(mut c) = cache().lock() {
        c.invalidate_app(app);
    }
}

/// The one entry point for an app's icon: the package at `path` (its `icon.json`, or a flat `icon.png`) drawn at
/// `size_pt` x `scale` device pixels in `variant`. Cached by [`IconKey`]. None when the path holds no icon.
pub fn app_icon(app: &str, path: &Path, size_pt: f32, scale: f32, variant: Variant) -> Option<Arc<Image>> {
    let key = IconKey::new(app, size_pt, scale, variant);
    let px = key.px();
    if px == 0 {
        return None;
    }
    let tint = current_style().1;
    let mut c = cache().lock().unwrap_or_else(|p| p.into_inner());
    c.get_or_try_insert_with(key, || Some(load_path(path)?.render(px, variant, tint)))
}

/// Renders an icon compiled into the binary: its `icon.json` text and its layer PNGs, keyed by the layer path.
/// Uses the current style. None when the document is bad or a layer is missing or not a PNG.
pub fn render_builtin(json: &str, layers: &[(&str, &[u8])], px: usize) -> Option<Image> {
    let doc = IconDocument::parse(json).ok()?;
    let mut decoded = Vec::with_capacity(doc.layers.len());
    for l in &doc.layers {
        let bytes = layers.iter().find(|(name, _)| *name == l.image)?.1;
        decoded.push(crate::cg::imageio::decode(bytes)?);
    }
    let (variant, tint) = current_style();
    Some(render(&doc, &decoded, px, variant, tint))
}

/// Minimal JSON reader for `icon.json` (objects, arrays, strings, finite numbers, booleans, null).
mod json {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Json {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    const MAX_DEPTH: usize = 16;

    pub fn parse(text: &str) -> Option<Json> {
        let mut p = P { b: text.as_bytes(), i: 0, depth: 0 };
        let v = p.value()?;
        p.ws();
        (p.i == p.b.len()).then_some(v)
    }

    struct P<'a> {
        b: &'a [u8],
        i: usize,
        depth: usize,
    }

    impl P<'_> {
        fn peek(&self) -> Option<u8> {
            self.b.get(self.i).copied()
        }

        fn ws(&mut self) {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
        }

        fn eat(&mut self, c: u8) -> bool {
            if self.peek() == Some(c) {
                self.i += 1;
                true
            } else {
                false
            }
        }

        fn lit(&mut self, word: &str, v: Json) -> Option<Json> {
            if self.b[self.i..].starts_with(word.as_bytes()) {
                self.i += word.len();
                Some(v)
            } else {
                None
            }
        }

        fn value(&mut self) -> Option<Json> {
            self.ws();
            match self.peek()? {
                b'{' => self.object(),
                b'[' => self.array(),
                b'"' => self.string().map(Json::Str),
                b't' => self.lit("true", Json::Bool(true)),
                b'f' => self.lit("false", Json::Bool(false)),
                b'n' => self.lit("null", Json::Null),
                b'-' | b'0'..=b'9' => self.number(),
                _ => None,
            }
        }

        fn enter(&mut self) -> Option<()> {
            self.depth += 1;
            (self.depth <= MAX_DEPTH).then_some(())
        }

        fn object(&mut self) -> Option<Json> {
            self.enter()?;
            self.i += 1;
            let mut fields = Vec::new();
            self.ws();
            if !self.eat(b'}') {
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b':') {
                        return None;
                    }
                    let v = self.value()?;
                    fields.push((key, v));
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        break;
                    }
                    return None;
                }
            }
            self.depth -= 1;
            Some(Json::Obj(fields))
        }

        fn array(&mut self) -> Option<Json> {
            self.enter()?;
            self.i += 1;
            let mut items = Vec::new();
            self.ws();
            if !self.eat(b']') {
                loop {
                    items.push(self.value()?);
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        break;
                    }
                    return None;
                }
            }
            self.depth -= 1;
            Some(Json::Arr(items))
        }

        fn string(&mut self) -> Option<String> {
            if !self.eat(b'"') {
                return None;
            }
            let mut out = Vec::new();
            loop {
                let c = self.peek()?;
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let e = self.peek()?;
                        self.i += 1;
                        match e {
                            b'"' | b'\\' | b'/' => out.push(e),
                            b'n' => out.push(b'\n'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let hex = self.b.get(self.i..self.i + 4)?;
                                let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                                self.i += 4;
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(char::from_u32(code)?.encode_utf8(&mut buf).as_bytes());
                            }
                            _ => return None,
                        }
                    }
                    c if c < 0x20 => return None,
                    c => out.push(c),
                }
            }
            String::from_utf8(out).ok()
        }

        fn number(&mut self) -> Option<Json> {
            let start = self.i;
            while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                self.i += 1;
            }
            let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
            let v: f64 = s.parse().ok()?;
            v.is_finite().then_some(Json::Num(v))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, p: u32) -> Image {
        Image { w, h, px: vec![p; w * h] }
    }

    fn at(img: &Image, x: usize, y: usize) -> u32 {
        img.px[y * img.w + x]
    }

    #[test]
    fn superellipse_fills_the_square_by_the_analytic_area() {
        // the area of |u|^5 + |v|^5 <= 1 is 4 * G(1.2)^2 / G(1.4) = 3.8015, so about 0.9502 of its square
        for side in [32usize, 64, 128, 256] {
            let s = side as f32;
            let sum: f32 = (0..side).flat_map(|y| (0..side).map(move |x| (x, y))).map(|(x, y)| body_coverage(x, y, 0.0, 0.0, s)).sum();
            let ratio = sum / (s * s);
            assert!((ratio - 0.9502).abs() < 0.01, "side {side}: ratio {ratio}");
        }
    }

    #[test]
    fn superellipse_mask_covers_the_middle_and_leaves_the_corners_empty() {
        for side in [16usize, 64, 200] {
            let s = side as f32;
            assert_eq!(body_coverage(0, 0, 0.0, 0.0, s), 0.0, "corner at {side}");
            assert_eq!(body_coverage(side - 1, side - 1, 0.0, 0.0, s), 0.0, "far corner at {side}");
            assert_eq!(body_coverage(side / 2, side / 2, 0.0, 0.0, s), 1.0, "centre at {side}");
            assert!(body_coverage(side / 2, 0, 0.0, 0.0, s) >= 0.5, "top edge middle at {side}");
        }
        assert_eq!(body_coverage(0, 0, 0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn normal_over_replaces_and_transparent_source_leaves_the_destination() {
        let dst = [0.2, 0.2, 0.2, 0.5];
        assert_eq!(composite(dst, [1.0, 0.0, 0.0, 1.0], Blend::Normal), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(composite(dst, [0.0; 4], Blend::Multiply), dst);
    }

    #[test]
    fn multiply_and_screen_follow_their_formulas() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        let black = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(composite(red, white, Blend::Multiply), red);
        assert_eq!(composite(red, black, Blend::Screen), red);
        assert_eq!(composite([0.0, 0.0, 0.0, 1.0], white, Blend::Screen), white);
        // half-transparent red over opaque white in multiply: red = 0.5 * 1 + 0.5 * (1 * 1) = 1.0,
        // green = 0.5 * 1 + 0.5 * (1 * 0) = 0.5
        let half_red = [0.5, 0.0, 0.0, 0.5];
        let out = composite([1.0, 1.0, 1.0, 1.0], half_red, Blend::Multiply);
        assert!((out[0] - 1.0).abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6 && out[3] == 1.0);
    }

    #[test]
    fn dark_variant_darkens_the_background_and_lightens_the_foreground() {
        let white = map_background([1.0; 3], Variant::Dark);
        assert!(luma(white) < 0.3, "dark plate from white: {white:?}");
        let black_fg = map_foreground([0.0; 3], Variant::Dark, DEFAULT_TINT);
        assert!(luma(black_fg) > 0.2, "lightened foreground from black: {black_fg:?}");
        assert_eq!(map_foreground([0.4, 0.5, 0.6], Variant::Default, DEFAULT_TINT), [0.4, 0.5, 0.6]);
    }

    #[test]
    fn tinted_variant_maps_the_foreground_luma_onto_the_tint() {
        let tint = [10u8, 132, 255];
        let white = map_foreground([1.0; 3], Variant::Tinted, tint);
        assert!((white[0] - 10.0 / 255.0).abs() < 1e-6 && (white[2] - 1.0).abs() < 1e-6);
        assert_eq!(map_foreground([0.0; 3], Variant::Tinted, tint), [0.0; 3]);
        let bg = map_background([0.9, 0.1, 0.1], Variant::Tinted);
        assert!(bg[0] == bg[1] && bg[1] == bg[2], "tinted plate is neutral grey: {bg:?}");
    }

    #[test]
    fn cache_keys_separate_app_size_scale_and_variant() {
        let a = IconKey::new("is.olibuijr.term", 29.0, 2.0, Variant::Default);
        assert_eq!(a, IconKey::new("is.olibuijr.term", 29.0, 2.0, Variant::Default));
        assert_eq!(a.px(), 58);
        assert_eq!(IconKey::new("is.olibuijr.term", 48.0, 1.5625, Variant::Default).px(), 75);
        assert_ne!(a, IconKey::new("bifrost.files", 29.0, 2.0, Variant::Default));
        assert_ne!(a, IconKey::new("is.olibuijr.term", 29.0, 1.0, Variant::Default));
        assert_ne!(a, IconKey::new("is.olibuijr.term", 29.0, 2.0, Variant::Dark));
    }

    #[test]
    fn invalidating_an_app_drops_only_its_icons() {
        let mut c = IconCache::new(8);
        for (app, size) in [("a", 29.0), ("a", 58.0), ("b", 29.0)] {
            c.get_or_try_insert_with(IconKey::new(app, size, 1.0, Variant::Default), || Some(solid(1, 1, 0)));
        }
        c.invalidate_app("a");
        assert_eq!(c.len(), 1);
        c.get_or_try_insert_with(IconKey::new("b", 29.0, 1.0, Variant::Default), || None);
        assert_eq!(c.len(), 1, "b's icon is still cached");
    }

    #[test]
    fn style_changes_bump_the_generation_only_when_they_change() {
        let before = style_generation();
        set_style(Variant::Dark, DEFAULT_TINT);
        let after = style_generation();
        assert!(after > before);
        set_style(Variant::Dark, DEFAULT_TINT);
        assert_eq!(style_generation(), after, "the same style is not a change");
        set_style(Variant::Default, DEFAULT_TINT);
    }

    #[test]
    fn cache_returns_the_same_image_and_empties_when_full() {
        let mut c = IconCache::new(2);
        let key = IconKey::new("app", 10.0, 1.0, Variant::Default);
        let mut calls = 0;
        let first = c.get_or_try_insert_with(key.clone(), || {
            calls += 1;
            Some(solid(1, 1, 0))
        });
        let second = c.get_or_try_insert_with(key, || {
            calls += 1;
            Some(solid(1, 1, 0))
        });
        assert_eq!(calls, 1);
        assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
        c.get_or_try_insert_with(IconKey::new("b", 10.0, 1.0, Variant::Default), || Some(solid(1, 1, 0)));
        c.get_or_try_insert_with(IconKey::new("c", 10.0, 1.0, Variant::Default), || Some(solid(1, 1, 0)));
        assert_eq!(c.len(), 1, "a full cache starts again");
        assert!(c.get_or_try_insert_with(IconKey::new("d", 1.0, 1.0, Variant::Default), || None).is_none());
        assert_eq!(c.len(), 1, "a failed render is not cached");
    }

    #[test]
    fn legacy_icon_is_masked_into_the_body_on_the_plate() {
        let red = pack(255, 0, 0, 255);
        let icon = IconSource::legacy(solid(4, 4, red));
        let img = icon.render(64, Variant::Default, DEFAULT_TINT);
        assert_eq!((img.w, img.h, img.px.len()), (64, 64, 64 * 64));
        assert_eq!(at(&img, 0, 0) >> 24, 0, "corner outside the squircle is transparent");
        assert_eq!(at(&img, 32, 32), red, "the picture fills the body");
    }

    #[test]
    fn transparent_legacy_icon_shows_the_neutral_plate() {
        let icon = IconSource::legacy(solid(2, 2, 0));
        let img = icon.render(64, Variant::Default, DEFAULT_TINT);
        let c = unpack(at(&img, 32, 32));
        assert!((c[0] * 255.0 - PLATE_RGB[0] as f32).abs() <= 1.0, "plate red: {c:?}");
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn drop_shadow_falls_below_the_body_and_not_above_it() {
        let img = IconSource::legacy(solid(2, 2, pack(255, 255, 255, 255))).render(64, Variant::Default, DEFAULT_TINT);
        // the body starts at 6.4 px and ends at 57.6 px; the shadow reaches 58 px and not row 0
        assert_eq!(at(&img, 32, 0) >> 24, 0, "nothing above the body");
        assert!(at(&img, 32, 58) >> 24 > 0, "shadow under the body");
    }

    #[test]
    fn builtin_terminal_icon_draws_its_prompt_on_the_tile() {
        let json = include_str!("../../res/icons/terminal/icon.json");
        let layers: [(&str, &[u8]); 1] = [("prompt.png", include_bytes!("../../res/icons/terminal/prompt.png"))];
        for px in [32usize, 100, 180] {
            let img = render_builtin(json, &layers, px).expect("terminal icon renders");
            assert_eq!((img.w, img.h), (px, px));
            assert_eq!(at(&img, 0, 0) >> 24, 0, "corner outside the squircle at {px}");
        }
        // the prompt's vertex is the canvas centre, so the centre is white ink on the dark tile
        let img = render_builtin(json, &layers, 180).unwrap();
        let c = at(&img, 90, 90);
        assert_eq!(c >> 24, 255);
        assert!(c & 255 > 200, "ink red channel at the vertex: {c:#x}");
        assert!(render_builtin(json, &[], 32).is_none(), "a missing layer is refused");
    }

    #[test]
    fn zero_size_renders_nothing() {
        let img = render(&IconDocument { background: Background::Solid([0; 4]), layers: vec![] }, &[], 0, Variant::Default, DEFAULT_TINT);
        assert!(img.px.is_empty());
    }

    #[test]
    fn document_parses_layers_background_and_defaults() {
        let text = r##"{"background": {"gradient": ["#2c2c30", "#0e0e10"]},
            "layers": [{"image": "prompt.png", "scale": 0.6, "blend": "screen", "opacity": 0.5},
                       {"image": "dot.png"}]}"##;
        let doc = IconDocument::parse(text).unwrap();
        assert_eq!(doc.background, Background::Gradient { top: [0x2c, 0x2c, 0x30, 255], bottom: [0x0e, 0x0e, 0x10, 255] });
        assert_eq!(doc.layers.len(), 2);
        assert_eq!(doc.layers[0].blend, Blend::Screen);
        assert_eq!((doc.layers[0].x, doc.layers[0].scale, doc.layers[0].opacity), (0.5, 0.6, 0.5));
        assert_eq!((doc.layers[1].blend, doc.layers[1].opacity, doc.layers[1].scale), (Blend::Normal, 1.0, 1.0));
        assert!(IconDocument::parse("{}").unwrap().layers.is_empty());
    }

    #[test]
    fn document_refuses_bad_input() {
        assert!(IconDocument::parse("not json").is_err());
        assert!(IconDocument::parse("[]").is_err());
        assert!(IconDocument::parse(r##"{"background": {"solid": "red"}}"##).is_err());
        assert!(IconDocument::parse(r#"{"layers": [{"image": "a.png", "blend": "difference"}]}"#).is_err());
        let five = r#"{"layers": [{"image":"a"},{"image":"a"},{"image":"a"},{"image":"a"},{"image":"a"}]}"#;
        assert!(IconDocument::parse(five).is_err(), "at most four layers");
        assert!(json::parse(r#"{"a": [1, 2.5e1, true, null, "xA"]}"#).is_some());
        assert!(json::parse(r#"{"a": 1,}"#).is_none());
    }

    #[test]
    fn hex_colours_take_six_or_eight_digits() {
        assert_eq!(parse_hex("#102030"), Some([0x10, 0x20, 0x30, 255]));
        assert_eq!(parse_hex("#10203080"), Some([0x10, 0x20, 0x30, 0x80]));
        assert_eq!(parse_hex("102030"), None);
        assert_eq!(parse_hex("#1020"), None);
    }
}
