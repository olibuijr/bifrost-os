//! The compact box arena (parallel to the DOM, u32 indices), box generation
//! from the DOM and the style tree, and the mapping of a `Computed` style to
//! a `taffy::Style` on demand.
//!
//! Only block-level boxes, inline formatting roots, atomic inlines (inline
//! blocks, replaced elements) and flex/grid items get a box. Inline elements
//! and text live inside their formatting root's `IfcData` (see `inline.rs`).

use super::super::dom::{Dom, NodeId, NodeKind};
use super::super::style::values::RepeatCount;
use super::super::style::{self as cs, Computed, StyleTree};
use super::{Ctx, ImageSizes, LayoutTree};
use std::collections::HashMap;
use taffy as tf;

/// "No box" / "no node" marker for u32 links.
pub const NONE: u32 = u32::MAX;

pub const F_ABS: u16 = 1;
pub const F_FIXED: u16 = 2;
pub const F_ATOMIC: u16 = 4;
pub const F_TABLE: u16 = 8;
pub const F_CELL: u16 = 16;
/// Set on a box whose subtree must never be culled (fixed, sticky, transforms).
pub const F_FORCE: u16 = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Block container whose children are boxes.
    Block,
    Flex,
    Grid,
    /// Block container (or anonymous box) whose content is inline.
    Ifc,
    /// Replaced element: a leaf with an intrinsic size.
    Replaced,
}

/// One box. Layout results are relative to the parent's border box, except
/// `abs` (page space, after the position pass).
#[derive(Clone, Debug)]
pub struct Bx {
    pub kind: Kind,
    /// 0 element, 1 `::before`, 2 `::after`, 3 anonymous (style from `dom`).
    pub pseudo: u8,
    pub flags: u16,
    pub dom: u32,
    pub parent: u32,
    pub depth: u16,
    /// Slice `kids[kids .. kids + nkids]` are in-flow, then `nabs` out-of-flow.
    pub kids: u32,
    pub nkids: u32,
    pub nabs: u32,
    /// Inline content range, in flattened child indexes of `dom`.
    pub ra: u32,
    pub rb: u32,
    pub ifc: u32,
    /// Out-of-flow: previous in-flow sibling box. Table cell: row element.
    pub aux: u32,
    /// Table cell grid position: row, column, row span, column span.
    pub grid: [u16; 4],
    /// Intrinsic size of a replaced element.
    pub intr: [f32; 2],
    /// Last parent size seen by layout (for percentage `calc()`), height NaN when unknown.
    pub pw: [f32; 2],
    pub loc: [f32; 2],
    pub size: [f32; 2],
    pub scroll_size: [f32; 2],
    /// Top, right, bottom, left.
    pub border: [f32; 4],
    pub padding: [f32; 4],
    pub abs: [f32; 2],
    /// Page-space bounds of the box and its visible overflow: x0, y0, x1, y1.
    pub ink: [f32; 4],
}

// ---------------------------------------------------------------- children

/// A child of an element after `display: contents` flattening.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FChild {
    Text(NodeId),
    Elem(NodeId),
    /// `::before` (1) or `::after` (2) of an element.
    Pseudo(NodeId, u8),
}

/// How a child takes part in its parent's formatting context.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Text,
    Br,
    /// Inline element: no box, its content joins the formatting root.
    InlineBox,
    /// Inline-level box laid out as a unit (inline-block, replaced, float).
    Atomic,
    Block,
    /// Absolutely positioned or fixed.
    Abs,
}

pub fn flatten(dom: &Dom, styles: &StyleTree, n: NodeId, out: &mut Vec<FChild>) {
    flatten_in(dom, styles, n, true, out);
}

fn flatten_in(dom: &Dom, styles: &StyleTree, n: NodeId, pseudo: bool, out: &mut Vec<FChild>) {
    if pseudo && styles.before(n).is_some() {
        out.push(FChild::Pseudo(n, 1));
    }
    for c in dom.children(n) {
        match dom.kind(c) {
            NodeKind::Text(t) => {
                if !t.is_empty() {
                    out.push(FChild::Text(c));
                }
            }
            NodeKind::Element { .. } => {
                if let Some(s) = styles.get(c) {
                    match s.display {
                        cs::Display::None => {}
                        cs::Display::Contents => flatten_in(dom, styles, c, false, out),
                        _ => out.push(FChild::Elem(c)),
                    }
                }
            }
            _ => {}
        }
    }
    if pseudo && styles.after(n).is_some() {
        out.push(FChild::Pseudo(n, 2));
    }
}

/// The computed style of an element or pseudo-element child.
pub fn comp<'a>(styles: &'a StyleTree, c: FChild) -> Option<&'a Computed> {
    match c {
        FChild::Text(_) => None,
        FChild::Elem(n) => styles.get(n),
        FChild::Pseudo(n, 1) => styles.before(n),
        FChild::Pseudo(n, _) => styles.after(n),
    }
}

pub fn is_replaced_tag(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some(
            "img"
                | "canvas"
                | "video"
                | "iframe"
                | "embed"
                | "object"
                | "svg"
                | "input"
                | "select"
                | "textarea"
                | "audio"
        )
    )
}

fn inline_has_block(dom: &Dom, styles: &StyleTree, n: NodeId) -> bool {
    let mut kids = Vec::new();
    flatten(dom, styles, n, &mut kids);
    for k in kids {
        match classify(dom, styles, k) {
            Class::Block => return true,
            _ => {}
        }
    }
    false
}

pub fn classify(dom: &Dom, styles: &StyleTree, c: FChild) -> Class {
    let cp = match comp(styles, c) {
        Some(cp) => cp,
        None => return Class::Text,
    };
    if matches!(cp.position, cs::Position::Absolute | cs::Position::Fixed) {
        return Class::Abs;
    }
    let mut replaced = false;
    if let FChild::Elem(n) = c {
        if dom.tag(n) == Some("br") {
            return Class::Br;
        }
        replaced = is_replaced_tag(dom.tag(n));
    }
    if cp.float != cs::Float::None {
        return Class::Atomic;
    }
    match cp.display {
        cs::Display::Inline => {
            if replaced {
                Class::Atomic
            } else if let FChild::Elem(n) = c {
                if inline_has_block(dom, styles, n) {
                    Class::Block
                } else {
                    Class::InlineBox
                }
            } else {
                Class::InlineBox
            }
        }
        cs::Display::InlineBlock
        | cs::Display::InlineFlex
        | cs::Display::InlineGrid
        | cs::Display::InlineTable => Class::Atomic,
        _ => Class::Block,
    }
}

/// Encodes an element id plus pseudo marker into one u32.
pub fn enc(n: NodeId, pseudo: u8) -> u32 {
    n.0 | ((pseudo as u32) << 30)
}

/// Splits `enc`'s result.
pub fn dec(v: u32) -> (NodeId, u8) {
    (NodeId(v & 0x3fff_ffff), (v >> 30) as u8)
}

fn attr_px(dom: &Dom, n: NodeId, name: &str) -> Option<f32> {
    let v = dom.attr(n, name)?.trim();
    if v.ends_with('%') {
        return None;
    }
    let digits: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse::<f32>().ok()
}

/// A hash of everything that decides the box structure under `n`: its own
/// display, the class of each flattened child, table spans and replaced
/// element attributes. A changed signature means the boxes must be rebuilt.
pub fn sig_of(dom: &Dom, styles: &StyleTree, n: NodeId) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    if let Some(cp) = styles.get(n) {
        mix(cp.display as u64);
        mix(cp.position as u64);
        mix(cp.float as u64);
    }
    for name in ["colspan", "rowspan", "width", "height", "src", "type"] {
        if let Some(v) = dom.attr(n, name) {
            if is_replaced_tag(dom.tag(n)) || name == "colspan" || name == "rowspan" {
                for b in v.bytes() {
                    mix(b as u64 + 1);
                }
                mix(0xff);
            }
        }
    }
    let mut kids = Vec::new();
    flatten(dom, styles, n, &mut kids);
    for k in kids {
        let cl = classify(dom, styles, k);
        mix(cl as u64 + 1);
        match k {
            FChild::Text(t) => {
                mix(t.0 as u64);
                mix(dom.text(t).map(super::text::only_space).unwrap_or(true) as u64);
            }
            FChild::Elem(c) => {
                mix(c.0 as u64);
                if cl == Class::InlineBox {
                    mix(sig_of(dom, styles, c));
                }
            }
            FChild::Pseudo(c, p) => mix(c.0 as u64 ^ ((p as u64) << 40)),
        }
    }
    h
}

// ---------------------------------------------------------------- builder

pub struct Builder<'a> {
    pub dom: &'a Dom,
    pub styles: &'a StyleTree,
    pub images: &'a dyn ImageSizes,
    pub t: &'a mut LayoutTree,
}

impl<'a> Builder<'a> {
    /// Generates the whole box tree. `t` must be empty.
    pub fn build_root(&mut self) {
        let root = self
            .dom
            .document_element()
            .filter(|n| self.styles.get(*n).is_some());
        let b = match root {
            Some(n) => self.make_box(FChild::Elem(n), NONE, NONE),
            None => self.push_box(Kind::Block, NONE, 0, NONE),
        };
        self.t.root = b;
    }

    pub fn push_box(&mut self, kind: Kind, dom: u32, pseudo: u8, parent: u32) -> u32 {
        let idx = self.t.boxes.len() as u32;
        let depth = if parent == NONE {
            0
        } else {
            self.t.boxes[parent as usize].depth + 1
        };
        self.t.boxes.push(Bx {
            kind,
            pseudo,
            flags: 0,
            dom,
            parent,
            depth,
            kids: 0,
            nkids: 0,
            nabs: 0,
            ra: 0,
            rb: NONE,
            ifc: NONE,
            aux: NONE,
            grid: [0, 0, 1, 1],
            intr: [0.0, 0.0],
            pw: [self.t.viewport.0, f32::NAN],
            loc: [0.0; 2],
            size: [0.0; 2],
            scroll_size: [0.0; 2],
            border: [0.0; 4],
            padding: [0.0; 4],
            abs: [0.0; 2],
            ink: [0.0; 4],
        });
        self.t.caches.push(tf::Cache::new());
        idx
    }

    fn set_kids(&mut self, b: u32, flow: &[u32], abs: &[u32]) {
        let start = self.t.kids.len() as u32;
        self.t.kids.extend_from_slice(flow);
        self.t.kids.extend_from_slice(abs);
        let bx = &mut self.t.boxes[b as usize];
        bx.kids = start;
        bx.nkids = flow.len() as u32;
        bx.nabs = abs.len() as u32;
    }

    /// Creates the box (and its subtree) for an element or pseudo-element child.
    pub fn make_box(&mut self, c: FChild, parent: u32, prev: u32) -> u32 {
        let (n, pseudo) = match c {
            FChild::Elem(n) => (n, 0),
            FChild::Pseudo(n, k) => (n, k),
            FChild::Text(n) => (n, 3),
        };
        let cp: Option<&Computed> = comp(self.styles, c);
        let (kind, table) = match cp {
            Some(cp) if pseudo == 0 && is_replaced_tag(self.dom.tag(n)) => {
                let _ = cp;
                (Kind::Replaced, false)
            }
            Some(cp) => match cp.display {
                cs::Display::Flex | cs::Display::InlineFlex => (Kind::Flex, false),
                cs::Display::Grid | cs::Display::InlineGrid => (Kind::Grid, false),
                cs::Display::Table | cs::Display::InlineTable => (Kind::Grid, true),
                _ => (Kind::Block, false),
            },
            None => (Kind::Block, false),
        };
        let idx = self.push_box(kind, n.0, pseudo, parent);
        if let Some(cp) = cp {
            if matches!(cp.position, cs::Position::Absolute | cs::Position::Fixed) {
                let fl = if cp.position == cs::Position::Fixed {
                    F_ABS | F_FIXED
                } else {
                    F_ABS
                };
                self.t.boxes[idx as usize].flags |= fl;
                self.t.boxes[idx as usize].aux = prev;
            }
        }
        if pseudo == 0 {
            let sig = sig_of(self.dom, self.styles, n);
            self.t.sigs.insert(n.0, sig);
            let i = n.0 as usize;
            if self.t.box_of.len() <= i {
                self.t.box_of.resize(i + 1, NONE);
            }
            self.t.box_of[i] = idx;
        } else {
            self.t.pbox.insert(enc(n, pseudo), idx);
        }
        match kind {
            Kind::Replaced => {
                let intr = self.intrinsic(n, cp);
                self.t.boxes[idx as usize].intr = intr;
            }
            Kind::Flex | Kind::Grid if table => {
                self.t.boxes[idx as usize].flags |= F_TABLE;
                super::table::build_table(self, idx, n);
            }
            Kind::Flex | Kind::Grid => self.fill_items(idx, n),
            _ => self.fill_flow(idx, c),
        }
        idx
    }

    fn intrinsic(&self, n: NodeId, cp: Option<&Computed>) -> [f32; 2] {
        let dom = self.dom;
        let (aw, ah) = (attr_px(dom, n, "width"), attr_px(dom, n, "height"));
        let tag = dom.tag(n).unwrap_or("");
        let fs = cp.map(|c| c.font_size).unwrap_or(16.0);
        match tag {
            "img" => {
                let nat = dom.attr(n, "src").and_then(|s| self.images.size(s));
                match (aw, ah, nat) {
                    (Some(w), Some(h), _) => [w, h],
                    (Some(w), None, Some((iw, ih))) if iw > 0 => [w, w * ih as f32 / iw as f32],
                    (None, Some(h), Some((iw, ih))) if ih > 0 => [h * iw as f32 / ih as f32, h],
                    (None, None, Some((iw, ih))) => [iw as f32, ih as f32],
                    (Some(w), None, None) => [w, 0.0],
                    (None, Some(h), None) => [0.0, h],
                    _ => [0.0, 0.0],
                }
            }
            "input" => {
                let ty = dom.attr(n, "type").unwrap_or("text").to_ascii_lowercase();
                match ty.as_str() {
                    "checkbox" | "radio" => [13.0, 13.0],
                    "submit" | "button" | "reset" | "image" => {
                        [aw.unwrap_or(80.0), ah.unwrap_or(fs * 1.25 + 8.0)]
                    }
                    _ => [aw.unwrap_or(176.0), ah.unwrap_or(fs * 1.25 + 6.0)],
                }
            }
            "select" => [aw.unwrap_or(120.0), ah.unwrap_or(fs * 1.25 + 6.0)],
            "textarea" => [aw.unwrap_or(200.0), ah.unwrap_or(fs * 1.25 * 3.0 + 6.0)],
            "audio" => [aw.unwrap_or(300.0), ah.unwrap_or(32.0)],
            _ => [aw.unwrap_or(300.0), ah.unwrap_or(150.0)],
        }
    }

    /// Fills a block-flow box: either inline content (an IFC) or boxes with
    /// anonymous wrappers around runs of inline-level children.
    fn fill_flow(&mut self, b: u32, c: FChild) {
        let n = match c {
            FChild::Pseudo(_, _) => {
                self.t.boxes[b as usize].kind = Kind::Ifc;
                return;
            }
            FChild::Elem(n) | FChild::Text(n) => n,
        };
        let mut flat = Vec::new();
        flatten(self.dom, self.styles, n, &mut flat);
        let classes: Vec<Class> = flat
            .iter()
            .map(|k| classify(self.dom, self.styles, *k))
            .collect();
        let mut flow: Vec<u32> = Vec::new();
        let mut absl: Vec<u32> = Vec::new();
        if !classes.iter().any(|k| *k == Class::Block) {
            {
                let bx = &mut self.t.boxes[b as usize];
                bx.kind = Kind::Ifc;
                bx.ra = 0;
                bx.rb = flat.len() as u32;
            }
            self.collect_inline(&flat, b, &mut flow, &mut absl);
            self.set_kids(b, &flow, &absl);
            return;
        }
        let mut run: Option<(usize, usize)> = None;
        let mut content = false;
        for i in 0..flat.len() {
            match classes[i] {
                Class::Block => {
                    self.flush_run(b, n, &flat, run.take(), content, &mut flow, &mut absl);
                    content = false;
                    let prev = flow.last().copied().unwrap_or(NONE);
                    let k = self.make_box(flat[i], b, prev);
                    flow.push(k);
                }
                Class::Abs => {
                    if let Some(r) = run.as_mut() {
                        r.1 = i + 1;
                    } else {
                        let prev = flow.last().copied().unwrap_or(NONE);
                        let k = self.make_box(flat[i], b, prev);
                        absl.push(k);
                    }
                }
                Class::Text => {
                    let ws = match flat[i] {
                        FChild::Text(t) => self
                            .dom
                            .text(t)
                            .map(super::text::only_space)
                            .unwrap_or(true),
                        _ => true,
                    };
                    if !ws {
                        content = true;
                    }
                    match run.as_mut() {
                        Some(r) => r.1 = i + 1,
                        None => run = Some((i, i + 1)),
                    }
                }
                _ => {
                    content = true;
                    match run.as_mut() {
                        Some(r) => r.1 = i + 1,
                        None => run = Some((i, i + 1)),
                    }
                }
            }
        }
        self.flush_run(b, n, &flat, run.take(), content, &mut flow, &mut absl);
        self.set_kids(b, &flow, &absl);
    }

    #[allow(clippy::too_many_arguments)]
    fn flush_run(
        &mut self,
        parent: u32,
        n: NodeId,
        flat: &[FChild],
        run: Option<(usize, usize)>,
        content: bool,
        flow: &mut Vec<u32>,
        absl: &mut Vec<u32>,
    ) {
        let (s, e) = match run {
            Some(r) => r,
            None => return,
        };
        if !content {
            let mut ignored = Vec::new();
            self.collect_inline(&flat[s..e], parent, &mut ignored, absl);
            return;
        }
        let anon = self.push_box(Kind::Ifc, n.0, 3, parent);
        {
            let bx = &mut self.t.boxes[anon as usize];
            bx.ra = s as u32;
            bx.rb = e as u32;
        }
        let mut f2 = Vec::new();
        let mut a2 = Vec::new();
        self.collect_inline(&flat[s..e], anon, &mut f2, &mut a2);
        self.set_kids(anon, &f2, &a2);
        flow.push(anon);
    }

    /// Creates the boxes of atomic inlines and out-of-flow children found in
    /// an inline subtree.
    fn collect_inline(
        &mut self,
        children: &[FChild],
        parent: u32,
        flow: &mut Vec<u32>,
        absl: &mut Vec<u32>,
    ) {
        for &ch in children {
            match classify(self.dom, self.styles, ch) {
                Class::Atomic => {
                    let prev = flow.last().copied().unwrap_or(NONE);
                    let k = self.make_box(ch, parent, prev);
                    self.t.boxes[k as usize].flags |= F_ATOMIC;
                    flow.push(k);
                }
                Class::Abs => {
                    let prev = flow.last().copied().unwrap_or(NONE);
                    let k = self.make_box(ch, parent, prev);
                    absl.push(k);
                }
                Class::InlineBox => {
                    if let FChild::Elem(n) = ch {
                        let mut sub = Vec::new();
                        flatten(self.dom, self.styles, n, &mut sub);
                        self.collect_inline(&sub, parent, flow, absl);
                    }
                }
                _ => {}
            }
        }
    }

    /// Items of a flex or grid container.
    fn fill_items(&mut self, b: u32, n: NodeId) {
        let mut flat = Vec::new();
        flatten(self.dom, self.styles, n, &mut flat);
        let mut items: Vec<(i32, u32)> = Vec::new();
        let mut absl: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < flat.len() {
            match flat[i] {
                FChild::Text(_) => {
                    let mut j = i;
                    let mut content = false;
                    while j < flat.len() {
                        match flat[j] {
                            FChild::Text(t) => {
                                if !self
                                    .dom
                                    .text(t)
                                    .map(super::text::only_space)
                                    .unwrap_or(true)
                                {
                                    content = true;
                                }
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    if content {
                        let anon = self.push_box(Kind::Ifc, n.0, 3, b);
                        let bx = &mut self.t.boxes[anon as usize];
                        bx.ra = i as u32;
                        bx.rb = j as u32;
                        self.set_kids(anon, &[], &[]);
                        items.push((0, anon));
                    }
                    i = j;
                }
                other => {
                    match classify(self.dom, self.styles, other) {
                        Class::Br => {}
                        Class::Abs => {
                            let prev = items.last().map(|x| x.1).unwrap_or(NONE);
                            let k = self.make_box(other, b, prev);
                            absl.push(k);
                        }
                        _ => {
                            let order = comp(self.styles, other).map(|c| c.order).unwrap_or(0);
                            let prev = items.last().map(|x| x.1).unwrap_or(NONE);
                            let k = self.make_box(other, b, prev);
                            items.push((order, k));
                        }
                    }
                    i += 1;
                }
            }
        }
        if items.iter().any(|x| x.0 != 0) {
            items.sort_by_key(|x| x.0);
        }
        let flow: Vec<u32> = items.iter().map(|x| x.1).collect();
        self.set_kids(b, &flow, &absl);
    }

    pub fn set_kids_pub(&mut self, b: u32, flow: &[u32], abs: &[u32]) {
        self.set_kids(b, flow, abs);
    }
}

// ---------------------------------------------------------------- style mapping

fn dim(l: &cs::Length, pw: Option<f32>, used: &std::cell::Cell<bool>) -> tf::Dimension {
    match l {
        cs::Length::Auto | cs::Length::None => tf::Dimension::auto(),
        cs::Length::Px(v) => tf::Dimension::length(*v),
        cs::Length::Percent(p) => tf::Dimension::percent(*p / 100.0),
        cs::Length::MinContent => tf::Dimension::min_content(),
        cs::Length::MaxContent => tf::Dimension::max_content(),
        cs::Length::FitContent => tf::Dimension::fit_content(),
        cs::Length::Dim(..) => match l.resolve(None) {
            Some(v) => tf::Dimension::length(v),
            None => tf::Dimension::auto(),
        },
        cs::Length::Calc(c) => match pw {
            Some(b) => {
                used.set(true);
                tf::Dimension::length(c.resolve(b))
            }
            None => tf::Dimension::auto(),
        },
    }
}

fn lpa(l: &cs::Length, pw: Option<f32>, used: &std::cell::Cell<bool>) -> tf::LengthPercentageAuto {
    match l {
        cs::Length::Px(v) => tf::LengthPercentageAuto::length(*v),
        cs::Length::Percent(p) => tf::LengthPercentageAuto::percent(*p / 100.0),
        cs::Length::Dim(..) => match l.resolve(None) {
            Some(v) => tf::LengthPercentageAuto::length(v),
            None => tf::LengthPercentageAuto::auto(),
        },
        cs::Length::Calc(c) => match pw {
            Some(b) => {
                used.set(true);
                tf::LengthPercentageAuto::length(c.resolve(b))
            }
            None => tf::LengthPercentageAuto::auto(),
        },
        _ => tf::LengthPercentageAuto::auto(),
    }
}

fn lp(l: &cs::Length, pw: Option<f32>, used: &std::cell::Cell<bool>) -> tf::LengthPercentage {
    match l {
        cs::Length::Px(v) => tf::LengthPercentage::length(*v),
        cs::Length::Percent(p) => tf::LengthPercentage::percent(*p / 100.0),
        cs::Length::Dim(..) => tf::LengthPercentage::length(l.resolve(None).unwrap_or(0.0)),
        cs::Length::Calc(c) => match pw {
            Some(b) => {
                used.set(true);
                tf::LengthPercentage::length(c.resolve(b))
            }
            None => tf::LengthPercentage::length(0.0),
        },
        _ => tf::LengthPercentage::length(0.0),
    }
}

fn overflow(o: cs::Overflow) -> tf::Overflow {
    match o {
        cs::Overflow::Visible => tf::Overflow::Visible,
        cs::Overflow::Clip => tf::Overflow::Clip,
        cs::Overflow::Hidden => tf::Overflow::Hidden,
        cs::Overflow::Scroll | cs::Overflow::Auto => tf::Overflow::Scroll,
    }
}

fn align_items(a: cs::Align) -> Option<tf::AlignItems> {
    use cs::Align as A;
    match a {
        A::Stretch => Some(tf::AlignItems::STRETCH),
        A::FlexStart => Some(tf::AlignItems::FLEX_START),
        A::FlexEnd => Some(tf::AlignItems::FLEX_END),
        A::Center => Some(tf::AlignItems::CENTER),
        A::Baseline => Some(tf::AlignItems::BASELINE),
        A::Start | A::Left => Some(tf::AlignItems::START),
        A::End | A::Right => Some(tf::AlignItems::END),
        A::SelfStart => Some(tf::AlignItems::SELF_START),
        A::SelfEnd => Some(tf::AlignItems::SELF_END),
        _ => None,
    }
}

fn align_content(a: cs::Align) -> Option<tf::AlignContent> {
    use cs::Align as A;
    match a {
        A::Stretch => Some(tf::AlignContent::STRETCH),
        A::FlexStart => Some(tf::AlignContent::FLEX_START),
        A::FlexEnd => Some(tf::AlignContent::FLEX_END),
        A::Center => Some(tf::AlignContent::CENTER),
        A::Start | A::Left => Some(tf::AlignContent::START),
        A::End | A::Right => Some(tf::AlignContent::END),
        A::SpaceBetween => Some(tf::AlignContent::SPACE_BETWEEN),
        A::SpaceAround => Some(tf::AlignContent::SPACE_AROUND),
        A::SpaceEvenly => Some(tf::AlignContent::SPACE_EVENLY),
        _ => None,
    }
}

fn min_fn(t: &cs::TrackSize) -> tf::MinTrackSizingFunction {
    match t {
        cs::TrackSize::Len(cs::Length::Px(v)) => tf::MinTrackSizingFunction::length(*v),
        cs::TrackSize::Len(cs::Length::Percent(p)) => {
            tf::MinTrackSizingFunction::percent(*p / 100.0)
        }
        cs::TrackSize::MinContent => tf::MinTrackSizingFunction::min_content(),
        cs::TrackSize::MaxContent => tf::MinTrackSizingFunction::max_content(),
        cs::TrackSize::MinMax(a, _) => min_fn(a),
        _ => tf::MinTrackSizingFunction::auto(),
    }
}

fn max_fn(t: &cs::TrackSize) -> tf::MaxTrackSizingFunction {
    match t {
        cs::TrackSize::Fr(f) => tf::MaxTrackSizingFunction::fr(*f),
        cs::TrackSize::Len(cs::Length::Px(v)) => tf::MaxTrackSizingFunction::length(*v),
        cs::TrackSize::Len(cs::Length::Percent(p)) => {
            tf::MaxTrackSizingFunction::percent(*p / 100.0)
        }
        cs::TrackSize::Len(cs::Length::MinContent) => tf::MaxTrackSizingFunction::min_content(),
        cs::TrackSize::Len(cs::Length::MaxContent) => tf::MaxTrackSizingFunction::max_content(),
        cs::TrackSize::MinContent => tf::MaxTrackSizingFunction::min_content(),
        cs::TrackSize::MaxContent => tf::MaxTrackSizingFunction::max_content(),
        cs::TrackSize::FitContent(cs::Length::Px(v)) => {
            tf::MaxTrackSizingFunction::fit_content_px(*v)
        }
        cs::TrackSize::FitContent(cs::Length::Percent(p)) => {
            tf::MaxTrackSizingFunction::fit_content_percent(*p / 100.0)
        }
        cs::TrackSize::MinMax(_, b) => max_fn(b),
        _ => tf::MaxTrackSizingFunction::auto(),
    }
}

fn track(t: &cs::TrackSize) -> tf::TrackSizingFunction {
    tf::MinMax {
        min: min_fn(t),
        max: max_fn(t),
    }
}

fn flat_tracks(items: &[cs::TrackItem], out: &mut Vec<tf::TrackSizingFunction>) {
    for it in items {
        match it {
            cs::TrackItem::Size(t) => out.push(track(t)),
            cs::TrackItem::Repeat(RepeatCount::Count(n), inner) => {
                for _ in 0..(*n).min(64) {
                    flat_tracks(inner, out);
                }
            }
            _ => {}
        }
    }
}

fn template(items: &[cs::TrackItem]) -> Vec<tf::GridTemplateComponent<String>> {
    let mut out = Vec::new();
    for it in items {
        match it {
            cs::TrackItem::Size(t) => out.push(tf::GridTemplateComponent::Single(track(t))),
            cs::TrackItem::Repeat(cnt, inner) => {
                let mut tracks = Vec::new();
                flat_tracks(inner, &mut tracks);
                if tracks.is_empty() {
                    continue;
                }
                let count = match cnt {
                    RepeatCount::Count(n) => tf::RepetitionCount::Count((*n).clamp(1, 1000) as u16),
                    RepeatCount::AutoFill => tf::RepetitionCount::AutoFill,
                    RepeatCount::AutoFit => tf::RepetitionCount::AutoFit,
                };
                out.push(tf::GridTemplateComponent::Repeat(
                    tf::GridTemplateRepetition {
                        count,
                        tracks,
                        line_names: Vec::new(),
                    },
                ));
            }
            cs::TrackItem::Names(_) => {}
        }
    }
    out
}

fn placement(l: &cs::GridLine) -> tf::GridPlacement {
    match l {
        cs::GridLine::Line(i) if *i != 0 => tf::style_helpers::line::<tf::GridPlacement>(*i as i16),
        cs::GridLine::Span(n) => tf::GridPlacement::Span((*n).clamp(1, 1000) as u16),
        _ => tf::GridPlacement::Auto,
    }
}

impl<'a> Ctx<'a> {
    /// The computed style a box lays out with (`None` for anonymous boxes).
    pub fn comp_of(&self, b: u32) -> Option<&'a Computed> {
        let styles: &'a StyleTree = self.styles;
        let bx = &self.t.boxes[b as usize];
        if bx.dom == NONE {
            return None;
        }
        let n = NodeId(bx.dom);
        match bx.pseudo {
            0 => styles.get(n),
            1 => styles.before(n),
            2 => styles.after(n),
            _ => None,
        }
    }

    /// The style whose inherited text properties apply inside a box.
    pub fn text_comp_of(&self, b: u32) -> Option<&'a Computed> {
        let styles: &'a StyleTree = self.styles;
        let bx = &self.t.boxes[b as usize];
        if bx.dom == NONE {
            return None;
        }
        let n = NodeId(bx.dom);
        match bx.pseudo {
            1 => styles.before(n),
            2 => styles.after(n),
            _ => styles.get(n),
        }
    }

    /// Builds the `taffy::Style` of a box from its computed style.
    pub fn style_for(&self, b: u32) -> tf::Style {
        let bx = &self.t.boxes[b as usize];
        let mut s = tf::Style::default();
        s.display = match bx.kind {
            Kind::Flex => tf::Display::Flex,
            Kind::Grid => tf::Display::Grid,
            _ => tf::Display::Block,
        };
        s.scrollbar_width = 0.0;
        s.position = tf::Position::Relative;
        s.item_is_replaced = bx.kind == Kind::Replaced;
        s.box_sizing = tf::BoxSizing::ContentBox;
        let cp = match self.comp_of(b) {
            Some(c) => c,
            None => return s,
        };
        let used = &self.t.calc_used;
        let pwv = bx.pw[0];
        let pw = if pwv.is_nan() { None } else { Some(pwv) };
        let ph = if bx.pw[1].is_nan() {
            None
        } else {
            Some(bx.pw[1])
        };
        s.box_sizing = match cp.box_sizing {
            cs::BoxSizing::ContentBox => tf::BoxSizing::ContentBox,
            cs::BoxSizing::BorderBox => tf::BoxSizing::BorderBox,
        };
        s.overflow = tf::Point {
            x: overflow(cp.overflow_x),
            y: overflow(cp.overflow_y),
        };
        s.size = tf::Size {
            width: dim(&cp.width, pw, used),
            height: dim(&cp.height, ph, used),
        };
        s.min_size = tf::Size {
            width: lpa(&cp.min_width, pw, used),
            height: lpa(&cp.min_height, ph, used),
        };
        s.max_size = tf::Size {
            width: lpa(&cp.max_width, pw, used),
            height: lpa(&cp.max_height, ph, used),
        };
        s.aspect_ratio = cp.aspect_ratio;
        if s.aspect_ratio.is_none()
            && bx.kind == Kind::Replaced
            && bx.intr[0] > 0.0
            && bx.intr[1] > 0.0
        {
            s.aspect_ratio = Some(bx.intr[0] / bx.intr[1]);
        }
        s.margin = tf::Rect {
            left: lpa(&cp.margin_left, pw, used),
            right: lpa(&cp.margin_right, pw, used),
            top: lpa(&cp.margin_top, pw, used),
            bottom: lpa(&cp.margin_bottom, pw, used),
        };
        s.padding = tf::Rect {
            left: lp(&cp.padding_left, pw, used),
            right: lp(&cp.padding_right, pw, used),
            top: lp(&cp.padding_top, pw, used),
            bottom: lp(&cp.padding_bottom, pw, used),
        };
        s.border = tf::Rect {
            left: tf::LengthPercentage::length(cp.border_left_width),
            right: tf::LengthPercentage::length(cp.border_right_width),
            top: tf::LengthPercentage::length(cp.border_top_width),
            bottom: tf::LengthPercentage::length(cp.border_bottom_width),
        };
        s.align_items = align_items(cp.align_items);
        s.align_self = align_items(cp.align_self);
        s.justify_items = align_items(cp.justify_items);
        s.justify_self = align_items(cp.justify_self);
        s.align_content = align_content(cp.align_content);
        s.justify_content = align_content(cp.justify_content);
        s.gap = tf::Size {
            width: lp(&cp.column_gap, pw, used),
            height: lp(&cp.row_gap, ph, used),
        };
        s.flex_direction = match cp.flex_direction {
            cs::FlexDirection::Row => tf::FlexDirection::Row,
            cs::FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
            cs::FlexDirection::Column => tf::FlexDirection::Column,
            cs::FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
        };
        s.flex_wrap = match cp.flex_wrap {
            cs::FlexWrap::Nowrap => tf::FlexWrap::NoWrap,
            cs::FlexWrap::Wrap => tf::FlexWrap::Wrap,
            cs::FlexWrap::WrapReverse => tf::FlexWrap::WrapReverse,
        };
        s.flex_grow = cp.flex_grow;
        s.flex_shrink = cp.flex_shrink;
        s.flex_basis = dim(&cp.flex_basis, pw, used);
        if bx.kind == Kind::Grid {
            s.grid_template_columns = template(&cp.grid_template_columns);
            s.grid_template_rows = template(&cp.grid_template_rows);
            s.grid_auto_columns = cp.grid_auto_columns.iter().map(track).collect();
            s.grid_auto_rows = cp.grid_auto_rows.iter().map(track).collect();
            s.grid_auto_flow = match cp.grid_auto_flow {
                cs::GridAutoFlow::Row => tf::GridAutoFlow::Row,
                cs::GridAutoFlow::Column => tf::GridAutoFlow::Column,
                cs::GridAutoFlow::RowDense => tf::GridAutoFlow::RowDense,
                cs::GridAutoFlow::ColumnDense => tf::GridAutoFlow::ColumnDense,
            };
        }
        s.grid_row = tf::Line {
            start: placement(&cp.grid_row_start),
            end: placement(&cp.grid_row_end),
        };
        s.grid_column = tf::Line {
            start: placement(&cp.grid_column_start),
            end: placement(&cp.grid_column_end),
        };
        if bx.flags & F_TABLE != 0 {
            super::table::style_table(self, b, &mut s);
        }
        if bx.flags & F_CELL != 0 {
            super::table::style_cell(self, b, &mut s);
        }
        s
    }
}

/// A map from a dom id to its box, for lookups by other modules.
pub fn box_for(map: &[u32], n: NodeId) -> Option<u32> {
    match map.get(n.0 as usize) {
        Some(&b) if b != NONE => Some(b),
        _ => None,
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<u32, u32>) {}
