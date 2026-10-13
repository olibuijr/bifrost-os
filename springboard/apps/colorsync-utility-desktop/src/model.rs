//! The ICC profile model: the header, the tag table, the structural checks, the text of the description tags and the
//! gamut of an RGB matrix profile. Pure: bytes in, values out. `library.rs` finds the files, `window.rs` holds the state
//! and `draw.rs` paints the result.
//!
//! The layout follows the ICC specification (ICC.1, versions 2 and 4): a 128-byte header, a tag table made of a count
//! and 12-byte entries (signature, offset, size), then the tag data, each tag starting on a 4-byte boundary. Numbers are
//! big-endian. Colorant and white point values are s15Fixed16 numbers: signed 32-bit values divided by 65536.

/// The size of the profile header in bytes.
pub const HEADER_LEN: usize = 128;
/// The size of one tag table entry: signature, offset and size.
const TAG_ENTRY_LEN: usize = 12;
/// The D50 white point, in xy, that the profile connection space is adapted to. Used when a profile has no `wtpt` tag.
pub const D50_WHITE: (f64, f64) = (0.3457, 0.3585);
/// The area of the sRGB (Rec. 709) primaries on the xy chart. The gamut readout is a ratio to it. The colorants of a
/// profile are D50-adapted, so the ratio is an estimate, a few per cent off for a display profile.
pub const SRGB_AREA: f64 = 0.1121;

/// Device classes the specification defines, with the names the window shows.
pub const CLASSES: &[(&str, &str)] = &[
    ("scnr", "Input device"),
    ("mntr", "Display device"),
    ("prtr", "Output device"),
    ("link", "Device link"),
    ("spac", "Colour space"),
    ("abst", "Abstract"),
    ("nmcl", "Named colour"),
];

/// Colour spaces the specification defines, by signature. The n-colour spaces ('2CLR' to 'FCLR') are accepted by
/// `is_known_space` as well.
const SPACES: &[(&str, &str)] = &[
    ("XYZ ", "XYZ"),
    ("Lab ", "L*a*b*"),
    ("Luv ", "L*u*v*"),
    ("YCbr", "YCbCr"),
    ("Yxy ", "Yxy"),
    ("RGB ", "RGB"),
    ("GRAY", "Greyscale"),
    ("HSV ", "HSV"),
    ("HLS ", "HLS"),
    ("CMYK", "CMYK"),
    ("CMY ", "CMY"),
];

/// Tags the specification names, with the names the tag list shows.
const TAG_NAMES: &[(&str, &str)] = &[
    ("A2B0", "Device to PCS, perceptual"),
    ("A2B1", "Device to PCS, relative colorimetric"),
    ("A2B2", "Device to PCS, saturation"),
    ("B2A0", "PCS to device, perceptual"),
    ("B2A1", "PCS to device, relative colorimetric"),
    ("B2A2", "PCS to device, saturation"),
    ("bXYZ", "Blue colorant"),
    ("bTRC", "Blue tone curve"),
    ("gXYZ", "Green colorant"),
    ("gTRC", "Green tone curve"),
    ("rXYZ", "Red colorant"),
    ("rTRC", "Red tone curve"),
    ("kTRC", "Grey tone curve"),
    ("wtpt", "Media white point"),
    ("bkpt", "Media black point"),
    ("chad", "Chromatic adaptation"),
    ("desc", "Profile description"),
    ("cprt", "Copyright"),
    ("dmnd", "Device manufacturer"),
    ("dmdd", "Device model"),
    ("lumi", "Luminance"),
    ("meas", "Measurement conditions"),
    ("tech", "Technology"),
    ("vcgt", "Video card gamma"),
    ("view", "Viewing conditions"),
    ("ndin", "Native display information"),
    ("gamt", "Gamut"),
];

/// The colorant tags of a matrix profile, which carry the primaries.
const COLORANT_TAGS: [&str; 3] = ["rXYZ", "gXYZ", "bXYZ"];
/// The XYZ tags and the curve tags, whose data types the checks read.
const XYZ_TAGS: [&str; 4] = ["rXYZ", "gXYZ", "bXYZ", "wtpt"];
const CURVE_TAGS: [&str; 4] = ["rTRC", "gTRC", "bTRC", "kTRC"];
/// The tags a matrix/curve RGB profile must carry.
const MATRIX_TAGS: [&str; 7] = ["rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC", "wtpt"];

/// The spectral locus of the CIE 1931 2 degree observer, every 10 nm from 380 to 700 nm, as chromaticity (x, y).
pub const SPECTRAL_LOCUS: &[(f64, f64)] = &[
    (0.1741, 0.0050),
    (0.1740, 0.0050),
    (0.1733, 0.0048),
    (0.1726, 0.0048),
    (0.1714, 0.0051),
    (0.1689, 0.0069),
    (0.1644, 0.0109),
    (0.1566, 0.0177),
    (0.1440, 0.0297),
    (0.1241, 0.0578),
    (0.0913, 0.1327),
    (0.0454, 0.2950),
    (0.0082, 0.5384),
    (0.0139, 0.7502),
    (0.0743, 0.8338),
    (0.1547, 0.8059),
    (0.2296, 0.7543),
    (0.3016, 0.6923),
    (0.3731, 0.6245),
    (0.4441, 0.5547),
    (0.5125, 0.4866),
    (0.5752, 0.4242),
    (0.6270, 0.3725),
    (0.6658, 0.3340),
    (0.6915, 0.3083),
    (0.7079, 0.2920),
    (0.7190, 0.2810),
    (0.7258, 0.2742),
    (0.7300, 0.2700),
    (0.7320, 0.2680),
    (0.7334, 0.2666),
    (0.7344, 0.2656),
    (0.7347, 0.2653),
];

/// The header of a profile, read as the specification lays it out.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    /// The size the header gives for the whole profile, in bytes.
    pub size: u32,
    /// The preferred CMM signature.
    pub cmm: String,
    /// The major and the minor (high nibble) version bytes.
    pub version: (u8, u8),
    /// The device class signature, such as 'mntr'.
    pub class: String,
    /// The colour space of the data, such as 'RGB '.
    pub space: String,
    /// The profile connection space: 'XYZ ' or 'Lab '.
    pub pcs: String,
    /// Year, month, day, hour, minute and second of creation.
    pub created: [u16; 6],
    pub platform: String,
    pub flags: u32,
    pub manufacturer: String,
    pub model: String,
    pub rendering_intent: u32,
    /// The PCS illuminant as XYZ.
    pub illuminant: [f64; 3],
    pub creator: String,
    /// The profile identifier (an MD5 digest in version 4); all zero when not set.
    pub id: [u8; 16],
}

impl Header {
    /// Reads the header from the first 128 bytes of `b`. None when the bytes are too short.
    pub fn parse(b: &[u8]) -> Option<Header> {
        if b.len() < HEADER_LEN {
            return None;
        }
        let mut id = [0u8; 16];
        id.copy_from_slice(b.get(84..100)?);
        Some(Header {
            size: u32_at(b, 0)?,
            cmm: sig_at(b, 4)?,
            version: (*b.get(8)?, *b.get(9)?),
            class: sig_at(b, 12)?,
            space: sig_at(b, 16)?,
            pcs: sig_at(b, 20)?,
            created: [u16_at(b, 24)?, u16_at(b, 26)?, u16_at(b, 28)?, u16_at(b, 30)?, u16_at(b, 32)?, u16_at(b, 34)?],
            platform: sig_at(b, 40)?,
            flags: u32_at(b, 44)?,
            manufacturer: sig_at(b, 48)?,
            model: sig_at(b, 52)?,
            rendering_intent: u32_at(b, 64)?,
            illuminant: [s15_at(b, 68)?, s15_at(b, 72)?, s15_at(b, 76)?],
            creator: sig_at(b, 80)?,
            id,
        })
    }

    /// The version as the header gives it, such as "2.1" or "4.3".
    pub fn version_text(&self) -> String {
        format!("{}.{}", self.version.0, self.version.1 >> 4)
    }
}

/// One entry of the tag table, with the type of its data when the data lies inside the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub sig: String,
    pub offset: u32,
    pub size: u32,
    /// The type signature at the start of the tag data, such as 'XYZ ' or 'curv'. Empty when the data is outside the file.
    pub kind: String,
    /// False when the offset and size run past the end of the file.
    pub in_bounds: bool,
}

/// A profile: its header, its tag table and its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub header: Header,
    pub tags: Vec<Tag>,
    data: Vec<u8>,
}

/// The chromaticities of the three primaries and the white point of an RGB matrix profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gamut {
    pub red: (f64, f64),
    pub green: (f64, f64),
    pub blue: (f64, f64),
    pub white: (f64, f64),
}

impl Gamut {
    /// The area of the triangle the primaries make on the xy chart (the shoelace formula).
    pub fn area(&self) -> f64 {
        let (r, g, b) = (self.red, self.green, self.blue);
        let twice = r.0 * g.1 - g.0 * r.1 + g.0 * b.1 - b.0 * g.1 + b.0 * r.1 - r.0 * b.1;
        twice.abs() / 2.0
    }
}

/// How serious a finding is. An error means a CMM may refuse or misread the profile; a warning means it is unusual or
/// incomplete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
        }
    }
}

/// One finding of the verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
}

impl Profile {
    /// Reads a profile from its bytes. Fails only when the header is missing; anything wrong further in is found by
    /// `verify`.
    pub fn parse(data: Vec<u8>) -> Result<Profile, String> {
        let header = Header::parse(&data)
            .ok_or_else(|| format!("the file is {} bytes, shorter than the {HEADER_LEN}-byte ICC header", data.len()))?;
        let tags = read_tags(&data);
        Ok(Profile { header, tags, data })
    }

    /// The first tag table entry with signature `sig`.
    pub fn tag(&self, sig: &str) -> Option<&Tag> {
        self.tags.iter().find(|t| t.sig == sig)
    }

    /// The bytes of a tag's data. None when the tag lies outside the file.
    pub fn tag_data(&self, tag: &Tag) -> Option<&[u8]> {
        if !tag.in_bounds {
            return None;
        }
        let start = tag.offset as usize;
        self.data.get(start..start + tag.size as usize)
    }

    /// An XYZ tag as three numbers.
    fn xyz(&self, sig: &str) -> Option<[f64; 3]> {
        let tag = self.tag(sig)?;
        if tag.kind != "XYZ " {
            return None;
        }
        let d = self.tag_data(tag)?;
        Some([s15_at(d, 8)?, s15_at(d, 12)?, s15_at(d, 16)?])
    }

    /// The text of a text tag: 'desc' (version 2), 'mluc' (version 4, the first translation) or 'text'.
    pub fn text(&self, sig: &str) -> Option<String> {
        let tag = self.tag(sig)?;
        let d = self.tag_data(tag)?;
        match tag.kind.as_str() {
            "desc" => Some(ascii(d.get(12..)?)),
            "text" => Some(ascii(d.get(8..)?)),
            "mluc" => {
                if u32_at(d, 8)? == 0 {
                    return None;
                }
                let len = u32_at(d, 20)? as usize;
                let off = u32_at(d, 24)? as usize;
                let raw = d.get(off..off.checked_add(len)?)?;
                let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
            }
            _ => None,
        }
    }

    /// The profile description, the name the list shows.
    pub fn title(&self) -> Option<String> {
        self.text("desc").filter(|s| !s.is_empty())
    }

    /// The primaries and white point of an RGB profile that carries its colorants. None for other colour spaces and
    /// for profiles whose colorant tags are missing or unreadable (LUT-based profiles, for one).
    pub fn gamut(&self) -> Option<Gamut> {
        if self.header.space != "RGB " {
            return None;
        }
        let red = xy(self.xyz("rXYZ")?)?;
        let green = xy(self.xyz("gXYZ")?)?;
        let blue = xy(self.xyz("bXYZ")?)?;
        let white = self.xyz("wtpt").and_then(xy).unwrap_or(D50_WHITE);
        Some(Gamut { red, green, blue, white })
    }

    /// The structural checks: the header, the tag table, each tag's bounds, alignment and type, and the tags a profile
    /// of its class and colour space must carry. An empty list means no finding.
    pub fn verify(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let d = &self.data;
        let h = &self.header;
        let len = d.len() as u64;

        if u64::from(h.size) > len {
            add(&mut out, Severity::Error, format!("the header gives {} bytes but the file has {}", h.size, len));
        } else if u64::from(h.size) != len {
            add(&mut out, Severity::Warning, format!("the header gives {} bytes but the file has {}", h.size, len));
        }
        if d.get(36..40) != Some(&b"acsp"[..]) {
            add(&mut out, Severity::Error, "the signature 'acsp' is missing from the header");
        }
        if h.version.0 == 0 || h.version.0 > 5 {
            add(&mut out, Severity::Error, format!("the version {} is not defined", h.version_text()));
        } else if h.version.0 == 5 {
            add(&mut out, Severity::Warning, "a version 5 (iccMAX) profile is not read by every colour engine");
        }
        if name_in(CLASSES, &h.class).is_none() {
            add(&mut out, Severity::Error, format!("the device class '{}' is not defined", h.class.trim_end()));
        }
        if !is_known_space(&h.space) {
            add(&mut out, Severity::Error, format!("the colour space '{}' is not defined", h.space.trim_end()));
        }
        if h.pcs != "XYZ " && h.pcs != "Lab " {
            add(&mut out, Severity::Error, format!("the connection space '{}' is neither XYZ nor Lab", h.pcs.trim_end()));
        }

        let Some(count) = u32_at(d, HEADER_LEN) else {
            add(&mut out, Severity::Error, "the tag table is missing");
            return out;
        };
        let table_end = HEADER_LEN as u64 + 4 + u64::from(count) * TAG_ENTRY_LEN as u64;
        if table_end > len {
            add(&mut out, Severity::Error, format!("the tag table of {count} entries runs past the end of the file"));
            return out;
        }

        let mut seen: Vec<&str> = Vec::new();
        for t in &self.tags {
            if !t.in_bounds {
                add(&mut out, Severity::Error, format!("the tag '{}' lies outside the file", t.sig));
            } else if t.size < 8 {
                add(&mut out, Severity::Error, format!("the tag '{}' is {} bytes; a tag is at least 8", t.sig, t.size));
            }
            if t.offset % 4 != 0 {
                add(&mut out, Severity::Warning, format!("the tag '{}' is not on a 4-byte boundary", t.sig));
            }
            if u64::from(t.offset) < table_end {
                add(&mut out, Severity::Error, format!("the tag '{}' overlaps the tag table", t.sig));
            }
            if seen.contains(&t.sig.as_str()) {
                add(&mut out, Severity::Error, format!("the tag '{}' appears more than once", t.sig));
            }
            seen.push(&t.sig);
        }

        let has = |sig: &str| self.tag(sig).is_some();
        let device = matches!(h.class.as_str(), "mntr" | "scnr" | "prtr");
        if device && h.space == "RGB " && !has("A2B0") {
            for sig in MATRIX_TAGS {
                if !has(sig) {
                    add(&mut out, Severity::Error, format!("the matrix profile has no '{sig}' ({}) tag", tag_name(sig)));
                }
            }
        }
        if device && h.space == "GRAY" && !has("kTRC") {
            add(&mut out, Severity::Error, "the greyscale profile has no 'kTRC' (Grey tone curve) tag");
        }
        if device && is_known_space(&h.space) && h.space != "RGB " && h.space != "GRAY" && !has("A2B0") {
            add(&mut out, Severity::Error, format!("the {} profile has no 'A2B0' device to PCS table", h.space.trim_end()));
        }
        if !has("desc") {
            add(&mut out, Severity::Warning, "the profile has no description ('desc') tag");
        }
        if !has("cprt") {
            add(&mut out, Severity::Warning, "the profile has no copyright ('cprt') tag");
        }

        for t in self.tags.iter().filter(|t| t.in_bounds) {
            if XYZ_TAGS.contains(&t.sig.as_str()) && t.kind != "XYZ " {
                add(&mut out, Severity::Error, format!("the tag '{}' should hold XYZ data, not '{}'", t.sig, t.kind.trim_end()));
            }
            if CURVE_TAGS.contains(&t.sig.as_str()) && t.kind != "curv" && t.kind != "para" {
                add(&mut out, Severity::Error, format!("the tag '{}' should hold a curve, not '{}'", t.sig, t.kind.trim_end()));
            }
        }
        for sig in COLORANT_TAGS {
            if let Some(v) = self.xyz(sig) {
                if xy(v).is_none() {
                    add(&mut out, Severity::Warning, format!("the colorant '{sig}' has no chromaticity (its XYZ sums to zero)"));
                }
            }
        }
        out
    }
}

/// The rows the header panel shows, in order.
pub fn header_rows(p: &Profile) -> Vec<(&'static str, String)> {
    let h = &p.header;
    let created = h.created;
    let id = if h.id.iter().all(|&b| b == 0) {
        "Not set".to_string()
    } else {
        h.id.iter().map(|b| format!("{b:02x}")).collect()
    };
    let intent = match h.rendering_intent {
        0 => "Perceptual",
        1 => "Media-relative colorimetric",
        2 => "Saturation",
        3 => "ICC-absolute colorimetric",
        _ => "Unknown",
    };
    let [x, y, z] = h.illuminant;
    vec![
        ("Size", format!("{} bytes", h.size)),
        ("Version", h.version_text()),
        ("Device class", format!("{} ({})", class_name(&h.class), h.class.trim_end())),
        ("Colour space", format!("{} ({})", space_name(&h.space), h.space.trim_end())),
        ("Connection space", h.pcs.trim_end().to_string()),
        (
            "Created",
            format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", created[0], created[1], created[2], created[3], created[4], created[5]),
        ),
        ("CMM", h.cmm.trim_end().to_string()),
        ("Platform", h.platform.trim_end().to_string()),
        ("Flags", format!("{:#010x}", h.flags)),
        ("Manufacturer", h.manufacturer.trim_end().to_string()),
        ("Model", h.model.trim_end().to_string()),
        ("Creator", h.creator.trim_end().to_string()),
        ("Rendering intent", intent.to_string()),
        ("PCS illuminant", format!("X {x:.4}, Y {y:.4}, Z {z:.4}")),
        ("Profile ID", id),
        ("Description", p.title().unwrap_or_else(|| "None".to_string())),
    ]
}

/// The name of a device class signature, or "Unknown class".
pub fn class_name(sig: &str) -> &'static str {
    name_in(CLASSES, sig).unwrap_or("Unknown class")
}

/// The name of a colour space signature.
pub fn space_name(sig: &str) -> &'static str {
    name_in(SPACES, sig).unwrap_or(if is_n_colour(sig) { "n-colour" } else { "Unknown space" })
}

/// The name of a tag the specification defines, or "Private or unnamed tag".
pub fn tag_name(sig: &str) -> &'static str {
    name_in(TAG_NAMES, sig).unwrap_or("Private or unnamed tag")
}

/// True when `sig` is a colour space the specification defines, or an n-colour space ('2CLR' to 'FCLR').
pub fn is_known_space(sig: &str) -> bool {
    name_in(SPACES, sig).is_some() || is_n_colour(sig)
}

/// The chromaticity (x, y) of an XYZ value; None when the XYZ sums to zero.
pub fn xy(xyz: [f64; 3]) -> Option<(f64, f64)> {
    let [x, y, z] = xyz;
    let sum = x + y + z;
    (sum.abs() > 1e-9).then(|| (x / sum, y / sum))
}

fn is_n_colour(sig: &str) -> bool {
    let first = sig.as_bytes().first().copied().unwrap_or(0);
    sig.len() == 4 && sig.ends_with("CLR") && ((b'2'..=b'9').contains(&first) || (b'A'..=b'F').contains(&first))
}

fn name_in(table: &[(&'static str, &'static str)], sig: &str) -> Option<&'static str> {
    table.iter().find(|(s, _)| *s == sig).map(|(_, name)| *name)
}

fn add(out: &mut Vec<Issue>, severity: Severity, message: impl Into<String>) {
    out.push(Issue { severity, message: message.into() });
}

fn read_tags(b: &[u8]) -> Vec<Tag> {
    let Some(count) = u32_at(b, HEADER_LEN) else {
        return Vec::new();
    };
    let mut tags = Vec::new();
    for i in 0..count as usize {
        let at = HEADER_LEN + 4 + i * TAG_ENTRY_LEN;
        let (Some(sig), Some(offset), Some(size)) = (sig_at(b, at), u32_at(b, at + 4), u32_at(b, at + 8)) else {
            break;
        };
        let in_bounds = u64::from(offset) + u64::from(size) <= b.len() as u64;
        let kind = if in_bounds && size >= 4 { sig_at(b, offset as usize).unwrap_or_default() } else { String::new() };
        tags.push(Tag { sig, offset, size, kind, in_bounds });
    }
    tags
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(s))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(s))
}

fn s15_at(b: &[u8], at: usize) -> Option<f64> {
    Some(u32_at(b, at)? as i32 as f64 / 65536.0)
}

fn sig_at(b: &[u8], at: usize) -> Option<String> {
    Some(String::from_utf8_lossy(b.get(at..at + 4)?).into_owned())
}

fn ascii(s: &[u8]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn s15(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }

    fn xyz(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut v = b"XYZ \0\0\0\0".to_vec();
        v.extend_from_slice(&s15(x));
        v.extend_from_slice(&s15(y));
        v.extend_from_slice(&s15(z));
        v
    }

    /// A curve of one entry: gamma 2.2 as an unsigned 8.8 fixed point number.
    fn curv_gamma() -> Vec<u8> {
        let mut v = b"curv\0\0\0\0".to_vec();
        v.extend_from_slice(&be32(1));
        v.extend_from_slice(&[0x02, 0x33]);
        v
    }

    fn text_tag(s: &str) -> Vec<u8> {
        let mut v = b"text\0\0\0\0".to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    fn desc_tag(s: &str) -> Vec<u8> {
        let mut v = b"desc\0\0\0\0".to_vec();
        v.extend_from_slice(&be32(s.len() as u32 + 1));
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// Builds a profile file: a header for `class` and `space`, the tag table, then the tag data, each tag on a
    /// 4-byte boundary.
    fn build(class: &str, space: &str, tags: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let table_len = 4 + 12 * tags.len();
        let mut header = vec![0u8; HEADER_LEN];
        header[8] = 2;
        header[9] = 0x10;
        header[12..16].copy_from_slice(class.as_bytes());
        header[16..20].copy_from_slice(space.as_bytes());
        header[20..24].copy_from_slice(b"XYZ ");
        header[36..40].copy_from_slice(b"acsp");
        let mut table = be32(tags.len() as u32).to_vec();
        let mut body = Vec::new();
        for (sig, bytes) in tags {
            while (HEADER_LEN + table_len + body.len()) % 4 != 0 {
                body.push(0);
            }
            let offset = (HEADER_LEN + table_len + body.len()) as u32;
            table.extend_from_slice(sig.as_bytes());
            table.extend_from_slice(&be32(offset));
            table.extend_from_slice(&be32(bytes.len() as u32));
            body.extend_from_slice(bytes);
        }
        let mut file = header;
        file.extend_from_slice(&table);
        file.extend_from_slice(&body);
        let size = file.len() as u32;
        file[0..4].copy_from_slice(&be32(size));
        file
    }

    fn srgb_tags() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("desc", desc_tag("sRGB test")),
            ("cprt", text_tag("No rights")),
            ("wtpt", xyz(0.9642, 1.0, 0.8249)),
            ("rXYZ", xyz(0.4361, 0.2225, 0.0139)),
            ("gXYZ", xyz(0.3851, 0.7169, 0.0971)),
            ("bXYZ", xyz(0.1431, 0.0606, 0.7141)),
            ("rTRC", curv_gamma()),
            ("gTRC", curv_gamma()),
            ("bTRC", curv_gamma()),
        ]
    }

    fn has_error(issues: &[Issue], needle: &str) -> bool {
        issues.iter().any(|i| i.severity == Severity::Error && i.message.contains(needle))
    }

    #[test]
    fn header_fields_are_read() {
        let file = build("mntr", "RGB ", &srgb_tags());
        let p = Profile::parse(file.clone()).unwrap();
        assert_eq!(p.header.class, "mntr");
        assert_eq!(p.header.space, "RGB ");
        assert_eq!(p.header.pcs, "XYZ ");
        assert_eq!(p.header.version_text(), "2.1");
        assert_eq!(p.header.size as usize, file.len());
        assert_eq!(p.data.len(), file.len());
    }

    #[test]
    fn short_file_is_rejected() {
        assert!(Profile::parse(vec![0u8; 100]).is_err());
    }

    #[test]
    fn clean_profile_has_no_findings() {
        let p = Profile::parse(build("mntr", "RGB ", &srgb_tags())).unwrap();
        assert!(p.verify().is_empty(), "{:?}", p.verify());
    }

    #[test]
    fn description_is_read() {
        let p = Profile::parse(build("mntr", "RGB ", &srgb_tags())).unwrap();
        assert_eq!(p.title().as_deref(), Some("sRGB test"));
    }

    #[test]
    fn multi_language_description_is_read() {
        let mut mluc = b"mluc\0\0\0\0".to_vec();
        mluc.extend_from_slice(&be32(1));
        mluc.extend_from_slice(&be32(12));
        mluc.extend_from_slice(b"enUS");
        mluc.extend_from_slice(&be32(8));
        mluc.extend_from_slice(&be32(28));
        for unit in "Test".encode_utf16() {
            mluc.extend_from_slice(&unit.to_be_bytes());
        }
        let mut tags = srgb_tags();
        tags[0] = ("desc", mluc);
        let p = Profile::parse(build("mntr", "RGB ", &tags)).unwrap();
        assert_eq!(p.title().as_deref(), Some("Test"));
    }

    #[test]
    fn gamut_of_matrix_profile_is_read() {
        let p = Profile::parse(build("mntr", "RGB ", &srgb_tags())).unwrap();
        let g = p.gamut().expect("a matrix profile has a gamut");
        assert!((g.red.0 - 0.6484).abs() < 1e-3, "red x {}", g.red.0);
        assert!((g.red.1 - 0.3309).abs() < 1e-3, "red y {}", g.red.1);
        let area = g.area();
        assert!(area > 0.105 && area < 0.115, "area {area}");
    }

    #[test]
    fn gamut_white_defaults_to_d50() {
        let tags: Vec<(&str, Vec<u8>)> = srgb_tags().into_iter().filter(|(s, _)| *s != "wtpt").collect();
        let p = Profile::parse(build("mntr", "RGB ", &tags)).unwrap();
        assert_eq!(p.gamut().unwrap().white, D50_WHITE);
    }

    #[test]
    fn lut_profile_has_no_gamut_and_no_errors() {
        let tags = vec![
            ("desc", desc_tag("Printer")),
            ("cprt", text_tag("No rights")),
            ("A2B0", b"mft2\0\0\0\0".to_vec()),
        ];
        let p = Profile::parse(build("prtr", "CMYK", &tags)).unwrap();
        assert!(p.gamut().is_none());
        assert!(p.verify().iter().all(|i| i.severity != Severity::Error), "{:?}", p.verify());
    }

    #[test]
    fn missing_curve_is_an_error() {
        let tags: Vec<(&str, Vec<u8>)> = srgb_tags().into_iter().filter(|(s, _)| *s != "rTRC").collect();
        let p = Profile::parse(build("mntr", "RGB ", &tags)).unwrap();
        assert!(has_error(&p.verify(), "'rTRC'"), "{:?}", p.verify());
    }

    #[test]
    fn bad_signature_is_an_error() {
        let mut file = build("mntr", "RGB ", &srgb_tags());
        file[36..40].copy_from_slice(b"xxxx");
        let p = Profile::parse(file).unwrap();
        assert!(has_error(&p.verify(), "acsp"));
    }

    #[test]
    fn size_mismatch_is_a_warning() {
        let mut file = build("mntr", "RGB ", &srgb_tags());
        file.push(0);
        let p = Profile::parse(file).unwrap();
        let issues = p.verify();
        assert!(issues.iter().any(|i| i.severity == Severity::Warning && i.message.contains("bytes")));
        assert!(!issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn truncated_tag_is_outside_the_file() {
        let mut file = build("mntr", "RGB ", &srgb_tags());
        let cut = file.len() - 10;
        file.truncate(cut);
        let p = Profile::parse(file).unwrap();
        assert!(p.tags.iter().any(|t| !t.in_bounds));
        assert!(has_error(&p.verify(), "lies outside the file"));
    }

    #[test]
    fn duplicate_tag_is_an_error() {
        let mut tags = srgb_tags();
        tags.push(("cprt", text_tag("Again")));
        let p = Profile::parse(build("mntr", "RGB ", &tags)).unwrap();
        assert!(has_error(&p.verify(), "more than once"));
    }

    #[test]
    fn colorant_with_wrong_type_is_an_error() {
        let mut tags = srgb_tags();
        tags[3] = ("rXYZ", text_tag("not xyz"));
        let p = Profile::parse(build("mntr", "RGB ", &tags)).unwrap();
        assert!(has_error(&p.verify(), "should hold XYZ data"));
    }

    #[test]
    fn xy_of_zero_is_none() {
        assert_eq!(xy([0.0, 0.0, 0.0]), None);
        assert_eq!(xy([0.5, 0.5, 0.0]), Some((0.5, 0.5)));
    }

    #[test]
    fn gamut_area_of_rec709_primaries() {
        let g = Gamut { red: (0.64, 0.33), green: (0.30, 0.60), blue: (0.15, 0.06), white: D50_WHITE };
        assert!((g.area() - 0.11205).abs() < 1e-4, "area {}", g.area());
    }

    #[test]
    fn spectral_locus_is_bounded() {
        assert_eq!(SPECTRAL_LOCUS.len(), 33);
        assert_eq!(SPECTRAL_LOCUS[0], (0.1741, 0.0050));
        assert!(SPECTRAL_LOCUS.iter().all(|&(x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)));
    }

    #[test]
    fn names_follow_the_signatures() {
        assert_eq!(class_name("mntr"), "Display device");
        assert_eq!(class_name("zzzz"), "Unknown class");
        assert_eq!(space_name("RGB "), "RGB");
        assert_eq!(space_name("2CLR"), "n-colour");
        assert_eq!(tag_name("wtpt"), "Media white point");
        assert!(is_known_space("GRAY"));
        assert!(!is_known_space("ABCD"));
    }

    #[test]
    fn header_rows_name_the_profile() {
        let p = Profile::parse(build("mntr", "RGB ", &srgb_tags())).unwrap();
        let rows = header_rows(&p);
        assert!(rows.iter().any(|(k, v)| *k == "Device class" && v == "Display device (mntr)"));
        assert!(rows.iter().any(|(k, v)| *k == "Description" && v == "sRGB test"));
        assert!(rows.iter().any(|(k, v)| *k == "Profile ID" && v == "Not set"));
    }
}
