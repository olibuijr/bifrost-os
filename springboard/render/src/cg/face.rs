//! The drawing stack's font face (the reference platform's `CGFont`): one face of a font file, with no size, that
//! reports its metrics in font units and that a drawing context draws glyphs with (`setFont` plus
//! `showGlyphsAtPositions`). It is made from file bytes (a data provider) or looked up by name in the font manager.
//!
//! Measured API (CoreGraphics bridgesupport of the 26A434 reference): create with a data provider or a font name;
//! PostScript and full names; units per em, ascent, descent, leading, cap height, x height, italic angle and the font
//! box; the glyph count; advances and boxes of glyphs; glyph by name and name of a glyph; table tags and a table by
//! tag; variation axes and a copy with variations. `CGFontGetStemV`, PostScript subsets and encodings have no
//! counterpart: the font tables BifrOSt reads do not carry a stem width for TrueType faces, and nothing in BifrOSt
//! prints PostScript.

use crate::font::{Font, Var};
use crate::fontinfo::{FaceInfo, GlyphTables, Tables};
use crate::fontobj::{Bytes, FontManager};
use std::sync::Arc;

/// A glyph box or font box in font units: origin and size, y up (`CGRect` in glyph space).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One variation axis of a variable face: its tag, minimum, default and maximum in user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariationAxis {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// One face of a font file. Cloning shares the bytes.
#[derive(Clone, Debug)]
pub struct CgFont {
    data: Bytes,
    info: Arc<FaceInfo>,
    variations: Vec<Var>,
}

impl PartialEq for CgFont {
    fn eq(&self, o: &CgFont) -> bool {
        self.info == o.info && self.variations == o.variations && *self.data == *o.data
    }
}

impl CgFont {
    /// A face from the bytes of a TrueType or OpenType file (`CGFontCreateWithDataProvider`). `None` when the bytes
    /// are not a font the renderer can draw.
    pub fn from_data(data: Vec<u8>) -> Option<CgFont> {
        Self::from_bytes(Bytes::Shared(Arc::new(data)))
    }

    /// A face by PostScript, full or family name from `mgr` (`CGFontCreateWithFontName`).
    pub fn from_name(mgr: &FontManager, name: &str) -> Option<CgFont> {
        let i = mgr.find_name(name)?;
        Self::from_bytes(mgr.bytes(i)?)
    }

    fn from_bytes(data: Bytes) -> Option<CgFont> {
        let info = FaceInfo::parse(&data)?;
        Font::parse(&data)?;
        Some(CgFont { data, info: Arc::new(info), variations: Vec::new() })
    }

    /// The renderer's view of the face, for the drawing calls that take a font.
    pub fn font(&self) -> Option<Font<'_>> {
        Font::parse(&self.data)
    }

    /// The file bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn postscript_name(&self) -> &str {
        &self.info.postscript_name
    }

    pub fn full_name(&self) -> &str {
        &self.info.full_name
    }

    pub fn units_per_em(&self) -> i32 {
        self.info.units_per_em as i32
    }

    /// Ascent above the baseline in font units.
    pub fn ascent(&self) -> i32 {
        self.info.ascent as i32
    }

    /// Descent in font units; negative, below the baseline, as the font stores it.
    pub fn descent(&self) -> i32 {
        self.info.descent as i32
    }

    /// The line gap in font units.
    pub fn leading(&self) -> i32 {
        self.info.line_gap as i32
    }

    /// Cap height in font units; 0 when the font does not say.
    pub fn cap_height(&self) -> i32 {
        self.info.cap_height as i32
    }

    /// x height in font units; 0 when the font does not say.
    pub fn x_height(&self) -> i32 {
        self.info.x_height as i32
    }

    /// Degrees counter-clockwise from vertical (negative leans right).
    pub fn italic_angle(&self) -> f32 {
        self.info.italic_angle
    }

    /// The box that holds every glyph, in font units.
    pub fn font_bbox(&self) -> UnitRect {
        let [x0, y0, x1, y1] = self.info.bbox.map(|v| v as i32);
        UnitRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    pub fn number_of_glyphs(&self) -> usize {
        self.info.glyph_count as usize
    }

    /// Advance widths in font units, one per glyph; a glyph past the last one gets 0 (`CGFontGetGlyphAdvances`).
    pub fn glyph_advances(&self, glyphs: &[u16]) -> Vec<i32> {
        let Some(t) = GlyphTables::new(&self.data) else { return vec![0; glyphs.len()] };
        glyphs.iter().map(|&g| if g < t.glyph_count { t.advance(g) as i32 } else { 0 }).collect()
    }

    /// Glyph boxes in font units (`CGFontGetGlyphBBoxes`); an empty glyph or one past the last gets a zero box.
    pub fn glyph_bboxes(&self, glyphs: &[u16]) -> Vec<UnitRect> {
        let t = GlyphTables::new(&self.data);
        glyphs
            .iter()
            .map(|&g| match t.as_ref().and_then(|t| t.glyph_box(g)) {
                Some(b) => UnitRect {
                    x: b.x_min as i32,
                    y: b.y_min as i32,
                    w: b.x_max as i32 - b.x_min as i32,
                    h: b.y_max as i32 - b.y_min as i32,
                },
                None => UnitRect::default(),
            })
            .collect()
    }

    /// The glyph with this PostScript glyph name (`CGFontGetGlyphWithGlyphName`), from the post table or the CFF
    /// charset.
    pub fn glyph_with_name(&self, name: &str) -> Option<u16> {
        if name == ".notdef" {
            return Some(0);
        }
        GlyphTables::new(&self.data)
            .and_then(|t| t.glyph_named(name))
            .or_else(|| self.font().and_then(|f| f.glyph_named(name)))
    }

    /// The PostScript name of a glyph (`CGFontCopyGlyphNameForGlyph`); a generated `gNNN` when the font names none.
    /// `None` past the last glyph.
    pub fn glyph_name(&self, glyph: u16) -> Option<String> {
        let t = GlyphTables::new(&self.data)?;
        (glyph < t.glyph_count).then(|| t.glyph_name(glyph))
    }

    /// The glyph the font's character map gives `ch`; 0 when it has none.
    pub fn glyph_for_char(&self, ch: char) -> u16 {
        self.font().map(|f| f.glyph(ch)).unwrap_or(0)
    }

    /// The tags of the font's tables (`CGFontCopyTableTags`).
    pub fn table_tags(&self) -> Vec<[u8; 4]> {
        Tables::new(&self.data).map(|t| t.tags()).unwrap_or_default()
    }

    /// A copy of one table (`CGFontCopyTableForTag`).
    pub fn table(&self, tag: &[u8; 4]) -> Option<Vec<u8>> {
        Tables::new(&self.data)?.table(tag).map(|t| t.to_vec())
    }

    /// The variation axes; empty for a static face (`CGFontCopyVariationAxes`).
    pub fn variation_axes(&self) -> Vec<VariationAxis> {
        self.font()
            .map(|f| f.axes().iter().map(|a| VariationAxis { tag: a.tag, min: a.min, default: a.default, max: a.max }).collect())
            .unwrap_or_default()
    }

    /// The variation settings of this copy (`CGFontCopyVariations`).
    pub fn variations(&self) -> &[Var] {
        &self.variations
    }

    /// A copy with `vars` set (`CGFontCreateCopyWithVariations`). Values clamp to their axis; tags the face has no
    /// axis for are dropped, so a static face gives a copy with no variations. `None` never happens today; it is
    /// kept for parity with the reference, which fails for a face it cannot vary.
    pub fn with_variations(&self, vars: &[Var]) -> Option<CgFont> {
        let axes = self.variation_axes();
        let mut out: Vec<Var> = Vec::new();
        for (tag, v) in vars {
            let Some(a) = axes.iter().find(|a| a.tag == *tag) else { continue };
            out.retain(|(t, _)| t != tag);
            out.push((*tag, v.clamp(a.min, a.max)));
        }
        Some(CgFont { variations: out, ..self.clone() })
    }

    /// Advances of `glyphs` at `size` points, from font units.
    pub fn scaled_advances(&self, glyphs: &[u16], size: f32) -> Vec<f32> {
        let upem = self.units_per_em().max(1) as f32;
        self.glyph_advances(glyphs).into_iter().map(|a| a as f32 * size / upem).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> CgFont {
        let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf");
        CgFont::from_data(data).expect("the face parses")
    }

    #[test]
    fn rejects_bytes_that_are_not_a_font() {
        assert!(CgFont::from_data(b"not a font".to_vec()).is_none());
        assert!(CgFont::from_data(Vec::new()).is_none());
    }

    #[test]
    fn metrics_are_in_font_units() {
        let f = face();
        assert!(f.units_per_em() >= 16);
        assert!(f.ascent() > 0);
        assert!(f.descent() < 0, "descent stays negative as the font stores it");
        let b = f.font_bbox();
        assert!(b.w > 0 && b.h > 0);
        assert!(f.number_of_glyphs() > 0);
        assert!(!f.postscript_name().is_empty());
    }

    #[test]
    fn glyph_advances_and_boxes_follow_the_glyphs() {
        let f = face();
        let a = f.glyph_for_char('a');
        let space = f.glyph_for_char(' ');
        assert_ne!(a, 0);
        let past = f.number_of_glyphs() as u16;
        let adv = f.glyph_advances(&[a, space, past]);
        assert!(adv[0] > 0 && adv[1] > 0);
        assert_eq!(adv[2], 0, "a glyph past the last has no advance");
        let boxes = f.glyph_bboxes(&[a, space, past]);
        assert!(boxes[0].w > 0 && boxes[0].h > 0);
        assert_eq!(boxes[1].w, 0, "a space has no ink");
        assert_eq!(boxes[2], UnitRect::default());
        let scaled = f.scaled_advances(&[a], f.units_per_em() as f32);
        assert!((scaled[0] - adv[0] as f32).abs() < 1e-3, "at one em per unit the advance is the font-unit advance");
    }

    #[test]
    fn glyph_names_round_trip() {
        let f = face();
        assert_eq!(f.glyph_with_name(".notdef"), Some(0));
        let a = f.glyph_for_char('a');
        let name = f.glyph_name(a).expect("the glyph exists");
        assert_eq!(f.glyph_with_name(&name).unwrap_or(a), a);
        assert!(f.glyph_name(f.number_of_glyphs() as u16).is_none());
    }

    #[test]
    fn tables_are_listed_and_copied() {
        let f = face();
        let tags = f.table_tags();
        assert!(tags.contains(b"head"));
        let head = f.table(b"head").unwrap();
        assert_eq!(u16::from_be_bytes([head[18], head[19]]) as i32, f.units_per_em());
        assert!(f.table(b"zzzz").is_none());
    }

    #[test]
    fn variations_clamp_to_their_axes_and_drop_unknown_tags() {
        let f = face();
        let v = f.with_variations(&[(*b"zzzz", 3.0)]).unwrap();
        assert!(v.variations().is_empty());
        if let Some(axis) = f.variation_axes().first() {
            let v = f.with_variations(&[(axis.tag, axis.max + 1000.0)]).unwrap();
            assert_eq!(v.variations(), &[(axis.tag, axis.max)]);
        }
    }
}
