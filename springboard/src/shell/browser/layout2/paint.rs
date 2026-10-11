//! Paint: the resolved paint data of each box (`BoxPaint`, filled by the
//! position pass) and `LayoutTree::display_list`, which walks the tree in
//! CSS paint order into a `DisplayListBuilder`.
//!
//! Stacking contexts (z-index, opacity, transforms, fixed, sticky) and
//! positioned boxes open builder contexts; an `overflow` clip pushes a clip
//! into every phase of the current context and clips nested contexts through
//! their opening items. Items are in page space; fixed boxes are moved by the
//! scroll offset and sticky boxes by their resolved push.

use super::super::css::Color;
use super::super::display::{
    DisplayItem, DisplayList, DisplayListBuilder, Fit, ImageId, Phase, Rect, Xf,
};
use super::super::dom::NodeId;
use super::super::style::values::BgRepeatKind;
use super::super::style::{self as cs, Computed};
use super::inline::FragKind;
use super::positioned::{page_matrix, sticky_delta};
use super::text::TextMetrics;
use super::tree::*;
use super::{col, Ctx, LayoutTree};

pub const POS_STATIC: u8 = 0;
pub const POS_RELATIVE: u8 = 1;
pub const POS_ABSOLUTE: u8 = 2;
pub const POS_FIXED: u8 = 3;
pub const POS_STICKY: u8 = 4;

#[derive(Clone, Debug)]
pub struct ShadowP {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

#[derive(Clone, Debug)]
pub enum LayerKind {
    Linear {
        angle: f32,
        stops: Vec<(Option<cs::Length>, Color)>,
    },
    Radial {
        stops: Vec<(Option<cs::Length>, Color)>,
    },
    Image {
        id: ImageId,
        iw: f32,
        ih: f32,
    },
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub kind: LayerKind,
    pub size: cs::BgSize,
    pub px: cs::Length,
    pub py: cs::Length,
    pub rx: BgRepeatKind,
    pub ry: BgRepeatKind,
    pub origin: cs::BgBox,
    pub clip: cs::BgBox,
}

#[derive(Clone, Debug)]
pub struct Marker {
    /// 0 disc, 1 circle, 2 square, 3 text.
    pub kind: u8,
    pub text: String,
    pub color: Color,
    pub fs: f32,
    pub bold: bool,
}

/// A table row with a background: the cells (indexes into `kids`) it spans.
#[derive(Clone, Debug)]
pub struct RowBg {
    pub first: u32,
    pub last: u32,
    pub color: Color,
}

#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub shadows: Vec<ShadowP>,
    pub layers: Vec<Layer>,
    pub marker: Option<Marker>,
    pub img: Option<ImageId>,
    pub rows: Vec<RowBg>,
}

/// Everything painting needs from the style, resolved against the box size.
#[derive(Clone, Debug)]
pub struct BoxPaint {
    pub visible: bool,
    pub bg: Color,
    pub bw: [f32; 4],
    pub bc: [Color; 4],
    pub radius: [f32; 4],
    pub opacity: f32,
    pub z: Option<i32>,
    pub position: u8,
    /// Resolved CSS matrix and origin relative to the box.
    pub xf: Option<([f32; 6], [f32; 2])>,
    pub clip: bool,
    pub scroll: bool,
    /// Top, right, bottom, left; NaN for auto.
    pub sticky: [f32; 4],
    /// Opens a stacking context.
    pub ctx: bool,
    /// Positioned with `z-index: auto`: painted atomically at layer 0.
    pub pseudo_ctx: bool,
    pub canvas: Option<Color>,
    pub extra: Option<Box<Extra>>,
}

const CLEAR: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

impl BoxPaint {
    pub fn empty() -> BoxPaint {
        BoxPaint {
            visible: true,
            bg: CLEAR,
            bw: [0.0; 4],
            bc: [Color::BLACK; 4],
            radius: [0.0; 4],
            opacity: 1.0,
            z: None,
            position: POS_STATIC,
            xf: None,
            clip: false,
            scroll: false,
            sticky: [f32::NAN; 4],
            ctx: false,
            pseudo_ctx: false,
            canvas: None,
            extra: None,
        }
    }

    /// Image ids the display list needs decoded.
    pub fn image_ids(&self) -> Vec<ImageId> {
        let mut out = Vec::new();
        if let Some(e) = &self.extra {
            if let Some(id) = e.img {
                out.push(id);
            }
            for l in &e.layers {
                if let LayerKind::Image { id, .. } = &l.kind {
                    out.push(*id);
                }
            }
        }
        out
    }
}

fn list_text(ty: cs::ListStyleType, n: i32) -> String {
    let alpha = |n: i32, base: u8| -> String {
        let mut v = n.max(1) as u32 - 1;
        let mut s = Vec::new();
        loop {
            s.push(base + (v % 26) as u8);
            if v < 26 {
                break;
            }
            v = v / 26 - 1;
        }
        s.reverse();
        String::from_utf8(s).unwrap_or_default()
    };
    let roman = |n: i32, upper: bool| -> String {
        let table = [
            (1000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ];
        let mut v = n.clamp(1, 3999);
        let mut s = String::new();
        for (val, sym) in table {
            while v >= val {
                s.push_str(sym);
                v -= val;
            }
        }
        if upper {
            s.to_uppercase()
        } else {
            s
        }
    };
    match ty {
        cs::ListStyleType::Decimal => format!("{}.", n),
        cs::ListStyleType::DecimalLeadingZero => format!("{:02}.", n),
        cs::ListStyleType::LowerAlpha => format!("{}.", alpha(n, b'a')),
        cs::ListStyleType::UpperAlpha => format!("{}.", alpha(n, b'A')),
        cs::ListStyleType::LowerRoman => format!("{}.", roman(n, false)),
        cs::ListStyleType::UpperRoman => format!("{}.", roman(n, true)),
        _ => String::new(),
    }
}

fn resolve_stops(stops: &[(Option<cs::Length>, Color)], len: f32) -> Vec<(f32, Color)> {
    let n = stops.len();
    if n == 0 {
        return Vec::new();
    }
    let mut pos: Vec<Option<f32>> = stops
        .iter()
        .map(|(l, _)| {
            l.as_ref().and_then(|l| match l {
                cs::Length::Percent(p) => Some(*p / 100.0),
                other => other
                    .resolve(None)
                    .map(|px| if len > 0.0 { px / len } else { 0.0 }),
            })
        })
        .collect();
    if pos[0].is_none() {
        pos[0] = Some(0.0);
    }
    if pos[n - 1].is_none() {
        pos[n - 1] = Some(1.0);
    }
    let mut last = 0.0f32;
    for p in pos.iter_mut() {
        if let Some(v) = p {
            if *v < last {
                *v = last;
            }
            last = *v;
        }
    }
    let mut i = 0;
    while i < n {
        if pos[i].is_none() {
            let mut j = i;
            while pos[j].is_none() {
                j += 1;
            }
            let a = pos[i - 1].unwrap_or(0.0);
            let b = pos[j].unwrap_or(1.0);
            for k in i..j {
                pos[k] = Some(a + (b - a) * ((k - i + 1) as f32) / ((j - i + 1) as f32));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    stops
        .iter()
        .zip(pos)
        .map(|((_, c), p)| (p.unwrap_or(0.0), *c))
        .collect()
}

fn pos_code(p: cs::Position) -> u8 {
    match p {
        cs::Position::Static => POS_STATIC,
        cs::Position::Relative => POS_RELATIVE,
        cs::Position::Absolute => POS_ABSOLUTE,
        cs::Position::Fixed => POS_FIXED,
        cs::Position::Sticky => POS_STICKY,
    }
}

impl<'a> Ctx<'a> {
    /// Resolves the paint data of a box (after its final size is known).
    pub fn prep_paint(&mut self, b: u32) {
        let bi = b as usize;
        let mut p = BoxPaint::empty();
        let propagated = self.propagates(b);
        if let Some(cp) = self.comp_of(b) {
            let (w, h) = (self.t.boxes[bi].size[0], self.t.boxes[bi].size[1]);
            p.visible = cp.visibility == cs::Visibility::Visible;
            p.bg = col(cp.background_color);
            p.bw = [
                cp.border_top_width,
                cp.border_right_width,
                cp.border_bottom_width,
                cp.border_left_width,
            ];
            p.bc = [
                col(cp.border_top_color),
                col(cp.border_right_color),
                col(cp.border_bottom_color),
                col(cp.border_left_color),
            ];
            let rad = |l: &[cs::Length; 2]| l[0].resolve(Some(w)).unwrap_or(0.0).max(0.0);
            let mut r = [
                rad(&cp.border_top_left_radius),
                rad(&cp.border_top_right_radius),
                rad(&cp.border_bottom_right_radius),
                rad(&cp.border_bottom_left_radius),
            ];
            let mut f = 1.0f32;
            for (sum, len) in [
                (r[0] + r[1], w),
                (r[3] + r[2], w),
                (r[0] + r[3], h),
                (r[1] + r[2], h),
            ] {
                if sum > len && sum > 0.0 {
                    f = f.min(len / sum);
                }
            }
            for v in r.iter_mut() {
                *v *= f;
            }
            p.radius = r;
            p.opacity = cp.opacity.clamp(0.0, 1.0);
            p.z = cp.z_index;
            p.position = pos_code(cp.position);
            if let Some(t) = &cp.transform {
                let m = t.resolve(w, h);
                let ox = cp.transform_origin[0].resolve(Some(w)).unwrap_or(w / 2.0);
                let oy = cp.transform_origin[1].resolve(Some(h)).unwrap_or(h / 2.0);
                p.xf = Some((m, [ox, oy]));
            }
            if !propagated {
                p.clip = !matches!(cp.overflow_x, cs::Overflow::Visible)
                    || !matches!(cp.overflow_y, cs::Overflow::Visible);
                p.scroll = matches!(cp.overflow_x, cs::Overflow::Scroll | cs::Overflow::Auto)
                    || matches!(cp.overflow_y, cs::Overflow::Scroll | cs::Overflow::Auto);
            }
            if cp.position == cs::Position::Sticky {
                let pw = self.t.boxes[bi].pw;
                let bw = if pw[0].is_nan() {
                    self.t.viewport.0
                } else {
                    pw[0]
                };
                let bh = if pw[1].is_nan() {
                    self.t.viewport.1
                } else {
                    pw[1]
                };
                let g = |l: &cs::Length, basis: f32| match l {
                    cs::Length::Auto | cs::Length::None => f32::NAN,
                    other => other.resolve(Some(basis)).unwrap_or(f32::NAN),
                };
                p.sticky = [
                    g(&cp.top, bh),
                    g(&cp.right, bw),
                    g(&cp.bottom, bh),
                    g(&cp.left, bw),
                ];
            }
            let parent_flex = {
                let par = self.t.boxes[bi].parent;
                par != NONE && matches!(self.t.boxes[par as usize].kind, Kind::Flex | Kind::Grid)
            };
            p.ctx = matches!(cp.position, cs::Position::Fixed | cs::Position::Sticky)
                || p.opacity < 1.0
                || p.xf.is_some()
                || (cp.z_index.is_some() && (cp.position != cs::Position::Static || parent_flex));
            p.pseudo_ctx =
                !p.ctx && matches!(cp.position, cs::Position::Relative | cs::Position::Absolute);
            let mut extra = Extra::default();
            for s in cp.box_shadow.iter() {
                let (x, y, blur, spread) = s.px();
                extra.shadows.push(ShadowP {
                    x,
                    y,
                    blur,
                    spread,
                    color: col(s.color.resolve(cp.color)),
                    inset: s.inset,
                });
            }
            self.prep_layers(cp, &mut extra);
            if self.t.boxes[bi].kind == Kind::Replaced {
                let n = NodeId(self.t.boxes[bi].dom);
                if self.dom.tag(n) == Some("img") {
                    if let Some(src) = self.dom.attr(n, "src") {
                        extra.img = self.images.id(src);
                    }
                }
            }
            if cp.display == cs::Display::ListItem && cp.list_style_type != cs::ListStyleType::None
            {
                extra.marker = Some(self.marker_for(b, cp));
            }
            if self.t.boxes[bi].flags & F_TABLE != 0 {
                self.prep_rows(b, &mut extra);
            }
            if b == self.t.root {
                let mut c = p.bg;
                if c.a == 0 {
                    let k0 = self.t.boxes[bi].kids as usize;
                    for i in 0..self.t.boxes[bi].nkids as usize {
                        let kid = self.t.kids[k0 + i];
                        let kn = NodeId(self.t.boxes[kid as usize].dom);
                        if self.dom.tag(kn) == Some("body") {
                            if let Some(bc) = self.comp_of(kid) {
                                c = col(bc.background_color);
                            }
                        }
                    }
                }
                if c.a > 0 {
                    p.canvas = Some(c);
                }
            }
            let empty = extra.shadows.is_empty()
                && extra.layers.is_empty()
                && extra.marker.is_none()
                && extra.img.is_none()
                && extra.rows.is_empty();
            if !empty {
                p.extra = Some(Box::new(extra));
            }
        }
        self.t.paint[bi] = p;
    }

    fn prep_layers(&self, cp: &Computed, extra: &mut Extra) {
        let n = cp.background_image.len();
        for i in 0..n {
            let kind = match &cp.background_image[i] {
                cs::BgImage::None => continue,
                cs::BgImage::Url(u) => match self.images.id(u) {
                    Some(id) => {
                        let (iw, ih) = self.images.size(u).unwrap_or((0, 0));
                        LayerKind::Image {
                            id,
                            iw: iw as f32,
                            ih: ih as f32,
                        }
                    }
                    None => continue,
                },
                cs::BgImage::Gradient(g) => {
                    let stops: Vec<(Option<cs::Length>, Color)> = g
                        .stops
                        .iter()
                        .map(|s| (s.pos.clone(), col(s.color.resolve(cp.color))))
                        .collect();
                    match g.kind {
                        cs::GradientKind::Linear(a) => LayerKind::Linear { angle: a, stops },
                        cs::GradientKind::Radial => LayerKind::Radial { stops },
                    }
                }
            };
            let at = |len: usize| if len == 0 { 0 } else { i % len };
            let rep = cp
                .background_repeat
                .get(at(cp.background_repeat.len()))
                .copied()
                .unwrap_or((BgRepeatKind::Repeat, BgRepeatKind::Repeat));
            extra.layers.push(Layer {
                kind,
                size: cp
                    .background_size
                    .get(at(cp.background_size.len()))
                    .cloned()
                    .unwrap_or(cs::BgSize::Explicit(cs::Length::Auto, cs::Length::Auto)),
                px: cp
                    .background_position_x
                    .get(at(cp.background_position_x.len()))
                    .cloned()
                    .unwrap_or(cs::Length::Percent(0.0)),
                py: cp
                    .background_position_y
                    .get(at(cp.background_position_y.len()))
                    .cloned()
                    .unwrap_or(cs::Length::Percent(0.0)),
                rx: rep.0,
                ry: rep.1,
                origin: cp
                    .background_origin
                    .get(at(cp.background_origin.len()))
                    .copied()
                    .unwrap_or(cs::BgBox::PaddingBox),
                clip: cp
                    .background_clip
                    .get(at(cp.background_clip.len()))
                    .copied()
                    .unwrap_or(cs::BgBox::BorderBox),
            });
        }
    }

    fn marker_for(&self, b: u32, cp: &Computed) -> Marker {
        let n = NodeId(self.t.boxes[b as usize].dom);
        let mut ordinal = 1i32;
        if let Some(p) = self.dom.parent(n) {
            if let Some(s) = self
                .dom
                .attr(p, "start")
                .and_then(|v| v.trim().parse::<i32>().ok())
            {
                ordinal = s;
            }
        }
        let mut prev = self.dom.prev_sibling(n);
        while let Some(s) = prev {
            if self
                .styles
                .get(s)
                .map(|c| c.display == cs::Display::ListItem)
                .unwrap_or(false)
            {
                ordinal += 1;
            }
            prev = self.dom.prev_sibling(s);
        }
        if let Some(v) = self
            .dom
            .attr(n, "value")
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            ordinal = v;
        }
        let kind = match cp.list_style_type {
            cs::ListStyleType::Disc => 0,
            cs::ListStyleType::Circle => 1,
            cs::ListStyleType::Square => 2,
            _ => 3,
        };
        Marker {
            kind,
            text: list_text(cp.list_style_type, ordinal),
            color: col(cp.color),
            fs: cp.font_size,
            bold: cp.font_weight >= 600.0,
        }
    }

    fn prep_rows(&self, b: u32, extra: &mut Extra) {
        let bx = &self.t.boxes[b as usize];
        let (k0, nk) = (bx.kids as usize, bx.nkids as usize);
        let mut i = 0;
        while i < nk {
            let row = self.t.boxes[self.t.kids[k0 + i] as usize].aux;
            let mut j = i;
            while j < nk && self.t.boxes[self.t.kids[k0 + j] as usize].aux == row {
                j += 1;
            }
            if row != NONE {
                if let Some(rc) = self.styles.get(NodeId(row)) {
                    let c = col(rc.background_color);
                    if c.a > 0 {
                        extra.rows.push(RowBg {
                            first: i as u32,
                            last: (j - 1) as u32,
                            color: c,
                        });
                    }
                }
            }
            i = j;
        }
    }
}

// ---------------------------------------------------------------- walking

/// Per-box walking state shared by painting and hit testing.
#[derive(Clone, Copy, Debug)]
pub struct St {
    /// Scrollport `[x, y, w, h]` in painted space.
    pub port: [f32; 4],
    /// Parent content box `[x, y, w, h]` in painted space.
    pub parent: [f32; 4],
    /// Depth of the nearest positioned ancestor.
    pub cb_depth: Option<u16>,
}

pub fn root_state(vp: (f32, f32), scroll: (f32, f32)) -> St {
    St {
        port: [scroll.0, scroll.1, vp.0, vp.1],
        parent: [0.0, 0.0, 1.0e9, 1.0e9],
        cb_depth: None,
    }
}

/// The offset the box itself is painted at: fixed boxes follow the
/// viewport, sticky boxes are pushed by their offsets.
pub fn own_off(t: &LayoutTree, k: u32, off: (f32, f32), scroll: (f32, f32), st: &St) -> (f32, f32) {
    let bx = &t.boxes[k as usize];
    let p = &t.paint[k as usize];
    let mut off = off;
    if p.position == POS_FIXED {
        off = scroll;
    }
    if p.position == POS_STICKY {
        let rect = [bx.abs[0] + off.0, bx.abs[1] + off.1, bx.size[0], bx.size[1]];
        let (dx, dy) = sticky_delta(p.sticky, rect, st.parent, st.port);
        off = (off.0 + dx, off.1 + dy);
    }
    off
}

/// The state and offset the children of `k` are walked with.
pub fn child_state(t: &LayoutTree, k: u32, own: (f32, f32), st: &St) -> (St, (f32, f32)) {
    let bx = &t.boxes[k as usize];
    let p = &t.paint[k as usize];
    let (x, y) = (bx.abs[0] + own.0, bx.abs[1] + own.1);
    let content = [
        x + bx.border[3] + bx.padding[3],
        y + bx.border[0] + bx.padding[0],
        bx.size[0] - bx.border[1] - bx.border[3] - bx.padding[1] - bx.padding[3],
        bx.size[1] - bx.border[0] - bx.border[2] - bx.padding[0] - bx.padding[2],
    ];
    let padding_box = [
        x + bx.border[3],
        y + bx.border[0],
        bx.size[0] - bx.border[1] - bx.border[3],
        bx.size[1] - bx.border[0] - bx.border[2],
    ];
    let mut ns = *st;
    ns.parent = content;
    if p.scroll || p.clip {
        ns.port = padding_box;
    }
    if p.position != POS_STATIC || p.xf.is_some() {
        ns.cb_depth = Some(bx.depth);
    }
    let mut child_off = own;
    if p.scroll && bx.pseudo == 0 {
        if let Some(s) = t.scrolls.get(&bx.dom) {
            child_off = (own.0 - s.0, own.1 - s.1);
        }
    }
    (ns, child_off)
}

/// Layer ordering key of a child among its siblings: negative z contexts,
/// in-flow content, then positioned boxes by z-index.
pub fn layer_key(p: &BoxPaint) -> (i32, i32) {
    if p.ctx || p.pseudo_ctx {
        let z = p.z.unwrap_or(0);
        if z < 0 {
            (-1, z)
        } else {
            (1, z)
        }
    } else {
        (0, 0)
    }
}

#[derive(Clone, Copy)]
struct Clip {
    rect: Rect,
    radius: f32,
    depth: u16,
}

struct Painter<'a> {
    t: &'a LayoutTree,
    text: &'a dyn TextMetrics,
    b: DisplayListBuilder,
    scroll: (f32, f32),
    view: Rect,
    clips: Vec<Clip>,
}

const ALL_PHASES: [Phase; 4] = [
    Phase::Background,
    Phase::Float,
    Phase::Inline,
    Phase::Positioned,
];

fn rect_of(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w.max(0.0), h.max(0.0))
}

impl LayoutTree {
    /// The display list for the part of the page near the viewport at
    /// `scroll` (items far from it are left out). Fixed and sticky boxes are
    /// placed for this scroll offset, which is remembered for hit testing.
    /// Text is rasterised through `text`.
    pub fn display_list(
        &self,
        scroll: (f32, f32),
        viewport: (f32, f32),
        text: &dyn TextMetrics,
    ) -> DisplayList {
        self.last_scroll.set(scroll);
        let (mx, my) = (viewport.0 * 0.5, viewport.1 * 0.5);
        let view = Rect::new(
            scroll.0 - mx,
            scroll.1 - my,
            viewport.0 + 2.0 * mx,
            viewport.1 + 2.0 * my,
        );
        let mut pt = Painter {
            t: self,
            text,
            b: DisplayListBuilder::new(),
            scroll,
            view,
            clips: Vec::new(),
        };
        for (id, img) in self.images.iter() {
            pt.b.add_image(*id, img.clone());
        }
        if self.boxes.is_empty() {
            return pt.b.build();
        }
        if let Some(c) = self.paint[self.root as usize].canvas {
            let w = self.doc_width().max(viewport.0);
            let h = self.doc_height().max(viewport.1);
            pt.b.push(
                Phase::Background,
                DisplayItem::Rect {
                    rect: Rect::new(0.0, 0.0, w, h),
                    color: c,
                },
            );
        }
        let st = root_state(viewport, scroll);
        pt.paint_box(self.root, (0.0, 0.0), st, Phase::Background);
        pt.b.build()
    }
}

impl<'a> Painter<'a> {
    fn push(&mut self, phase: Phase, item: DisplayItem) {
        self.b.push(phase, item);
    }

    fn push_all(&mut self, item: DisplayItem) {
        for ph in ALL_PHASES {
            self.b.push(ph, item.clone());
        }
    }

    fn paint_box(&mut self, k: u32, off: (f32, f32), st: St, bg_phase: Phase) {
        let t = self.t;
        let bx = &t.boxes[k as usize];
        let p = &t.paint[k as usize];
        let own = own_off(t, k, off, self.scroll, &st);
        if bx.flags & F_FORCE == 0 {
            let ink = rect_of(
                bx.ink[0] + own.0,
                bx.ink[1] + own.1,
                bx.ink[2] - bx.ink[0],
                bx.ink[3] - bx.ink[1],
            );
            if !ink.intersects(&self.view) {
                return;
            }
        }
        let rect = rect_of(bx.abs[0] + own.0, bx.abs[1] + own.1, bx.size[0], bx.size[1]);
        let is_ctx = p.ctx || p.pseudo_ctx;
        let saved_clips = if is_ctx {
            Some(std::mem::take(&mut self.clips))
        } else {
            None
        };
        let mut bg_phase = bg_phase;
        if is_ctx {
            let mut opens: Vec<DisplayItem> = Vec::new();
            let saved = saved_clips.as_ref().map(|c| c.as_slice()).unwrap_or(&[]);
            if p.position != POS_FIXED {
                let abs_kid = bx.flags & F_ABS != 0;
                for c in saved {
                    let keep = !abs_kid || st.cb_depth.map(|d| c.depth >= d).unwrap_or(false);
                    if keep {
                        opens.push(DisplayItem::PushClip {
                            rect: c.rect,
                            radius: c.radius,
                        });
                    }
                }
            }
            if let Some((m, o)) = &p.xf {
                opens.push(DisplayItem::PushTransform {
                    matrix: page_matrix(m, *o, (rect.x, rect.y)),
                });
            }
            if p.opacity < 1.0 {
                opens.push(DisplayItem::PushOpacity { alpha: p.opacity });
            }
            self.b.begin_context(p.z.unwrap_or(0), opens);
            bg_phase = Phase::Background;
        }
        if p.visible {
            self.paint_self(k, rect, bg_phase);
        }
        let (cst, coff) = child_state(t, k, own, &st);
        let clipping = p.clip;
        if clipping {
            let pad = rect_of(
                rect.x + bx.border[3],
                rect.y + bx.border[0],
                rect.w - bx.border[1] - bx.border[3],
                rect.h - bx.border[0] - bx.border[2],
            );
            let radius = p.radius[0];
            self.clips.push(Clip {
                rect: pad,
                radius,
                depth: bx.depth,
            });
            self.push_all(DisplayItem::PushClip { rect: pad, radius });
        }
        self.paint_content(k, coff, cst, bg_phase);
        if clipping {
            self.clips.pop();
            self.push_all(DisplayItem::PopClip);
        }
        if is_ctx {
            self.b.end_context();
            if let Some(c) = saved_clips {
                self.clips = c;
            }
        }
    }

    /// Children, inline content and out-of-flow boxes of `k`.
    fn paint_content(&mut self, k: u32, off: (f32, f32), st: St, bg_phase: Phase) {
        let t = self.t;
        let bx = &t.boxes[k as usize];
        let (k0, nk, na) = (bx.kids as usize, bx.nkids as usize, bx.nabs as usize);
        if bx.kind == Kind::Ifc {
            if t.paint[k as usize].visible {
                self.paint_ifc(k, off, st, bg_phase);
            }
        } else {
            for i in 0..nk {
                let kid = t.kids[k0 + i];
                self.paint_box(kid, off, st, bg_phase);
            }
        }
        for j in 0..na {
            let kid = t.kids[k0 + nk + j];
            self.paint_box(kid, off, st, bg_phase);
        }
    }

    fn paint_self(&mut self, k: u32, rect: Rect, phase: Phase) {
        let t = self.t;
        let p = &t.paint[k as usize];
        let bx = &t.boxes[k as usize];
        let empty = Extra::default();
        let ex: &Extra = p.extra.as_deref().unwrap_or(&empty);
        for s in ex.shadows.iter().filter(|s| !s.inset) {
            let r = rect.translate(s.x, s.y);
            self.push(
                phase,
                DisplayItem::Shadow {
                    rect: r,
                    radius: p.radius[0],
                    blur: s.blur,
                    spread: s.spread,
                    color: s.color,
                    inset: false,
                },
            );
        }
        // Table row backgrounds sit under the cells.
        if !ex.rows.is_empty() {
            let (k0, _) = (bx.kids as usize, bx.nkids as usize);
            for row in ex.rows.iter() {
                let mut u = Rect::ZERO;
                for i in row.first..=row.last {
                    let cell = &t.boxes[t.kids[k0 + i as usize] as usize];
                    let cr = rect_of(
                        cell.abs[0] + (rect.x - bx.abs[0]),
                        cell.abs[1] + (rect.y - bx.abs[1]),
                        cell.size[0],
                        cell.size[1],
                    );
                    u = if u.is_empty() { cr } else { u.union(&cr) };
                }
                if !u.is_empty() {
                    self.push(
                        phase,
                        DisplayItem::Rect {
                            rect: u,
                            color: row.color,
                        },
                    );
                }
            }
        }
        let rounded = p.radius.iter().any(|r| *r > 0.0);
        if p.bg.a > 0 && !rect.is_empty() {
            if rounded {
                self.push(
                    phase,
                    DisplayItem::RoundedRect {
                        rect,
                        radius: p.radius,
                        color: p.bg,
                    },
                );
            } else {
                self.push(phase, DisplayItem::Rect { rect, color: p.bg });
            }
        }
        for layer in ex.layers.iter().rev() {
            self.paint_layer(layer, rect, p, bx, phase);
        }
        for s in ex.shadows.iter().filter(|s| s.inset) {
            let inner = rect_of(
                rect.x + bx.border[3],
                rect.y + bx.border[0],
                rect.w - bx.border[1] - bx.border[3],
                rect.h - bx.border[0] - bx.border[2],
            );
            self.push(
                phase,
                DisplayItem::Shadow {
                    rect: inner.translate(s.x, s.y),
                    radius: p.radius[0],
                    blur: s.blur,
                    spread: s.spread,
                    color: s.color,
                    inset: true,
                },
            );
        }
        if p.bw.iter().any(|w| *w > 0.0) && !rect.is_empty() {
            self.push(
                phase,
                DisplayItem::Border {
                    rect,
                    widths: p.bw,
                    colors: p.bc,
                    radius: p.radius,
                },
            );
        }
        if let Some(id) = ex.img {
            let c = rect_of(
                rect.x + bx.border[3] + bx.padding[3],
                rect.y + bx.border[0] + bx.padding[0],
                rect.w - bx.border[1] - bx.border[3] - bx.padding[1] - bx.padding[3],
                rect.h - bx.border[0] - bx.border[2] - bx.padding[0] - bx.padding[2],
            );
            if !c.is_empty() {
                self.push(
                    Phase::Inline,
                    DisplayItem::Image {
                        rect: c,
                        src: id,
                        fit: Fit::Fill,
                        radius: p.radius[0],
                    },
                );
            }
        }
        if let Some(m) = &ex.marker {
            self.paint_marker(k, rect, m);
        }
    }

    fn paint_layer(&mut self, l: &Layer, rect: Rect, p: &BoxPaint, bx: &Bx, phase: Phase) {
        let boxrect = |which: cs::BgBox| -> Rect {
            match which {
                cs::BgBox::BorderBox | cs::BgBox::Text => rect,
                cs::BgBox::PaddingBox => rect_of(
                    rect.x + bx.border[3],
                    rect.y + bx.border[0],
                    rect.w - bx.border[1] - bx.border[3],
                    rect.h - bx.border[0] - bx.border[2],
                ),
                cs::BgBox::ContentBox => rect_of(
                    rect.x + bx.border[3] + bx.padding[3],
                    rect.y + bx.border[0] + bx.padding[0],
                    rect.w - bx.border[1] - bx.border[3] - bx.padding[1] - bx.padding[3],
                    rect.h - bx.border[0] - bx.border[2] - bx.padding[0] - bx.padding[2],
                ),
            }
        };
        let clip = boxrect(l.clip);
        let origin = boxrect(l.origin);
        if clip.is_empty() || origin.is_empty() {
            return;
        }
        let (iw, ih) = match &l.kind {
            LayerKind::Image { iw, ih, .. } => (*iw, *ih),
            _ => (origin.w, origin.h),
        };
        // Tile size.
        let (mut tw, mut th) = match &l.size {
            cs::BgSize::Cover | cs::BgSize::Contain => {
                if iw <= 0.0 || ih <= 0.0 {
                    (origin.w, origin.h)
                } else {
                    let sx = origin.w / iw;
                    let sy = origin.h / ih;
                    let s = if matches!(l.size, cs::BgSize::Cover) {
                        sx.max(sy)
                    } else {
                        sx.min(sy)
                    };
                    (iw * s, ih * s)
                }
            }
            cs::BgSize::Explicit(a, b) => {
                let rw = a.resolve(Some(origin.w));
                let rh = b.resolve(Some(origin.h));
                match (rw, rh) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => (w, if iw > 0.0 { w * ih / iw } else { origin.h }),
                    (None, Some(h)) => (if ih > 0.0 { h * iw / ih } else { origin.w }, h),
                    (None, None) => (iw, ih),
                }
            }
        };
        if tw <= 0.0 || th <= 0.0 {
            tw = origin.w;
            th = origin.h;
        }
        let ox = l.px.resolve(Some(origin.w - tw)).unwrap_or(0.0);
        let oy = l.py.resolve(Some(origin.h - th)).unwrap_or(0.0);
        let (x0, y0) = (origin.x + ox, origin.y + oy);
        let rep_x = !matches!(l.rx, BgRepeatKind::NoRepeat);
        let rep_y = !matches!(l.ry, BgRepeatKind::NoRepeat);
        // Which tiles intersect the clip area.
        let first = |start: f32, size: f32, lo: f32, rep: bool| -> i32 {
            if rep {
                ((lo - start) / size).floor() as i32
            } else {
                0
            }
        };
        let last = |start: f32, size: f32, hi: f32, rep: bool| -> i32 {
            if rep {
                ((hi - start) / size).ceil() as i32 - 1
            } else {
                0
            }
        };
        let (i0, i1) = (
            first(x0, tw, clip.x, rep_x),
            last(x0, tw, clip.right(), rep_x),
        );
        let (j0, j1) = (
            first(y0, th, clip.y, rep_y),
            last(y0, th, clip.bottom(), rep_y),
        );
        if (i1 - i0 + 1).max(0) as i64 * (j1 - j0 + 1).max(0) as i64 > 400 {
            return;
        }
        let needs_clip = p.radius.iter().any(|r| *r > 0.0)
            || i1 > i0
            || j1 > j0
            || x0 < clip.x
            || y0 < clip.y
            || x0 + tw > clip.right() + 0.01
            || y0 + th > clip.bottom() + 0.01;
        if needs_clip {
            self.push(
                phase,
                DisplayItem::PushClip {
                    rect: clip,
                    radius: p.radius[0],
                },
            );
        }
        for j in j0..=j1 {
            for i in i0..=i1 {
                let tr = Rect::new(x0 + i as f32 * tw, y0 + j as f32 * th, tw, th);
                if !tr.intersects(&clip) {
                    continue;
                }
                match &l.kind {
                    LayerKind::Linear { angle, stops } => {
                        let a = angle.to_radians();
                        let len = (tr.w * a.sin()).abs() + (tr.h * a.cos()).abs();
                        let s = resolve_stops(stops, len);
                        if s.len() >= 2 {
                            self.push(
                                phase,
                                DisplayItem::LinearGradient {
                                    rect: tr,
                                    angle: *angle,
                                    stops: s,
                                },
                            );
                        }
                    }
                    LayerKind::Radial { stops } => {
                        let (rx, ry) = (
                            tr.w / 2.0 * std::f32::consts::SQRT_2,
                            tr.h / 2.0 * std::f32::consts::SQRT_2,
                        );
                        let s = resolve_stops(stops, rx);
                        if s.len() >= 2 {
                            self.push(
                                phase,
                                DisplayItem::RadialGradient {
                                    rect: tr,
                                    center: (tr.x + tr.w / 2.0, tr.y + tr.h / 2.0),
                                    radii: (rx, ry),
                                    stops: s,
                                },
                            );
                        }
                    }
                    LayerKind::Image { id, .. } => {
                        let fit = match l.size {
                            cs::BgSize::Cover => Fit::Cover,
                            cs::BgSize::Contain => Fit::Contain,
                            _ => Fit::Fill,
                        };
                        let fit = if fit == Fit::Cover || fit == Fit::Contain {
                            Fit::Fill
                        } else {
                            fit
                        };
                        self.push(
                            phase,
                            DisplayItem::Image {
                                rect: tr,
                                src: *id,
                                fit,
                                radius: 0.0,
                            },
                        );
                    }
                }
            }
        }
        if needs_clip {
            self.push(phase, DisplayItem::PopClip);
        }
    }

    fn first_baseline(&self, k: u32) -> Option<f32> {
        let t = self.t;
        let bx = &t.boxes[k as usize];
        if bx.kind == Kind::Ifc && bx.ifc != NONE {
            if let Some(l) = t.ifcs[bx.ifc as usize].lines.first() {
                return Some(bx.abs[1] + bx.border[0] + bx.padding[0] + l.baseline);
            }
            return None;
        }
        for i in 0..bx.nkids as usize {
            let kid = t.kids[bx.kids as usize + i];
            if let Some(b) = self.first_baseline(kid) {
                return Some(b);
            }
        }
        None
    }

    fn paint_marker(&mut self, k: u32, rect: Rect, m: &Marker) {
        let t = self.t;
        let bx = &t.boxes[k as usize];
        let base_abs = match self.first_baseline(k) {
            Some(b) => b + (rect.y - bx.abs[1]),
            None => rect.y + bx.border[0] + bx.padding[0] + m.fs,
        };
        let right = rect.x + bx.border[3] + bx.padding[3] - m.fs * 0.4;
        let s = m.fs * 0.34;
        let cy = base_abs - m.fs * 0.3;
        match m.kind {
            0 => self.push(
                Phase::Inline,
                DisplayItem::RoundedRect {
                    rect: Rect::new(right - s - m.fs * 0.2, cy - s / 2.0, s, s),
                    radius: [s / 2.0; 4],
                    color: m.color,
                },
            ),
            1 => self.push(
                Phase::Inline,
                DisplayItem::Border {
                    rect: Rect::new(right - s - m.fs * 0.2, cy - s / 2.0, s, s),
                    widths: [1.0; 4],
                    colors: [m.color; 4],
                    radius: [s / 2.0; 4],
                },
            ),
            2 => self.push(
                Phase::Inline,
                DisplayItem::Rect {
                    rect: Rect::new(right - s - m.fs * 0.2, cy - s / 2.0, s, s),
                    color: m.color,
                },
            ),
            _ => {
                if m.text.is_empty() {
                    return;
                }
                let w = self.text.width(&m.text, m.fs, m.bold, false);
                if let Some(run) = self.text.glyph_run(&m.text, m.fs, m.bold, false) {
                    let origin = (right - w, base_abs - self.text.ascent(m.fs));
                    self.push(
                        Phase::Inline,
                        DisplayItem::Text {
                            origin,
                            run,
                            color: m.color,
                        },
                    );
                }
            }
        }
    }

    fn paint_ifc(&mut self, k: u32, off: (f32, f32), st: St, bg_phase: Phase) {
        let t = self.t;
        let bx = &t.boxes[k as usize];
        if bx.ifc == NONE {
            return;
        }
        let d = &t.ifcs[bx.ifc as usize];
        let ox = bx.abs[0] + off.0 + bx.border[3] + bx.padding[3];
        let oy = bx.abs[1] + off.1 + bx.border[0] + bx.padding[0];
        let _ = bg_phase;
        // Inline element backgrounds and borders first.
        for f in d.frags.iter().filter(|f| f.kind == FragKind::InlineBox) {
            let s = &d.st[f.st as usize];
            let r = rect_of(ox + f.x, oy + f.y, f.w, f.h);
            if r.intersects(&self.view) {
                let rad = s.radius.min(r.h / 2.0).min(r.w / 2.0);
                if s.bg.a > 0 {
                    if rad > 0.0 {
                        self.push(
                            Phase::Inline,
                            DisplayItem::RoundedRect {
                                rect: r,
                                radius: [rad; 4],
                                color: s.bg,
                            },
                        );
                    } else {
                        self.push(
                            Phase::Inline,
                            DisplayItem::Rect {
                                rect: r,
                                color: s.bg,
                            },
                        );
                    }
                }
                let mut w = s.bw;
                if f.flags & 1 == 0 {
                    w[3] = 0.0;
                }
                if f.flags & 2 == 0 {
                    w[1] = 0.0;
                }
                if w.iter().any(|v| *v > 0.0) {
                    self.push(
                        Phase::Inline,
                        DisplayItem::Border {
                            rect: r,
                            widths: w,
                            colors: s.bc,
                            radius: [rad; 4],
                        },
                    );
                }
            }
        }
        for f in d.frags.iter() {
            match f.kind {
                FragKind::Atomic => {
                    self.paint_box(f.node, off, st, Phase::Inline);
                }
                FragKind::Text => {
                    let s = &d.st[f.st as usize];
                    let r = rect_of(ox + f.x, oy + f.y, f.w, f.h);
                    if !r.intersects(&self.view) {
                        continue;
                    }
                    let text = &d.text[f.t0 as usize..f.t1 as usize];
                    if let Some((sx, sy, sc)) = s.shadow {
                        self.emit_text(text, (r.x + sx, r.y + sy), s, sc);
                    }
                    self.emit_text(text, (r.x, r.y), s, s.color);
                    let base = r.y + s.asc;
                    let th = (s.fs / 16.0).max(1.0);
                    let dc = if s.deco_color.a > 0 {
                        s.deco_color
                    } else {
                        s.color
                    };
                    if s.deco.underline {
                        self.push(
                            Phase::Inline,
                            DisplayItem::Rect {
                                rect: Rect::new(r.x, base + s.fs * 0.1, r.w, th),
                                color: dc,
                            },
                        );
                    }
                    if s.deco.overline {
                        self.push(
                            Phase::Inline,
                            DisplayItem::Rect {
                                rect: Rect::new(r.x, r.y, r.w, th),
                                color: dc,
                            },
                        );
                    }
                    if s.deco.line_through {
                        self.push(
                            Phase::Inline,
                            DisplayItem::Rect {
                                rect: Rect::new(r.x, base - s.fs * 0.3, r.w, th),
                                color: dc,
                            },
                        );
                    }
                }
                FragKind::InlineBox => {}
            }
        }
    }

    fn emit_text(
        &mut self,
        text: &str,
        origin: (f32, f32),
        s: &super::inline::IStyle,
        color: Color,
    ) {
        if text.is_empty() {
            return;
        }
        if s.ls.abs() > 0.01 {
            let mut x = origin.0;
            let mut buf = [0u8; 4];
            for c in text.chars() {
                let cs_ = c.encode_utf8(&mut buf);
                let w = self.text.width(cs_, s.fs, s.bold, s.italic);
                if c != ' ' {
                    if let Some(run) = self.text.glyph_run(cs_, s.fs, s.bold, s.italic) {
                        self.push(
                            Phase::Inline,
                            DisplayItem::Text {
                                origin: (x, origin.1),
                                run,
                                color,
                            },
                        );
                    }
                }
                x += w + s.ls;
            }
            return;
        }
        if let Some(run) = self.text.glyph_run(text, s.fs, s.bold, s.italic) {
            self.push(Phase::Inline, DisplayItem::Text { origin, run, color });
        }
    }
}

/// The matrix of a box's transform in page space, if it has one.
pub fn box_matrix(t: &LayoutTree, k: u32, own: (f32, f32)) -> Option<Xf> {
    let bx = &t.boxes[k as usize];
    let p = &t.paint[k as usize];
    p.xf.as_ref()
        .map(|(m, o)| page_matrix(m, *o, (bx.abs[0] + own.0, bx.abs[1] + own.1)))
}
