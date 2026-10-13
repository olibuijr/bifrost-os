//! Laying out and drawing an [`AttributedString`] with the typesetter objects of `bifrost_ui::typeset`.
//!
//! * `AttributedString::layout_input` turns the attributes into run styles and paragraph styles in UTF-16 ranges.
//! * `AttributedString` gets `*_utf16` range methods, the unit of the platform's range type; the older methods keep
//!   counting characters.
//! * `TableSource` is the glyph source: it measures every distinct (font, character) pair once with a caller's
//!   measure function, so the layout objects own their widths and do not borrow the UI. It is the single place
//!   advances come from; a shaping engine replaces it by providing another `GlyphSource`.
//! * `TextLayout` lays a string out in a width and answers position queries in pixels; `SurfacePainter` draws a
//!   frame onto a surface. Positions the layout reports are character indices of the string, like `RichText::selection`.
use crate::attributed::{Attribute, AttributeKey, AttributedString, Attrs};
use bifrost_render::gfx::Surface;
use bifrost_render::ui::{self, system, Ui, Weight};
use bifrost_ui::paragraph::ParagraphStyle;
use bifrost_ui::textmetrics::{FontMetrics, FontSpec, GlyphSource};
use bifrost_ui::typeset::{Frame, FramePath, Framesetter, RunStyle, TextPainter, Typesetter, TypesetterOptions, UnderlineStyle};
use bifrost_ui::Rect;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

/// Text of an attributed string with its styles, ready for a typesetter. Ranges are UTF-16 units.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutInput {
    pub text: String,
    pub spans: Vec<(Range<usize>, RunStyle)>,
    pub paragraphs: Vec<(Range<usize>, ParagraphStyle)>,
    /// Fonts whose metrics are needed though no text uses them (the font of an empty string).
    pub extra_fonts: Vec<FontSpec>,
}

/// The run style of one set of attributes. Links get the link colour and a line under them.
pub fn run_style(a: &Attrs) -> RunStyle {
    let linked = a.link.is_some();
    let foreground = a.foreground.or(if linked { Some(system::LINK.now()) } else { a.color.map(|c| c.now()) });
    let underline = if a.underline || linked {
        if a.underline_style == UnderlineStyle::None {
            UnderlineStyle::Single
        } else {
            a.underline_style
        }
    } else {
        a.underline_style
    };
    RunStyle {
        font: FontSpec {
            family: a.font_family.clone(),
            size: a.font_size.unwrap_or_else(|| a.size.pt()),
            weight: if a.bold { 700 } else { 400 },
            italic: a.italic,
        },
        foreground,
        background: a.background,
        baseline_offset: a.baseline_offset,
        superscript: a.superscript,
        kern: a.kern,
        tracking: a.tracking,
        stroke_width: a.stroke_width,
        features: a.features.items().to_vec(),
        stroke_color: a.stroke_color,
        underline,
        underline_color: a.underline_color,
        strikethrough: a.strikethrough,
        link: a.link.clone(),
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2029}' | '\u{85}')
}

impl AttributedString {
    /// The length in UTF-16 units.
    pub fn utf16_len(&self) -> usize {
        self.string().encode_utf16().count()
    }

    /// The character index at UTF-16 offset `u`, rounded up to a character start and clamped to the length.
    pub fn char_index_from_utf16(&self, u: usize) -> usize {
        let (mut acc, mut n) = (0usize, 0usize);
        for ch in self.string().chars() {
            if acc >= u {
                break;
            }
            acc += ch.len_utf16();
            n += 1;
        }
        n
    }

    /// The UTF-16 offset of character index `c` (clamped to the length).
    pub fn utf16_index_from_char(&self, c: usize) -> usize {
        self.string().chars().take(c).map(char::len_utf16).sum()
    }

    fn char_range(&self, r: Range<usize>) -> Range<usize> {
        let s = self.char_index_from_utf16(r.start);
        let e = self.char_index_from_utf16(r.end).max(s);
        s..e
    }

    /// `attributes(at:)` for a UTF-16 index.
    pub fn attributes_at_utf16(&self, index: usize) -> Attrs {
        self.attributes_at(self.char_index_from_utf16(index))
    }

    /// `attributedSubstring(from:)` for a UTF-16 range.
    pub fn substring_utf16(&self, range: Range<usize>) -> AttributedString {
        self.substring(self.char_range(range))
    }

    /// `replaceCharacters(in:with:)` for a UTF-16 range.
    pub fn replace_characters_utf16(&mut self, range: Range<usize>, with: &str) {
        let r = self.char_range(range);
        self.replace_characters(r, with);
    }

    /// `replaceCharacters(in:with: NativeAttributedString)` for a UTF-16 range.
    pub fn replace_attributed_utf16(&mut self, range: Range<usize>, with: &AttributedString) {
        let r = self.char_range(range);
        self.replace_attributed(r, with);
    }

    /// `addAttribute(_:value:range:)` for a UTF-16 range.
    pub fn add_attribute_utf16(&mut self, attr: Attribute, range: Range<usize>) {
        let r = self.char_range(range);
        self.add_attribute(attr, r);
    }

    /// `removeAttribute(_:range:)` for a UTF-16 range.
    pub fn remove_attribute_utf16(&mut self, key: AttributeKey, range: Range<usize>) {
        let r = self.char_range(range);
        self.remove_attribute(key, r);
    }

    /// `enumerateAttribute(_:in:)` for a UTF-16 range; `f` gets UTF-16 ranges.
    pub fn enumerate_attribute_utf16(&self, key: AttributeKey, range: Range<usize>, mut f: impl FnMut(Range<usize>, Attribute)) {
        let mut found: Vec<(Range<usize>, Attribute)> = Vec::new();
        self.enumerate_attribute(key, self.char_range(range), |r, a| found.push((r, a)));
        for (r, a) in found {
            f(self.utf16_index_from_char(r.start)..self.utf16_index_from_char(r.end), a);
        }
    }

    /// Sets the paragraph style of every paragraph that `range` (UTF-16) touches. `None` clears it.
    pub fn set_paragraph_style(&mut self, range: Range<usize>, style: Option<ParagraphStyle>) {
        let text: Vec<char> = self.string().chars().collect();
        let r = self.char_range(range);
        let mut s = r.start.min(text.len());
        while s > 0 && !is_separator(text[s - 1]) {
            s -= 1;
        }
        let mut e = r.end.min(text.len()).max(s);
        // A range that ends right after a separator does not touch the next paragraph.
        if !(e > r.start && e > 0 && is_separator(text[e - 1])) {
            while e < text.len() && !is_separator(text[e]) {
                e += 1;
            }
            if e < text.len() {
                e += 1;
            }
        }
        if s < e {
            self.add_attribute(Attribute::Paragraph(style), s..e);
        }
    }

    /// The paragraph style at a UTF-16 index.
    pub fn paragraph_style_at_utf16(&self, index: usize) -> Option<ParagraphStyle> {
        self.attributes_at_utf16(index).paragraph
    }

    /// The text with run and paragraph styles for a typesetter.
    pub fn layout_input(&self) -> LayoutInput {
        let mut spans = Vec::new();
        let mut paragraphs = Vec::new();
        for r in self.ax_runs() {
            let range = r.utf16_start..r.utf16_start + r.utf16_len;
            if let Some(p) = &r.attrs.paragraph {
                paragraphs.push((range.clone(), p.clone()));
            }
            spans.push((range, run_style(&r.attrs)));
        }
        LayoutInput { text: self.string(), spans, paragraphs, extra_fonts: vec![run_style(&Attrs::default()).effective_font()] }
    }
}

// ---- glyph source ----

type FontKey = (Option<String>, u32, u16, bool);

fn key(f: &FontSpec) -> FontKey {
    (f.family.clone(), f.size.to_bits(), f.weight, f.italic)
}

/// Advances and metrics measured up front for the fonts a text uses.
pub struct TableSource {
    adv: HashMap<(FontKey, char), f32>,
    metrics: HashMap<FontKey, FontMetrics>,
}

impl TableSource {
    /// Measures every distinct (font, character) pair of `input`, plus the ellipsis for each font. `measure` returns
    /// the width of a string in points; `metrics` the vertical metrics of a font.
    pub fn build(input: &LayoutInput, measure: &dyn Fn(&FontSpec, &str) -> f32, metrics: &dyn Fn(&FontSpec) -> FontMetrics) -> Self {
        let mut t = TableSource { adv: HashMap::new(), metrics: HashMap::new() };
        for font in &input.extra_fonts {
            t.metrics.entry(key(font)).or_insert_with(|| metrics(font));
        }
        let mut spans = input.spans.iter().peekable();
        let mut off = 0usize;
        let mut buf = [0u8; 4];
        for ch in input.text.chars() {
            while let Some((r, _)) = spans.peek() {
                if r.end <= off {
                    spans.next();
                } else {
                    break;
                }
            }
            if let Some((r, style)) = spans.peek() {
                if r.start <= off {
                    let font = style.effective_font();
                    let k = key(&font);
                    t.metrics.entry(k.clone()).or_insert_with(|| metrics(&font));
                    t.adv.entry((k.clone(), ch)).or_insert_with(|| measure(&font, ch.encode_utf8(&mut buf)));
                    t.adv.entry((k, '\u{2026}')).or_insert_with(|| measure(&font, "\u{2026}"));
                }
            }
            off += ch.len_utf16();
        }
        t
    }
}

impl GlyphSource for TableSource {
    fn advances(&self, font: &FontSpec, text: &str) -> Vec<f32> {
        let k = key(font);
        text.chars().map(|c| self.adv.get(&(k.clone(), c)).copied().unwrap_or(0.0)).collect()
    }

    fn metrics(&self, font: &FontSpec) -> FontMetrics {
        self.metrics.get(&key(font)).copied().unwrap_or_default()
    }
}

fn weight_of(w: u16) -> Weight {
    match w {
        0..=150 => Weight::Thin,
        151..=349 => Weight::Light,
        350..=449 => Weight::Regular,
        450..=549 => Weight::Medium,
        550..=649 => Weight::Semibold,
        _ => Weight::Bold,
    }
}

/// A table source measured with the UI font. Widths and metrics are in points (the UI scale divided out).
pub fn ui_source(ui: &Ui, input: &LayoutInput) -> TableSource {
    let s = ui.pt(1.0).max(f32::EPSILON);
    let measure = move |f: &FontSpec, t: &str| ui.measure(t, f.size, weight_of(f.weight)) / s;
    let metrics = move |f: &FontSpec| {
        let lm = ui.font.line_metrics(ui.pt(f.size * ui.text_scale));
        let thickness = lm.underline_thickness / s;
        FontMetrics {
            ascent: lm.ascent / s,
            descent: lm.descent / s,
            leading: lm.leading / s,
            underline_position: if lm.underline_position < 0.0 { -lm.underline_position / s } else { lm.descent / s / 2.0 },
            underline_thickness: if thickness > 0.0 { thickness } else { f.size / 14.0 },
        }
    };
    TableSource::build(input, &measure, &metrics)
}

// ---- layout ----

/// A string laid out in a width, with queries in pixels.
pub struct TextLayout {
    framesetter: Framesetter,
    frame: Frame,
    /// Pixels per point.
    scale: f32,
    /// UTF-16 offset of each character, plus the total.
    u16_at: Vec<usize>,
    /// Height of an empty string's one line, in points.
    empty_height: f32,
}

impl TextLayout {
    /// Lays `content` out in `width_px` pixels with the UI font.
    pub fn new(content: &AttributedString, ui: &Ui, width_px: f32) -> Self {
        let input = content.layout_input();
        let source: Rc<dyn GlyphSource> = Rc::new(ui_source(ui, &input));
        Self::with_source(content, source, width_px / ui.pt(1.0).max(f32::EPSILON), ui.pt(1.0))
    }

    /// Lays `content` out in `width_pt` points with any glyph source. `scale` is pixels per point.
    pub fn with_source(content: &AttributedString, source: Rc<dyn GlyphSource>, width_pt: f32, scale: f32) -> Self {
        let input = content.layout_input();
        let ts = Typesetter::new(&input.text, &input.spans, &input.paragraphs, TypesetterOptions::default(), Rc::clone(&source));
        let framesetter = Framesetter::new(ts);
        let frame = framesetter.create_frame(0..0, FramePath::Rect(Rect { x: 0.0, y: 0.0, w: width_pt.max(1.0), h: f32::INFINITY }));
        let mut u16_at = Vec::new();
        let mut u = 0usize;
        for c in input.text.chars() {
            u16_at.push(u);
            u += c.len_utf16();
        }
        u16_at.push(u);
        let m = source.metrics(&run_style(&content.attributes_at(0)).effective_font());
        TextLayout { framesetter, frame, scale, u16_at, empty_height: m.ascent + m.descent + m.leading }
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn framesetter(&self) -> &Framesetter {
        &self.framesetter
    }

    fn px(&self, r: Rect) -> Rect {
        Rect { x: r.x * self.scale, y: r.y * self.scale, w: r.w * self.scale, h: r.h * self.scale }
    }

    fn u16_of(&self, ch: usize) -> usize {
        self.u16_at[ch.min(self.u16_at.len() - 1)]
    }

    fn char_of(&self, u: usize) -> usize {
        self.u16_at.partition_point(|&x| x < u).min(self.u16_at.len() - 1)
    }

    /// The height of the text in pixels, including an empty line after a trailing line break.
    pub fn height_px(&self) -> i32 {
        let total = self.u16_at.last().copied().unwrap_or(0);
        let mut bottom = self.frame.lines().last().map_or(0.0, |l| l.top + l.height);
        if let Some(c) = self.frame.caret_rect(total) {
            bottom = bottom.max(c.y + c.h);
        }
        if self.frame.lines().is_empty() {
            bottom = self.empty_height;
        }
        (bottom * self.scale).ceil() as i32
    }

    /// The character index nearest to the pixel (`x`, `y`) relative to the layout's origin.
    pub fn index_at_px(&self, x: f32, y: f32) -> usize {
        let (px, py) = (x / self.scale, y / self.scale);
        match self.frame.string_index_for_point(px, py) {
            Some(u) => self.char_of(u),
            None => 0,
        }
    }

    /// The caret at character index `ch` in pixels (zero width): x, top, height.
    pub fn caret_px(&self, ch: usize) -> Rect {
        match self.frame.caret_rect(self.u16_of(ch)) {
            Some(r) => self.px(r),
            None => self.px(Rect { x: 0.0, y: 0.0, w: 0.0, h: self.empty_height }),
        }
    }

    /// The rectangles covering the characters in `range`, in pixels.
    pub fn selection_px(&self, range: Range<usize>) -> Vec<Rect> {
        let r = self.u16_of(range.start)..self.u16_of(range.end);
        self.frame.selection_rects(r).into_iter().map(|r| self.px(r)).collect()
    }
}

// ---- painter ----

/// The pad a rendered text image has around its glyphs, in px (see the font renderer).
const IMAGE_PAD_PX: f32 = 2.0;

/// Draws text objects on a surface. Layout points become pixels through `scale`; (`ox`, `oy`) is the pixel origin.
pub struct SurfacePainter<'a, 's> {
    pub ui: &'a Ui,
    pub surface: &'a mut Surface<'s>,
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
}

impl SurfacePainter<'_, '_> {
    fn fill_px(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: u32) {
        let (x0, y0) = (x0.round().max(0.0), y0.round().max(0.0));
        let (w, h) = ((x1.round() - x0).max(1.0), (y1.round() - y0).max(1.0));
        self.surface.fill_rect(x0 as usize, y0 as usize, w as usize, h as usize, color);
    }
}

impl TextPainter for SurfacePainter<'_, '_> {
    fn default_color(&self) -> u32 {
        system::LABEL.now()
    }

    fn fill_rect(&mut self, r: Rect, color: u32) {
        let (x0, y0) = (self.ox + r.x * self.scale, self.oy + r.y * self.scale);
        self.fill_px(x0, y0, x0 + r.w * self.scale, y0 + r.h * self.scale, color);
    }

    fn draw_text(&mut self, text: &str, x: f32, baseline: f32, style: &RunStyle) {
        let font = style.effective_font();
        let weight = weight_of(font.weight);
        let color = style.foreground.unwrap_or_else(|| self.default_color());
        let px = self.ui.pt(font.size * self.ui.text_scale);
        let ascent = self.ui.font.line_metrics(px).ascent;
        let ix = (self.ox + x * self.scale - IMAGE_PAD_PX).round() as i32;
        let iy = (self.oy + baseline * self.scale - ascent - IMAGE_PAD_PX).round() as i32;
        let img = if style.features.is_empty() {
            self.ui.text(text, font.size, weight, ui::WHITE)
        } else {
            let mut f = crate::attributed::FontFeatures::new();
            for &(t, v) in &style.features {
                f.set(t, v);
            }
            self.ui.text_features(text, font.size, weight, ui::WHITE, &f)
        };
        if let Some(sc) = style.stroke_color {
            if style.stroke_width != 0.0 {
                let r = ((style.stroke_width.abs() / 100.0) * font.size * self.scale).round().max(1.0) as i32;
                for (dx, dy) in [(-r, 0), (r, 0), (0, -r), (0, r)] {
                    self.surface.blit_colored(&img, ix + dx, iy + dy, sc, 255);
                }
                if style.stroke_width > 0.0 {
                    return;
                }
            }
        }
        self.surface.blit_colored(&img, ix, iy, color, 255);
    }

    fn draw_rule(&mut self, x0: f32, x1: f32, y: f32, thickness: f32, color: u32) {
        let (a, b) = (self.ox + x0 * self.scale, self.ox + x1 * self.scale);
        let top = self.oy + (y - thickness / 2.0) * self.scale;
        self.fill_px(a, top, b, top + thickness * self.scale, color);
    }
}

impl TextLayout {
    /// Draws the selection, the text and, when `caret` is set, the caret, with the layout's origin at pixel (`x`, `y`).
    pub fn draw(&self, ui: &Ui, s: &mut Surface, x: i32, y: i32, selection: Option<Range<usize>>, caret: Option<usize>) {
        if let Some(sel) = selection {
            if sel.start < sel.end {
                for r in self.selection_px(sel) {
                    let (rx, ry) = (x as f32 + r.x, y as f32 + r.y);
                    s.fill_rect(rx.max(0.0) as usize, ry.max(0.0) as usize, r.w.round().max(1.0) as usize, r.h.round().max(1.0) as usize, ui::MATERIAL_SELECTION.now());
                }
            }
        }
        let mut p = SurfacePainter { ui, surface: s, scale: self.scale, ox: x as f32, oy: y as f32 };
        self.frame.draw(&mut p, 0.0, 0.0);
        if let Some(c) = caret {
            let r = self.caret_px(c);
            let w = ui.pt(1.0).round().max(1.0);
            p.surface.fill_rect((x as f32 + r.x).max(0.0) as usize, (y as f32 + r.y).max(0.0) as usize, w as usize, r.h.round().max(1.0) as usize, system::LABEL.now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributed::ColorToken;
    use bifrost_ui::paragraph::Alignment;

    /// Ten points per character at ten points; ascent 8, descent 2.
    fn source(input: &LayoutInput) -> Rc<dyn GlyphSource> {
        let measure = |f: &FontSpec, t: &str| t.chars().count() as f32 * f.size;
        let metrics = |f: &FontSpec| FontMetrics { ascent: f.size * 0.8, descent: f.size * 0.2, leading: 0.0, underline_position: 1.0, underline_thickness: 1.0 };
        Rc::new(TableSource::build(input, &measure, &metrics))
    }

    fn sized(text: &str) -> AttributedString {
        AttributedString::with_attrs(text, Attrs { font_size: Some(10.0), ..Default::default() })
    }

    fn layout(s: &AttributedString, w: f32) -> TextLayout {
        TextLayout::with_source(s, source(&s.layout_input()), w, 2.0)
    }

    #[test]
    fn utf16_conversions_and_range_methods() {
        let mut s = AttributedString::plain("a\u{1f600}bc");
        assert_eq!(s.utf16_len(), 5);
        assert_eq!(s.len(), 4);
        assert_eq!(s.char_index_from_utf16(3), 2);
        assert_eq!(s.utf16_index_from_char(2), 3);
        assert_eq!(s.char_index_from_utf16(99), 4);
        s.add_attribute_utf16(Attribute::Bold(true), 3..5);
        assert!(!s.attributes_at_utf16(1).bold);
        assert!(s.attributes_at_utf16(3).bold);
        assert_eq!(s.substring_utf16(3..5).string(), "bc");
        s.replace_characters_utf16(1..3, "-");
        assert_eq!(s.string(), "a-bc");
        let mut seen = Vec::new();
        s.enumerate_attribute_utf16(AttributeKey::Bold, 0..4, |r, a| seen.push((r, a)));
        assert_eq!(seen, vec![(0..2, Attribute::Bold(false)), (2..4, Attribute::Bold(true))]);
        s.remove_attribute_utf16(AttributeKey::Bold, 0..4);
        assert!(!s.attributes_at_utf16(3).bold);
    }

    #[test]
    fn extended_attributes_round_trip_through_the_model() {
        let mut s = AttributedString::plain("hello");
        s.add_attribute(Attribute::Kern(2.0), 0..2);
        s.add_attribute(Attribute::Foreground(Some(0x12345678)), 0..2);
        s.add_attribute(Attribute::FontFamily(Some("Mono".into())), 2..5);
        s.add_attribute(Attribute::Superscript(1), 4..5);
        s.add_attribute(Attribute::UnderlineStyle(UnderlineStyle::Double), 0..1);
        assert_eq!(s.attributes_at(0).kern, 2.0);
        assert_eq!(s.attributes_at(3).font_family.as_deref(), Some("Mono"));
        assert_eq!(s.attributes_at(4).superscript, 1);
        s.remove_attribute(AttributeKey::Kern, 0..5);
        assert_eq!(s.attributes_at(0).kern, 0.0);
        let input = s.layout_input();
        assert_eq!(input.text, "hello");
        assert_eq!(input.spans[0].1.foreground, Some(0x12345678));
        assert_eq!(input.spans[0].1.underline, UnderlineStyle::Double);
        assert!(input.spans.iter().any(|(_, st)| st.superscript == 1));
    }

    #[test]
    fn run_style_maps_the_model_attributes() {
        let a = Attrs { bold: true, link: Some("x".into()), color: Some(ColorToken::Red), strikethrough: true, ..Default::default() };
        let st = run_style(&a);
        assert_eq!(st.font.weight, 700);
        assert_eq!(st.underline, UnderlineStyle::Single);
        assert_eq!(st.foreground, Some(system::LINK.now()));
        assert!(st.strikethrough);
        assert_eq!(run_style(&Attrs { font_size: Some(20.0), ..Default::default() }).font.size, 20.0);
        assert_eq!(run_style(&Attrs::default()).foreground, None);
    }

    #[test]
    fn paragraph_styles_cover_whole_paragraphs() {
        let mut s = AttributedString::plain("one\ntwo\nthree");
        let ps = ParagraphStyle { alignment: Alignment::Center, ..Default::default() };
        s.set_paragraph_style(5..6, Some(ps.clone()));
        assert_eq!(s.paragraph_style_at_utf16(4), Some(ps.clone()));
        assert_eq!(s.paragraph_style_at_utf16(6), Some(ps.clone()));
        assert_eq!(s.paragraph_style_at_utf16(0), None);
        assert_eq!(s.paragraph_style_at_utf16(8), None);
        let input = s.layout_input();
        assert_eq!(input.paragraphs.len(), 1);
        assert_eq!(input.paragraphs[0].0, 4..8);
    }

    #[test]
    fn table_source_measures_each_font_once() {
        let s = sized("ab");
        let input = s.layout_input();
        let t = TableSource::build(&input, &|f, t| f.size * t.chars().count() as f32, &|f| FontMetrics { ascent: f.size, ..Default::default() });
        let font = input.spans[0].1.effective_font();
        assert_eq!(t.advances(&font, "ab"), vec![10.0, 10.0]);
        assert_eq!(t.advances(&font, "z"), vec![0.0]);
        assert_eq!(t.advances(&font, "\u{2026}"), vec![10.0]);
        assert_eq!(t.metrics(&font).ascent, 10.0);
    }

    #[test]
    fn layout_wraps_and_answers_pixel_queries() {
        // 50 points wide at 2 px per point: 100 px.
        let s = sized("aa bb cc dd");
        let l = layout(&s, 50.0);
        assert_eq!(l.frame().lines().len(), 2);
        assert_eq!(l.height_px(), 40);
        assert_eq!(l.index_at_px(0.0, 25.0), 6);
        assert_eq!(l.index_at_px(500.0, 5.0), 6);
        assert_eq!(l.caret_px(2), Rect { x: 40.0, y: 0.0, w: 0.0, h: 20.0 });
        assert_eq!(l.caret_px(6), Rect { x: 0.0, y: 20.0, w: 0.0, h: 20.0 });
        let rects = l.selection_px(3..8);
        assert_eq!(rects, vec![Rect { x: 60.0, y: 0.0, w: 60.0, h: 20.0 }, Rect { x: 0.0, y: 20.0, w: 40.0, h: 20.0 }]);
    }

    #[test]
    fn layout_uses_character_indices_across_surrogate_pairs() {
        let s = sized("a\u{1f600}b");
        let l = layout(&s, 500.0);
        // Character 2 ("b") is at UTF-16 offset 3: x = 20 points = 40 px.
        assert_eq!(l.caret_px(2).x, 40.0);
        assert_eq!(l.index_at_px(41.0, 5.0), 2);
        assert_eq!(l.selection_px(1..2), vec![Rect { x: 20.0, y: 0.0, w: 20.0, h: 20.0 }]);
    }

    #[test]
    fn empty_and_trailing_newline_layouts_have_a_caret_line() {
        let l = layout(&AttributedString::new(), 100.0);
        assert!(l.frame().lines().is_empty());
        // The line of an empty string is as tall as the body font: 13 points at 2 px per point.
        let body = ui::MAC_BODY_PT * 2.0;
        assert!((l.height_px() as f32 - body).abs() <= 1.0, "{}", l.height_px());
        assert!((l.caret_px(0).h - body).abs() < 0.01);
        assert_eq!(l.index_at_px(5.0, 5.0), 0);
        let t = layout(&sized("ab\n"), 100.0);
        assert_eq!(t.height_px(), 40);
        assert_eq!(t.caret_px(3), Rect { x: 0.0, y: 20.0, w: 0.0, h: 20.0 });
    }

    #[test]
    fn paragraph_alignment_reaches_the_frame() {
        let mut s = sized("aa");
        s.set_paragraph_style(0..2, Some(ParagraphStyle { alignment: Alignment::Right, ..Default::default() }));
        let l = layout(&s, 100.0);
        assert_eq!(l.frame().line_origins()[0].0, 80.0);
    }
}
