//! Font file facts read straight from the sfnt tables: the table directory, the name records, the weight, width
//! and style classes, the head box, hhea/OS2/post metrics, glyph counts, glyph names, per-glyph boxes and side
//! bearings. Everything borrows the file bytes and never allocates beyond the strings it returns. This is the data
//! layer under the app-facing font objects in `fontobj`.

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o.checked_add(2)?)?.try_into().ok()?))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

/// Symbolic style bits of a face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traits(pub u32);

impl Traits {
    pub const ITALIC: u32 = 1;
    pub const BOLD: u32 = 1 << 1;
    pub const MONO: u32 = 1 << 2;
    pub const CONDENSED: u32 = 1 << 3;
    pub const EXPANDED: u32 = 1 << 4;
    pub const VERTICAL: u32 = 1 << 5;
    pub const COLOR_GLYPHS: u32 = 1 << 6;

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
    pub fn with(self, bit: u32, on: bool) -> Traits {
        Traits(if on { self.0 | bit } else { self.0 & !bit })
    }
    pub fn is_italic(self) -> bool {
        self.has(Self::ITALIC)
    }
    pub fn is_bold(self) -> bool {
        self.has(Self::BOLD)
    }
    pub fn is_mono(self) -> bool {
        self.has(Self::MONO)
    }
    pub fn is_condensed(self) -> bool {
        self.has(Self::CONDENSED)
    }
    pub fn is_expanded(self) -> bool {
        self.has(Self::EXPANDED)
    }
}

/// Name ids of the `name` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameId {
    Family = 1,
    Style = 2,
    Full = 4,
    PostScript = 6,
    TypoFamily = 16,
    TypoStyle = 17,
}

/// A parsed table directory over the bytes of one font file.
#[derive(Clone, Copy)]
pub struct Tables<'a> {
    data: &'a [u8],
    base: usize,
}

impl<'a> Tables<'a> {
    /// Reads the table directory. A collection file opens its first font.
    pub fn new(data: &'a [u8]) -> Option<Self> {
        let mut base = 0;
        if data.get(0..4)? == b"ttcf" {
            base = u32_at(data, 12)? as usize;
        }
        let tag = data.get(base..base + 4)?;
        if !(tag == [0, 1, 0, 0] || tag == b"OTTO" || tag == b"true" || tag == b"typ1") {
            return None;
        }
        u16_at(data, base + 4)?;
        Some(Tables { data, base })
    }

    fn count(&self) -> usize {
        u16_at(self.data, self.base + 4).unwrap_or(0) as usize
    }

    /// Every table tag in the file, in directory order.
    pub fn tags(&self) -> Vec<[u8; 4]> {
        (0..self.count())
            .filter_map(|i| {
                let rec = self.base + 12 + 16 * i;
                self.data.get(rec..rec + 4)?.try_into().ok()
            })
            .collect()
    }

    /// The bytes of the table `tag`, or None when absent or cut off.
    pub fn table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        (0..self.count()).find_map(|i| {
            let rec = self.base + 12 + 16 * i;
            if self.data.get(rec..rec + 4)? != tag {
                return None;
            }
            let off = u32_at(self.data, rec + 8)? as usize;
            let len = u32_at(self.data, rec + 12)? as usize;
            self.data.get(off..off.checked_add(len)?)
        })
    }

    pub fn has(&self, tag: &[u8; 4]) -> bool {
        self.table(tag).is_some()
    }

    /// The string for `id`, preferring a Windows or Unicode record (UTF-16) over a Mac Roman one.
    pub fn name(&self, id: NameId) -> Option<String> {
        let t = self.table(b"name")?;
        let count = u16_at(t, 2)? as usize;
        let strings = u16_at(t, 4)? as usize;
        let mut best: Option<(u8, String)> = None;
        for i in 0..count {
            let r = 6 + 12 * i;
            if u16_at(t, r + 6)? != id as u16 {
                continue;
            }
            let (pid, eid) = (u16_at(t, r)?, u16_at(t, r + 2)?);
            let len = u16_at(t, r + 8)? as usize;
            let off = strings + u16_at(t, r + 10)? as usize;
            let Some(raw) = t.get(off..off + len) else { continue };
            let (rank, s) = match (pid, eid) {
                (3, 1) | (3, 10) | (0, _) => {
                    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    (0u8, String::from_utf16_lossy(&units))
                }
                (1, 0) => (1u8, raw.iter().map(|&b| if b < 128 { b as char } else { '?' }).collect()),
                _ => continue,
            };
            if !s.is_empty() && best.as_ref().is_none_or(|b| rank < b.0) {
                best = Some((rank, s));
            }
        }
        best.map(|b| b.1)
    }
}

/// Facts about one face.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    pub family: String,
    pub style: String,
    pub full_name: String,
    pub postscript_name: String,
    pub units_per_em: u16,
    /// hhea ascent, descent (negative) and line gap in font units
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
    pub cap_height: i16,
    pub x_height: i16,
    /// head box: xMin, yMin, xMax, yMax in font units
    pub bbox: [i16; 4],
    pub underline_position: i16,
    pub underline_thickness: i16,
    /// degrees counter-clockwise from vertical, as post stores it (negative leans right)
    pub italic_angle: f32,
    pub glyph_count: u16,
    /// OS/2 usWeightClass (100..900), 400 when absent
    pub weight_class: u16,
    /// OS/2 usWidthClass (1..9), 5 when absent
    pub width_class: u16,
    pub traits: Traits,
    pub variable: bool,
}

impl FaceInfo {
    /// Reads the face. Needs head, hhea and maxp; everything else falls back to a neutral value.
    pub fn parse(data: &[u8]) -> Option<FaceInfo> {
        let t = Tables::new(data)?;
        let head = t.table(b"head")?;
        let hhea = t.table(b"hhea")?;
        let maxp = t.table(b"maxp")?;
        let os2 = t.table(b"OS/2");
        let post = t.table(b"post");
        let os2_ver = os2.and_then(|o| u16_at(o, 0)).unwrap_or(0);
        let weight_class = os2.and_then(|o| u16_at(o, 4)).filter(|&w| w > 0).unwrap_or(400);
        let width_class = os2.and_then(|o| u16_at(o, 6)).filter(|w| (1..=9).contains(w)).unwrap_or(5);
        let fs_selection = os2.and_then(|o| u16_at(o, 62)).unwrap_or(0);
        let mac_style = u16_at(head, 44).unwrap_or(0);
        let italic_angle = post.and_then(|p| i16_at(p, 4)).map(|v| v as f32).unwrap_or(0.0)
            + post.and_then(|p| u16_at(p, 6)).map(|f| f as f32 / 65536.0).unwrap_or(0.0);
        let fixed = post.and_then(|p| u32_at(p, 12)).is_some_and(|v| v != 0);
        let family = t.name(NameId::TypoFamily).or_else(|| t.name(NameId::Family)).unwrap_or_default();
        let style = t.name(NameId::TypoStyle).or_else(|| t.name(NameId::Style)).unwrap_or_default();
        let lstyle = style.to_ascii_lowercase();
        let mut traits = Traits::default()
            .with(Traits::ITALIC, fs_selection & 1 != 0 || mac_style & 2 != 0 || lstyle.contains("italic") || lstyle.contains("oblique") || italic_angle != 0.0)
            .with(Traits::BOLD, weight_class >= 600 || fs_selection & 32 != 0 || mac_style & 1 != 0)
            .with(Traits::MONO, fixed)
            .with(Traits::CONDENSED, width_class <= 4 || mac_style & 32 != 0)
            .with(Traits::EXPANDED, width_class >= 6 || mac_style & 64 != 0)
            .with(Traits::COLOR_GLYPHS, t.has(b"COLR") || t.has(b"sbix") || t.has(b"CBDT"));
        if t.has(b"vhea") && t.has(b"vmtx") {
            traits = traits.with(Traits::VERTICAL, true);
        }
        let full_name = t.name(NameId::Full).unwrap_or_else(|| format!("{family} {style}").trim().to_string());
        let postscript_name = t.name(NameId::PostScript).unwrap_or_else(|| full_name.replace(' ', "-"));
        Some(FaceInfo {
            family,
            style,
            full_name,
            postscript_name,
            units_per_em: u16_at(head, 18).filter(|&u| u > 0)?,
            ascent: i16_at(hhea, 4)?,
            descent: i16_at(hhea, 6)?,
            line_gap: i16_at(hhea, 8).unwrap_or(0),
            cap_height: if os2_ver >= 2 { os2.and_then(|o| i16_at(o, 88)).unwrap_or(0) } else { 0 },
            x_height: if os2_ver >= 2 { os2.and_then(|o| i16_at(o, 86)).unwrap_or(0) } else { 0 },
            bbox: [i16_at(head, 36)?, i16_at(head, 38)?, i16_at(head, 40)?, i16_at(head, 42)?],
            underline_position: post.and_then(|p| i16_at(p, 8)).unwrap_or(0),
            underline_thickness: post.and_then(|p| i16_at(p, 10)).unwrap_or(0),
            italic_angle,
            glyph_count: u16_at(maxp, 4)?,
            weight_class,
            width_class,
            traits,
            variable: t.has(b"fvar"),
        })
    }

    /// Weight on the -1.0 (thinnest) to 1.0 (heaviest) scale, 0.0 for Regular.
    pub fn weight_value(&self) -> f32 {
        weight_class_to_value(self.weight_class)
    }

    /// Width on the -1.0 (most condensed) to 1.0 (most expanded) scale, 0.0 for normal.
    pub fn width_value(&self) -> f32 {
        width_class_to_value(self.width_class)
    }
}

/// Maps a 100..900 weight class onto -1.0..1.0 with 400 at 0.
pub fn weight_class_to_value(class: u16) -> f32 {
    let c = class.clamp(100, 900) as f32;
    if c <= 400.0 {
        (c - 400.0) / 300.0
    } else {
        (c - 400.0) / 500.0
    }
}

/// Inverse of `weight_class_to_value`, rounded to the nearest hundred.
pub fn value_to_weight_class(v: f32) -> u16 {
    let v = v.clamp(-1.0, 1.0);
    let c = if v <= 0.0 { 400.0 + v * 300.0 } else { 400.0 + v * 500.0 };
    ((c / 100.0).round() as u16 * 100).clamp(100, 900)
}

/// Maps the 1..9 width class onto -1.0..1.0 with 5 at 0.
pub fn width_class_to_value(class: u16) -> f32 {
    (class.clamp(1, 9) as f32 - 5.0) / 4.0
}

/// Glyph level reads over one file.
pub struct GlyphTables<'a> {
    hmtx: &'a [u8],
    glyf: Option<&'a [u8]>,
    loca: Option<&'a [u8]>,
    post: Option<&'a [u8]>,
    num_hmetrics: u16,
    pub glyph_count: u16,
    loca_long: bool,
}

/// One glyph's box in font units; y grows upward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphBox {
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
}

impl<'a> GlyphTables<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        let t = Tables::new(data)?;
        let head = t.table(b"head")?;
        let hhea = t.table(b"hhea")?;
        Some(GlyphTables {
            hmtx: t.table(b"hmtx")?,
            glyf: t.table(b"glyf"),
            loca: t.table(b"loca"),
            post: t.table(b"post"),
            num_hmetrics: u16_at(hhea, 34)?.max(1),
            glyph_count: u16_at(t.table(b"maxp")?, 4)?,
            loca_long: i16_at(head, 50)? != 0,
        })
    }

    /// Advance width in font units. Glyphs past the last full metric share its advance.
    pub fn advance(&self, gid: u16) -> u16 {
        let i = gid.min(self.num_hmetrics - 1) as usize;
        u16_at(self.hmtx, 4 * i).unwrap_or(0)
    }

    /// Left side bearing from hmtx in font units.
    pub fn left_bearing(&self, gid: u16) -> i16 {
        if gid < self.num_hmetrics {
            i16_at(self.hmtx, 4 * gid as usize + 2).unwrap_or(0)
        } else {
            let o = 4 * self.num_hmetrics as usize + 2 * (gid - self.num_hmetrics) as usize;
            i16_at(self.hmtx, o).unwrap_or(0)
        }
    }

    /// The glyph's box from its outline header. Empty glyphs (spaces) give a zero box. None for outline-less
    /// (CFF) files or a glyph out of range.
    pub fn glyph_box(&self, gid: u16) -> Option<GlyphBox> {
        if gid >= self.glyph_count {
            return None;
        }
        let (glyf, loca) = (self.glyf?, self.loca?);
        let (a, b) = if self.loca_long {
            (u32_at(loca, 4 * gid as usize)? as usize, u32_at(loca, 4 * gid as usize + 4)? as usize)
        } else {
            (2 * u16_at(loca, 2 * gid as usize)? as usize, 2 * u16_at(loca, 2 * gid as usize + 2)? as usize)
        };
        if b <= a {
            return Some(GlyphBox::default());
        }
        Some(GlyphBox { x_min: i16_at(glyf, a + 2)?, y_min: i16_at(glyf, a + 4)?, x_max: i16_at(glyf, a + 6)?, y_max: i16_at(glyf, a + 8)? })
    }

    /// Distances from the origin and from the advance edge to the ink, in font units: (left, right). Negative
    /// when ink overhangs the advance box.
    pub fn optical_bounds(&self, gid: u16) -> Option<(i16, i32)> {
        let b = self.glyph_box(gid)?;
        let adv = self.advance(gid) as i32;
        Some((b.x_min, adv - b.x_max as i32))
    }

    /// The glyph's PostScript name from a format 2 `post` table; `.notdef` for 0 and a generated `gNNN` when
    /// the font carries no names.
    pub fn glyph_name(&self, gid: u16) -> String {
        if let Some(n) = self.post_name(gid) {
            return n;
        }
        if gid == 0 { ".notdef".to_string() } else { format!("g{gid}") }
    }

    fn post_name(&self, gid: u16) -> Option<String> {
        let p = self.post?;
        if u32_at(p, 0)? != 0x0002_0000 {
            return None;
        }
        let n = u16_at(p, 32)? as usize;
        if gid as usize >= n {
            return None;
        }
        let idx = u16_at(p, 34 + 2 * gid as usize)? as usize;
        if idx < 258 {
            return (gid == 0 && idx == 0).then(|| ".notdef".to_string());
        }
        // Pascal strings follow the index array
        let mut o = 34 + 2 * n;
        for _ in 0..(idx - 258) {
            o += 1 + *p.get(o)? as usize;
        }
        let len = *p.get(o)? as usize;
        let raw = p.get(o + 1..o + 1 + len)?;
        Some(raw.iter().map(|&b| b as char).collect())
    }

    /// The glyph whose post name is `name` (format 2 only).
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        (0..self.glyph_count).find(|&g| self.post_name(g).as_deref() == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf")
    }

    #[test]
    fn weight_scale_round_trips() {
        assert_eq!(weight_class_to_value(400), 0.0);
        assert_eq!(weight_class_to_value(900), 1.0);
        assert_eq!(weight_class_to_value(100), -1.0);
        for c in [100u16, 300, 400, 500, 700, 900] {
            assert_eq!(value_to_weight_class(weight_class_to_value(c)), c);
        }
        assert_eq!(width_class_to_value(5), 0.0);
        assert_eq!(width_class_to_value(1), -1.0);
    }

    #[test]
    fn traits_set_and_clear() {
        let t = Traits::default().with(Traits::BOLD, true).with(Traits::ITALIC, true);
        assert!(t.is_bold() && t.is_italic() && !t.is_mono());
        assert!(!t.with(Traits::BOLD, false).is_bold());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Tables::new(b"nonsense bytes here").is_none());
        assert!(FaceInfo::parse(&[0, 1, 0, 0, 0, 0]).is_none());
        assert!(GlyphTables::new(&[]).is_none());
    }

    #[test]
    fn reads_the_bundled_face() {
        let d = sf();
        let t = Tables::new(&d).unwrap();
        assert!(t.has(b"head") && t.has(b"cmap"));
        assert!(t.tags().contains(b"hhea"));
        let info = FaceInfo::parse(&d).unwrap();
        assert!(info.units_per_em > 0 && info.glyph_count > 10);
        assert!(info.ascent > 0 && info.descent < 0);
        assert!(!info.family.is_empty() && !info.postscript_name.is_empty());
        assert!(info.bbox[2] > info.bbox[0]);
        let g = GlyphTables::new(&d).unwrap();
        assert_eq!(g.glyph_count, info.glyph_count);
    }

    #[test]
    fn glyph_boxes_sit_inside_the_head_box() {
        let d = sf();
        let info = FaceInfo::parse(&d).unwrap();
        let g = GlyphTables::new(&d).unwrap();
        let font = crate::font::Font::parse(&d).unwrap();
        let a = font.glyph('H');
        let b = g.glyph_box(a).unwrap();
        assert!(b.x_max > b.x_min && b.y_max > b.y_min);
        assert!(b.y_max <= info.bbox[3] && b.y_min >= info.bbox[1]);
        assert_eq!(g.advance(a) as f32, font.advance(a));
        let (l, r) = g.optical_bounds(a).unwrap();
        assert!(l >= 0 && r >= 0);
        assert!(g.glyph_box(u16::MAX).is_none());
        assert_eq!(g.glyph_box(font.glyph(' ')).unwrap(), GlyphBox::default());
    }

    #[test]
    fn glyph_names_never_panic() {
        let d = sf();
        let g = GlyphTables::new(&d).unwrap();
        assert_eq!(g.glyph_name(0), ".notdef");
        assert!(!g.glyph_name(5).is_empty());
        assert!(!g.glyph_name(u16::MAX).is_empty());
    }
}
