//! The standard 14 fonts (and any other font a PDF names without embedding it): which openly licensed face draws
//! it, the advance widths of the standard faces' AFM metrics for layout when the file gives no `/Widths`, and the
//! built-in encodings (StandardEncoding for the text faces, the Symbol and ZapfDingbats encodings).
//!
//! Faces come from the system font directories, in order: the URW base 35 clones (`gsfonts`, AGPL with the font
//! exception: same glyph names and metrics as the originals), Liberation (SIL OFL, metric-compatible with the sans,
//! serif and mono faces), TeX Gyre (GUST font licence) and DejaVu or Noto as a last resort. No vendor face is read.

use crate::font::Font as Face;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Sans,
    Serif,
    Mono,
    Symbol,
    Dingbats,
}

/// A standard face: Helvetica, Times or Courier in one of four styles, Symbol or ZapfDingbats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Standard {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
}

/// The built-in encoding of a non-embedded simple font that names no base encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Builtin {
    #[default]
    None,
    Standard,
    Symbol,
    Dingbats,
}

impl Standard {
    pub const SANS: Standard = Standard { family: Family::Sans, bold: false, italic: false };

    /// The standard face for a font name. Names are matched loosely (`Helvetica-BoldOblique`, `Arial,Bold`,
    /// `ABCDEF+TimesNewRomanPS-ItalicMT`); a name that says nothing falls back to the descriptor: `flags` (fixed
    /// pitch, serif, italic, force bold), `weight` and `italic_angle`.
    pub fn from_name(base_font: &str, flags: i64, weight: Option<f64>, italic_angle: f64) -> Standard {
        let name = match base_font.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
            _ => base_font,
        };
        let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        let family = if has(&["dingbat"]) {
            Family::Dingbats
        } else if n.starts_with("symbol") || has(&["standardsymbol"]) {
            Family::Symbol
        } else if has(&["courier", "mono", "consol", "menlo", "typewriter", "fixed"]) {
            Family::Mono
        } else if has(&["helvetica", "arial", "sans", "verdana", "tahoma", "calibri", "geneva", "gothic", "frutiger", "univers"]) {
            Family::Sans
        } else if has(&["times", "roman", "serif", "georgia", "garamond", "palatino", "bookman", "cambria", "minion", "century", "schoolbook", "baskerville"]) {
            Family::Serif
        } else if flags & 1 != 0 {
            Family::Mono
        } else if flags & 2 != 0 {
            Family::Serif
        } else {
            Family::Sans
        };
        let bold = has(&["bold", "black", "heavy", "demi", "semibold"]) || weight.is_some_and(|w| w >= 600.0) || flags & (1 << 18) != 0;
        let italic = has(&["italic", "oblique"]) || flags & 64 != 0 || italic_angle != 0.0;
        match family {
            Family::Symbol | Family::Dingbats => Standard { family, bold: false, italic: false },
            _ => Standard { family, bold, italic },
        }
    }

    /// The face files to try, best match first.
    pub fn files(&self) -> Vec<String> {
        let (b, i) = (self.bold, self.italic);
        let style = match (b, i) {
            (false, false) => "Regular",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (true, true) => "BoldItalic",
        };
        let gyre = style.to_ascii_lowercase();
        let dejavu = |oblique: &str| match (b, i) {
            (false, false) => String::new(),
            (true, false) => "-Bold".into(),
            (false, true) => format!("-{oblique}"),
            (true, true) => format!("-Bold{oblique}"),
        };
        let gs = "/usr/share/fonts/gsfonts";
        let lib = "/usr/share/fonts/liberation";
        let ttf = "/usr/share/fonts/TTF";
        let noto = "/usr/share/fonts/noto";
        let gyre_dirs = ["/usr/share/fonts/tex-gyre", "/usr/share/fonts/OTF", "/usr/share/texmf-dist/fonts/opentype/public/tex-gyre"];
        let text = |urw: &str, liberation: &str, tex: &str, dv: String| -> Vec<String> {
            let mut v = vec![format!("{gs}/{urw}-{style}.otf"), format!("{lib}/{liberation}-{style}.ttf")];
            v.extend(gyre_dirs.iter().map(|d| format!("{d}/{tex}-{gyre}.otf")));
            v.push(format!("{ttf}/{dv}.ttf"));
            v
        };
        let mut out = Vec::new();
        match self.family {
            Family::Sans => out = text("NimbusSans", "LiberationSans", "texgyreheros", format!("DejaVuSans{}", dejavu("Oblique"))),
            Family::Serif => out = text("NimbusRoman", "LiberationSerif", "texgyretermes", format!("DejaVuSerif{}", dejavu("Italic"))),
            Family::Mono => out = text("NimbusMonoPS", "LiberationMono", "texgyrecursor", format!("DejaVuSansMono{}", dejavu("Oblique"))),
            Family::Symbol => out.extend([
                format!("{gs}/StandardSymbolsPS.otf"),
                format!("{noto}/NotoSansMath-Regular.ttf"),
                format!("{noto}/NotoSansSymbols-Regular.ttf"),
                format!("{ttf}/DejaVuSans.ttf"),
            ]),
            Family::Dingbats => out.extend([
                format!("{gs}/D050000L.otf"),
                format!("{noto}/NotoSansSymbols2-Regular.ttf"),
                format!("{ttf}/DejaVuSans.ttf"),
            ]),
        }
        if matches!(self.family, Family::Sans | Family::Serif | Family::Mono) {
            out.push(format!("{noto}/NotoSans-{style}.ttf"));
            out.push(format!("{noto}/NotoSans-Regular.ttf"));
        }
        out
    }

    /// True for Symbol and ZapfDingbats, whose codes are not Latin text.
    pub fn is_symbol(&self) -> bool {
        matches!(self.family, Family::Symbol | Family::Dingbats)
    }

    /// The width a glyph gets when neither the file nor the AFM table knows it.
    pub fn default_width(&self) -> f32 {
        match self.family {
            Family::Sans => 556.0,
            Family::Serif => 500.0,
            Family::Mono => 600.0,
            Family::Symbol => 500.0,
            Family::Dingbats => 788.0,
        }
    }

    /// The AFM advance (1/1000 em) of a glyph whose text is `text`, when the standard metrics give it.
    pub fn width(&self, text: &str) -> Option<f32> {
        let mut it = text.chars();
        let (c, rest) = (it.next()?, it.next());
        if self.family == Family::Mono {
            return Some(600.0);
        }
        let table = match (self.family, self.bold, self.italic) {
            (Family::Sans, false, _) => &HELVETICA,
            (Family::Sans, true, _) => &HELVETICA_BOLD,
            (Family::Serif, false, false) => &TIMES,
            (Family::Serif, true, false) => &TIMES_BOLD,
            (Family::Serif, false, true) => &TIMES_ITALIC,
            (Family::Serif, true, true) => &TIMES_BOLD_ITALIC,
            _ => return None,
        };
        let style = match (self.family, self.bold, self.italic) {
            (Family::Sans, false, _) => 0,
            (Family::Sans, true, _) => 1,
            (_, false, false) => 2,
            (_, true, false) => 3,
            (_, false, true) => 4,
            (_, true, true) => 5,
        };
        if rest.is_some() {
            // ligatures a ToUnicode or Differences entry spelt out
            return match text {
                "fi" | "fl" => Some([500.0, 611.0, 556.0, 556.0, 500.0, 556.0][style]),
                _ => None,
            };
        }
        let w = |c: char| (' '..='~').contains(&c).then(|| table[c as usize - 32] as f32);
        if let Some(v) = w(c) {
            return Some(v);
        }
        let per_style = |v: [u16; 6]| Some(v[style] as f32);
        match c {
            '\u{a0}' => w(' '),
            '\u{2019}' | '\u{2018}' => per_style([222, 278, 333, 333, 333, 333]),
            '\u{201c}' | '\u{201d}' => per_style([333, 500, 444, 500, 556, 500]),
            '\u{2013}' => per_style([556, 556, 500, 500, 500, 500]),
            '\u{2014}' | '\u{2026}' | '\u{2030}' => Some(1000.0),
            '\u{2022}' => Some(350.0),
            '\u{fb01}' | '\u{fb02}' => per_style([500, 611, 556, 556, 500, 556]),
            _ => latin_base(c).and_then(w),
        }
    }
}

/// The unaccented letter of a Latin-1 letter (accented AFM glyphs keep their base letter's advance).
fn latin_base(c: char) -> Option<char> {
    const MAP: [(&str, char); 16] = [
        ("ÀÁÂÃÄÅ", 'A'),
        ("Ç", 'C'),
        ("ÈÉÊË", 'E'),
        ("ÌÍÎÏ", 'I'),
        ("Ñ", 'N'),
        ("ÒÓÔÕÖØ", 'O'),
        ("ÙÚÛÜ", 'U'),
        ("Ý", 'Y'),
        ("àáâãäå", 'a'),
        ("ç", 'c'),
        ("èéêë", 'e'),
        ("ìíîï", 'i'),
        ("ñ", 'n'),
        ("òóôõöø", 'o'),
        ("ùúûü", 'u'),
        ("ýÿ", 'y'),
    ];
    MAP.iter().find(|(set, _)| set.contains(c)).map(|(_, b)| *b)
}

/// Adobe StandardEncoding, the base encoding of a non-symbolic Type 1 font that names none.
pub fn standard_encoding(code: u8) -> Option<char> {
    let c = match code {
        0x27 => '\u{2019}',
        0x60 => '\u{2018}',
        0x20..=0x7e => code as char,
        0xa1 => '¡',
        0xa2 => '¢',
        0xa3 => '£',
        0xa4 => '\u{2044}',
        0xa5 => '¥',
        0xa6 => '\u{192}',
        0xa7 => '§',
        0xa8 => '¤',
        0xa9 => '\'',
        0xaa => '\u{201c}',
        0xab => '«',
        0xac => '\u{2039}',
        0xad => '\u{203a}',
        0xae => '\u{fb01}',
        0xaf => '\u{fb02}',
        0xb1 => '\u{2013}',
        0xb2 => '\u{2020}',
        0xb3 => '\u{2021}',
        0xb4 => '·',
        0xb6 => '¶',
        0xb7 => '\u{2022}',
        0xb8 => '\u{201a}',
        0xb9 => '\u{201e}',
        0xba => '\u{201d}',
        0xbb => '»',
        0xbc => '\u{2026}',
        0xbd => '\u{2030}',
        0xbf => '¿',
        0xc1 => '`',
        0xc2 => '´',
        0xc3 => '\u{2c6}',
        0xc4 => '\u{2dc}',
        0xc5 => '¯',
        0xc6 => '\u{2d8}',
        0xc7 => '\u{2d9}',
        0xc8 => '¨',
        0xca => '\u{2da}',
        0xcb => '¸',
        0xcd => '\u{2dd}',
        0xce => '\u{2db}',
        0xcf => '\u{2c7}',
        0xd0 => '\u{2014}',
        0xe1 => 'Æ',
        0xe3 => 'ª',
        0xe8 => '\u{141}',
        0xe9 => 'Ø',
        0xea => '\u{152}',
        0xeb => 'º',
        0xf1 => 'æ',
        0xf5 => '\u{131}',
        0xf8 => '\u{142}',
        0xf9 => 'ø',
        0xfa => '\u{153}',
        0xfb => 'ß',
        _ => return None,
    };
    Some(c)
}

/// The Symbol font's built-in encoding, as Unicode.
pub fn symbol_encoding(code: u8) -> Option<char> {
    const UPPER: &str = "ΑΒΧΔΕΦΓΗΙϑΚΛΜΝΟΠΘΡΣΤΥςΩΞΨΖ";
    const LOWER: &str = "αβχδεφγηιϕκλμνοπθρστυϖωξψζ";
    let c = match code {
        0x22 => '∀',
        0x24 => '∃',
        0x27 => '∋',
        0x2a => '∗',
        0x2d => '−',
        0x40 => '≅',
        0x41..=0x5a => UPPER.chars().nth((code - 0x41) as usize)?,
        0x5c => '∴',
        0x5e => '⊥',
        0x60 => '\u{203e}',
        0x61..=0x7a => LOWER.chars().nth((code - 0x61) as usize)?,
        0x7e => '∼',
        0x20..=0x7d => code as char,
        0xa0 => '€',
        0xa1 => 'ϒ',
        0xa2 => '′',
        0xa3 => '≤',
        0xa4 => '⁄',
        0xa5 => '∞',
        0xa6 => 'ƒ',
        0xa7 => '♣',
        0xa8 => '♦',
        0xa9 => '♥',
        0xaa => '♠',
        0xab => '↔',
        0xac => '←',
        0xad => '↑',
        0xae => '→',
        0xaf => '↓',
        0xb0 => '°',
        0xb1 => '±',
        0xb2 => '″',
        0xb3 => '≥',
        0xb4 => '×',
        0xb5 => '∝',
        0xb6 => '∂',
        0xb7 => '•',
        0xb8 => '÷',
        0xb9 => '≠',
        0xba => '≡',
        0xbb => '≈',
        0xbc => '…',
        0xbd => '\u{23d0}',
        0xbe => '\u{23af}',
        0xbf => '↵',
        0xc0 => 'ℵ',
        0xc1 => 'ℑ',
        0xc2 => 'ℜ',
        0xc3 => '℘',
        0xc4 => '⊗',
        0xc5 => '⊕',
        0xc6 => '∅',
        0xc7 => '∩',
        0xc8 => '∪',
        0xc9 => '⊃',
        0xca => '⊇',
        0xcb => '⊄',
        0xcc => '⊂',
        0xcd => '⊆',
        0xce => '∈',
        0xcf => '∉',
        0xd0 => '∠',
        0xd1 => '∇',
        0xd2 | 0xe2 => '®',
        0xd3 | 0xe3 => '©',
        0xd4 | 0xe4 => '™',
        0xd5 => '∏',
        0xd6 => '√',
        0xd7 => '⋅',
        0xd8 => '¬',
        0xd9 => '∧',
        0xda => '∨',
        0xdb => '⇔',
        0xdc => '⇐',
        0xdd => '⇑',
        0xde => '⇒',
        0xdf => '⇓',
        0xe0 => '◊',
        0xe1 => '\u{2329}',
        0xe5 => '∑',
        0xe6..=0xe8 => char::from_u32(0x239b + (code - 0xe6) as u32)?,
        0xe9..=0xeb => char::from_u32(0x23a1 + (code - 0xe9) as u32)?,
        0xec..=0xee => char::from_u32(0x23a7 + (code - 0xec) as u32)?,
        0xef => '\u{23aa}',
        0xf1 => '\u{232a}',
        0xf2 => '∫',
        0xf3 => '\u{2320}',
        0xf4 => '\u{23ae}',
        0xf5 => '\u{2321}',
        0xf6..=0xf8 => char::from_u32(0x239e + (code - 0xf6) as u32)?,
        0xf9..=0xfb => char::from_u32(0x23a4 + (code - 0xf9) as u32)?,
        0xfc..=0xfe => char::from_u32(0x23ab + (code - 0xfc) as u32)?,
        _ => return None,
    };
    Some(c)
}

/// The ZapfDingbats built-in encoding, as Unicode (the Dingbats block follows its order).
pub fn dingbats_encoding(code: u8) -> Option<char> {
    let u = match code {
        0x20 => 0x20,
        0x25 => 0x260e,
        0x2a => 0x261b,
        0x2b => 0x261e,
        0x48 => 0x2605,
        0x6c => 0x25cf,
        0x6e => 0x25a0,
        0x73 => 0x25b2,
        0x74 => 0x25bc,
        0x75 => 0x25c6,
        0x77 => 0x25d7,
        0x21..=0x7e => 0x2700 + (code as u32 - 0x20),
        0x80..=0x8d => 0x2768 + (code as u32 - 0x80),
        0xa1..=0xa7 => 0x2761 + (code as u32 - 0xa1),
        0xa8 => 0x2663,
        0xa9 => 0x2666,
        0xaa => 0x2665,
        0xab => 0x2660,
        0xac..=0xb5 => 0x2460 + (code as u32 - 0xac),
        0xb6..=0xd4 => 0x2776 + (code as u32 - 0xb6),
        0xd5 => 0x2192,
        0xd6 => 0x2194,
        0xd7 => 0x2195,
        0xd8..=0xfe => 0x2798 + (code as u32 - 0xd8),
        _ => return None,
    };
    char::from_u32(u)
}

impl Builtin {
    /// The text of `code` in this built-in encoding (`None` for [`Builtin::None`] or an unencoded code).
    pub fn text(self, code: u8) -> Option<char> {
        match self {
            Builtin::None => None,
            Builtin::Standard => standard_encoding(code),
            Builtin::Symbol => symbol_encoding(code),
            Builtin::Dingbats => dingbats_encoding(code),
        }
    }
}

thread_local! {
    static FACES: RefCell<HashMap<Standard, Option<Rc<Vec<u8>>>>> = RefCell::new(HashMap::new());
}

/// The bytes of the first installed face for `s`, read once per thread. None when no candidate is installed.
pub fn face_bytes(s: Standard) -> Option<Rc<Vec<u8>>> {
    FACES.with(|cache| {
        cache
            .borrow_mut()
            .entry(s)
            .or_insert_with(|| {
                let found = s.files().iter().find_map(|p| std::fs::read(p).ok().filter(|d| Face::parse(d).is_some()));
                if found.is_none() {
                    eprintln!("pdf: no installed face for the standard font {s:?}; its text falls back to the system cascade");
                }
                found.map(Rc::new)
            })
            .clone()
    })
}

/// The glyph of `face` for a fallback-drawn character: by its text, then by glyph name (CFF faces), then, for the
/// symbol faces whose cmap is keyed by the raw code, by the code.
pub fn glyph_in(face: &Face, s: Standard, ch: Option<char>, name: Option<&str>, code: u8) -> u16 {
    if let Some(c) = ch {
        let g = face.glyph(c);
        if g != 0 {
            return g;
        }
    }
    if let Some(g) = name.and_then(|n| face.glyph_named(n)).filter(|g| *g != 0) {
        return g;
    }
    if s.is_symbol() {
        return face.glyph_symbolic(code);
    }
    0
}

// AFM advance widths of printable ASCII (32 to 126; 39 is quotesingle and 96 grave, as in WinAnsi) for the
// standard faces. The oblique sans faces share the upright widths; Courier is 600 throughout.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space to /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, // 0 to @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, // A to M
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // N to Z
    278, 278, 278, 469, 556, 333, // [ to `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, // a to m
    556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // n to z
    334, 260, 334, 584, // { to ~
];
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, //
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, //
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, //
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, //
    333, 278, 333, 584, 556, 333, //
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, //
    611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, //
    389, 280, 389, 584,
];
const TIMES: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, //
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, //
    722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, //
    722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, //
    333, 278, 333, 469, 500, 333, //
    444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, //
    500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, //
    480, 200, 480, 541,
];
const TIMES_BOLD: [u16; 95] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, //
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 930, //
    722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, //
    722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, //
    333, 278, 333, 581, 500, 333, //
    500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, //
    556, 500, 556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, //
    394, 220, 394, 520,
];
const TIMES_ITALIC: [u16; 95] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278, //
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 675, 675, 675, 500, 920, //
    611, 611, 667, 722, 611, 611, 722, 722, 333, 444, 667, 556, 833, //
    667, 722, 611, 722, 611, 500, 556, 722, 611, 833, 611, 556, 556, //
    389, 278, 389, 422, 500, 333, //
    500, 500, 444, 500, 444, 278, 500, 500, 278, 278, 444, 278, 722, //
    500, 500, 500, 500, 389, 389, 278, 500, 444, 667, 444, 444, 389, //
    400, 275, 400, 541,
];
const TIMES_BOLD_ITALIC: [u16; 95] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278, //
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 832, //
    667, 667, 667, 722, 667, 667, 722, 778, 389, 500, 667, 611, 889, //
    722, 722, 611, 722, 667, 556, 611, 722, 667, 889, 667, 611, 611, //
    333, 278, 333, 570, 500, 333, //
    500, 500, 444, 500, 444, 333, 500, 556, 278, 278, 500, 278, 778, //
    556, 500, 500, 500, 389, 389, 278, 556, 444, 667, 500, 444, 389, //
    348, 220, 348, 570,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn std(f: Family, bold: bool, italic: bool) -> Standard {
        Standard { family: f, bold, italic }
    }

    #[test]
    fn the_fourteen_names_and_common_aliases_map_to_their_faces() {
        let n = |s: &str| Standard::from_name(s, 0, None, 0.0);
        assert_eq!(n("Helvetica"), std(Family::Sans, false, false));
        assert_eq!(n("Helvetica-BoldOblique"), std(Family::Sans, true, true));
        assert_eq!(n("Times-Roman"), std(Family::Serif, false, false));
        assert_eq!(n("Times-BoldItalic"), std(Family::Serif, true, true));
        assert_eq!(n("Courier-Oblique"), std(Family::Mono, false, true));
        assert_eq!(n("Symbol"), std(Family::Symbol, false, false));
        assert_eq!(n("ZapfDingbats"), std(Family::Dingbats, false, false));
        assert_eq!(n("Arial,Bold"), std(Family::Sans, true, false));
        assert_eq!(n("ABCDEF+TimesNewRomanPS-ItalicMT"), std(Family::Serif, false, true));
        assert_eq!(n("CourierNewPSMT"), std(Family::Mono, false, false));
        assert_eq!(n("MicrosoftSansSerif"), std(Family::Sans, false, false), "sans wins over serif");
        // an unknown name falls back to the descriptor flags
        assert_eq!(Standard::from_name("Foo", 2, Some(700.0), -12.0), std(Family::Serif, true, true));
        assert_eq!(Standard::from_name("Foo", 1, None, 0.0).family, Family::Mono);
    }

    #[test]
    fn afm_widths_lay_out_the_standard_faces() {
        let w = |s: Standard, t: &str| t.chars().map(|c| s.width(&c.to_string()).unwrap()).sum::<f32>();
        assert_eq!(w(Standard::SANS, "Hello"), 722.0 + 556.0 + 222.0 + 222.0 + 556.0);
        assert_eq!(w(std(Family::Sans, true, true), "Hi"), 722.0 + 278.0);
        assert_eq!(w(std(Family::Serif, false, false), "The"), 611.0 + 500.0 + 444.0);
        assert_eq!(w(std(Family::Mono, true, false), "anything"), 8.0 * 600.0);
        assert_eq!(Standard::SANS.width("\u{2019}"), Some(222.0));
        assert_eq!(Standard::SANS.width("é"), Some(556.0), "accented letters keep the base advance");
        assert_eq!(Standard::SANS.width("fi"), Some(500.0));
        assert_eq!(std(Family::Symbol, false, false).width("α"), None);
        assert_eq!(std(Family::Dingbats, false, false).default_width(), 788.0);
    }

    #[test]
    fn builtin_encodings_decode_to_unicode() {
        assert_eq!(standard_encoding(b'A'), Some('A'));
        assert_eq!(standard_encoding(0x27), Some('\u{2019}'));
        assert_eq!(standard_encoding(0x60), Some('\u{2018}'));
        assert_eq!(standard_encoding(0xae), Some('\u{fb01}'));
        assert_eq!(standard_encoding(0xd0), Some('\u{2014}'));
        assert_eq!(standard_encoding(0xfb), Some('ß'));
        assert_eq!(standard_encoding(0xb0), None);
        assert_eq!(symbol_encoding(b'a'), Some('α'));
        assert_eq!(symbol_encoding(b'W'), Some('Ω'));
        assert_eq!(symbol_encoding(0xe5), Some('∑'));
        assert_eq!(symbol_encoding(b'1'), Some('1'));
        assert_eq!(dingbats_encoding(0x21), Some('\u{2701}'));
        assert_eq!(dingbats_encoding(0x6c), Some('●'));
        assert_eq!(dingbats_encoding(0xac), Some('①'));
        assert_eq!(dingbats_encoding(0xd8), Some('\u{2798}'));
    }

    #[test]
    fn candidate_files_are_open_faces_best_match_first() {
        let f = std(Family::Serif, true, true).files();
        assert_eq!(f[0], "/usr/share/fonts/gsfonts/NimbusRoman-BoldItalic.otf");
        assert_eq!(f[1], "/usr/share/fonts/liberation/LiberationSerif-BoldItalic.ttf");
        assert!(f.iter().any(|p| p.ends_with("DejaVuSerif-BoldItalic.ttf")));
        assert!(Standard::SANS.files().iter().any(|p| p.ends_with("TTF/DejaVuSans.ttf")));
        assert!(std(Family::Mono, false, true).files().iter().any(|p| p.ends_with("DejaVuSansMono-Oblique.ttf")));
        assert_eq!(std(Family::Dingbats, false, false).files()[0], "/usr/share/fonts/gsfonts/D050000L.otf");
        let all: Vec<String> = [Family::Sans, Family::Serif, Family::Mono, Family::Symbol, Family::Dingbats]
            .iter()
            .flat_map(|f| std(*f, false, false).files())
            .collect();
        for vendor in ["Helvetica", "Arial", "Times New", "Courier"] {
            assert!(!all.iter().any(|p| p.contains(vendor)), "no vendor face is read: {vendor}");
        }
    }
}
