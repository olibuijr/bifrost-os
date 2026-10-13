//! Display information: the list of displays an app can read (active and online), the main display, bounds and pixel
//! size, modes, refresh rate, rotation, mirroring, the built-in flag, vendor / model / serial identity, and the
//! changes between two lists (what a reconfiguration callback reports). The shell fills the list from the output it
//! drives (see `springboard/src/shell/services/display_services.rs`); this module is the model and the wire form, so the
//! shell and the SDK client agree on it. Nothing here reads hardware.

use crate::wire_json::*;
use crate::Rect;
use bifrost_core::json::Json;

/// The identifier of one display. Stable while the display stays connected.
pub type DisplayId = u32;

/// One mode a display can run: logical size in points, pixel size and refresh rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Hertz; 0 when the rate is not known.
    pub refresh_hz: f32,
    /// Whether the mode can be used to drive the display.
    pub usable: bool,
}

impl DisplayMode {
    /// A mode whose logical size equals its pixel size.
    pub fn pixels(width: u32, height: u32, refresh_hz: f32) -> Self {
        DisplayMode { width, height, pixel_width: width, pixel_height: height, refresh_hz, usable: true }
    }

    /// Pixels per point.
    pub fn scale(&self) -> f32 {
        if self.width == 0 {
            1.0
        } else {
            self.pixel_width as f32 / self.width as f32
        }
    }

    pub fn to_json(&self) -> Json {
        obj(vec![
            ("width", num(self.width)),
            ("height", num(self.height)),
            ("pixel_width", num(self.pixel_width)),
            ("pixel_height", num(self.pixel_height)),
            ("refresh_hz", num(self.refresh_hz)),
            ("usable", Json::Bool(self.usable)),
        ])
    }

    pub fn from_json(j: &Json) -> Option<DisplayMode> {
        Some(DisplayMode {
            width: u32_of(j, "width")?,
            height: u32_of(j, "height")?,
            pixel_width: u32_of(j, "pixel_width")?,
            pixel_height: u32_of(j, "pixel_height")?,
            refresh_hz: f32_of(j, "refresh_hz").unwrap_or(0.0),
            usable: bool_of(j, "usable").unwrap_or(true),
        })
    }
}

/// A clockwise quarter-turn rotation of a display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Rotation {
    pub fn degrees(self) -> u32 {
        match self {
            Rotation::Deg0 => 0,
            Rotation::Deg90 => 90,
            Rotation::Deg180 => 180,
            Rotation::Deg270 => 270,
        }
    }

    /// `None` for anything but 0, 90, 180 or 270.
    pub fn from_degrees(d: u32) -> Option<Rotation> {
        match d {
            0 => Some(Rotation::Deg0),
            90 => Some(Rotation::Deg90),
            180 => Some(Rotation::Deg180),
            270 => Some(Rotation::Deg270),
            _ => None,
        }
    }
}

/// Everything known about one display.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub name: String,
    pub is_main: bool,
    pub is_builtin: bool,
    /// The display is part of the desktop (a mirror target that is hidden behind its primary is not).
    pub is_active: bool,
    /// The display is connected.
    pub is_online: bool,
    pub is_asleep: bool,
    /// Bounds in global points.
    pub bounds: Rect,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub mode: DisplayMode,
    pub modes: Vec<DisplayMode>,
    pub rotation: Rotation,
    /// The display this one mirrors, when it does.
    pub mirror_of: Option<DisplayId>,
    /// Manufacturer code as the two EDID bytes (three packed letters), 0 when unknown.
    pub vendor: u32,
    pub model: u32,
    pub serial: u32,
    /// Physical size in millimetres, (0, 0) when unknown.
    pub size_mm: (f32, f32),
}

impl DisplayInfo {
    /// The refresh rate of the current mode in hertz.
    pub fn refresh_hz(&self) -> f32 {
        self.mode.refresh_hz
    }

    pub fn is_mirrored(&self) -> bool {
        self.mirror_of.is_some()
    }

    /// The vendor as its three-letter code ("DEL", "SAM"), or an empty string when unknown.
    pub fn vendor_code(&self) -> String {
        vendor_letters(self.vendor as u16)
    }

    pub fn to_json(&self) -> Json {
        obj(vec![
            ("id", num(self.id)),
            ("name", text(&self.name)),
            ("main", Json::Bool(self.is_main)),
            ("builtin", Json::Bool(self.is_builtin)),
            ("active", Json::Bool(self.is_active)),
            ("online", Json::Bool(self.is_online)),
            ("asleep", Json::Bool(self.is_asleep)),
            ("bounds", rect_json(self.bounds)),
            ("pixel_width", num(self.pixel_width)),
            ("pixel_height", num(self.pixel_height)),
            ("mode", self.mode.to_json()),
            ("modes", Json::Arr(self.modes.iter().map(DisplayMode::to_json).collect())),
            ("rotation", num(self.rotation.degrees())),
            ("mirror_of", self.mirror_of.map_or(Json::Null, |m| num(m))),
            ("vendor", num(self.vendor)),
            ("model", num(self.model)),
            ("serial", num(self.serial)),
            ("width_mm", num(self.size_mm.0)),
            ("height_mm", num(self.size_mm.1)),
        ])
    }

    pub fn from_json(j: &Json) -> Option<DisplayInfo> {
        let modes = j.get("modes").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(DisplayMode::from_json).collect()).unwrap_or_default();
        Some(DisplayInfo {
            id: u32_of(j, "id")?,
            name: str_of(j, "name").unwrap_or_default(),
            is_main: bool_of(j, "main").unwrap_or(false),
            is_builtin: bool_of(j, "builtin").unwrap_or(false),
            is_active: bool_of(j, "active").unwrap_or(true),
            is_online: bool_of(j, "online").unwrap_or(true),
            is_asleep: bool_of(j, "asleep").unwrap_or(false),
            bounds: rect_of(j, "bounds")?,
            pixel_width: u32_of(j, "pixel_width")?,
            pixel_height: u32_of(j, "pixel_height")?,
            mode: DisplayMode::from_json(j.get("mode")?)?,
            modes,
            rotation: u32_of(j, "rotation").and_then(Rotation::from_degrees).unwrap_or_default(),
            mirror_of: u32_of(j, "mirror_of"),
            vendor: u32_of(j, "vendor").unwrap_or(0),
            model: u32_of(j, "model").unwrap_or(0),
            serial: u32_of(j, "serial").unwrap_or(0),
            size_mm: (f32_of(j, "width_mm").unwrap_or(0.0), f32_of(j, "height_mm").unwrap_or(0.0)),
        })
    }
}

/// The wire form of a display list.
pub fn list_to_json(list: &[DisplayInfo]) -> Json {
    Json::Arr(list.iter().map(DisplayInfo::to_json).collect())
}

/// Decodes a display list; entries that do not parse are skipped.
pub fn list_from_json(j: &Json) -> Vec<DisplayInfo> {
    j.as_array().map(|a| a.iter().filter_map(DisplayInfo::from_json).collect()).unwrap_or_default()
}

/// The displays that are part of the desktop.
pub fn active(list: &[DisplayInfo]) -> Vec<&DisplayInfo> {
    list.iter().filter(|d| d.is_online && d.is_active).collect()
}

/// The displays that are connected, active or not.
pub fn online(list: &[DisplayInfo]) -> Vec<&DisplayInfo> {
    list.iter().filter(|d| d.is_online).collect()
}

/// The main display: the one flagged main, else the first online one.
pub fn main_display(list: &[DisplayInfo]) -> Option<&DisplayInfo> {
    list.iter().find(|d| d.is_main && d.is_online).or_else(|| list.iter().find(|d| d.is_online))
}

/// The display that has this id.
pub fn by_id(list: &[DisplayInfo], id: DisplayId) -> Option<&DisplayInfo> {
    list.iter().find(|d| d.id == id)
}

/// The display whose bounds contain the point.
pub fn at_point(list: &[DisplayInfo], x: f32, y: f32) -> Option<&DisplayInfo> {
    active(list).into_iter().find(|d| d.bounds.contains(x, y))
}

// ---- reconfiguration ----

/// What changed about one display between two lists. A bit set; the bits are part of the wire form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChangeFlags(pub u32);

impl ChangeFlags {
    pub const MOVED: u32 = 1 << 0;
    pub const SET_MAIN: u32 = 1 << 1;
    pub const SET_MODE: u32 = 1 << 2;
    pub const ADDED: u32 = 1 << 3;
    pub const REMOVED: u32 = 1 << 4;
    pub const ENABLED: u32 = 1 << 5;
    pub const DISABLED: u32 = 1 << 6;
    pub const MIRRORED: u32 = 1 << 7;
    pub const UNMIRRORED: u32 = 1 << 8;
    pub const DESKTOP_SHAPE: u32 = 1 << 9;

    pub fn contains(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
}

/// The per-display changes from `old` to `new`, in id order. Empty when nothing changed.
pub fn diff(old: &[DisplayInfo], new: &[DisplayInfo]) -> Vec<(DisplayId, ChangeFlags)> {
    let mut ids: Vec<DisplayId> = old.iter().chain(new.iter()).map(|d| d.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let mut bits = 0u32;
        match (by_id(old, id), by_id(new, id)) {
            (None, Some(_)) => bits |= ChangeFlags::ADDED,
            (Some(_), None) => bits |= ChangeFlags::REMOVED,
            (Some(a), Some(b)) => {
                if a.bounds.x != b.bounds.x || a.bounds.y != b.bounds.y {
                    bits |= ChangeFlags::MOVED;
                }
                if a.mode != b.mode || a.rotation != b.rotation {
                    bits |= ChangeFlags::SET_MODE;
                }
                if a.bounds.w != b.bounds.w || a.bounds.h != b.bounds.h {
                    bits |= ChangeFlags::DESKTOP_SHAPE;
                }
                if !a.is_main && b.is_main {
                    bits |= ChangeFlags::SET_MAIN;
                }
                if !a.is_active && b.is_active {
                    bits |= ChangeFlags::ENABLED;
                }
                if a.is_active && !b.is_active {
                    bits |= ChangeFlags::DISABLED;
                }
                if a.mirror_of.is_none() && b.mirror_of.is_some() {
                    bits |= ChangeFlags::MIRRORED;
                }
                if a.mirror_of.is_some() && b.mirror_of.is_none() {
                    bits |= ChangeFlags::UNMIRRORED;
                }
            }
            (None, None) => {}
        }
        if bits != 0 {
            out.push((id, ChangeFlags(bits)));
        }
    }
    out
}

// ---- identity data ----

/// What the identity block of a display's EDID says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edid {
    pub vendor: u16,
    pub model: u16,
    pub serial: u32,
    pub size_mm: (f32, f32),
}

/// Reads the identity block (manufacturer, product code, serial, physical size in centimetres) from EDID bytes.
/// `None` when the data is shorter than the base block or lacks the fixed header.
pub fn parse_edid(data: &[u8]) -> Option<Edid> {
    const HEADER: [u8; 8] = [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0];
    if data.len() < 23 || data[..8] != HEADER {
        return None;
    }
    Some(Edid {
        vendor: u16::from_be_bytes([data[8], data[9]]),
        model: u16::from_le_bytes([data[10], data[11]]),
        serial: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        size_mm: (data[21] as f32 * 10.0, data[22] as f32 * 10.0),
    })
}

/// The three letters packed in the manufacturer field, five bits each (1 is A).
pub fn vendor_letters(packed: u16) -> String {
    if packed == 0 {
        return String::new();
    }
    let letter = |shift: u16| {
        let v = ((packed >> shift) & 0x1f) as u8;
        if (1..=26).contains(&v) {
            (b'A' + v - 1) as char
        } else {
            '?'
        }
    };
    [letter(10), letter(5), letter(0)].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: DisplayId, x: f32, w: f32, main: bool) -> DisplayInfo {
        DisplayInfo {
            id,
            name: format!("Display {id}"),
            is_main: main,
            is_builtin: false,
            is_active: true,
            is_online: true,
            is_asleep: false,
            bounds: Rect { x, y: 0.0, w, h: 900.0 },
            pixel_width: (w * 2.0) as u32,
            pixel_height: 1800,
            mode: DisplayMode { width: w as u32, height: 900, pixel_width: (w * 2.0) as u32, pixel_height: 1800, refresh_hz: 60.0, usable: true },
            modes: vec![DisplayMode::pixels(1920, 1080, 60.0)],
            rotation: Rotation::Deg0,
            mirror_of: None,
            vendor: 0x10ac,
            model: 0xd0a1,
            serial: 77,
            size_mm: (340.0, 210.0),
        }
    }

    #[test]
    fn a_display_survives_the_wire() {
        let mut d = info(3, 0.0, 1440.0, true);
        d.mirror_of = Some(1);
        d.rotation = Rotation::Deg90;
        let back = DisplayInfo::from_json(&d.to_json()).unwrap();
        assert_eq!(back, d);
        assert_eq!(list_from_json(&list_to_json(&[d.clone()])), vec![d]);
    }

    #[test]
    fn a_mode_reports_its_scale() {
        let m = DisplayMode { width: 1440, height: 900, pixel_width: 2880, pixel_height: 1800, refresh_hz: 120.0, usable: true };
        assert_eq!(m.scale(), 2.0);
        assert_eq!(DisplayMode::pixels(0, 0, 0.0).scale(), 1.0);
    }

    #[test]
    fn rotation_maps_degrees_both_ways() {
        for r in [Rotation::Deg0, Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
            assert_eq!(Rotation::from_degrees(r.degrees()), Some(r));
        }
        assert_eq!(Rotation::from_degrees(45), None);
    }

    #[test]
    fn lists_filter_active_online_and_main() {
        let mut off = info(2, 1440.0, 1920.0, false);
        off.is_active = false;
        let mut gone = info(3, 0.0, 800.0, false);
        gone.is_online = false;
        let list = vec![info(1, 0.0, 1440.0, true), off, gone];
        assert_eq!(active(&list).len(), 1);
        assert_eq!(online(&list).len(), 2);
        assert_eq!(main_display(&list).map(|d| d.id), Some(1));
        assert_eq!(by_id(&list, 2).map(|d| d.id), Some(2));
        assert_eq!(at_point(&list, 10.0, 10.0).map(|d| d.id), Some(1));
        assert_eq!(at_point(&list, 1500.0, 10.0), None);
    }

    #[test]
    fn main_falls_back_to_the_first_online_display() {
        let list = vec![info(5, 0.0, 800.0, false)];
        assert_eq!(main_display(&list).map(|d| d.id), Some(5));
        assert_eq!(main_display(&[]), None);
    }

    #[test]
    fn diff_reports_each_kind_of_change() {
        let old = vec![info(1, 0.0, 1440.0, true), info(2, 1440.0, 1920.0, false)];
        let mut moved = info(2, 1500.0, 1920.0, false);
        moved.mirror_of = Some(1);
        let new = vec![info(1, 0.0, 1280.0, true), moved, info(3, 4000.0, 800.0, false)];
        let d = diff(&old, &new);
        let flags = |id| d.iter().find(|(i, _)| *i == id).map(|(_, f)| *f).unwrap();
        assert!(flags(1).contains(ChangeFlags::DESKTOP_SHAPE));
        assert!(flags(1).contains(ChangeFlags::SET_MODE));
        assert!(flags(2).contains(ChangeFlags::MOVED));
        assert!(flags(2).contains(ChangeFlags::MIRRORED));
        assert!(flags(3).contains(ChangeFlags::ADDED));
        assert!(diff(&new, &new).is_empty());
        let removed = diff(&new, &old);
        assert!(removed.iter().any(|(i, f)| *i == 3 && f.contains(ChangeFlags::REMOVED)));
    }

    #[test]
    fn a_main_change_and_a_disable_are_flagged() {
        let old = vec![info(1, 0.0, 800.0, true), info(2, 800.0, 800.0, false)];
        let mut b = info(1, 0.0, 800.0, false);
        b.is_active = false;
        let new = vec![b, info(2, 800.0, 800.0, true)];
        let d = diff(&old, &new);
        assert!(d.iter().any(|(i, f)| *i == 2 && f.contains(ChangeFlags::SET_MAIN)));
        assert!(d.iter().any(|(i, f)| *i == 1 && f.contains(ChangeFlags::DISABLED)));
    }

    #[test]
    fn the_identity_block_of_an_edid_is_read() {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        // D E L = 4, 5, 12
        let packed: u16 = (4 << 10) | (5 << 5) | 12;
        e[8..10].copy_from_slice(&packed.to_be_bytes());
        e[10..12].copy_from_slice(&0xa0b1u16.to_le_bytes());
        e[12..16].copy_from_slice(&1234u32.to_le_bytes());
        e[21] = 34;
        e[22] = 21;
        let edid = parse_edid(&e).unwrap();
        assert_eq!(vendor_letters(edid.vendor), "DEL");
        assert_eq!(edid.model, 0xa0b1);
        assert_eq!(edid.serial, 1234);
        assert_eq!(edid.size_mm, (340.0, 210.0));
        assert!(parse_edid(&e[..10]).is_none());
        e[1] = 0;
        assert!(parse_edid(&e).is_none());
        assert_eq!(vendor_letters(0), "");
    }
}
