//! The card's mixer (ALSA control interface) without alsa-lib: list, read
//! and write controls through ioctls on /dev/snd/controlC0.
//!
//! The uapi structs below follow `include/uapi/sound/asound.h` of the
//! Linux kernel on a 64-bit target; their sizes and field offsets
//! are pinned by tests the way kms.rs pins the DRM ones. The one that bit
//! before: `snd_ctl_elem_list` is `offset, space, used, count` (the caller
//! says how much room it has in `space`; the kernel answers in `used` and
//! `count`), and listing is a two-step dance: an empty query for `count`,
//! then a second call with that much room.

use crate::bare::sys::{close, ioc, ioctl_ptr, iowr, open_dev, O_RDWR};
use std::ffi::c_int;
use std::io;

/// struct snd_ctl_elem_id
#[repr(C)]
#[derive(Clone, Copy)]
struct ElemId {
    numid: u32,
    iface: i32,
    device: u32,
    subdevice: u32,
    name: [u8; 44],
    index: u32,
}

/// struct snd_ctl_elem_list
#[repr(C)]
struct ElemList {
    offset: u32,
    space: u32,
    used: u32,
    count: u32,
    pids: *mut ElemId,
    reserved: [u8; 50],
}

/// struct snd_ctl_elem_info; `value` is the union: integer min/max/step
/// as three `long`s, integer64 as three `long long`s, or enumerated
/// `items, item, name[64]`.
#[repr(C)]
struct ElemInfo {
    id: ElemId,
    kind: i32,
    access: u32,
    count: u32,
    owner: i32,
    value: [u64; 16],
    dimen: [u16; 4],
    reserved: [u8; 56],
}

/// struct snd_ctl_elem_value; `value` is the union: `long[128]`,
/// `long long[64]`, enumerated `u32[128]` or `u8[512]`.
#[repr(C)]
struct ElemValue {
    id: ElemId,
    indirect: u32,
    pad: u32,
    value: [u64; 128],
    tstamp: [u64; 2],
    reserved: [u8; 112],
}

/// struct snd_ctl_card_info
#[repr(C)]
struct CardInfo {
    card: i32,
    pad: i32,
    id: [u8; 16],
    driver: [u8; 16],
    name: [u8; 32],
    longname: [u8; 80],
    reserved: [u8; 16],
    mixername: [u8; 80],
    components: [u8; 128],
}

/// struct snd_ctl_tlv with room for a dB-scale entry (type, length, two
/// values) after the header; the ioctl number counts the header only.
#[repr(C)]
struct Tlv {
    numid: u32,
    length: u32,
    tlv: [u32; 4],
}

const IFACE_MIXER: i32 = 2;
/// SNDRV_CTL_ELEM_ACCESS_TLV_READ
const ACCESS_TLV_READ: u32 = 1 << 4;
/// SNDRV_CTL_TLVT_DB_SCALE
const TLVT_DB_SCALE: u32 = 1;
/// SNDRV_CTL_TLVT_DB_SCALE: bit 16 of the step word marks min as mute.
const DB_SCALE_MUTE: u32 = 0x10000;
const NAME_LEN: usize = 44;

fn ior<T>(ty: u8, nr: u8) -> u32 {
    ioc(2, ty, nr, std::mem::size_of::<T>())
}
fn ctl_card_info() -> u32 {
    ior::<CardInfo>(b'U', 0x01)
}
fn ctl_elem_list() -> u32 {
    iowr::<ElemList>(b'U', 0x10)
}
fn ctl_elem_info() -> u32 {
    iowr::<ElemInfo>(b'U', 0x11)
}
fn ctl_elem_read() -> u32 {
    iowr::<ElemValue>(b'U', 0x12)
}
fn ctl_elem_write() -> u32 {
    iowr::<ElemValue>(b'U', 0x13)
}
fn ctl_tlv_read() -> u32 {
    ioc(3, b'U', 0x1a, 8)
}

fn cstr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|&b| b == 0).next().unwrap_or(&[])).into_owned()
}

impl ElemId {
    /// A mixer-interface id by name; the kernel fills in `numid`.
    fn named(name: &str) -> Self {
        let mut id = ElemId { numid: 0, iface: IFACE_MIXER, device: 0, subdevice: 0, name: [0; NAME_LEN], index: 0 };
        let n = name.len().min(NAME_LEN - 1);
        id.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        id
    }
}

/// What a control holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Enum,
    Bytes,
    Iec958,
    Int64,
    Other(i32),
}

impl Kind {
    fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Kind::Bool,
            2 => Kind::Int,
            3 => Kind::Enum,
            4 => Kind::Bytes,
            5 => Kind::Iec958,
            6 => Kind::Int64,
            other => Kind::Other(other),
        }
    }

    /// tinymix's column name.
    fn label(self) -> String {
        match self {
            Kind::Bool => "BOOL".into(),
            Kind::Int => "INT".into(),
            Kind::Enum => "ENUM".into(),
            Kind::Bytes => "BYTE".into(),
            Kind::Iec958 => "IEC958".into(),
            Kind::Int64 => "INT64".into(),
            Kind::Other(k) => format!("type{k}"),
        }
    }
}

/// A control's dB scale from its TLV, in hundredths of a dB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbScale {
    /// dB at the control's minimum value
    pub min_cdb: i32,
    /// dB per step; negative when a larger value is quieter
    pub step_cdb: i32,
    /// the minimum value is a mute, not `min_cdb`
    pub mute_at_min: bool,
}

/// One mixer control with its current value(s).
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub numid: u32,
    pub name: String,
    pub kind: Kind,
    pub count: u32,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    /// enumerated item names
    pub items: Vec<String>,
    pub values: Vec<i64>,
    pub db: Option<DbScale>,
}

impl Control {
    /// The value as `tinymix` prints it: On/Off, the item name, the number,
    /// or two hex digits per byte.
    fn value_text(&self, v: i64) -> String {
        match self.kind {
            Kind::Bool => (if v != 0 { "On" } else { "Off" }).to_string(),
            Kind::Enum => self.items.get(v as usize).cloned().unwrap_or_else(|| v.to_string()),
            Kind::Bytes => format!("{v:02x}"),
            _ => v.to_string(),
        }
    }

    /// One line in `tinymix`'s table format: index, type, count, name, values.
    pub fn describe(&self) -> String {
        let vals: Vec<String> = self.values.iter().map(|&v| self.value_text(v)).collect();
        let sep = if self.kind == Kind::Bytes { "  " } else { " " };
        format!("{}\t{}\t{}\t{:<40} {}{}", self.numid.wrapping_sub(1), self.kind.label(), self.count, self.name, if self.kind == Kind::Bytes { " " } else { "" }, vals.join(sep))
    }

    /// The control's range and scale.
    pub fn detail(&self) -> String {
        let mut s = match self.kind {
            Kind::Enum => format!("items [{}]", self.items.join("|")),
            Kind::Int | Kind::Int64 => format!("range {}..{} step {}", self.min, self.max, self.step),
            _ => String::new(),
        };
        if let Some(db) = self.db {
            s.push_str(&format!(" dB: min {:.2} step {:.2}{}", db.min_cdb as f64 / 100.0, db.step_cdb as f64 / 100.0, if db.mute_at_min { " (min mutes)" } else { "" }));
        }
        s.trim().to_string()
    }

    /// Parses a value for this control: a number, an item name, or on/off.
    pub fn parse(&self, s: &str) -> io::Result<i64> {
        if let Some(i) = self.items.iter().position(|n| n.eq_ignore_ascii_case(s)) {
            return Ok(i as i64);
        }
        match s.to_ascii_lowercase().as_str() {
            "on" | "true" => Ok(1),
            "off" | "false" => Ok(0),
            _ => s.parse().map_err(|_| io::Error::other(format!("bad value {s:?} for {}", self.name))),
        }
    }
}

/// Unpacks `count` values of `kind` from the value union (little-endian,
/// as x86-64 and ARM64 Linux are).
fn unpack_values(kind: Kind, count: usize, raw: &[u64; 128]) -> Vec<i64> {
    match kind {
        Kind::Enum => (0..count.min(128)).map(|i| ((raw[i / 2] >> ((i % 2) * 32)) & 0xffff_ffff) as i64).collect(),
        Kind::Bytes | Kind::Iec958 => (0..count.min(512)).map(|i| ((raw[i / 8] >> ((i % 8) * 8)) & 0xff) as i64).collect(),
        Kind::Int64 => raw[..count.min(64)].iter().map(|&x| x as i64).collect(),
        _ => raw[..count.min(128)].iter().map(|&x| x as i64).collect(),
    }
}

/// Packs `values` of `kind` into the value union.
fn pack_values(kind: Kind, values: &[i64], raw: &mut [u64; 128]) {
    for (i, &v) in values.iter().enumerate() {
        match kind {
            Kind::Enum if i < 128 => {
                let shift = (i % 2) * 32;
                raw[i / 2] = (raw[i / 2] & !(0xffff_ffffu64 << shift)) | ((v as u32 as u64) << shift);
            }
            Kind::Bytes | Kind::Iec958 if i < 512 => {
                let shift = (i % 8) * 8;
                raw[i / 8] = (raw[i / 8] & !(0xffu64 << shift)) | ((v as u8 as u64) << shift);
            }
            Kind::Int64 if i < 64 => raw[i] = v as u64,
            Kind::Bool | Kind::Int if i < 128 => raw[i] = v as u64,
            _ => {}
        }
    }
}

/// Decodes a dB-scale TLV: `[type, length, min, step|flags]`.
fn db_scale(tlv: &[u32; 4]) -> Option<DbScale> {
    (tlv[0] == TLVT_DB_SCALE && tlv[1] >= 8).then(|| DbScale { min_cdb: tlv[2] as i32, step_cdb: (tlv[3] & 0xffff) as i32, mute_at_min: tlv[3] & DB_SCALE_MUTE != 0 })
}

/// The card's control device (/dev/snd/controlC0).
pub struct Mixer {
    fd: c_int,
}

impl Mixer {
    pub fn open(path: &str) -> io::Result<Mixer> {
        Ok(Mixer { fd: open_dev(path, O_RDWR)? })
    }

    /// The card as SNDRV_CTL_IOCTL_CARD_INFO reports it.
    pub fn card_info(&self) -> io::Result<String> {
        // SAFETY: CardInfo is plain data; all-zero is a valid value and the kernel fills it.
        let mut c: CardInfo = unsafe { std::mem::zeroed() };
        unsafe { ioctl_ptr(self.fd, ctl_card_info(), &mut c) }?;
        Ok(format!("card {} id={} driver={} name={} mixer={} ({})", c.card, cstr(&c.id), cstr(&c.driver), cstr(&c.name), cstr(&c.mixername), cstr(&c.longname)))
    }

    /// Every element id, in two calls: count, then that much room.
    fn ids(&self) -> io::Result<Vec<ElemId>> {
        // SAFETY: ElemList is plain data; zero means "no room, just count".
        let mut l: ElemList = unsafe { std::mem::zeroed() };
        unsafe { ioctl_ptr(self.fd, ctl_elem_list(), &mut l) }?;
        let n = l.count as usize;
        // SAFETY: ElemId is plain data; the kernel overwrites the zeros.
        let mut ids: Vec<ElemId> = vec![unsafe { std::mem::zeroed() }; n];
        l.space = n as u32;
        l.pids = ids.as_mut_ptr();
        unsafe { ioctl_ptr(self.fd, ctl_elem_list(), &mut l) }?;
        ids.truncate(l.used as usize);
        Ok(ids)
    }

    fn info(&self, id: ElemId, item: u32) -> io::Result<ElemInfo> {
        // SAFETY: ElemInfo is plain data; the kernel fills everything but the id.
        let mut i: ElemInfo = unsafe { std::mem::zeroed() };
        i.id = id;
        // enumerated.item sits at byte 4 of the union
        i.value[0] = (item as u64) << 32;
        unsafe { ioctl_ptr(self.fd, ctl_elem_info(), &mut i) }?;
        Ok(i)
    }

    fn read(&self, id: ElemId) -> io::Result<ElemValue> {
        // SAFETY: ElemValue is plain data; the kernel fills the values.
        let mut v: ElemValue = unsafe { std::mem::zeroed() };
        v.id = id;
        unsafe { ioctl_ptr(self.fd, ctl_elem_read(), &mut v) }?;
        Ok(v)
    }

    fn tlv(&self, numid: u32) -> Option<DbScale> {
        let mut t = Tlv { numid, length: 16, tlv: [0; 4] };
        unsafe { ioctl_ptr(self.fd, ctl_tlv_read(), &mut t) }.ok()?;
        db_scale(&t.tlv)
    }

    /// The control behind `id` with its range, items and current values.
    fn control(&self, id: ElemId) -> io::Result<Control> {
        let info = self.info(id, 0)?;
        let id = info.id;
        let kind = Kind::from_raw(info.kind);
        let (mut min, mut max, mut step) = (0, 0, 0);
        let mut items = Vec::new();
        match kind {
            Kind::Bool => max = 1,
            Kind::Int | Kind::Int64 => {
                min = info.value[0] as i64;
                max = info.value[1] as i64;
                step = info.value[2] as i64;
            }
            Kind::Enum => {
                let n = (info.value[0] & 0xffff_ffff) as u32;
                for item in 0..n.min(256) {
                    let i = self.info(id, item)?;
                    // enumerated.name is the 64 bytes from byte 8 of the union
                    let name: Vec<u8> = i.value[1..9].iter().flat_map(|w| w.to_le_bytes()).collect();
                    items.push(cstr(&name));
                }
                max = n as i64 - 1;
            }
            _ => {}
        }
        let values = unpack_values(kind, info.count as usize, &self.read(id)?.value);
        let db = (info.access & ACCESS_TLV_READ != 0).then(|| self.tlv(id.numid)).flatten();
        Ok(Control { numid: id.numid, name: cstr(&id.name), kind, count: info.count, min, max, step, items, values, db })
    }

    /// Every control with its current value(s), in numid order.
    pub fn list(&self) -> io::Result<Vec<Control>> {
        self.ids()?.into_iter().map(|id| self.control(id)).collect()
    }

    /// One control by its exact name.
    pub fn get(&self, name: &str) -> io::Result<Control> {
        self.control(ElemId::named(name)).map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))
    }

    /// Writes `values` (one per channel; fewer leave the rest as they are)
    /// to the control named `name` and returns its new state.
    pub fn set(&self, name: &str, values: &[i64]) -> io::Result<Control> {
        let c = self.get(name)?;
        let id = ElemId { numid: c.numid, ..ElemId::named(name) };
        let mut v = self.read(id)?;
        pack_values(c.kind, &values[..values.len().min(c.count as usize)], &mut v.value);
        unsafe { ioctl_ptr(self.fd, ctl_elem_write(), &mut v) }?;
        self.get(name)
    }

    /// `set` with values as text: numbers, item names, or on/off.
    pub fn set_text(&self, name: &str, values: &[&str]) -> io::Result<Control> {
        let c = self.get(name)?;
        let parsed = values.iter().map(|s| c.parse(s)).collect::<io::Result<Vec<i64>>>()?;
        self.set(name, &parsed)
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        // SAFETY: the fd is ours and not used after this.
        unsafe { close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn uapi_struct_sizes() {
        assert_eq!(size_of::<ElemId>(), 64);
        assert_eq!(size_of::<ElemList>(), 80);
        assert_eq!(size_of::<ElemInfo>(), 272);
        assert_eq!(size_of::<ElemValue>(), 1224);
        assert_eq!(size_of::<CardInfo>(), 376);
        assert_eq!(size_of::<Tlv>(), 24);
    }

    #[test]
    fn uapi_field_offsets() {
        assert_eq!(offset_of!(ElemId, name), 16);
        assert_eq!(offset_of!(ElemId, index), 60);
        // the list header is offset, space, used, count — space before used
        assert_eq!(offset_of!(ElemList, space), 4);
        assert_eq!(offset_of!(ElemList, used), 8);
        assert_eq!(offset_of!(ElemList, count), 12);
        assert_eq!(offset_of!(ElemList, pids), 16);
        assert_eq!(offset_of!(ElemInfo, kind), 64);
        assert_eq!(offset_of!(ElemInfo, access), 68);
        assert_eq!(offset_of!(ElemInfo, count), 72);
        assert_eq!(offset_of!(ElemInfo, owner), 76);
        assert_eq!(offset_of!(ElemInfo, value), 80);
        assert_eq!(offset_of!(ElemInfo, dimen), 208);
        assert_eq!(offset_of!(ElemValue, indirect), 64);
        assert_eq!(offset_of!(ElemValue, value), 72);
        assert_eq!(offset_of!(ElemValue, tstamp), 1096);
        assert_eq!(offset_of!(CardInfo, mixername), 168);
        assert_eq!(offset_of!(CardInfo, components), 248);
        assert_eq!(offset_of!(Tlv, tlv), 8);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        assert_eq!(ctl_card_info(), 0x8178_5501);
        assert_eq!(ctl_elem_list(), 0xc050_5510);
        assert_eq!(ctl_elem_info(), 0xc110_5511);
        assert_eq!(ctl_elem_read(), 0xc4c8_5512);
        assert_eq!(ctl_elem_write(), 0xc4c8_5513);
        assert_eq!(ctl_tlv_read(), 0xc008_551a);
    }

    #[test]
    fn named_id_is_a_mixer_element_with_a_nul_terminated_name() {
        let id = ElemId::named("VBC DAC0 DG Set");
        assert_eq!((id.numid, id.iface, id.index), (0, IFACE_MIXER, 0));
        assert_eq!(cstr(&id.name), "VBC DAC0 DG Set");
        let long = ElemId::named(&"x".repeat(60));
        assert_eq!(cstr(&long.name).len(), 43, "truncated to leave the NUL");
    }

    #[test]
    fn values_pack_and_unpack_by_kind() {
        let mut raw = [0u64; 128];
        pack_values(Kind::Enum, &[1, 2, 3], &mut raw);
        assert_eq!(raw[0], 1 | 2 << 32);
        assert_eq!(raw[1], 3);
        assert_eq!(unpack_values(Kind::Enum, 3, &raw), vec![1, 2, 3]);
        let mut raw = [0u64; 128];
        pack_values(Kind::Bytes, &[1, 2, 0xff, 4, 5, 6, 7, 8, 9], &mut raw);
        assert_eq!(raw[0], 0x0807_0605_04ff_0201);
        assert_eq!(raw[1], 9);
        assert_eq!(unpack_values(Kind::Bytes, 9, &raw), vec![1, 2, 0xff, 4, 5, 6, 7, 8, 9]);
        let mut raw = [0u64; 128];
        pack_values(Kind::Int, &[26, -1], &mut raw);
        assert_eq!(raw[..2], [26, u64::MAX]);
        assert_eq!(unpack_values(Kind::Int, 2, &raw), vec![26, -1]);
        pack_values(Kind::Int64, &[i64::MIN], &mut raw);
        assert_eq!(unpack_values(Kind::Int64, 1, &raw), vec![i64::MIN]);
        // a short write leaves the other channels alone
        let mut raw = [7u64; 128];
        pack_values(Kind::Int, &[1], &mut raw);
        assert_eq!(raw[..2], [1, 7]);
    }

    #[test]
    fn db_scale_tlv_decodes_min_step_and_mute() {
        assert_eq!(db_scale(&[1, 8, (-6350i32) as u32, 50]), Some(DbScale { min_cdb: -6350, step_cdb: 50, mute_at_min: false }));
        assert_eq!(db_scale(&[1, 8, (-9999i32) as u32, 0x10000 | 100]), Some(DbScale { min_cdb: -9999, step_cdb: 100, mute_at_min: true }));
        assert_eq!(db_scale(&[2, 8, 0, 0]), None, "only a plain dB scale is understood");
    }

    fn control(kind: Kind, count: u32, values: Vec<i64>) -> Control {
        Control { numid: 168, name: "Speaker Function".into(), kind, count, min: 0, max: 1, step: 0, items: vec!["Off".into(), "On".into()], values, db: None }
    }

    #[test]
    fn describe_matches_tinymix_columns() {
        assert_eq!(control(Kind::Bool, 1, vec![0]).describe(), "167\tBOOL\t1\tSpeaker Function                         Off");
        assert_eq!(control(Kind::Enum, 1, vec![1]).describe(), "167\tENUM\t1\tSpeaker Function                         On");
        assert_eq!(control(Kind::Int, 2, vec![26, 26]).describe(), "167\tINT\t2\tSpeaker Function                         26 26");
        assert_eq!(control(Kind::Bytes, 3, vec![0, 255, 1]).describe(), "167\tBYTE\t3\tSpeaker Function                          00  ff  01");
    }

    #[test]
    fn detail_shows_range_items_and_db() {
        let mut c = control(Kind::Int, 2, vec![26, 26]);
        c.max = 127;
        assert_eq!(c.detail(), "range 0..127 step 0");
        c.db = Some(DbScale { min_cdb: -6350, step_cdb: 50, mute_at_min: true });
        assert_eq!(c.detail(), "range 0..127 step 0 dB: min -63.50 step 0.50 (min mutes)");
        assert_eq!(control(Kind::Enum, 1, vec![0]).detail(), "items [Off|On]");
    }

    #[test]
    fn parse_accepts_items_switches_and_numbers() {
        let c = control(Kind::Enum, 1, vec![0]);
        assert_eq!(c.parse("on").unwrap(), 1);
        assert_eq!(c.parse("OFF").unwrap(), 0);
        assert_eq!(c.parse("1").unwrap(), 1);
        assert_eq!(c.parse("-3").unwrap(), -3);
        assert!(c.parse("loud").is_err());
    }

    #[test]
    fn cstr_stops_at_nul() {
        assert_eq!(cstr(b"abc\0def"), "abc");
        assert_eq!(cstr(b"abc"), "abc");
    }
}
