//! The PDF page context: a drawing context whose output is a PDF file instead of pixels (the reference platform's
//! PDF context, `CGPDFContextCreate` / `CGPDFContextBeginPage` / `CGPDFContextEndPage` / `CGPDFContextClose`).
//!
//! Drawing happens between [`PdfContext::begin_page`] and [`PdfContext::end_page`]; [`PdfContext::close`] ends an
//! open page and returns the file. Every page starts with a fresh graphics state whose user space is the PDF default
//! user space of the page: points, origin at the bottom left of the media box, y up. Drawing outside a page is
//! ignored, as on the reference platform.
//!
//! What is written as PDF vector content: paths (lines, quadratic and cubic curves; quadratics become cubics),
//! fills under the non-zero and even-odd rules, strokes with width, caps, joins, miter limit and dash, clipping to
//! paths and rectangles, the transform (`q`/`Q` and `cm`), grey, RGB and CMYK colours, alpha and blend modes
//! (`/ExtGState`), linear and radial gradients and axial and radial shadings (`sh`, or as the paint of a fill or a
//! stroke through a clip), raster images (RGB with an `/SMask` for alpha), text in the twelve standard Latin faces
//! (WinAnsi, with font size, character spacing, text matrix and the fill or invisible drawing mode), marked content
//! (`BMC`/`EMC`), link annotations to a URL or to a named destination, named destinations, an outline, the document
//! information dictionary and an XML metadata stream. A pattern paint has no vector form here: it is drawn as an
//! image of the pattern, clipped to the shape.
//!
//! The page boxes follow the context's page dictionary keys: a media box (the default comes from the context), and
//! optional crop, bleed, trim and art boxes.
//!
//! Not written: encryption and passwords, linearized files, PDF/A and output intents, a tagged structure tree, and
//! shadows (the PDF imaging model has none).

use super::obj::{fmt_num, write_name, Obj};
use super::writer::{text_string, winansi};
use crate::cg::color::{Color, ColorModel};
use crate::cg::context::{BlendMode, PathDrawing, Rect, TextDrawing};
use crate::cg::paint::{LinearGradient, Paint, RadialGradient, Shading, ShadingKind};
use crate::cg::path::{Affine, Element, Path, Rect as PathRect};
use crate::cg::pdf::standard::{Family, Standard};
use crate::cg::raster::{FillRule, LineCap, LineJoin, StrokeStyle};
use crate::gfx::Image;

/// The page size a context uses when none is given: US Letter, 612 x 792 points, as on the reference platform.
pub const DEFAULT_MEDIA: Rect = Rect { x: 0.0, y: 0.0, w: 612.0, h: 792.0 };

/// The longest side, in pixels, of the image a pattern fill is drawn as.
const MAX_PATTERN_SIDE: usize = 2048;

/// The number of colour samples a gradient's function gets between its two ends (stop locations are added on top).
const RAMP_SAMPLES: usize = 32;

/// The document information dictionary (`kCGPDFContextTitle`, `Author`, `Subject`, `Keywords`, `Creator`). Empty
/// fields are left out of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfInfo {
    pub title: String,
    pub author: String,
    pub subject: String,
    /// One string, as the reference platform writes an array of keywords joined.
    pub keywords: String,
    pub creator: String,
}

/// The boxes of one page (`kCGPDFContextMediaBox`, `CropBox`, `BleedBox`, `TrimBox`, `ArtBox`), in points.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PageBoxes {
    pub media: Rect,
    pub crop: Option<Rect>,
    pub bleed: Option<Rect>,
    pub trim: Option<Rect>,
    pub art: Option<Rect>,
}

impl PageBoxes {
    /// A page with only a media box.
    pub fn media(media: Rect) -> PageBoxes {
        PageBoxes { media, ..PageBoxes::default() }
    }
}

/// The twelve standard Latin faces text can be drawn in. Text is written in WinAnsi, so characters outside it come
/// out as `?`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum StandardFace {
    #[default]
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
}

impl StandardFace {
    pub const ALL: [StandardFace; 12] = [
        StandardFace::Helvetica,
        StandardFace::HelveticaBold,
        StandardFace::HelveticaOblique,
        StandardFace::HelveticaBoldOblique,
        StandardFace::TimesRoman,
        StandardFace::TimesBold,
        StandardFace::TimesItalic,
        StandardFace::TimesBoldItalic,
        StandardFace::Courier,
        StandardFace::CourierBold,
        StandardFace::CourierOblique,
        StandardFace::CourierBoldOblique,
    ];

    /// The standard font name written as `/BaseFont`.
    pub fn base_font(self) -> &'static str {
        match self {
            StandardFace::Helvetica => "Helvetica",
            StandardFace::HelveticaBold => "Helvetica-Bold",
            StandardFace::HelveticaOblique => "Helvetica-Oblique",
            StandardFace::HelveticaBoldOblique => "Helvetica-BoldOblique",
            StandardFace::TimesRoman => "Times-Roman",
            StandardFace::TimesBold => "Times-Bold",
            StandardFace::TimesItalic => "Times-Italic",
            StandardFace::TimesBoldItalic => "Times-BoldItalic",
            StandardFace::Courier => "Courier",
            StandardFace::CourierBold => "Courier-Bold",
            StandardFace::CourierOblique => "Courier-Oblique",
            StandardFace::CourierBoldOblique => "Courier-BoldOblique",
        }
    }

    fn index(self) -> usize {
        StandardFace::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    fn metrics(self) -> Standard {
        let i = self.index();
        let family = match i / 4 {
            0 => Family::Sans,
            1 => Family::Serif,
            _ => Family::Mono,
        };
        Standard { family, bold: i % 4 == 1 || i % 4 == 3, italic: i % 4 >= 2 }
    }

    /// The advance of `text` at `size` points, from the standard metrics (1/1000 em per glyph).
    pub fn width(self, text: &str, size: f32) -> f32 {
        let m = self.metrics();
        let units: f32 = text
            .chars()
            .map(|c| {
                let shown = if winansi(c) == b'?' && c != '?' { '?' } else { c };
                let mut buf = [0u8; 4];
                m.width(shown.encode_utf8(&mut buf)).unwrap_or_else(|| m.default_width())
            })
            .sum();
        units * size / 1000.0
    }
}

/// One outline (bookmark) entry (`CGPDFContextSetOutline`): its title, the zero based page it opens, the top of the
/// view on that page in default user space (`None` keeps the reader's position), and the entries under it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PdfOutlineItem {
    pub title: String,
    pub page: usize,
    pub top: Option<f32>,
    pub children: Vec<PdfOutlineItem>,
}

#[derive(Clone, Debug, PartialEq)]
enum LinkTarget {
    Url(String),
    Named(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Link {
    /// page space: x0 y0 x1 y1
    rect: [f32; 4],
    target: LinkTarget,
}

struct PageOut {
    boxes: PageBoxes,
    content: String,
    links: Vec<Link>,
}

#[derive(Clone)]
struct State {
    ctm: Affine,
    fill: Paint,
    stroke: Paint,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
    dash: Vec<f32>,
    dash_phase: f32,
    flatness: f32,
    alpha: f32,
    blend: BlendMode,
    face: StandardFace,
    font_size: f32,
    char_spacing: f32,
    text_mode: TextDrawing,
}

impl State {
    fn new() -> State {
        State {
            ctm: Affine::IDENTITY,
            fill: Paint::Solid(Color::black()),
            stroke: Paint::Solid(Color::black()),
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            flatness: 0.0,
            alpha: 1.0,
            blend: BlendMode::Normal,
            face: StandardFace::Helvetica,
            font_size: 12.0,
            char_spacing: 0.0,
            text_mode: TextDrawing::Fill,
        }
    }

    fn style(&self) -> StrokeStyle {
        StrokeStyle {
            width: self.line_width,
            cap: self.cap,
            join: self.join,
            miter_limit: self.miter_limit,
            dash: self.dash.clone(),
            dash_phase: self.dash_phase,
        }
    }
}

/// An image XObject: RGB rows top to bottom, and an alpha plane when any pixel is not opaque.
struct ImageOut {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

/// A drawing context that writes a PDF file. See the module documentation.
pub struct PdfContext {
    info: PdfInfo,
    default_media: Rect,
    pages: Vec<PageOut>,
    page: Option<PageOut>,
    gs: State,
    stack: Vec<State>,
    /// the current path, page space
    path: Path,
    /// text space to user space; its translation is the text position
    tm: Affine,
    images: Vec<ImageOut>,
    /// shading dictionaries, written inline into the resources
    shadings: Vec<String>,
    /// (fill alpha, stroke alpha, blend mode)
    ext: Vec<(f32, f32, BlendMode)>,
    fonts: [bool; 12],
    /// (name, page index, x, y in page space)
    dests: Vec<(String, usize, f32, f32)>,
    outline: Vec<PdfOutlineItem>,
    metadata: Option<Vec<u8>>,
    marked_depth: usize,
}

fn n(v: f32) -> String {
    fmt_num(v as f64)
}

fn cm(t: &Affine) -> String {
    format!("{} {} {} {} {} {} cm ", n(t.a), n(t.b), n(t.c), n(t.d), n(t.tx), n(t.ty))
}

/// The path as PDF path construction operators (each followed by a space); quadratic curves become cubic ones.
fn path_ops(path: &Path) -> String {
    let mut s = String::new();
    let mut cur = (0.0f32, 0.0f32);
    let mut start = cur;
    for e in path.elements() {
        match *e {
            Element::Move(p) => {
                s.push_str(&format!("{} {} m ", n(p.0), n(p.1)));
                cur = p;
                start = p;
            }
            Element::Line(p) => {
                s.push_str(&format!("{} {} l ", n(p.0), n(p.1)));
                cur = p;
            }
            Element::Quad(c, p) => {
                let c1 = (cur.0 + 2.0 / 3.0 * (c.0 - cur.0), cur.1 + 2.0 / 3.0 * (c.1 - cur.1));
                let c2 = (p.0 + 2.0 / 3.0 * (c.0 - p.0), p.1 + 2.0 / 3.0 * (c.1 - p.1));
                s.push_str(&format!("{} {} {} {} {} {} c ", n(c1.0), n(c1.1), n(c2.0), n(c2.1), n(p.0), n(p.1)));
                cur = p;
            }
            Element::Cubic(a, b, p) => {
                s.push_str(&format!("{} {} {} {} {} {} c ", n(a.0), n(a.1), n(b.0), n(b.1), n(p.0), n(p.1)));
                cur = p;
            }
            Element::Close => {
                s.push_str("h ");
                cur = start;
            }
        }
    }
    s
}

/// The colour operator for a solid colour: grey (`g`/`G`), CMYK (`k`/`K`) or RGB (`rg`/`RG`). Other spaces are
/// converted to sRGB.
fn color_op(c: &Color, stroke: bool) -> String {
    let comps = c.components();
    match c.space().model() {
        ColorModel::Monochrome if !comps.is_empty() => {
            format!("{} {} ", n(comps[0].clamp(0.0, 1.0)), if stroke { "G" } else { "g" })
        }
        ColorModel::Cmyk if comps.len() >= 4 => {
            let [c0, m, y, k] = [comps[0], comps[1], comps[2], comps[3]].map(|v| n(v.clamp(0.0, 1.0)));
            format!("{c0} {m} {y} {k} {} ", if stroke { "K" } else { "k" })
        }
        _ => {
            let [r, g, b, _] = c.to_srgb().unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let [r, g, b] = [r, g, b].map(|v| n(v.clamp(0.0, 1.0)));
            format!("{r} {g} {b} {} ", if stroke { "RG" } else { "rg" })
        }
    }
}

fn style_ops(st: &StrokeStyle) -> String {
    let cap = match st.cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    };
    let join = match st.join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    };
    let mut s = format!("{} w {cap} J {join} j {} M ", n(st.width.max(0.0)), n(st.miter_limit.max(1.0)));
    let dash_ok = !st.dash.is_empty() && st.dash.iter().all(|d| d.is_finite() && *d >= 0.0) && st.dash.iter().sum::<f32>() > 0.0;
    if dash_ok {
        let lens: Vec<String> = st.dash.iter().map(|d| n(*d)).collect();
        s.push_str(&format!("[{}] {} d ", lens.join(" "), n(st.dash_phase)));
    }
    s
}

/// The PDF name of a blend mode. The Porter-Duff operators and the two additive modes have no PDF blend mode and are
/// written as Normal.
fn blend_name(m: BlendMode) -> &'static str {
    match m {
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
        BlendMode::ColorDodge => "ColorDodge",
        BlendMode::ColorBurn => "ColorBurn",
        BlendMode::SoftLight => "SoftLight",
        BlendMode::HardLight => "HardLight",
        BlendMode::Difference => "Difference",
        BlendMode::Exclusion => "Exclusion",
        BlendMode::Hue => "Hue",
        BlendMode::Saturation => "Saturation",
        BlendMode::Color => "Color",
        BlendMode::Luminosity => "Luminosity",
        _ => "Normal",
    }
}

fn paint_alpha(p: &Paint) -> f32 {
    match p {
        Paint::Solid(c) => c.alpha().clamp(0.0, 1.0),
        _ => 1.0,
    }
}

/// A Type 2 (exponential, N = 1) segment from `c0` to `c1`, inline.
fn segment(c0: [f32; 3], c1: [f32; 3]) -> String {
    let f = |c: [f32; 3]| format!("[{} {} {}]", n(c[0]), n(c[1]), n(c[2]));
    format!("<< /FunctionType 2 /Domain [0 1] /C0 {} /C1 {} /N 1 >>", f(c0), f(c1))
}

/// A stitching function over 0..1 through the sRGB samples `(t, rgb)` (sorted, at least two), linear between them.
fn stitched(samples: &[(f32, [f32; 3])]) -> String {
    let mut funcs = Vec::new();
    let mut bounds = Vec::new();
    let mut encode = Vec::new();
    for w in samples.windows(2) {
        funcs.push(segment(w[0].1, w[1].1));
        encode.push("0 1".to_string());
    }
    for (t, _) in &samples[1..samples.len() - 1] {
        bounds.push(n(*t));
    }
    format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
        funcs.join(" "),
        bounds.join(" "),
        encode.join(" ")
    )
}

/// Samples `f` (0..1 to straight sRGB) at evenly spaced points and at `extra` locations, sorted, deduplicated and
/// nudged so that every segment has a positive length.
fn sample_ramp(extra: &[f32], f: &dyn Fn(f32) -> [f32; 3]) -> Vec<(f32, [f32; 3])> {
    let mut ts: Vec<f32> = (0..=RAMP_SAMPLES).map(|i| i as f32 / RAMP_SAMPLES as f32).collect();
    ts.extend(extra.iter().copied().filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
    ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    ts.into_iter().map(|t| (t, f(t))).collect()
}

fn rgb8(c: [u8; 4]) -> [f32; 3] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

fn rect_path(r: Rect) -> Path {
    Path::from_rect(PathRect::new(r.x, r.y, r.w, r.h))
}

fn ellipse_path(r: Rect) -> Path {
    Path::from_ellipse(PathRect::new(r.x, r.y, r.w, r.h))
}

fn box_array(r: Rect) -> String {
    format!("[{} {} {} {}]", n(r.x), n(r.y), n(r.x + r.w), n(r.y + r.h))
}

impl PdfContext {
    /// A context whose pages default to `media` (`None`: US Letter) and whose file carries `info`.
    pub fn new(media: Option<Rect>, info: PdfInfo) -> PdfContext {
        let default_media = match media {
            Some(r) if !r.is_empty() => r,
            _ => DEFAULT_MEDIA,
        };
        PdfContext {
            info,
            default_media,
            pages: Vec::new(),
            page: None,
            gs: State::new(),
            stack: Vec::new(),
            path: Path::new(),
            tm: Affine::IDENTITY,
            images: Vec::new(),
            shadings: Vec::new(),
            ext: Vec::new(),
            fonts: [false; 12],
            dests: Vec::new(),
            outline: Vec::new(),
            metadata: None,
            marked_depth: 0,
        }
    }

    /// The media box pages get when [`begin_page`](Self::begin_page) is given none.
    pub fn default_media_box(&self) -> Rect {
        self.default_media
    }

    // ---- pages ------------------------------------------------------------------------------------------

    /// Starts a page with `boxes` (`None`: the context's media box only). An open page is ended first. The graphics
    /// state, the path and the text matrix start fresh.
    pub fn begin_page(&mut self, boxes: Option<PageBoxes>) {
        self.end_page();
        let mut boxes = boxes.unwrap_or_else(|| PageBoxes::media(self.default_media));
        if boxes.media.is_empty() {
            boxes.media = self.default_media;
        }
        self.page = Some(PageOut { boxes, content: String::new(), links: Vec::new() });
        self.gs = State::new();
        self.stack.clear();
        self.path = Path::new();
        self.tm = Affine::IDENTITY;
        self.marked_depth = 0;
    }

    /// Ends the open page; saved states and marked content left open are closed. Does nothing without a page.
    pub fn end_page(&mut self) {
        let Some(mut page) = self.page.take() else { return };
        for _ in 0..self.marked_depth {
            page.content.push_str("EMC\n");
        }
        for _ in 0..self.stack.len() {
            page.content.push_str("Q\n");
        }
        self.stack.clear();
        self.marked_depth = 0;
        self.gs = State::new();
        self.path = Path::new();
        self.pages.push(page);
    }

    /// Whether a page is open.
    pub fn in_page(&self) -> bool {
        self.page.is_some()
    }

    /// The number of pages ended so far (an open page is not counted).
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The media box of the open page.
    pub fn page_media_box(&self) -> Option<Rect> {
        self.page.as_ref().map(|p| p.boxes.media)
    }

    fn emit(&mut self, s: &str) {
        if let Some(p) = self.page.as_mut() {
            p.content.push_str(s);
        }
    }

    // ---- document-level content ------------------------------------------------------------------------

    /// Sets the outline (bookmarks). Entries naming a page past the last one are dropped when the file is written.
    pub fn set_outline(&mut self, items: Vec<PdfOutlineItem>) {
        self.outline = items;
    }

    /// Adds an XML metadata stream to the catalog (`CGPDFContextAddDocumentMetadata`). A later call replaces it.
    pub fn add_document_metadata(&mut self, xml: Vec<u8>) {
        self.metadata = Some(xml);
    }

    /// Makes `name` a destination at the user-space point `p` of the open page (`CGPDFContextAddDestinationAtPoint`).
    /// A second destination with the same name replaces the first.
    pub fn add_destination_at_point(&mut self, name: &str, p: (f32, f32)) {
        if self.page.is_none() || name.is_empty() {
            return;
        }
        let (x, y) = self.gs.ctm.apply(p.0, p.1);
        let page = self.pages.len();
        self.dests.retain(|d| d.0 != name);
        self.dests.push((name.to_string(), page, x, y));
    }

    /// A link over the user-space rectangle `r` of the open page to the destination `name`
    /// (`CGPDFContextSetDestinationForRect`). The destination may be added later, on any page.
    pub fn set_destination_for_rect(&mut self, name: &str, r: Rect) {
        self.add_link(r, LinkTarget::Named(name.to_string()));
    }

    /// A link over the user-space rectangle `r` of the open page that opens `url` (`CGPDFContextSetURLForRect`).
    pub fn set_url_for_rect(&mut self, url: &str, r: Rect) {
        self.add_link(r, LinkTarget::Url(url.to_string()));
    }

    fn add_link(&mut self, r: Rect, target: LinkTarget) {
        if r.is_empty() {
            return;
        }
        let bounds = rect_path(r).transformed(&self.gs.ctm).control_bounds();
        let (Some(page), Some(b)) = (self.page.as_mut(), bounds) else { return };
        page.links.push(Link { rect: [b.x, b.y, b.x + b.w, b.y + b.h], target });
    }

    /// Opens a marked-content sequence tagged `tag` (`BMC`), for example `P` or `Artifact`. There is no structure
    /// tree, so the tags carry no logical structure yet.
    pub fn begin_marked_content(&mut self, tag: &str) {
        if self.page.is_none() || tag.is_empty() {
            return;
        }
        let mut name = Vec::new();
        write_name(tag, &mut name);
        let line = format!("{} BMC\n", String::from_utf8_lossy(&name));
        self.emit(&line);
        self.marked_depth += 1;
    }

    /// Closes the innermost marked-content sequence of this page.
    pub fn end_marked_content(&mut self) {
        if self.marked_depth > 0 {
            self.marked_depth -= 1;
            self.emit("EMC\n");
        }
    }

    // ---- graphics state --------------------------------------------------------------------------------

    pub fn save_gstate(&mut self) {
        if self.page.is_none() {
            return;
        }
        self.stack.push(self.gs.clone());
        self.emit("q\n");
    }

    pub fn restore_gstate(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.gs = s;
            self.emit("Q\n");
        }
    }

    pub fn saved_depth(&self) -> usize {
        self.stack.len()
    }

    pub fn ctm(&self) -> Affine {
        self.gs.ctm
    }

    /// Prepends `t` to the transform, as the reference platform's `concatenate` does.
    pub fn concat_ctm(&mut self, t: Affine) {
        self.gs.ctm = t.then(self.gs.ctm);
    }

    pub fn translate_ctm(&mut self, tx: f32, ty: f32) {
        self.concat_ctm(Affine::translation(tx, ty));
    }

    pub fn scale_ctm(&mut self, sx: f32, sy: f32) {
        self.concat_ctm(Affine::scaling(sx, sy));
    }

    pub fn rotate_ctm(&mut self, radians: f32) {
        self.concat_ctm(Affine::rotation(radians));
    }

    pub fn set_line_width(&mut self, w: f32) {
        self.gs.line_width = w.max(0.0);
    }

    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.gs.cap = cap;
    }

    pub fn set_line_join(&mut self, join: LineJoin) {
        self.gs.join = join;
    }

    pub fn set_miter_limit(&mut self, limit: f32) {
        self.gs.miter_limit = limit;
    }

    pub fn set_line_dash(&mut self, phase: f32, lengths: &[f32]) {
        self.gs.dash_phase = phase;
        self.gs.dash = lengths.to_vec();
    }

    /// The flatness tolerance written with every path (`i`); 0 lets the reader choose.
    pub fn set_flatness(&mut self, f: f32) {
        self.gs.flatness = f.clamp(0.0, 100.0);
    }

    pub fn set_alpha(&mut self, a: f32) {
        self.gs.alpha = a.clamp(0.0, 1.0);
    }

    pub fn set_blend_mode(&mut self, mode: BlendMode) {
        self.gs.blend = mode;
    }

    pub fn set_fill_paint(&mut self, p: Paint) {
        self.gs.fill = p;
    }

    pub fn set_stroke_paint(&mut self, p: Paint) {
        self.gs.stroke = p;
    }

    pub fn set_fill_color(&mut self, c: Color) {
        self.gs.fill = Paint::Solid(c);
    }

    pub fn set_stroke_color(&mut self, c: Color) {
        self.gs.stroke = Paint::Solid(c);
    }

    pub fn set_fill_rgb(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.set_fill_color(Color::srgb(r, g, b, a));
    }

    pub fn set_stroke_rgb(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.set_stroke_color(Color::srgb(r, g, b, a));
    }

    pub fn set_fill_gray(&mut self, w: f32, a: f32) {
        self.set_fill_color(Color::generic_gray(w, a));
    }

    pub fn set_stroke_gray(&mut self, w: f32, a: f32) {
        self.set_stroke_color(Color::generic_gray(w, a));
    }

    pub fn set_fill_cmyk(&mut self, c: f32, m: f32, y: f32, k: f32, a: f32) {
        self.set_fill_color(Color::generic_cmyk(c, m, y, k, a));
    }

    pub fn set_stroke_cmyk(&mut self, c: f32, m: f32, y: f32, k: f32, a: f32) {
        self.set_stroke_color(Color::generic_cmyk(c, m, y, k, a));
    }

    /// The `/GSn gs ` operator for these alphas and the current blend mode, or nothing when all are the defaults.
    fn ext_op(&mut self, fill_alpha: f32, stroke_alpha: f32) -> String {
        let key = (fill_alpha.clamp(0.0, 1.0), stroke_alpha.clamp(0.0, 1.0), self.gs.blend);
        if key.0 >= 1.0 && key.1 >= 1.0 && blend_name(key.2) == "Normal" {
            return String::new();
        }
        let i = match self.ext.iter().position(|e| (e.0 - key.0).abs() < 1e-4 && (e.1 - key.1).abs() < 1e-4 && e.2 == key.2) {
            Some(i) => i,
            None => {
                self.ext.push(key);
                self.ext.len() - 1
            }
        };
        format!("/GS{i} gs ")
    }

    // ---- path building ----------------------------------------------------------------------------------

    pub fn begin_path(&mut self) {
        self.path = Path::new();
    }

    pub fn is_path_empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        let p = self.gs.ctm.apply(x, y);
        self.path.move_to(p);
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        let p = self.gs.ctm.apply(x, y);
        self.path.line_to(p);
    }

    pub fn quad_curve_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let c = self.gs.ctm;
        self.path.quad_to(c.apply(cx, cy), c.apply(x, y));
    }

    pub fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c = self.gs.ctm;
        self.path.cubic_to(c.apply(c1x, c1y), c.apply(c2x, c2y), c.apply(x, y));
    }

    pub fn close_path(&mut self) {
        self.path.close();
    }

    /// Adds `p`, given in user space, to the current path.
    pub fn add_path(&mut self, p: &Path) {
        self.path.add_path(&p.transformed(&self.gs.ctm));
    }

    pub fn add_rect(&mut self, r: Rect) {
        self.add_path(&rect_path(r));
    }

    pub fn add_ellipse_in_rect(&mut self, r: Rect) {
        self.add_path(&ellipse_path(r));
    }

    /// A polyline through `points` (a new subpath).
    pub fn add_lines(&mut self, points: &[(f32, f32)]) {
        let Some(first) = points.first() else { return };
        self.move_to(first.0, first.1);
        for p in &points[1..] {
            self.line_to(p.0, p.1);
        }
    }

    pub fn add_arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, clockwise: bool) {
        let mut t = self.user_continuation();
        t.add_arc((cx, cy), r, start, end, clockwise);
        self.path.add_path(&t.transformed(&self.gs.ctm));
    }

    fn user_continuation(&self) -> Path {
        let mut t = Path::new();
        if let (Some(cp), Some(inv)) = (self.path.current_point(), self.gs.ctm.invert()) {
            t.move_to(inv.apply(cp.0, cp.1));
        }
        t
    }

    /// The current path in user space.
    pub fn copy_path(&self) -> Path {
        match self.gs.ctm.invert() {
            Some(inv) => self.path.transformed(&inv),
            None => self.path.clone(),
        }
    }

    // ---- clipping ---------------------------------------------------------------------------------------

    /// Intersects the clip with the current path (non-zero) and clears the path.
    pub fn clip(&mut self) {
        self.clip_with(FillRule::NonZero);
    }

    /// Intersects the clip with the current path (even-odd) and clears the path.
    pub fn eo_clip(&mut self) {
        self.clip_with(FillRule::EvenOdd);
    }

    fn clip_with(&mut self, rule: FillRule) {
        let path = std::mem::take(&mut self.path);
        if path.is_empty() {
            return;
        }
        let op = if rule == FillRule::EvenOdd { "W* n" } else { "W n" };
        // the content stream's own transform is the identity outside paint operations, so the page-space path is
        // the clip as it is
        let line = format!("{}{op}\n", path_ops(&path));
        self.emit(&line);
    }

    pub fn clip_to_rect(&mut self, r: Rect) {
        let saved = std::mem::take(&mut self.path);
        self.add_rect(r);
        self.clip();
        self.path = saved;
    }

    pub fn clip_to_path(&mut self, p: &Path, rule: FillRule) {
        let saved = std::mem::take(&mut self.path);
        self.add_path(p);
        self.clip_with(rule);
        self.path = saved;
    }

    // ---- painting -----------------------------------------------------------------------------------------

    pub fn fill_path(&mut self) {
        self.draw_path(PathDrawing::Fill);
    }

    pub fn eo_fill_path(&mut self) {
        self.draw_path(PathDrawing::EoFill);
    }

    pub fn stroke_path(&mut self) {
        self.draw_path(PathDrawing::Stroke);
    }

    /// Paints the current path as `mode` says and clears it.
    pub fn draw_path(&mut self, mode: PathDrawing) {
        let path = std::mem::take(&mut self.path);
        if self.page.is_none() || path.is_empty() {
            return;
        }
        // paint operators run under the transform, so the path goes back to user space (line widths and dashes are
        // user-space lengths); a singular transform paints nothing
        let Some(inv) = self.gs.ctm.invert() else { return };
        let user = path.transformed(&inv);
        let (fill, rule, stroke) = match mode {
            PathDrawing::Fill => (true, FillRule::NonZero, false),
            PathDrawing::EoFill => (true, FillRule::EvenOdd, false),
            PathDrawing::Stroke => (false, FillRule::NonZero, true),
            PathDrawing::FillStroke => (true, FillRule::NonZero, true),
            PathDrawing::EoFillStroke => (true, FillRule::EvenOdd, true),
        };
        let solid_fill = matches!(self.gs.fill, Paint::Solid(_));
        let solid_stroke = matches!(self.gs.stroke, Paint::Solid(_));
        if fill && stroke && solid_fill && solid_stroke {
            let (Paint::Solid(fc), Paint::Solid(sc)) = (self.gs.fill.clone(), self.gs.stroke.clone()) else { return };
            let gs = self.ext_op(self.gs.alpha * fc.alpha(), self.gs.alpha * sc.alpha());
            let op = if rule == FillRule::EvenOdd { "B*" } else { "B" };
            let line = format!(
                "q {gs}{}{}{}{}{}{}{op} Q\n",
                cm(&self.gs.ctm),
                self.flat_op(),
                color_op(&fc, false),
                color_op(&sc, true),
                style_ops(&self.gs.style()),
                path_ops(&user)
            );
            self.emit(&line);
            return;
        }
        if fill {
            let paint = self.gs.fill.clone();
            self.paint_area(&user, rule, &paint, false);
        }
        if stroke {
            let paint = self.gs.stroke.clone();
            if let Paint::Solid(c) = &paint {
                let gs = self.ext_op(1.0, self.gs.alpha * c.alpha());
                let line = format!(
                    "q {gs}{}{}{}{}{}S Q\n",
                    cm(&self.gs.ctm),
                    self.flat_op(),
                    color_op(c, true),
                    style_ops(&self.gs.style()),
                    path_ops(&user)
                );
                self.emit(&line);
            } else {
                let outline = user.copy_by_stroking(&self.gs.style());
                self.paint_area(&outline, FillRule::NonZero, &paint, true);
            }
        }
    }

    fn flat_op(&self) -> String {
        if self.gs.flatness > 0.0 {
            format!("{} i ", n(self.gs.flatness))
        } else {
            String::new()
        }
    }

    /// Fills the user-space `area` with `paint`: a colour directly, gradients and shadings through a clip, a pattern
    /// as a clipped image of the pattern.
    fn paint_area(&mut self, area: &Path, rule: FillRule, paint: &Paint, as_stroke: bool) {
        let ctm = self.gs.ctm;
        if let Paint::Solid(c) = paint {
            let gs = self.ext_op(self.gs.alpha * c.alpha(), 1.0);
            let op = if rule == FillRule::EvenOdd { "f*" } else { "f" };
            let line = format!("q {gs}{}{}{}{}{op} Q\n", cm(&ctm), self.flat_op(), color_op(c, false), path_ops(area));
            self.emit(&line);
            return;
        }
        let clip = if rule == FillRule::EvenOdd { "W* n" } else { "W n" };
        let Some(body) = self.paint_body(paint, area.control_bounds()) else { return };
        let alpha = self.gs.alpha * paint_alpha(paint);
        // `sh` and image drawing use the fill alpha (`ca`), also when they paint a stroke's outline
        let gs = self.ext_op(alpha, if as_stroke { alpha } else { 1.0 });
        let line = format!("q {gs}{}{}{clip} {body}Q\n", cm(&ctm), path_ops(area));
        self.emit(&line);
    }

    /// The operators that paint a non-solid paint over the clip, in user space: `/ShN sh` for gradients and
    /// shadings, an image of the pattern over `bounds` for patterns.
    fn paint_body(&mut self, paint: &Paint, bounds: Option<PathRect>) -> Option<String> {
        match paint {
            Paint::Solid(_) => None,
            Paint::Linear(g) => Some(format!("/Sh{} sh ", self.linear_shading(g))),
            Paint::Radial(g) => Some(format!("/Sh{} sh ", self.radial_shading(g))),
            Paint::Shading(s) => self.function_shading(s).map(|i| format!("/Sh{i} sh ")),
            Paint::Pattern(_) => {
                let b = bounds?;
                let i = self.pattern_image(paint, b)?;
                Some(format!("q {} 0 0 {} {} {} cm /Im{i} Do Q ", n(b.w), n(b.h), n(b.x), n(b.y)))
            }
        }
    }

    fn push_shading(&mut self, dict: String) -> usize {
        self.shadings.push(dict);
        self.shadings.len() - 1
    }

    fn linear_shading(&mut self, g: &LinearGradient) -> usize {
        let samples = sample_ramp(g.ramp.locations(), &|t: f32| rgb8(g.ramp.exact(t)));
        let dict = format!(
            "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{} {} {} {}] /Function {} /Extend [{} {}] >>",
            n(g.start.0),
            n(g.start.1),
            n(g.end.0),
            n(g.end.1),
            stitched(&samples),
            g.extend_start,
            g.extend_end
        );
        self.push_shading(dict)
    }

    fn radial_shading(&mut self, g: &RadialGradient) -> usize {
        let samples = sample_ramp(g.ramp.locations(), &|t: f32| rgb8(g.ramp.exact(t)));
        let dict = format!(
            "<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [{} {} {} {} {} {}] /Function {} /Extend [{} {}] >>",
            n(g.start_center.0),
            n(g.start_center.1),
            n(g.start_radius.max(0.0)),
            n(g.end_center.0),
            n(g.end_center.1),
            n(g.end_radius.max(0.0)),
            stitched(&samples),
            g.extend_start,
            g.extend_end
        );
        self.push_shading(dict)
    }

    /// A shading whose function is sampled into sRGB over its domain.
    fn function_shading(&mut self, s: &Shading) -> Option<usize> {
        let f = s.function();
        let (d0, d1) = f.domain();
        let space = s.space().clone();
        let at = |t: f32| -> [f32; 3] {
            let comps = f.eval(d0 + (d1 - d0) * t);
            let c = Color::new(space.clone(), &comps, 1.0).and_then(|c| c.to_srgb()).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
        };
        let samples = sample_ramp(&[], &at);
        let (kind, coords) = match s.kind() {
            ShadingKind::Axial { start, end } => (2, format!("{} {} {} {}", n(start.0), n(start.1), n(end.0), n(end.1))),
            ShadingKind::Radial { start_center, start_radius, end_center, end_radius } => (
                3,
                format!(
                    "{} {} {} {} {} {}",
                    n(start_center.0),
                    n(start_center.1),
                    n(start_radius.max(0.0)),
                    n(end_center.0),
                    n(end_center.1),
                    n(end_radius.max(0.0))
                ),
            ),
        };
        let dict = format!(
            "<< /ShadingType {kind} /ColorSpace /DeviceRGB /Coords [{coords}] /Function {} /Extend [{} {}] >>",
            stitched(&samples),
            s.extend_start,
            s.extend_end
        );
        Some(self.push_shading(dict))
    }

    /// The pattern drawn into an image over the user-space rectangle `b`, at about two pixels per page point.
    fn pattern_image(&mut self, paint: &Paint, b: PathRect) -> Option<usize> {
        if !(b.w > 0.0 && b.h > 0.0) {
            return None;
        }
        let k = 2.0 * self.gs.ctm.determinant().abs().sqrt().max(1e-3);
        let w = ((b.w * k).ceil() as usize).clamp(1, MAX_PATTERN_SIDE);
        let h = ((b.h * k).ceil() as usize).clamp(1, MAX_PATTERN_SIDE);
        let mut px = Vec::with_capacity(w * h);
        for j in 0..h {
            // the first row is the top of the rectangle (largest y)
            let y = b.y + b.h - (j as f32 + 0.5) * b.h / h as f32;
            for i in 0..w {
                let x = b.x + (i as f32 + 0.5) * b.w / w as f32;
                px.push(paint.shade(x, y));
            }
        }
        Some(self.push_image(&Image { w, h, px }))
    }

    /// Adds `img` (premultiplied, rows top to bottom) as an image XObject; returns its index.
    fn push_image(&mut self, img: &Image) -> usize {
        let mut rgb = Vec::with_capacity(img.w * img.h * 3);
        let mut alpha = Vec::with_capacity(img.w * img.h);
        let mut opaque = true;
        for &p in &img.px {
            let [r, g, b, a] = [(p & 255) as u8, (p >> 8 & 255) as u8, (p >> 16 & 255) as u8, (p >> 24 & 255) as u8];
            let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
            rgb.extend([un(r), un(g), un(b)]);
            alpha.push(a);
            opaque &= a == 255;
        }
        self.images.push(ImageOut { w: img.w, h: img.h, rgb, alpha: (!opaque).then_some(alpha) });
        self.images.len() - 1
    }

    pub fn fill_rect(&mut self, r: Rect) {
        let saved = std::mem::take(&mut self.path);
        self.add_rect(r);
        self.fill_path();
        self.path = saved;
    }

    pub fn stroke_rect(&mut self, r: Rect) {
        let saved = std::mem::take(&mut self.path);
        self.add_rect(r);
        self.stroke_path();
        self.path = saved;
    }

    pub fn fill_ellipse_in_rect(&mut self, r: Rect) {
        let saved = std::mem::take(&mut self.path);
        self.add_ellipse_in_rect(r);
        self.fill_path();
        self.path = saved;
    }

    pub fn stroke_ellipse_in_rect(&mut self, r: Rect) {
        let saved = std::mem::take(&mut self.path);
        self.add_ellipse_in_rect(r);
        self.stroke_path();
        self.path = saved;
    }

    /// Strokes each pair of points as its own segment.
    pub fn stroke_line_segments(&mut self, points: &[(f32, f32)]) {
        let saved = std::mem::take(&mut self.path);
        for pair in points.chunks_exact(2) {
            self.move_to(pair[0].0, pair[0].1);
            self.line_to(pair[1].0, pair[1].1);
        }
        self.stroke_path();
        self.path = saved;
    }

    /// Paints a gradient over the clip, in user space (`drawLinearGradient`).
    pub fn draw_linear_gradient(&mut self, g: &LinearGradient) {
        self.draw_paint(&Paint::Linear(g.clone()));
    }

    pub fn draw_radial_gradient(&mut self, g: &RadialGradient) {
        self.draw_paint(&Paint::Radial(g.clone()));
    }

    pub fn draw_shading(&mut self, s: &Shading) {
        self.draw_paint(&Paint::Shading(s.clone()));
    }

    /// Paints `paint` over the whole clip; a pattern covers the page.
    pub fn draw_paint(&mut self, paint: &Paint) {
        let Some(media) = self.page_media_box() else { return };
        let Some(inv) = self.gs.ctm.invert() else { return };
        let page_area = rect_path(media).transformed(&inv);
        match paint {
            Paint::Solid(_) | Paint::Pattern(_) => self.paint_area(&page_area, FillRule::NonZero, paint, false),
            _ => {
                let Some(body) = self.paint_body(paint, None) else { return };
                let gs = self.ext_op(self.gs.alpha, 1.0);
                let line = format!("q {gs}{}{body}Q\n", cm(&self.gs.ctm));
                self.emit(&line);
            }
        }
    }

    /// Draws `img` (premultiplied pixels, rows top to bottom) into the user-space rectangle `dest`, its first row at
    /// the top (largest y) of `dest`.
    pub fn draw_image(&mut self, img: &Image, dest: Rect) {
        if self.page.is_none() || img.w == 0 || img.h == 0 || img.px.len() != img.w * img.h || dest.is_empty() {
            return;
        }
        let i = self.push_image(img);
        let gs = self.ext_op(self.gs.alpha, 1.0);
        let line = format!(
            "q {gs}{}{} 0 0 {} {} {} cm /Im{i} Do Q\n",
            cm(&self.gs.ctm),
            n(dest.w),
            n(dest.h),
            n(dest.x),
            n(dest.y)
        );
        self.emit(&line);
    }

    // ---- text ---------------------------------------------------------------------------------------------

    pub fn set_font(&mut self, face: StandardFace) {
        self.gs.face = face;
    }

    pub fn set_font_size(&mut self, size: f32) {
        self.gs.font_size = size.max(0.0);
    }

    pub fn set_character_spacing(&mut self, s: f32) {
        self.gs.char_spacing = s;
    }

    pub fn set_text_drawing_mode(&mut self, m: TextDrawing) {
        self.gs.text_mode = m;
    }

    pub fn set_text_matrix(&mut self, m: Affine) {
        self.tm = m;
    }

    pub fn text_matrix(&self) -> Affine {
        self.tm
    }

    pub fn set_text_position(&mut self, x: f32, y: f32) {
        self.tm.tx = x;
        self.tm.ty = y;
    }

    pub fn text_position(&self) -> (f32, f32) {
        (self.tm.tx, self.tm.ty)
    }

    /// The advance `text` gets with the current face, size and character spacing, in text space.
    pub fn text_advance(&self, text: &str) -> f32 {
        self.gs.face.width(text, self.gs.font_size) + self.gs.char_spacing * text.chars().count() as f32
    }

    /// Draws `text` at the text position with the fill paint (a non-solid paint is drawn as black) and moves the
    /// position past it. The text is real PDF text, so a reader can select and search it; the invisible mode keeps
    /// it selectable without painting it.
    pub fn show_text(&mut self, text: &str) {
        let adv = self.text_advance(text);
        if self.page.is_some() && !text.is_empty() {
            let face = self.gs.face;
            self.fonts[face.index()] = true;
            let color = match &self.gs.fill {
                Paint::Solid(c) => c.clone(),
                _ => Color::black(),
            };
            let gs = self.ext_op(self.gs.alpha * color.alpha(), 1.0);
            let hex: String = text.chars().map(|c| format!("{:02X}", winansi(c))).collect();
            let mode = if self.gs.text_mode == TextDrawing::Invisible { 3 } else { 0 };
            let t = self.tm;
            let line = format!(
                "q {gs}{}{}BT /F{} {} Tf {} Tc {mode} Tr {} {} {} {} {} {} Tm <{hex}> Tj ET Q\n",
                cm(&self.gs.ctm),
                color_op(&color, false),
                face.index(),
                n(self.gs.font_size),
                n(self.gs.char_spacing),
                n(t.a),
                n(t.b),
                n(t.c),
                n(t.d),
                n(t.tx),
                n(t.ty)
            );
            self.emit(&line);
        }
        self.tm.tx += adv * self.tm.a;
        self.tm.ty += adv * self.tm.b;
    }

    pub fn show_text_at_point(&mut self, x: f32, y: f32, text: &str) {
        self.set_text_position(x, y);
        self.show_text(text);
    }

    // ---- the file -------------------------------------------------------------------------------------------

    /// Ends an open page and writes the file (`CGPDFContextClose`). A context with no pages gives a valid file with
    /// zero pages.
    pub fn close(mut self) -> Vec<u8> {
        self.end_page();
        self.write()
    }

    fn write(&self) -> Vec<u8> {
        // object numbers: 1 catalog, 2 page tree, 3 shared resources, 4 information, then per page its dictionary and
        // content, then images (and their masks), the metadata stream and the outline items
        let mut objects: Vec<Vec<u8>> = vec![Vec::new(); 4];
        let mut next = 5usize;
        let mut page_ids = Vec::with_capacity(self.pages.len());
        for _ in &self.pages {
            page_ids.push((next, next + 1));
            next += 2;
        }
        let mut image_ids = Vec::with_capacity(self.images.len());
        for img in &self.images {
            let mask = img.alpha.as_ref().map(|_| next + 1);
            image_ids.push((next, mask));
            next += if mask.is_some() { 2 } else { 1 };
        }
        let metadata_id = self.metadata.as_ref().map(|_| {
            next += 1;
            next - 1
        });
        let total_pages = self.pages.len();
        let outline = self.outline_objects(&mut next, &page_ids);
        objects.resize(next - 1, Vec::new());

        // catalog
        let mut cat = String::from("<< /Type /Catalog /Pages 2 0 R");
        if let Some(id) = metadata_id {
            cat.push_str(&format!(" /Metadata {id} 0 R"));
        }
        let dests: Vec<_> = self.dests.iter().filter(|d| d.1 < total_pages).collect();
        if !dests.is_empty() {
            let mut d = String::from(" /Dests <<");
            for (name, page, x, y) in dests {
                let mut nm = Vec::new();
                write_name(name, &mut nm);
                d.push_str(&format!(" {} [{} 0 R /XYZ {} {} null]", String::from_utf8_lossy(&nm), page_ids[*page].0, n(*x), n(*y)));
            }
            d.push_str(" >>");
            cat.push_str(&d);
        }
        if let Some((root, _)) = &outline {
            cat.push_str(&format!(" /Outlines {root} 0 R /PageMode /UseOutlines"));
        }
        cat.push_str(" >>");
        objects[0] = cat.into_bytes();

        let kids: Vec<String> = page_ids.iter().map(|(id, _)| format!("{id} 0 R")).collect();
        objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), total_pages).into_bytes();
        objects[2] = self.resources(&image_ids).into_bytes();
        objects[3] = self.info_dict();

        for (page, (pid, cid)) in self.pages.iter().zip(&page_ids) {
            let b = &page.boxes;
            let mut d = format!("<< /Type /Page /Parent 2 0 R /MediaBox {}", box_array(b.media));
            for (key, r) in [("CropBox", b.crop), ("BleedBox", b.bleed), ("TrimBox", b.trim), ("ArtBox", b.art)] {
                if let Some(r) = r.filter(|r| !r.is_empty()) {
                    d.push_str(&format!(" /{key} {}", box_array(r)));
                }
            }
            d.push_str(&format!(" /Resources 3 0 R /Contents {cid} 0 R"));
            let annots: Vec<String> = page.links.iter().filter_map(|l| self.link_dict(l)).collect();
            if !annots.is_empty() {
                d.push_str(&format!(" /Annots [{}]", annots.join(" ")));
            }
            d.push_str(" >>");
            objects[pid - 1] = d.into_bytes();
            objects[cid - 1] = stream("", page.content.as_bytes());
        }
        for (img, (id, mask)) in self.images.iter().zip(&image_ids) {
            let packed = miniz_oxide::deflate::compress_to_vec_zlib(&img.rgb, 6);
            let smask = mask.map(|m| format!(" /SMask {m} 0 R")).unwrap_or_default();
            objects[id - 1] = stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode{smask}",
                    img.w, img.h
                ),
                &packed,
            );
            if let (Some(m), Some(alpha)) = (mask, &img.alpha) {
                let packed = miniz_oxide::deflate::compress_to_vec_zlib(alpha, 6);
                objects[m - 1] = stream(
                    &format!(
                        "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode",
                        img.w, img.h
                    ),
                    &packed,
                );
            }
        }
        if let (Some(id), Some(xml)) = (metadata_id, &self.metadata) {
            objects[id - 1] = stream("/Type /Metadata /Subtype /XML", xml);
        }
        if let Some((_, items)) = outline {
            for (id, body) in items {
                objects[id - 1] = body.into_bytes();
            }
        }

        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
            out.extend(body);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).into_bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").into_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).into_bytes(),
        );
        out
    }

    /// The shared resource dictionary: the fonts used, the alpha and blend states, the shadings and the images.
    fn resources(&self, image_ids: &[(usize, Option<usize>)]) -> String {
        let mut r = String::from("<<");
        if self.fonts.iter().any(|u| *u) {
            r.push_str(" /Font <<");
            for (i, face) in StandardFace::ALL.iter().enumerate().filter(|(i, _)| self.fonts[*i]) {
                r.push_str(&format!(
                    " /F{i} << /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                    face.base_font()
                ));
            }
            r.push_str(" >>");
        }
        if !self.ext.is_empty() {
            r.push_str(" /ExtGState <<");
            for (i, (ca, sa, bm)) in self.ext.iter().enumerate() {
                r.push_str(&format!(" /GS{i} << /Type /ExtGState /ca {} /CA {} /BM /{} >>", n(*ca), n(*sa), blend_name(*bm)));
            }
            r.push_str(" >>");
        }
        if !self.shadings.is_empty() {
            r.push_str(" /Shading <<");
            for (i, s) in self.shadings.iter().enumerate() {
                r.push_str(&format!(" /Sh{i} {s}"));
            }
            r.push_str(" >>");
        }
        if !image_ids.is_empty() {
            r.push_str(" /XObject <<");
            for (i, (id, _)) in image_ids.iter().enumerate() {
                r.push_str(&format!(" /Im{i} {id} 0 R"));
            }
            r.push_str(" >>");
        }
        r.push_str(" /ProcSet [/PDF /Text /ImageB /ImageC] >>");
        r
    }

    fn info_dict(&self) -> Vec<u8> {
        let mut out = b"<< /Producer ".to_vec();
        text_string("BifrOSt").write(&mut out);
        let i = &self.info;
        for (key, value) in [("Title", &i.title), ("Author", &i.author), ("Subject", &i.subject), ("Keywords", &i.keywords), ("Creator", &i.creator)] {
            if !value.is_empty() {
                out.extend(format!(" /{key} ").into_bytes());
                text_string(value).write(&mut out);
            }
        }
        out.extend(b" >>");
        out
    }

    fn link_dict(&self, l: &Link) -> Option<String> {
        let [x0, y0, x1, y1] = l.rect;
        let rect = format!("[{} {} {} {}]", n(x0), n(y0), n(x1), n(y1));
        let target = match &l.target {
            LinkTarget::Url(url) => {
                let mut s = Vec::new();
                Obj::Str(url.as_bytes().to_vec()).write(&mut s);
                format!("/A << /Type /Action /S /URI /URI {} >>", String::from_utf8_lossy(&s))
            }
            LinkTarget::Named(name) => {
                let mut nm = Vec::new();
                write_name(name, &mut nm);
                format!("/Dest {}", String::from_utf8_lossy(&nm))
            }
        };
        Some(format!("<< /Type /Annot /Subtype /Link /Rect {rect} /Border [0 0 0] {target} >>"))
    }

    /// The outline root's number and every outline object `(number, body)`, numbered from `next`. `None` when no
    /// entry names an existing page.
    fn outline_objects(&self, next: &mut usize, page_ids: &[(usize, usize)]) -> Option<(usize, Vec<(usize, String)>)> {
        fn keep(items: &[PdfOutlineItem], pages: usize) -> Vec<PdfOutlineItem> {
            items
                .iter()
                .filter(|it| it.page < pages)
                .map(|it| PdfOutlineItem { children: keep(&it.children, pages), ..it.clone() })
                .collect()
        }
        fn count(items: &[PdfOutlineItem]) -> usize {
            items.iter().map(|it| 1 + count(&it.children)).sum()
        }
        fn number(items: &[PdfOutlineItem], next: &mut usize) -> Vec<usize> {
            items.iter().map(|_| {
                *next += 1;
                *next - 1
            }).collect()
        }
        fn level(
            items: &[PdfOutlineItem],
            ids: &[usize],
            parent: usize,
            next: &mut usize,
            page_ids: &[(usize, usize)],
            out: &mut Vec<(usize, String)>,
        ) {
            for (i, it) in items.iter().enumerate() {
                let mut d = Vec::from(&b"<< /Title "[..]);
                text_string(&it.title).write(&mut d);
                let mut s = format!("{} /Parent {parent} 0 R", String::from_utf8_lossy(&d));
                if i > 0 {
                    s.push_str(&format!(" /Prev {} 0 R", ids[i - 1]));
                }
                if i + 1 < ids.len() {
                    s.push_str(&format!(" /Next {} 0 R", ids[i + 1]));
                }
                let top = it.top.map(n).unwrap_or_else(|| "null".to_string());
                s.push_str(&format!(" /Dest [{} 0 R /XYZ null {top} null]", page_ids[it.page].0));
                let kids = number(&it.children, next);
                if let (Some(first), Some(last)) = (kids.first(), kids.last()) {
                    s.push_str(&format!(" /First {first} 0 R /Last {last} 0 R /Count {}", count(&it.children)));
                }
                s.push_str(" >>");
                out.push((ids[i], s));
                level(&it.children, &kids, ids[i], next, page_ids, out);
            }
        }
        let items = keep(&self.outline, page_ids.len());
        if items.is_empty() {
            return None;
        }
        let root = *next;
        *next += 1;
        let ids = number(&items, next);
        let mut out = vec![(
            root,
            format!("<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>", ids[0], ids[ids.len() - 1], count(&items)),
        )];
        level(&items, &ids, root, next, page_ids, &mut out);
        Some((root, out))
    }
}

fn stream(entries: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {entries} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::paint::ColorRamp;
    use crate::cg::pdf::doc::Doc;
    use crate::cg::pdf::{draw, page};

    fn letter_page(f: impl FnOnce(&mut PdfContext)) -> Vec<u8> {
        let mut cx = PdfContext::new(Some(Rect::new(0.0, 0.0, 100.0, 100.0)), PdfInfo::default());
        cx.begin_page(None);
        f(&mut cx);
        cx.close()
    }

    /// The page drawn back at one pixel per point; (x, y) in PDF space (y up).
    fn pixel(pdf: &[u8], x: usize, y: usize) -> [u8; 4] {
        let doc = Doc::open(pdf.to_vec()).expect("the file opens");
        let pages = page::pages(&doc);
        let img = draw::render_page(&doc, &pages[0], 100, 100).expect("the page draws");
        let p = img.px[(99 - y) * img.w + x];
        [(p & 255) as u8, (p >> 8 & 255) as u8, (p >> 16 & 255) as u8, (p >> 24) as u8]
    }

    fn near(a: [u8; 4], b: [u8; 3]) -> bool {
        (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= 8)
    }

    #[test]
    fn a_filled_rect_lands_where_user_space_says() {
        let pdf = letter_page(|cx| {
            cx.set_fill_rgb(1.0, 0.0, 0.0, 1.0);
            cx.fill_rect(Rect::new(10.0, 10.0, 30.0, 20.0));
        });
        assert!(near(pixel(&pdf, 20, 15), [255, 0, 0]), "inside");
        assert!(near(pixel(&pdf, 60, 60), [255, 255, 255]), "outside stays paper");
        assert!(near(pixel(&pdf, 20, 80), [255, 255, 255]), "y grows upwards");
    }

    #[test]
    fn the_transform_moves_later_drawing_and_restore_brings_it_back() {
        let pdf = letter_page(|cx| {
            cx.save_gstate();
            cx.translate_ctm(50.0, 50.0);
            cx.set_fill_rgb(0.0, 0.0, 1.0, 1.0);
            cx.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
            cx.restore_gstate();
            cx.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        });
        assert!(near(pixel(&pdf, 55, 55), [0, 0, 255]));
        assert!(near(pixel(&pdf, 5, 5), [0, 0, 0]), "the restored fill colour is black");
    }

    #[test]
    fn a_path_built_before_a_transform_keeps_its_place() {
        let pdf = letter_page(|cx| {
            cx.move_to(0.0, 0.0);
            cx.line_to(20.0, 0.0);
            cx.line_to(20.0, 20.0);
            cx.line_to(0.0, 20.0);
            cx.close_path();
            cx.scale_ctm(4.0, 4.0);
            cx.fill_path();
        });
        assert!(near(pixel(&pdf, 10, 10), [0, 0, 0]));
        assert!(near(pixel(&pdf, 40, 40), [255, 255, 255]), "points were fixed when they were added");
    }

    #[test]
    fn clipping_limits_the_fill() {
        let pdf = letter_page(|cx| {
            cx.clip_to_rect(Rect::new(0.0, 0.0, 50.0, 100.0));
            cx.set_fill_gray(0.0, 1.0);
            cx.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0));
        });
        assert!(near(pixel(&pdf, 25, 50), [0, 0, 0]));
        assert!(near(pixel(&pdf, 75, 50), [255, 255, 255]));
    }

    #[test]
    fn strokes_carry_width_cap_and_dash() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.begin_page(None);
        cx.set_line_width(4.0);
        cx.set_line_cap(LineCap::Round);
        cx.set_line_dash(1.0, &[6.0, 2.0]);
        cx.set_stroke_cmyk(0.0, 1.0, 1.0, 0.0, 1.0);
        cx.stroke_line_segments(&[(0.0, 0.0), (100.0, 0.0)]);
        let pdf = cx.close();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("4 w 1 J 0 j 10 M [6 2] 1 d"), "{text}");
        assert!(text.contains("0 1 1 0 K"), "CMYK stays CMYK");
        assert!(text.contains("0 0 m 100 0 l S"));
    }

    #[test]
    fn alpha_and_blend_modes_become_graphics_states() {
        let pdf = letter_page(|cx| {
            cx.set_alpha(0.5);
            cx.set_blend_mode(BlendMode::Multiply);
            cx.set_fill_rgb(0.0, 0.0, 0.0, 1.0);
            cx.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0));
        });
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/GS0 << /Type /ExtGState /ca 0.5 /CA 1 /BM /Multiply >>"), "{text}");
        let p = pixel(&pdf, 50, 50);
        assert!(p[0] > 100 && p[0] < 160, "half black over white is grey: {p:?}");
    }

    #[test]
    fn a_linear_gradient_is_a_vector_shading() {
        let ramp = ColorRamp::from_rgba8(&[(0.0, [255, 0, 0, 255]), (1.0, [0, 0, 255, 255])]).unwrap();
        let g = LinearGradient::new((0.0, 0.0), (100.0, 0.0), ramp).extended(true, true);
        let pdf = letter_page(|cx| cx.draw_linear_gradient(&g));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/ShadingType 2"), "{text}");
        assert!(text.contains("/Sh0 sh"));
        let left = pixel(&pdf, 2, 50);
        let right = pixel(&pdf, 97, 50);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    }

    #[test]
    fn a_gradient_fill_is_clipped_to_the_shape() {
        let ramp = ColorRamp::from_rgba8(&[(0.0, [0, 0, 0, 255]), (1.0, [0, 0, 0, 255])]).unwrap();
        let g = LinearGradient::new((0.0, 0.0), (100.0, 0.0), ramp).extended(true, true);
        let pdf = letter_page(|cx| {
            cx.set_fill_paint(Paint::Linear(g));
            cx.fill_rect(Rect::new(0.0, 0.0, 50.0, 50.0));
        });
        assert!(near(pixel(&pdf, 25, 25), [0, 0, 0]));
        assert!(near(pixel(&pdf, 75, 75), [255, 255, 255]));
    }

    #[test]
    fn images_are_drawn_upright_into_their_rectangle() {
        // two rows: red on top, blue below
        let red = 0xff00_00ff;
        let blue = 0xffff_0000;
        let img = Image { w: 1, h: 2, px: vec![red, blue] };
        let pdf = letter_page(|cx| cx.draw_image(&img, Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert!(near(pixel(&pdf, 50, 80), [255, 0, 0]), "first row at the top");
        assert!(near(pixel(&pdf, 50, 20), [0, 0, 255]));
    }

    #[test]
    fn transparent_images_carry_a_soft_mask() {
        let img = Image { w: 1, h: 1, px: vec![0x8000_0080] };
        let pdf = letter_page(|cx| cx.draw_image(&img, Rect::new(0.0, 0.0, 10.0, 10.0)));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/SMask"));
        assert!(text.contains("/ColorSpace /DeviceGray"));
    }

    #[test]
    fn text_is_written_as_text_and_advances_the_position() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.begin_page(None);
        cx.set_font(StandardFace::TimesBold);
        cx.set_font_size(20.0);
        cx.show_text_at_point(72.0, 700.0, "Hello");
        let (x, y) = cx.text_position();
        assert!((x - (72.0 + StandardFace::TimesBold.width("Hello", 20.0))).abs() < 1e-3);
        assert_eq!(y, 700.0);
        let pdf = cx.close();
        let doc = Doc::open(pdf.clone()).unwrap();
        let pages = page::pages(&doc);
        let mut ex = crate::cg::pdf::extract::Extractor::new(&doc);
        ex.page(&pages[0]);
        let found = ex.pieces.iter().any(|p| matches!(p, crate::cg::pdf::extract::Piece::Line { text, .. } if text.contains("Hello")));
        assert!(found, "the text reads back");
        assert!(String::from_utf8_lossy(&pdf).contains("/BaseFont /Times-Bold"));
    }

    #[test]
    fn page_boxes_info_and_page_count_reach_the_file() {
        let info = PdfInfo { title: "Report".into(), author: "Lab".into(), ..PdfInfo::default() };
        let mut cx = PdfContext::new(None, info);
        cx.begin_page(None);
        cx.begin_page(Some(PageBoxes {
            media: Rect::new(0.0, 0.0, 300.0, 400.0),
            crop: Some(Rect::new(10.0, 10.0, 280.0, 380.0)),
            ..PageBoxes::default()
        }));
        assert_eq!(cx.page_count(), 1, "beginning a page ends the open one");
        let pdf = cx.close();
        let doc = Doc::open(pdf).unwrap();
        let pages = page::pages(&doc);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].media_box(), [0.0, 0.0, 612.0, 792.0], "US Letter by default");
        assert_eq!(pages[1].media_box(), [0.0, 0.0, 300.0, 400.0]);
        assert_eq!(pages[1].crop_box(), [10.0, 10.0, 290.0, 390.0]);
        let info = page::info(&doc);
        assert_eq!(info.title, "Report");
        assert_eq!(info.author, "Lab");
    }

    #[test]
    fn links_and_named_destinations_resolve() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.begin_page(None);
        cx.set_url_for_rect("https://example.org/", Rect::new(10.0, 10.0, 50.0, 20.0));
        cx.set_destination_for_rect("chapter", Rect::new(10.0, 40.0, 50.0, 20.0));
        cx.begin_page(None);
        cx.add_destination_at_point("chapter", (0.0, 500.0));
        let pdf = cx.close();
        let doc = Doc::open(pdf).unwrap();
        let pages = page::pages(&doc);
        let links = page::links(&doc, &pages[0]);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].uri.as_deref(), Some("https://example.org/"));
        assert_eq!(links[0].rect, [10.0, 10.0, 60.0, 30.0]);
        let dest = links[1].dest.as_ref().expect("the named destination resolves");
        assert_eq!(Some(dest.page), pages[1].num);
        assert_eq!(dest.top, Some(500.0));
    }

    #[test]
    fn the_outline_nests_and_skips_missing_pages() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.begin_page(None);
        cx.begin_page(None);
        cx.set_outline(vec![
            PdfOutlineItem {
                title: "One".into(),
                page: 0,
                top: None,
                children: vec![PdfOutlineItem { title: "One.A".into(), page: 1, top: Some(300.0), children: vec![] }],
            },
            PdfOutlineItem { title: "Gone".into(), page: 9, top: None, children: vec![] },
        ]);
        let pdf = cx.close();
        let doc = Doc::open(pdf).unwrap();
        let items = page::outline(&doc);
        let titles: Vec<(&str, u8)> = items.iter().map(|i| (i.title.as_str(), i.level)).collect();
        assert_eq!(titles, vec![("One", 0), ("One.A", 1)]);
        assert_eq!(items[1].dest.top, Some(300.0));
    }

    #[test]
    fn drawing_outside_a_page_is_ignored_and_open_states_close() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        cx.begin_page(None);
        cx.save_gstate();
        cx.save_gstate();
        cx.begin_marked_content("P");
        let pdf = cx.close();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("q\nq\n/P BMC\nEMC\nQ\nQ\n"), "{text}");
        assert!(!text.contains(" f Q"), "nothing was drawn before the page");
        assert!(Doc::open(pdf).is_ok());
    }

    #[test]
    fn an_empty_context_is_a_valid_file() {
        let pdf = PdfContext::new(None, PdfInfo::default()).close();
        let doc = Doc::open(pdf).unwrap();
        assert_eq!(page::page_count(&doc), 0);
    }

    #[test]
    fn metadata_is_a_catalog_stream() {
        let mut cx = PdfContext::new(None, PdfInfo::default());
        cx.add_document_metadata(b"<x:xmpmeta/>".to_vec());
        let pdf = cx.close();
        let doc = Doc::open(pdf).unwrap();
        let cat = doc.catalog().unwrap();
        assert!(matches!(doc.field(&cat, "Metadata"), Obj::Stream(_)));
    }
}
