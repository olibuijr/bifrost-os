//! TeX-style layout of a [`Node`] tree into positioned glyphs and rules.
//!
//! The rules follow TeX (the TeXbook, chapters 17 and 18), simplified where the glyph data
//! we have cannot support the exact rule: styles D, T, S and SS with the cramped variants;
//! inter-atom spacing from the 8x8 class table, with medium and thick spaces dropped in
//! script styles; fraction numerator and denominator shifts and rule clearance; superscript
//! and subscript shifts with the sub/superscript gap; radical sign stretching and the
//! overline; big operators with limits in display style; delimiters stretched to the height
//! of their contents and centred on the math axis.
//!
//! Every box is measured in pixels with `y` growing downward from its baseline. A child is
//! placed with [`Boxed::put`], which folds the child's extent into the parent's.

use super::metrics::{stretch, FontMetrics, MathConstants};
use super::parse::ParseError;
use super::{Class, Glyph, Layout, Node, Options, Rule};

/// Style size: D and T share the text size, S and SS are the script sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// A TeX style: size plus the cramped flag (cramped styles raise superscripts less).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Style {
    size: Size,
    cramped: bool,
}

impl Style {
    fn num(self) -> Style {
        let size = match self.size {
            Size::Display => Size::Text,
            Size::Text => Size::Script,
            _ => Size::ScriptScript,
        };
        Style { size, cramped: self.cramped }
    }

    fn den(self) -> Style {
        Style { cramped: true, ..self.num() }
    }

    fn sup(self) -> Style {
        let size = match self.size {
            Size::Display | Size::Text => Size::Script,
            _ => Size::ScriptScript,
        };
        Style { size, cramped: self.cramped }
    }

    fn sub(self) -> Style {
        Style { cramped: true, ..self.sup() }
    }

    fn cramp(self) -> Style {
        Style { cramped: true, ..self }
    }

    fn is_display(self) -> bool {
        self.size == Size::Display
    }
}

/// Spacing between two atom classes, as in the TeXbook's table.
// the full TeX spacing set; the table uses the script-aware Med and Thick variants (MedNs, ThickNs) today
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gap {
    None,
    Thin,
    Med,
    Thick,
    /// Thin space, but only outside script styles.
    ThinNs,
    MedNs,
    ThickNs,
    /// Not a valid pair; a bin operator in such a position becomes an ordinary atom.
    Forbidden,
}

use Gap::{Forbidden as X, None as N, Thick, ThickNs, Thin, ThinNs, Med, MedNs};

/// Rows and columns in the order Ord, Op, Bin, Rel, Open, Close, Punct, Inner.
const SPACING: [[Gap; 8]; 8] = [
    [N, Thin, MedNs, ThickNs, N, N, N, ThinNs],
    [Thin, Thin, X, ThickNs, N, N, N, ThinNs],
    [MedNs, MedNs, X, X, MedNs, X, X, MedNs],
    [ThickNs, ThickNs, X, N, ThickNs, N, N, ThickNs],
    [N, N, X, N, N, N, N, N],
    [N, Thin, MedNs, ThickNs, N, N, N, ThinNs],
    [ThinNs, ThinNs, X, ThinNs, ThinNs, ThinNs, ThinNs, ThinNs],
    [ThinNs, Thin, MedNs, ThickNs, ThinNs, N, ThinNs, ThinNs],
];

/// Spacing sizes in em: 3, 4 and 5 mu of the 18 mu quad.
const THIN: f32 = 3.0 / 18.0;
const MED: f32 = 4.0 / 18.0;
const THICK: f32 = 5.0 / 18.0;

/// Width of an inter-word space in text runs, in em.
const TEXT_SPACE: f32 = 0.28;

fn class_index(c: Class) -> usize {
    match c {
        Class::Ord => 0,
        Class::Op => 1,
        Class::Bin => 2,
        Class::Rel => 3,
        Class::Open => 4,
        Class::Close => 5,
        Class::Punct => 6,
        Class::Inner => 7,
    }
}

/// The class a node takes part in spacing as. `None` for spaces, which do not separate atoms.
fn class_of_node(n: &Node) -> Option<Class> {
    match n {
        Node::Sym { class, .. } => Some(*class),
        Node::Text(_) | Node::Row(_) | Node::Radical { .. } | Node::Matrix { .. } => Some(Class::Ord),
        Node::Frac { .. } | Node::Fenced { .. } => Some(Class::Inner),
        Node::Scripts { base, .. } => class_of_node(base),
        Node::BigOp { .. } => Some(Class::Op),
        Node::Space(_) => None,
    }
}

/// A drawn item, positioned relative to its box origin (on the baseline, left edge).
#[derive(Clone, Debug)]
enum Item {
    Glyph { ch: char, x: f32, y: f32, size: f32 },
    Rule { x: f32, y: f32, w: f32, h: f32 },
}

impl Item {
    fn shifted(self, dx: f32, dy: f32) -> Item {
        match self {
            Item::Glyph { ch, x, y, size } => Item::Glyph { ch, x: x + dx, y: y + dy, size },
            Item::Rule { x, y, w, h } => Item::Rule { x: x + dx, y: y + dy, w, h },
        }
    }
}

/// A measured box: width, height above and depth below the baseline, and its items.
#[derive(Clone, Debug, Default)]
struct Boxed {
    w: f32,
    asc: f32,
    dsc: f32,
    items: Vec<Item>,
}

impl Boxed {
    /// Places `child` with its origin at `(x, y)` and folds its extent into `self`.
    fn put(&mut self, child: Boxed, x: f32, y: f32) {
        self.asc = self.asc.max(-y + child.asc);
        self.dsc = self.dsc.max(y + child.dsc);
        self.w = self.w.max(x + child.w);
        for it in child.items {
            self.items.push(it.shifted(x, y));
        }
    }

    /// Places `child` at the current right edge.
    fn append(&mut self, child: Boxed) {
        let x = self.w;
        self.put(child, x, 0.0);
    }

    fn kern(&mut self, w: f32) {
        self.w += w;
    }
}

struct Ctx<'a> {
    m: &'a dyn FontMetrics,
    c: MathConstants,
    font_px: f32,
}

impl<'a> Ctx<'a> {
    /// Pixel size of a style's glyphs.
    fn px(&self, style: Style) -> f32 {
        let scale = match style.size {
            Size::Display | Size::Text => 1.0,
            Size::Script => self.c.script_scale,
            Size::ScriptScript => self.c.script_script_scale,
        };
        self.font_px * scale
    }

    /// An em-valued constant in pixels at `style`.
    fn k(&self, em: f32, style: Style) -> f32 {
        em * self.px(style)
    }

    fn sym_box(&self, ch: char, style: Style) -> Boxed {
        let s = self.px(style);
        let g = self.m.glyph(ch);
        Boxed {
            w: g.advance * s,
            asc: g.ascent * s,
            dsc: g.descent * s,
            items: vec![Item::Glyph { ch, x: 0.0, y: 0.0, size: s }],
        }
    }

    fn text(&self, s: &str, style: Style) -> Boxed {
        let mut out = Boxed::default();
        for ch in s.chars() {
            if ch == ' ' {
                out.kern(self.k(TEXT_SPACE, style));
            } else {
                out.append(self.sym_box(ch, style));
            }
        }
        out
    }

    /// Inter-atom space for the pair `(l, r)`, in pixels.
    fn spacing(&self, l: Class, r: Class, style: Style) -> f32 {
        let script = matches!(style.size, Size::Script | Size::ScriptScript);
        let em = match SPACING[class_index(l)][class_index(r)] {
            N | X => 0.0,
            Thin => THIN,
            Med => MED,
            Thick => THICK,
            ThinNs => if script { 0.0 } else { THIN },
            MedNs => if script { 0.0 } else { MED },
            ThickNs => if script { 0.0 } else { THICK },
        };
        self.k(em, style)
    }

    fn node(&self, n: &Node, style: Style) -> Boxed {
        match n {
            Node::Sym { ch, .. } => self.sym_box(*ch, style),
            Node::Text(s) => self.text(s, style),
            Node::Row(items) => self.row(items, style),
            Node::Space(em) => Boxed { w: self.k(*em, style), ..Boxed::default() },
            Node::Frac { num, den } => self.frac(num, den, style),
            Node::Radical { index, body } => self.radical(index.as_deref(), body, style),
            Node::Scripts { base, sub, sup } => {
                let b = self.node(base, style);
                self.attach_scripts(b, sub.as_deref(), sup.as_deref(), style)
            }
            Node::BigOp { ch, limits, sub, sup } => {
                self.bigop(*ch, *limits, sub.as_deref(), sup.as_deref(), style)
            }
            Node::Fenced { open, close, body } => self.fenced(*open, *close, body, style),
            Node::Matrix { open, close, rows } => self.matrix(*open, *close, rows, style),
        }
    }

    /// A horizontal list: bin operators are demoted to ordinary atoms where TeX demotes them,
    /// then each neighbouring pair gets its table space.
    fn row(&self, nodes: &[Node], style: Style) -> Boxed {
        let mut atoms: Vec<(Boxed, Option<Class>)> =
            nodes.iter().map(|n| (self.node(n, style), class_of_node(n))).collect();
        for i in 0..atoms.len() {
            if atoms[i].1 != Some(Class::Bin) {
                continue;
            }
            let prev = atoms[..i].iter().rev().find_map(|a| a.1);
            let next = atoms[i + 1..].iter().find_map(|a| a.1);
            let prev_ok = matches!(
                prev,
                None | Some(Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct)
            );
            let next_ok = matches!(next, None | Some(Class::Rel | Class::Close | Class::Punct));
            if prev_ok || next_ok {
                atoms[i].1 = Some(Class::Ord);
            }
        }
        let mut out = Boxed::default();
        let mut prev: Option<Class> = None;
        for (b, cls) in atoms {
            if let Some(cur) = cls {
                if let Some(p) = prev {
                    out.kern(self.spacing(p, cur, style));
                }
                prev = Some(cur);
            }
            out.append(b);
        }
        out
    }

    /// A base with optional subscript and superscript (TeX rules 18a to 18f, simplified).
    fn attach_scripts(&self, base: Boxed, sub: Option<&Node>, sup: Option<&Node>, style: Style) -> Boxed {
        let c = &self.c;
        let sup_b = sup.map(|n| self.node(n, style.sup()));
        let sub_b = sub.map(|n| self.node(n, style.sub()));
        if sup_b.is_none() && sub_b.is_none() {
            return base;
        }
        let mut u = 0.0f32;
        let mut v = 0.0f32;
        if let Some(p) = &sup_b {
            let min_u = if style.cramped { c.sup_shift_cramped } else { c.sup_shift };
            u = self
                .k(min_u, style)
                .max(base.asc - self.k(c.sup_baseline_drop, style))
                .max(p.dsc + self.k(c.sup_bottom_min, style));
        }
        if let Some(s) = &sub_b {
            v = self
                .k(c.sub_shift, style)
                .max(base.dsc + self.k(c.sub_baseline_drop, style))
                .max(s.asc - self.k(c.sub_top_max, style));
        }
        if let (Some(p), Some(s)) = (&sup_b, &sub_b) {
            // Keep the superscript's bottom and the subscript's top apart.
            let gap = (u - p.dsc) - (s.asc - v);
            let min_gap = self.k(c.sub_sup_gap_min, style);
            if gap < min_gap {
                v += min_gap - gap;
            }
        }
        let base_w = base.w;
        let script_w = sup_b.as_ref().map_or(0.0, |p| p.w).max(sub_b.as_ref().map_or(0.0, |s| s.w));
        let total_w = base_w + script_w + self.k(c.space_after_script, style);
        let mut out = Boxed::default();
        out.w = total_w;
        out.put(base, 0.0, 0.0);
        if let Some(p) = sup_b {
            out.put(p, base_w, -u);
        }
        if let Some(s) = sub_b {
            out.put(s, base_w, v);
        }
        out.w = total_w;
        out
    }

    /// `\frac` (TeX rule 15d): numerator above the bar, denominator below, null delimiters
    /// on each side.
    fn frac(&self, num: &Node, den: &Node, style: Style) -> Boxed {
        let c = &self.c;
        let n = self.node(num, style.num());
        let d = self.node(den, style.den());
        let t = self.k(c.fraction_rule, style);
        let axis = self.k(c.axis_height, style);
        let (up_min, down_min, gap_n, gap_d) = if style.is_display() {
            (c.num_shift_display, c.den_shift_display, c.num_gap_display, c.den_gap_display)
        } else {
            (c.num_shift_text, c.den_shift_text, c.num_gap_text, c.den_gap_text)
        };
        let num_up = self
            .k(up_min, style)
            .max(axis + t / 2.0 + self.k(gap_n, style) + n.dsc);
        let den_down = self
            .k(down_min, style)
            .max(d.asc + self.k(gap_d, style) - (axis - t / 2.0));
        let null = self.k(0.12, style);
        let w = n.w.max(d.w) + 2.0 * null;
        let (nw, dw) = (n.w, d.w);
        let mut out = Boxed::default();
        out.w = w;
        out.put(n, (w - nw) / 2.0, -num_up);
        out.put(d, (w - dw) / 2.0, den_down);
        out.items.push(Item::Rule { x: 0.0, y: -(axis + t / 2.0), w, h: t });
        out.w = w;
        out
    }

    /// `\sqrt[index]{body}` (TeX rule 11 and 11a): a stretched sign with an overline over the body.
    fn radical(&self, index: Option<&Node>, body: &Node, style: Style) -> Boxed {
        let c = &self.c;
        let s = self.px(style);
        let b = self.node(body, style.cramp());
        let rule = self.k(c.radical_rule, style);
        let gap = if style.is_display() {
            self.k(c.radical_display_gap, style)
        } else {
            self.k(c.radical_gap, style)
        };
        let extra = self.k(c.radical_extra_ascender, style);
        let st = stretch(self.m, '√', (b.asc + b.dsc + gap + rule) / s.max(1e-6));
        let sign = Boxed {
            w: st.advance * s,
            asc: st.ascent * s,
            dsc: st.descent * s,
            items: vec![Item::Glyph { ch: st.ch, x: 0.0, y: 0.0, size: s * st.scale }],
        };
        // Height of the overline above the baseline.
        let h = b.asc + gap + rule;
        let sign_w = sign.w;
        let sign_asc = sign.asc;
        let bw = b.w;
        let mut out = Boxed::default();
        let mut x0 = 0.0;
        if let Some(idx) = index {
            let ib = self.node(idx, Style { size: Size::ScriptScript, cramped: false });
            x0 = ib.w + self.k(0.06, style);
            out.put(ib, 0.0, -h * c.radical_degree_raise);
        }
        out.put(sign, x0, -h + sign_asc);
        let bx = x0 + sign_w;
        out.put(b, bx, 0.0);
        out.items.push(Item::Rule { x: bx, y: -h, w: bw, h: rule });
        out.asc = out.asc.max(h + extra);
        out.w = bx + bw;
        out
    }

    /// A glyph centred on the math axis, the way big operators and delimiters sit.
    fn centered(&self, g: Boxed, style: Style) -> Boxed {
        let y = -self.k(self.c.axis_height, style) + (g.asc - g.dsc) / 2.0;
        let mut out = Boxed::default();
        out.put(g, 0.0, y);
        out
    }

    /// A delimiter stretched to `total_px` tall and centred on the axis.
    fn delim_box(&self, ch: char, total_px: f32, style: Style) -> Boxed {
        let s = self.px(style).max(1e-6);
        let st = stretch(self.m, ch, total_px / s);
        let g = Boxed {
            w: st.advance * s,
            asc: st.ascent * s,
            dsc: st.descent * s,
            items: vec![Item::Glyph { ch: st.ch, x: 0.0, y: 0.0, size: s * st.scale }],
        };
        let mut out = self.centered(g, style);
        out.w = st.advance * s;
        out
    }

    /// A big operator. In display style with limits, the scripts sit above and below it;
    /// otherwise they attach as ordinary scripts.
    fn bigop(&self, ch: char, limits: bool, sub: Option<&Node>, sup: Option<&Node>, style: Style) -> Boxed {
        let s = self.px(style);
        let min_em = if style.is_display() { self.c.display_op_min_height } else { 0.0 };
        let st = stretch(self.m, ch, min_em);
        let op = Boxed {
            w: st.advance * s,
            asc: st.ascent * s,
            dsc: st.descent * s,
            items: vec![Item::Glyph { ch: st.ch, x: 0.0, y: 0.0, size: s * st.scale }],
        };
        let op = self.centered(op, style);
        if !(limits && style.is_display()) || (sub.is_none() && sup.is_none()) {
            return self.attach_scripts(op, sub, sup, style);
        }
        let sup_b = sup.map(|n| self.node(n, style.sup()));
        let sub_b = sub.map(|n| self.node(n, style.sub()));
        let (oa, od, ow) = (op.asc, op.dsc, op.w);
        let w = ow
            .max(sup_b.as_ref().map_or(0.0, |p| p.w))
            .max(sub_b.as_ref().map_or(0.0, |s| s.w));
        let mut out = Boxed::default();
        out.w = w;
        out.put(op, (w - ow) / 2.0, 0.0);
        if let Some(p) = sup_b {
            let gap = self.k(self.c.upper_limit_gap, style);
            let pw = p.w;
            let dsc = p.dsc;
            out.put(p, (w - pw) / 2.0, -(oa + gap + dsc));
        }
        if let Some(sb) = sub_b {
            let gap = self.k(self.c.lower_limit_gap, style);
            let sw = sb.w;
            let asc = sb.asc;
            out.put(sb, (w - sw) / 2.0, od + gap + asc);
        }
        out.w = w;
        out
    }

    /// `\left...\right`: the body between delimiters sized to its height (TeX rule 19, simplified).
    fn fenced(&self, open: Option<char>, close: Option<char>, body: &Node, style: Style) -> Boxed {
        let axis = self.k(self.c.axis_height, style);
        let b = self.node(body, style);
        let delta = (b.asc - axis).max(b.dsc + axis);
        let total = (2.0 * delta * 0.901).max(2.0 * delta - self.k(0.05, style));
        let mut out = Boxed::default();
        if let Some(o) = open {
            out.append(self.delim_box(o, total, style));
        }
        out.append(b);
        if let Some(cl) = close {
            out.append(self.delim_box(cl, total, style));
        }
        out
    }

    /// A matrix: cells centred in columns, columns one em apart, rows stacked and the whole
    /// block centred on the axis, with the requested delimiters stretched to its height.
    fn matrix(&self, open: Option<char>, close: Option<char>, rows: &[Vec<Node>], style: Style) -> Boxed {
        let s = self.px(style);
        let cells: Vec<Vec<Boxed>> =
            rows.iter().map(|r| r.iter().map(|n| self.node(n, style)).collect()).collect();
        if cells.is_empty() {
            return Boxed::default();
        }
        let ncols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut col_w = vec![0.0f32; ncols];
        for row in &cells {
            for (j, b) in row.iter().enumerate() {
                col_w[j] = col_w[j].max(b.w);
            }
        }
        let row_sep = 0.2 * s;
        let col_sep = s;
        let mut ys = Vec::with_capacity(cells.len());
        let mut top = 0.0f32;
        let mut bottom = 0.0f32;
        let mut y = 0.0f32;
        let mut prev_dsc = 0.0f32;
        for (r, row) in cells.iter().enumerate() {
            let asc = row.iter().fold(0.0f32, |a, b| a.max(b.asc));
            let dsc = row.iter().fold(0.0f32, |a, b| a.max(b.dsc));
            if r == 0 {
                top = -asc;
            } else {
                y += prev_dsc + row_sep + asc;
            }
            ys.push(y);
            prev_dsc = dsc;
            bottom = y + dsc;
        }
        let dy = -self.k(self.c.axis_height, style) - (top + bottom) / 2.0;
        let pad = self.k(0.12, style);
        let width = 2.0 * pad + col_w.iter().sum::<f32>() + col_sep * (ncols.saturating_sub(1)) as f32;
        let mut grid = Boxed::default();
        grid.w = width;
        for (r, row) in cells.into_iter().enumerate() {
            let mut x = pad;
            for (j, b) in row.into_iter().enumerate() {
                let off = (col_w[j] - b.w) / 2.0;
                grid.put(b, x + off, ys[r] + dy);
                x += col_w[j] + col_sep;
            }
        }
        grid.w = width;
        let h = bottom - top;
        let mut out = Boxed::default();
        if let Some(o) = open {
            out.append(self.delim_box(o, h, style));
        }
        out.append(grid);
        if let Some(cl) = close {
            out.append(self.delim_box(cl, h, style));
        }
        out
    }
}

/// Lays out a parsed formula with the given metrics. A font with a MATH table supplies its own
/// constants through [`FontMetrics::constants`]; otherwise the TeX defaults are used.
pub fn layout(node: &Node, opts: Options, metrics: &dyn FontMetrics) -> Layout {
    let ctx = Ctx {
        m: metrics,
        c: metrics.constants().unwrap_or(MathConstants::DEFAULT),
        font_px: opts.font_px,
    };
    let style = Style { size: if opts.display { Size::Display } else { Size::Text }, cramped: false };
    let b = ctx.node(node, style);
    let mut out = Layout { width: b.w, ascent: b.asc, depth: b.dsc, glyphs: Vec::new(), rules: Vec::new() };
    for it in b.items {
        match it {
            Item::Glyph { ch, x, y, size } => out.glyphs.push(Glyph { ch, x, y, size }),
            Item::Rule { x, y, w, h } => out.rules.push(Rule { x, y, w, h }),
        }
    }
    out
}

/// Parses LaTeX-subset source and lays it out.
pub fn typeset_tex(src: &str, opts: Options, metrics: &dyn FontMetrics) -> Result<Layout, ParseError> {
    let node = super::parse::parse_tex(src)?;
    Ok(layout(&node, opts, metrics))
}

/// Parses linear input (`a/b`, `x^2`, `sqrt(x)`) and lays it out.
pub fn typeset_linear(src: &str, opts: Options, metrics: &dyn FontMetrics) -> Result<Layout, ParseError> {
    let node = super::parse::parse_linear(src)?;
    Ok(layout(&node, opts, metrics))
}

#[cfg(test)]
mod tests {
    use super::super::metrics::GlyphMetrics;
    use super::*;

    /// Every glyph is 0.5 em wide, 0.7 em above and 0.2 em below the baseline; no MATH data.
    struct Fake;

    impl FontMetrics for Fake {
        fn glyph(&self, ch: char) -> GlyphMetrics {
            GlyphMetrics { ch, advance: 0.5, ascent: 0.7, descent: 0.2 }
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn lay(src: &str) -> Layout {
        typeset_tex(src, Options::text(10.0), &Fake).expect("parses")
    }

    fn glyph_at(l: &Layout, ch: char) -> &Glyph {
        l.glyphs.iter().find(|g| g.ch == ch).expect("glyph present")
    }

    #[test]
    fn fraction_box_metrics() {
        // Numerator and denominator are script size (7 px): 3.5 wide, 4.9 up, 1.4 down.
        // Text style: num shift max(3.94, 2.5 + 0.2 + 0.4 + 1.4) = 4.5; den shift 3.45.
        // Null delimiters add 1.2 px each side: width 3.5 + 2.4 = 5.9.
        let l = lay(r"\frac{a}{b}");
        assert!(close(l.width, 5.9), "width {}", l.width);
        assert!(close(l.ascent, 9.4), "ascent {}", l.ascent);
        assert!(close(l.depth, 4.85), "depth {}", l.depth);
        assert_eq!(l.rules.len(), 1);
        assert!(close(l.rules[0].y, -2.7));
        assert!(close(l.rules[0].h, 0.4));
        assert!(close(l.rules[0].w, 5.9));
        assert!(close(glyph_at(&l, 'a').size, 7.0));
    }

    #[test]
    fn superscript_shift_and_width() {
        // Base 5 wide; superscript 3.5 wide at 7 px; shift max(4.13, 7 - 3.86, 1.4 + 1.0) = 4.13.
        // Width adds the 0.5 px script space after.
        let l = lay("x^2");
        assert!(close(l.width, 9.0), "width {}", l.width);
        assert!(close(glyph_at(&l, '2').y, -4.13), "y {}", glyph_at(&l, '2').y);
        assert!(close(l.ascent, 9.03), "ascent {}", l.ascent);
    }

    #[test]
    fn subscript_and_superscript_keep_a_gap() {
        // Sub shift is raised from 2.5 to 3.77 so that the superscript bottom (2.73 above
        // its baseline) clears the subscript top by the 1.6 px minimum.
        let l = lay("x_1^2");
        assert!(close(glyph_at(&l, '1').y, 3.77), "sub y {}", glyph_at(&l, '1').y);
        assert!(close(glyph_at(&l, '2').y, -4.13), "sup y {}", glyph_at(&l, '2').y);
    }

    #[test]
    fn binary_operator_gets_medium_space_in_text_style() {
        // Three 5 px glyphs and two medium spaces of 4/18 em (2.222 px).
        let l = lay("a+b");
        assert!(close(l.width, 15.0 + 2.0 * 4.0 / 18.0 * 10.0), "width {}", l.width);
    }

    #[test]
    fn relation_gets_thick_space() {
        let l = lay("a=b");
        assert!(close(l.width, 15.0 + 2.0 * 5.0 / 18.0 * 10.0), "width {}", l.width);
    }

    #[test]
    fn medium_space_is_dropped_in_script_style() {
        // Superscript {a+b} is 10.5 px in script style with no operator spacing.
        let l = lay("x^{a+b}");
        assert!(close(l.width, 5.0 + 10.5 + 0.5), "width {}", l.width);
    }

    #[test]
    fn bin_at_start_becomes_ordinary_and_takes_no_space() {
        let l = lay("-a");
        assert!(close(l.width, 10.0), "width {}", l.width);
    }

    #[test]
    fn delimiter_is_stretched_to_fraction_height() {
        // Body ascent 9.4, descent 4.85, axis 2.5: half height 7.35, total 14.2 px.
        // The base parenthesis is 0.9 em tall, so it is scaled to 1.42 / 0.9 = 1.578.
        let l = lay(r"\left( \frac{a}{b} \right)");
        let paren = l.glyphs.iter().find(|g| g.ch == '(').expect("open paren");
        assert!(close(paren.size, 10.0 * 1.42 / 0.9), "size {}", paren.size);
    }

    #[test]
    fn small_body_keeps_delimiter_at_its_own_size() {
        let l = lay(r"\left( x \right)");
        let paren = l.glyphs.iter().find(|g| g.ch == '(').expect("open paren");
        assert!(close(paren.size, 10.0), "size {}", paren.size);
    }

    #[test]
    fn radical_overline_and_sign_size() {
        // Body 7 up and 2 down; gap 0.5 px; rule 0.4 px; sign target 9.9 px, scaled 1.1.
        // Overline top at 7.9 above the baseline; box ascent adds 0.4 px extra ascender.
        let l = lay(r"\sqrt{x}");
        let sign = glyph_at(&l, '√');
        assert!(close(sign.size, 11.0), "sign size {}", sign.size);
        assert_eq!(l.rules.len(), 1);
        assert!(close(l.rules[0].y, -7.9), "rule y {}", l.rules[0].y);
        assert!(close(l.ascent, 8.3), "ascent {}", l.ascent);
    }

    #[test]
    fn pmatrix_is_two_columns_with_delimiters() {
        // Columns 5 px wide, one em apart, 1.2 px pad each side: 22.4 px; delimiters 5 px each.
        let l = lay(r"\begin{pmatrix} a & b \end{pmatrix}");
        assert!(close(l.width, 32.4), "width {}", l.width);
        assert_eq!(l.glyphs.len(), 4);
    }

    #[test]
    fn sum_uses_display_size_with_limits_above_and_below() {
        let l = typeset_tex(r"\sum_{i}^{n}", Options::display(10.0), &Fake).unwrap();
        let sum = l.glyphs.iter().find(|g| g.ch == '∑').expect("sum glyph");
        assert!(sum.size > 10.0, "display sum should be larger, got {}", sum.size);
        let n = glyph_at(&l, 'n');
        let i = glyph_at(&l, 'i');
        assert!(n.y < sum.y, "upper limit above the operator");
        assert!(i.y > sum.y, "lower limit below the operator");
    }

    #[test]
    fn linear_fraction_lays_out_like_tex() {
        let a = typeset_linear("a/b", Options::text(10.0), &Fake).unwrap();
        let b = lay(r"\frac{a}{b}");
        assert!(close(a.width, b.width));
        assert!(close(a.ascent, b.ascent));
    }

    #[test]
    fn math_constants_from_font_are_used_when_present() {
        struct WithConstants;
        impl FontMetrics for WithConstants {
            fn glyph(&self, ch: char) -> GlyphMetrics {
                Fake.glyph(ch)
            }
            fn constants(&self) -> Option<MathConstants> {
                Some(MathConstants { sup_shift: 0.8, ..MathConstants::DEFAULT })
            }
        }
        let l = typeset_tex("x^2", Options::text(10.0), &WithConstants).unwrap();
        // Shift is now max(8.0, 7 - 3.86, 2.4) = 8.0.
        assert!(close(glyph_at(&l, '2').y, -8.0), "y {}", glyph_at(&l, '2').y);
    }

    #[test]
    fn parse_errors_surface_from_typeset() {
        assert!(typeset_tex(r"\frac{a}", Options::text(10.0), &Fake).is_err());
    }
}
