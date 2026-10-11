//! Inline formatting: gathering the inline content of a formatting root
//! (text runs, inline boxes, atomic inlines, `<br>`), greedy line breaking
//! with white-space modes and `overflow-wrap`, vertical metrics with
//! `vertical-align`, `text-align`, and the fragments painting and hit
//! testing read.

use super::super::css::Color;
use super::super::dom::{Dom, NodeId};
use super::super::style::values::{OverflowWrap, WordBreak};
use super::super::style::{self as cs, Computed};
use super::text::{collapse_into, keeps_newlines, keeps_spaces, no_wrap};
use super::tree::*;
use super::{col, Ctx};
use taffy as tf;
use taffy::LayoutPartialTree;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VA {
    Base,
    Top,
    Bottom,
    Middle,
    /// Raise by px (negative lowers).
    Raise(f32),
    /// Raise by a fraction of the element's line height.
    RaisePct(f32),
}

/// Inline style of one element (or of the formatting root), resolved once.
#[derive(Clone, Debug)]
pub struct IStyle {
    /// `enc`-coded owning element.
    pub node: u32,
    pub fs: f32,
    pub bold: bool,
    pub italic: bool,
    pub ls: f32,
    pub wsp: f32,
    pub asc: f32,
    pub desc: f32,
    pub lh: f32,
    pub white: cs::WhiteSpace,
    /// 0 normal, 1 break long words, 2 break anywhere.
    pub brk: u8,
    pub va: VA,
    /// Horizontal margin + border + padding, left and right.
    pub left: f32,
    pub right: f32,
    pub ml: f32,
    pub mr: f32,
    /// Vertical padding + border, top and bottom.
    pub pt: f32,
    pub pb: f32,
    pub color: Color,
    pub deco: cs::DecoLine,
    pub deco_color: Color,
    /// The inline box has a background or border to paint.
    pub paints: bool,
    pub bg: Color,
    pub bc: [Color; 4],
    pub bw: [f32; 4],
    pub radius: f32,
    pub shadow: Option<(f32, f32, Color)>,
    pub indent: f32,
    pub align: cs::TextAlign,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    Text,
    Open,
    Close,
    Atomic,
    Br,
}

/// One entry of the inline content. `Text`: bytes `a..b` of the collapsed
/// text; `Atomic`: `a` is the box index.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub st: u32,
    pub a: u32,
    pub b: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FragKind {
    Text,
    InlineBox,
    Atomic,
}

/// A laid-out piece of a line. Coordinates are relative to the content box
/// of the formatting root. `Text`: `y` is the top of the glyph row and `up`
/// the ascent. `InlineBox`: the painted border box of an inline element's
/// segment (`flags` bit 0 first, bit 1 last). `Atomic`: `node` is the box.
#[derive(Clone, Copy, Debug)]
pub struct Frag {
    pub kind: FragKind,
    pub st: u32,
    pub node: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub up: f32,
    pub t0: u32,
    pub t1: u32,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Line {
    pub y: f32,
    pub h: f32,
    /// Absolute within the content box.
    pub baseline: f32,
    pub x: f32,
    pub w: f32,
    pub f0: u32,
    pub f1: u32,
}

#[derive(Default, Debug)]
pub struct IfcData {
    pub text: String,
    pub items: Vec<Item>,
    pub st: Vec<IStyle>,
    pub lines: Vec<Line>,
    pub frags: Vec<Frag>,
    pub gathered: bool,
    /// Lines were computed for a measurement, not for the final width.
    pub stale: bool,
    pub laid_w: f32,
}

impl Default for IStyle {
    fn default() -> IStyle {
        IStyle {
            node: 0,
            fs: 16.0,
            bold: false,
            italic: false,
            ls: 0.0,
            wsp: 0.0,
            asc: 16.0,
            desc: 4.0,
            lh: 20.0,
            white: cs::WhiteSpace::Normal,
            brk: 0,
            va: VA::Base,
            left: 0.0,
            right: 0.0,
            ml: 0.0,
            mr: 0.0,
            pt: 0.0,
            pb: 0.0,
            color: Color::BLACK,
            deco: cs::DecoLine::default(),
            deco_color: Color::BLACK,
            paints: false,
            bg: Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            bc: [Color::BLACK; 4],
            bw: [0.0; 4],
            radius: 0.0,
            shadow: None,
            indent: 0.0,
            align: cs::TextAlign::Start,
        }
    }
}

fn lpx(l: &cs::Length) -> f32 {
    l.resolve(None).unwrap_or(0.0)
}

fn generated_text(dom: &Dom, n: NodeId, cp: &Computed) -> String {
    let mut out = String::new();
    if let cs::Content::Items(items) = &cp.content {
        for it in items {
            match it {
                cs::ContentItem::Str(s) => out.push_str(s),
                cs::ContentItem::Attr(a) => out.push_str(dom.attr(n, a).unwrap_or("")),
                cs::ContentItem::OpenQuote => out.push('\u{201c}'),
                cs::ContentItem::CloseQuote => out.push('\u{201d}'),
                cs::ContentItem::Url(_) => {}
            }
        }
    }
    out
}

pub enum Wrap {
    None,
    At(f32),
}

pub struct LineOut {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

/// Size and metrics of an atomic inline.
pub struct AtomicInfo {
    pub w: f32,
    pub h: f32,
    pub base: f32,
    pub ml: f32,
    pub mr: f32,
    pub mt: f32,
    pub mb: f32,
    pub va: VA,
    pub lh: f32,
}

struct Meta {
    idx: usize,
    raise: f32,
    up: f32,
    above: f32,
    below: f32,
    pad: f32,
    mode: u8,
}

struct OpenBox {
    st: u32,
    node: u32,
    x0: f32,
    first: bool,
    up: f32,
    h: f32,
    raise: f32,
    paints: bool,
}

/// Line builder state.
struct Lb {
    frags: Vec<Frag>,
    lines: Vec<Line>,
    y: f32,
    x: f32,
    limit: f32,
    has: bool,
    pending: f32,
    maxw: f32,
    s_above: f32,
    s_below: f32,
    above: f32,
    below: f32,
    tall: f32,
    meta: Vec<Meta>,
    open: Vec<OpenBox>,
    f0: usize,
    first: bool,
    indent: f32,
    rstack: Vec<(f32, u8)>,
    cur: Option<(usize, u32)>,
    align: u8,
}

const EPS: f32 = 0.01;

impl Lb {
    fn new(limit: f32, root: &IStyle, frags: Vec<Frag>, lines: Vec<Line>) -> Lb {
        let hl = (root.lh - (root.asc + root.desc)) / 2.0;
        let align = match root.align {
            cs::TextAlign::End | cs::TextAlign::Right => 1,
            cs::TextAlign::Center => 2,
            _ => 0,
        };
        let mut lb = Lb {
            frags,
            lines,
            y: 0.0,
            x: 0.0,
            limit,
            has: false,
            pending: 0.0,
            maxw: 0.0,
            s_above: root.asc + hl,
            s_below: root.desc + hl,
            above: 0.0,
            below: 0.0,
            tall: 0.0,
            meta: Vec::new(),
            open: Vec::new(),
            f0: 0,
            first: true,
            indent: root.indent,
            rstack: Vec::new(),
            cur: None,
            align,
        };
        lb.start_line();
        lb
    }

    fn start_line(&mut self) {
        self.x = if self.first { self.indent } else { 0.0 };
        self.has = false;
        self.pending = 0.0;
        self.above = self.s_above;
        self.below = self.s_below;
        self.tall = 0.0;
        self.meta.clear();
        self.f0 = self.frags.len();
        self.cur = None;
        let x = self.x;
        for o in self.open.iter_mut() {
            o.x0 = x;
            o.first = false;
        }
    }

    fn fits(&self, w: f32) -> bool {
        self.limit.is_infinite() || self.x + self.pending + w <= self.limit + EPS
    }

    fn cur_raise(&self) -> (f32, u8) {
        self.rstack.last().copied().unwrap_or((0.0, 0))
    }

    fn push_meta(&mut self, m: Meta) {
        if m.mode == 0 {
            self.above = self.above.max(m.above + m.raise);
            self.below = self.below.max(m.below - m.raise);
        } else {
            self.tall = self.tall.max(m.above + m.below);
        }
        self.meta.push(m);
    }

    fn seg(&mut self, oi: usize, x1: f32, last: bool) {
        let (st, node, x0, first, up, h, raise) = {
            let o = &self.open[oi];
            (o.st, o.node, o.x0, o.first, o.up, o.h, o.raise)
        };
        let w = x1 - x0;
        if w <= 0.0 && !first {
            return;
        }
        let idx = self.frags.len();
        self.frags.push(Frag {
            kind: FragKind::InlineBox,
            st,
            node,
            x: x0,
            y: 0.0,
            w: w.max(0.0),
            h,
            up,
            t0: 0,
            t1: 0,
            flags: (first as u8) | ((last as u8) << 1),
        });
        self.meta.push(Meta {
            idx,
            raise,
            up,
            above: 0.0,
            below: 0.0,
            pad: 0.0,
            mode: 0,
        });
    }

    fn end_line(&mut self) {
        let xe = self.x;
        for i in 0..self.open.len() {
            if self.open[i].paints {
                self.seg(i, xe, false);
            }
        }
        let h = (self.above + self.below).max(self.tall);
        let base = self.y + self.above;
        for m in &self.meta {
            let f = &mut self.frags[m.idx];
            f.y = match m.mode {
                0 => base - m.raise - m.up,
                1 => self.y + m.pad,
                _ => self.y + h - (m.above + m.below) + m.pad,
            };
        }
        let mut line = Line {
            y: self.y,
            h,
            baseline: base,
            x: 0.0,
            w: xe,
            f0: self.f0 as u32,
            f1: self.frags.len() as u32,
        };
        if self.limit.is_finite() && self.align != 0 {
            let free = self.limit - xe;
            if free > 0.0 {
                let off = if self.align == 1 { free } else { free / 2.0 };
                for f in &mut self.frags[self.f0..] {
                    f.x += off;
                }
                line.x = off;
            }
        }
        self.lines.push(line);
        self.maxw = self.maxw.max(xe);
        self.y += h;
        self.first = false;
        self.start_line();
    }

    fn place_word(&mut self, st: u32, is: &IStyle, a: u32, b: u32, w: f32) {
        let gap = self.pending;
        self.pending = 0.0;
        self.has = true;
        if let Some((fi, cst)) = self.cur {
            if cst == st && fi + 1 == self.frags.len() {
                let f = &mut self.frags[fi];
                f.t1 = b;
                f.w += gap + w;
                self.x += gap + w;
                return;
            }
        }
        self.x += gap;
        let hl = (is.lh - (is.asc + is.desc)) / 2.0;
        let idx = self.frags.len();
        self.frags.push(Frag {
            kind: FragKind::Text,
            st,
            node: is.node,
            x: self.x,
            y: 0.0,
            w,
            h: is.asc + is.desc,
            up: is.asc,
            t0: a,
            t1: b,
            flags: 0,
        });
        self.x += w;
        self.cur = Some((idx, st));
        let (raise, mode) = self.cur_raise();
        self.push_meta(Meta {
            idx,
            raise,
            up: is.asc,
            above: is.asc + hl,
            below: is.desc + hl,
            pad: hl,
            mode,
        });
    }

    fn place_atomic(&mut self, node: u32, parent_fs: f32, i: &AtomicInfo) {
        let gap = self.pending;
        self.pending = 0.0;
        self.x += gap;
        self.has = true;
        self.cur = None;
        let (mut raise, mut mode) = self.cur_raise();
        let above = i.mt + i.base;
        let below = (i.h - i.base) + i.mb;
        match i.va {
            VA::Base => {}
            VA::Raise(r) => raise += r,
            VA::RaisePct(p) => raise += p * i.lh,
            VA::Top => mode = 1,
            VA::Bottom => mode = 2,
            VA::Middle => raise += 0.25 * parent_fs - (above - below) / 2.0,
        }
        let idx = self.frags.len();
        self.frags.push(Frag {
            kind: FragKind::Atomic,
            st: 0,
            node,
            x: self.x + i.ml,
            y: 0.0,
            w: i.w,
            h: i.h,
            up: i.base,
            t0: 0,
            t1: 0,
            flags: 0,
        });
        self.x += i.ml + i.w + i.mr;
        self.push_meta(Meta {
            idx,
            raise,
            up: i.base,
            above,
            below,
            pad: i.mt,
            mode,
        });
    }

    fn open_box(&mut self, st: u32, is: &IStyle) {
        let (r0, m0) = self.cur_raise();
        let (raise, mode) = match is.va {
            VA::Base => (r0, m0),
            VA::Raise(v) => (r0 + v, m0),
            VA::RaisePct(p) => (r0 + p * is.lh, m0),
            VA::Top => (r0, 1),
            VA::Bottom => (r0, 2),
            VA::Middle => (r0 + 0.25 * is.fs - (is.asc - is.desc) / 2.0, m0),
        };
        self.rstack.push((raise, mode));
        self.x += self.pending;
        self.pending = 0.0;
        self.open.push(OpenBox {
            st,
            node: is.node,
            x0: self.x + is.ml,
            first: true,
            up: is.asc + is.pt,
            h: is.asc + is.desc + is.pt + is.pb,
            raise,
            paints: is.paints,
        });
        self.x += is.left;
    }

    fn close_box(&mut self, is: &IStyle) {
        self.x += is.right;
        if let Some(oi) = self.open.len().checked_sub(1) {
            if self.open[oi].paints {
                let x1 = self.x - is.mr;
                self.seg(oi, x1, true);
            }
            self.open.pop();
            self.rstack.pop();
        }
    }
}

impl<'a> Ctx<'a> {
    /// Resolves the inline style of an element.
    pub fn make_istyle(&self, cp: &Computed, node: u32, root: bool) -> IStyle {
        let text = self.text;
        let fs = cp.font_size;
        let asc = text.ascent(fs);
        let desc = text.descent(fs);
        let lh = match cp.line_height {
            cs::LineHeight::Normal => asc + desc,
            cs::LineHeight::Number(n) => n * fs,
            cs::LineHeight::Px(p) => p,
        };
        let va = match &cp.vertical_align {
            cs::VAlign::Keyword(k) => match k {
                cs::VAlignKw::Sub => VA::Raise(-0.2 * fs),
                cs::VAlignKw::Super => VA::Raise(0.33 * fs),
                cs::VAlignKw::Top => VA::Top,
                cs::VAlignKw::Bottom => VA::Bottom,
                cs::VAlignKw::Middle => VA::Middle,
                _ => VA::Base,
            },
            cs::VAlign::Length(cs::Length::Percent(p)) => VA::RaisePct(*p / 100.0),
            cs::VAlign::Length(l) => VA::Raise(lpx(l)),
        };
        let bw = [
            cp.border_top_width,
            cp.border_right_width,
            cp.border_bottom_width,
            cp.border_left_width,
        ];
        let bg = col(cp.background_color);
        let has_border = bw.iter().any(|w| *w > 0.0);
        let (ml, mr) = (lpx(&cp.margin_left), lpx(&cp.margin_right));
        let (pl, pr) = (lpx(&cp.padding_left), lpx(&cp.padding_right));
        let shadow = cp.text_shadow.first().map(|s| {
            let (x, y, _, _) = s.px();
            (x, y, col(s.color.resolve(cp.color)))
        });
        let r = &cp.border_top_left_radius[0];
        IStyle {
            node,
            fs,
            bold: cp.font_weight >= 600.0,
            italic: cp.font_style != cs::FontStyle::Normal,
            ls: cp.letter_spacing,
            wsp: cp.word_spacing,
            asc,
            desc,
            lh,
            white: cp.white_space,
            brk: if cp.word_break == WordBreak::BreakAll {
                2
            } else if cp.word_break == WordBreak::BreakWord
                || matches!(
                    cp.overflow_wrap,
                    OverflowWrap::BreakWord | OverflowWrap::Anywhere
                )
            {
                1
            } else {
                0
            },
            va,
            left: if root { 0.0 } else { ml + bw[3] + pl },
            right: if root { 0.0 } else { mr + bw[1] + pr },
            ml: if root { 0.0 } else { ml },
            mr: if root { 0.0 } else { mr },
            pt: if root {
                0.0
            } else {
                bw[0] + lpx(&cp.padding_top)
            },
            pb: if root {
                0.0
            } else {
                bw[2] + lpx(&cp.padding_bottom)
            },
            color: col(cp.color),
            deco: cp.text_decoration_line,
            deco_color: col(cp.text_decoration_color),
            paints: !root && (bg.a > 0 || has_border),
            bg,
            bc: [
                col(cp.border_top_color),
                col(cp.border_right_color),
                col(cp.border_bottom_color),
                col(cp.border_left_color),
            ],
            bw,
            radius: lpx(r),
            shadow,
            indent: lpx(&cp.text_indent),
            align: cp.text_align,
        }
    }

    /// Collects the inline content of formatting root `b` (once).
    pub fn ensure_gathered(&mut self, b: u32) {
        let bi = b as usize;
        if self.t.boxes[bi].ifc == NONE {
            self.t.boxes[bi].ifc = self.t.ifcs.len() as u32;
            self.t.ifcs.push(IfcData::default());
        }
        let ii = self.t.boxes[bi].ifc as usize;
        if self.t.ifcs[ii].gathered {
            return;
        }
        let mut d = std::mem::take(&mut self.t.ifcs[ii]);
        d.text.clear();
        d.items.clear();
        d.st.clear();
        d.lines.clear();
        d.frags.clear();
        self.gather_into(b, &mut d);
        d.gathered = true;
        d.stale = true;
        self.t.ifcs[ii] = d;
    }

    fn gather_into(&mut self, b: u32, d: &mut IfcData) {
        let (dom_id, pseudo, ra, rb) = {
            let bx = &self.t.boxes[b as usize];
            (bx.dom, bx.pseudo, bx.ra, bx.rb)
        };
        if dom_id == NONE {
            return;
        }
        let n = NodeId(dom_id);
        let rootc = match self.text_comp_of(b) {
            Some(c) => c,
            None => return,
        };
        let marker = if pseudo == 1 || pseudo == 2 {
            pseudo
        } else {
            0
        };
        let root = self.make_istyle(rootc, enc(n, marker), true);
        d.st.push(root);
        let mut prev_space = false;
        if marker != 0 {
            let s = generated_text(self.dom, n, rootc);
            self.push_text(d, 0, &s, rootc, &mut prev_space);
            return;
        }
        let mut flat = Vec::new();
        flatten(self.dom, self.styles, n, &mut flat);
        let end = if rb == NONE {
            flat.len()
        } else {
            (rb as usize).min(flat.len())
        };
        let start = (ra as usize).min(end);
        self.walk(d, &flat[start..end], 0, rootc, &mut prev_space);
    }

    fn push_text(&self, d: &mut IfcData, st: u32, s: &str, cp: &Computed, prev_space: &mut bool) {
        let a = d.text.len();
        collapse_into(
            s,
            cp.white_space,
            cp.text_transform,
            prev_space,
            &mut d.text,
        );
        let e = d.text.len();
        if e > a {
            d.items.push(Item {
                kind: ItemKind::Text,
                st,
                a: a as u32,
                b: e as u32,
            });
        }
    }

    fn walk(
        &self,
        d: &mut IfcData,
        children: &[FChild],
        st: u32,
        parent: &Computed,
        prev_space: &mut bool,
    ) {
        let dom = self.dom;
        let styles = self.styles;
        for &ch in children {
            if let FChild::Text(t) = ch {
                if let Some(s) = dom.text(t) {
                    self.push_text(d, st, s, parent, prev_space);
                }
                continue;
            }
            match classify(dom, styles, ch) {
                Class::Br => {
                    d.items.push(Item {
                        kind: ItemKind::Br,
                        st,
                        a: 0,
                        b: 0,
                    });
                    *prev_space = false;
                }
                Class::Atomic => {
                    let k = match ch {
                        FChild::Elem(n) => box_for(&self.t.box_of, n),
                        FChild::Pseudo(n, p) => self.t.pbox.get(&enc(n, p)).copied(),
                        FChild::Text(_) => None,
                    };
                    if let Some(k) = k {
                        d.items.push(Item {
                            kind: ItemKind::Atomic,
                            st,
                            a: k,
                            b: 0,
                        });
                        *prev_space = false;
                    }
                }
                Class::InlineBox => {
                    let cp = match comp(styles, ch) {
                        Some(c) => c,
                        None => continue,
                    };
                    let node = match ch {
                        FChild::Elem(n) => enc(n, 0),
                        FChild::Pseudo(n, p) => enc(n, p),
                        FChild::Text(n) => n.0,
                    };
                    let si = d.st.len() as u32;
                    let mut ist = self.make_istyle(cp, node, false);
                    {
                        // Text decorations of ancestors draw across inline children.
                        let parent_st = &d.st[st as usize];
                        let pd = parent_st.deco;
                        if pd.underline || pd.overline || pd.line_through {
                            let own = ist.deco;
                            if !(own.underline || own.overline || own.line_through) {
                                ist.deco_color = parent_st.deco_color;
                            }
                            ist.deco = cs::DecoLine {
                                underline: own.underline || pd.underline,
                                overline: own.overline || pd.overline,
                                line_through: own.line_through || pd.line_through,
                            };
                        }
                    }
                    d.st.push(ist);
                    d.items.push(Item {
                        kind: ItemKind::Open,
                        st: si,
                        a: 0,
                        b: 0,
                    });
                    match ch {
                        FChild::Elem(n) => {
                            let mut sub = Vec::new();
                            flatten(dom, styles, n, &mut sub);
                            self.walk(d, &sub, si, cp, prev_space);
                        }
                        FChild::Pseudo(n, _) => {
                            let s = generated_text(dom, n, cp);
                            self.push_text(d, si, &s, cp, prev_space);
                        }
                        FChild::Text(_) => {}
                    }
                    d.items.push(Item {
                        kind: ItemKind::Close,
                        st: si,
                        a: 0,
                        b: 0,
                    });
                }
                _ => {}
            }
        }
    }

    fn tw(&mut self, s: &str, is: &IStyle) -> f32 {
        let text = self.text;
        let w = self.t.widths.width(text, s, is.fs, is.bold, is.italic);
        w + is.ls * s.chars().count() as f32
    }

    /// Margin, border and padding of a box in px (percentages of `basis`).
    pub fn insets(&self, k: u32, basis: f32) -> ([f32; 4], [f32; 4]) {
        match self.comp_of(k) {
            Some(c) => {
                let p = |l: &cs::Length| l.resolve(Some(basis)).unwrap_or(0.0);
                (
                    [
                        c.border_top_width,
                        c.border_right_width,
                        c.border_bottom_width,
                        c.border_left_width,
                    ],
                    [
                        p(&c.padding_top),
                        p(&c.padding_right),
                        p(&c.padding_bottom),
                        p(&c.padding_left),
                    ],
                )
            }
            None => ([0.0; 4], [0.0; 4]),
        }
    }

    /// Stores a layout result for a box laid out outside a parent algorithm.
    pub fn commit_layout(&mut self, k: u32, out: &tf::LayoutOutput, loc: [f32; 2], basis: f32) {
        let (border, padding) = self.insets(k, basis);
        let bx = &mut self.t.boxes[k as usize];
        bx.loc = loc;
        bx.size = [out.size.width, out.size.height];
        bx.scroll_size = [
            out.scrollable_overflow_rect.right,
            out.scrollable_overflow_rect.bottom,
        ];
        bx.border = border;
        bx.padding = padding;
    }

    pub fn perform(
        &mut self,
        k: u32,
        known: tf::Size<Option<f32>>,
        parent: tf::Size<Option<f32>>,
        avail: tf::Size<tf::AvailableSpace>,
        mode: tf::RunMode,
    ) -> tf::LayoutOutput {
        self.compute_child_layout(
            tf::NodeId::from(k as usize),
            tf::LayoutInput {
                run_mode: mode,
                sizing_mode: tf::SizingMode::InherentSize,
                axis: tf::RequestedAxis::Both,
                known_dimensions: known,
                known_dimensions_are_definite: tf::Size {
                    width: true,
                    height: true,
                },
                parent_size: parent,
                available_space: avail,
                vertical_margins_are_collapsible: tf::Line {
                    start: false,
                    end: false,
                },
            },
        )
    }

    /// Shrink-to-fit width of a box: `min(max-content, max(min-content, avail))`.
    pub fn shrink_to_fit(&mut self, k: u32, avail: Option<f32>) -> f32 {
        let parent = tf::Size {
            width: avail,
            height: None,
        };
        let q = |cx: &mut Ctx<'a>, w: tf::AvailableSpace| -> f32 {
            cx.perform(
                k,
                tf::Size::NONE,
                parent,
                tf::Size {
                    width: w,
                    height: tf::AvailableSpace::MaxContent,
                },
                tf::RunMode::ComputeSize,
            )
            .size
            .width
        };
        let maxw = q(&mut *self, tf::AvailableSpace::MaxContent);
        match avail {
            Some(a) if maxw > a => {
                let minw = q(&mut *self, tf::AvailableSpace::MinContent);
                minw.max(a).min(maxw)
            }
            _ => maxw,
        }
    }

    /// Lays out an atomic inline at the width it takes in a line of `avail`.
    pub fn atomic_layout(&mut self, k: u32, avail: Option<f32>) -> AtomicInfo {
        let cp = self.comp_of(k);
        let basis = avail.unwrap_or(self.t.viewport.0);
        let mg = |l: &cs::Length| -> f32 {
            match l {
                cs::Length::Auto => 0.0,
                other => other.resolve(Some(basis)).unwrap_or(0.0),
            }
        };
        let (ml, mr, mt, mb) = match cp {
            Some(c) => (
                mg(&c.margin_left),
                mg(&c.margin_right),
                mg(&c.margin_top),
                mg(&c.margin_bottom),
            ),
            None => (0.0, 0.0, 0.0, 0.0),
        };
        let kind = self.t.boxes[k as usize].kind;
        let auto_w = kind != Kind::Replaced
            && cp
                .map(|c| {
                    matches!(
                        c.width,
                        cs::Length::Auto
                            | cs::Length::MinContent
                            | cs::Length::MaxContent
                            | cs::Length::FitContent
                    )
                })
                .unwrap_or(true);
        let inner_avail = avail.map(|a| (a - ml - mr).max(0.0));
        let mut known = tf::Size::NONE;
        if auto_w {
            known.width = Some(self.shrink_to_fit(k, inner_avail));
        }
        let aw = match known.width {
            Some(w) => tf::AvailableSpace::Definite(w),
            None => tf::AvailableSpace::MaxContent,
        };
        let out = self.perform(
            k,
            known,
            tf::Size {
                width: inner_avail,
                height: None,
            },
            tf::Size {
                width: aw,
                height: tf::AvailableSpace::MaxContent,
            },
            tf::RunMode::PerformLayout,
        );
        self.commit_layout(k, &out, [0.0, 0.0], basis);
        let h = out.size.height;
        let base = out.baselines.first.unwrap_or(h).clamp(-1.0e6, h.max(0.0));
        let (va, lh) = match cp {
            Some(c) => {
                let st = self.make_istyle(c, 0, false);
                (st.va, st.lh)
            }
            None => (VA::Base, 0.0),
        };
        AtomicInfo {
            w: out.size.width,
            h,
            base,
            ml,
            mr,
            mt,
            mb,
            va,
            lh,
        }
    }

    /// Breaks the content of formatting root `b` into lines at `wrap`.
    /// With `align` the lines are aligned and the result is the final one.
    pub fn run_lines(&mut self, b: u32, wrap: Wrap, align: bool) -> LineOut {
        self.ensure_gathered(b);
        let ii = self.t.boxes[b as usize].ifc as usize;
        let mut d = std::mem::take(&mut self.t.ifcs[ii]);
        let out = self.lines_core(&mut d, wrap, align);
        d.stale = !align;
        self.t.ifcs[ii] = d;
        out
    }

    fn lines_core(&mut self, d: &mut IfcData, wrap: Wrap, align: bool) -> LineOut {
        let limit = match wrap {
            Wrap::At(w) => w.max(0.0),
            Wrap::None => f32::INFINITY,
        };
        let avail = match wrap {
            Wrap::At(w) => Some(w.max(0.0)),
            Wrap::None => None,
        };
        let mut root = match d.st.first() {
            Some(r) => r.clone(),
            None => IStyle::default(),
        };
        if !align {
            root.align = cs::TextAlign::Start;
        }
        let frags = std::mem::take(&mut d.frags);
        let lines = std::mem::take(&mut d.lines);
        let mut frags = frags;
        let mut lines = lines;
        frags.clear();
        lines.clear();
        let mut lb = Lb::new(limit, &root, frags, lines);
        for ii in 0..d.items.len() {
            let item = d.items[ii];
            match item.kind {
                ItemKind::Open => {
                    let is = d.st[item.st as usize].clone();
                    lb.open_box(item.st, &is);
                }
                ItemKind::Close => {
                    let is = d.st[item.st as usize].clone();
                    lb.close_box(&is);
                }
                ItemKind::Br => lb.end_line(),
                ItemKind::Atomic => {
                    let is = &d.st[item.st as usize];
                    let can_wrap = !no_wrap(is.white) && limit.is_finite();
                    let info = self.atomic_layout(item.a, avail);
                    let full = info.ml + info.w + info.mr;
                    if can_wrap && lb.has && !lb.fits(full) {
                        lb.end_line();
                    }
                    lb.place_atomic(item.a, is.fs, &info);
                }
                ItemKind::Text => {
                    let is = d.st[item.st as usize].clone();
                    let s: &str = &d.text[item.a as usize..item.b as usize];
                    self.break_text(&mut lb, item.st, &is, s, item.a);
                }
            }
        }
        if lb.has {
            lb.end_line();
        }
        let out = LineOut {
            width: lb.maxw,
            height: lb.y,
            baseline: lb.lines.first().map(|l| l.baseline).unwrap_or(0.0),
        };
        d.frags = lb.frags;
        d.lines = lb.lines;
        out
    }

    /// Places one text run: words, collapsible spaces, forced breaks.
    fn break_text(&mut self, lb: &mut Lb, st: u32, is: &IStyle, s: &str, base: u32) {
        let white = is.white;
        let nl = keeps_newlines(white);
        let sp = keeps_spaces(white);
        let wrap_ok = !no_wrap(white) && lb.limit.is_finite();
        let space_w = self.tw(" ", is) + is.wsp;
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'\n' {
                if nl {
                    lb.end_line();
                } else if lb.has {
                    lb.pending = space_w;
                }
                i += 1;
                continue;
            }
            if c == b' ' {
                if sp {
                    let mut j = i;
                    while j < bytes.len() && bytes[j] == b' ' {
                        j += 1;
                    }
                    let w = (j - i) as f32 * space_w;
                    lb.place_word(st, is, base + i as u32, base + j as u32, w);
                    i = j;
                } else {
                    if lb.has {
                        lb.pending = space_w;
                    }
                    i += 1;
                }
                continue;
            }
            // A run of non-space chars, cut into break segments.
            let mut j = i;
            while j < bytes.len() && bytes[j] != b' ' && bytes[j] != b'\n' {
                j += 1;
            }
            let mut k = i;
            while k < j {
                let e = seg_end(s, k, j, is.brk);
                let seg = &s[k..e];
                let ww = self.tw(seg, is);
                if wrap_ok && lb.has && !lb.fits(ww) {
                    lb.end_line();
                }
                if wrap_ok && !lb.fits(ww) && is.brk != 0 {
                    // Too wide even for an empty line: split by chars.
                    let mut p = k;
                    while p < e {
                        let mut q = p;
                        let mut acc = 0.0f32;
                        loop {
                            let nx = next_char(s, q, e);
                            let cw = self.tw(&s[q..nx], is);
                            if q > p && !lb.fits(acc + cw) {
                                break;
                            }
                            acc += cw;
                            q = nx;
                            if q >= e {
                                break;
                            }
                        }
                        lb.place_word(st, is, base + p as u32, base + q as u32, acc);
                        p = q;
                        if p < e {
                            lb.end_line();
                        }
                    }
                } else {
                    lb.place_word(st, is, base + k as u32, base + e as u32, ww);
                }
                k = e;
            }
            i = j;
        }
    }

    /// The final line layout of formatting root `b` and the placement of
    /// its atomic inlines. Called from the position pass.
    pub fn final_lines(&mut self, b: u32) {
        self.ensure_gathered(b);
        let (w, ox, oy) = {
            let bx = &self.t.boxes[b as usize];
            (
                bx.size[0] - bx.border[1] - bx.border[3] - bx.padding[1] - bx.padding[3],
                bx.border[3] + bx.padding[3],
                bx.border[0] + bx.padding[0],
            )
        };
        let ii = self.t.boxes[b as usize].ifc as usize;
        let redo = {
            let d = &self.t.ifcs[ii];
            d.stale || (d.laid_w - w).abs() > EPS
        };
        if redo {
            self.run_lines(b, Wrap::At(w.max(0.0)), true);
            let d = &mut self.t.ifcs[ii];
            d.laid_w = w;
            d.stale = false;
        }
        let atoms: Vec<(u32, f32, f32)> = self.t.ifcs[ii]
            .frags
            .iter()
            .filter(|f| f.kind == FragKind::Atomic)
            .map(|f| (f.node, f.x, f.y))
            .collect();
        for (k, x, y) in atoms {
            self.t.boxes[k as usize].loc = [ox + x, oy + y];
        }
    }
}

fn next_char(s: &str, i: usize, end: usize) -> usize {
    let mut e = i + 1;
    while e < end && !s.is_char_boundary(e) {
        e += 1;
    }
    e.min(end)
}

/// End of the break segment starting at `i` within the word `i..j`.
fn seg_end(s: &str, i: usize, j: usize, brk: u8) -> usize {
    if brk == 2 {
        return next_char(s, i, j);
    }
    let b = s.as_bytes();
    let mut p = i;
    while p + 1 < j {
        if b[p] == b'-' && p > i {
            return p + 1;
        }
        p += 1;
    }
    j
}

#[allow(dead_code)]
fn _unused(_: &Dom) {}
