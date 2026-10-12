//! The card's control device (`/dev/snd/controlC{n}`) without alsa-lib: card and PCM info, element listing,
//! reading and writing, dB scales, event subscription, the `* Jack` booleans and the ELD of HDMI/DP outputs.
//! The uapi structs follow `include/uapi/sound/asound.h` on a 64-bit target (grown from `media/mixer.rs`); sizes
//! and offsets are pinned by the tests.
//!
//! The one that bit before: `snd_ctl_elem_list` is `offset, space, used, count` (the caller says how much room it
//! has in `space`; the kernel answers in `used` and `count`), and listing is a two-step dance: an empty query for
//! `count`, then a second call with that much room.

use crate::sys::{close_fd, ioc, ior, iowr, ioctl, open_dev, DIR_READ, DIR_WRITE};
use crate::uapi::PcmInfo;
use std::io;
use std::os::fd::RawFd;

/// struct snd_ctl_elem_id
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ElemId {
    pub numid: u32,
    pub iface: i32,
    pub device: u32,
    pub subdevice: u32,
    pub name: [u8; 44],
    pub index: u32,
}

/// struct snd_ctl_elem_list
#[repr(C)]
pub struct ElemList {
    pub offset: u32,
    pub space: u32,
    pub used: u32,
    pub count: u32,
    pub pids: *mut ElemId,
    pub reserved: [u8; 50],
}

/// struct snd_ctl_elem_info; `value` is the union: integer min/max/step as three `long`s, integer64 as three
/// `long long`s, or enumerated `items, item, name[64]`.
#[repr(C)]
pub struct ElemInfo {
    pub id: ElemId,
    pub kind: i32,
    pub access: u32,
    pub count: u32,
    pub owner: i32,
    pub value: [u64; 16],
    pub dimen: [u16; 4],
    pub reserved: [u8; 56],
}

/// struct snd_ctl_elem_value; `value` is the union: `long[128]`, `long long[64]`, enumerated `u32[128]` or `u8[512]`.
#[repr(C)]
pub struct ElemValue {
    pub id: ElemId,
    pub indirect: u32,
    pub pad: u32,
    pub value: [u64; 128],
    pub tstamp: [u64; 2],
    pub reserved: [u8; 112],
}

/// struct snd_ctl_card_info
#[repr(C)]
pub struct CardInfo {
    pub card: i32,
    pub pad: i32,
    pub id: [u8; 16],
    pub driver: [u8; 16],
    pub name: [u8; 32],
    pub longname: [u8; 80],
    pub reserved: [u8; 16],
    pub mixername: [u8; 80],
    pub components: [u8; 128],
}

/// struct snd_ctl_tlv with room for a dB-scale entry (type, length, two values) after the header; the ioctl
/// number counts the header only.
#[repr(C)]
pub struct Tlv {
    pub numid: u32,
    pub length: u32,
    pub tlv: [u32; 4],
}

/// struct snd_ctl_event (type 0 is an element event): `mask` says what changed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawEvent {
    pub kind: i32,
    pub mask: u32,
    pub id: ElemId,
}

pub const IFACE_CARD: i32 = 0;
pub const IFACE_MIXER: i32 = 2;
pub const IFACE_PCM: i32 = 3;
/// SNDRV_CTL_ELEM_ACCESS_TLV_READ
pub const ACCESS_TLV_READ: u32 = 1 << 4;
/// SNDRV_CTL_TLVT_DB_SCALE
pub const TLVT_DB_SCALE: u32 = 1;
/// bit 16 of the step word marks min as mute.
pub const DB_SCALE_MUTE: u32 = 0x10000;
pub const NAME_LEN: usize = 44;
pub const EVENT_MASK_VALUE: u32 = 1 << 0;
pub const EVENT_MASK_INFO: u32 = 1 << 1;
pub const EVENT_MASK_ADD: u32 = 1 << 2;
pub const EVENT_MASK_REMOVE: u32 = !0;

pub const CTL_CARD_INFO: u32 = ior::<CardInfo>(b'U', 0x01);
pub const CTL_ELEM_LIST: u32 = iowr::<ElemList>(b'U', 0x10);
pub const CTL_ELEM_INFO: u32 = iowr::<ElemInfo>(b'U', 0x11);
pub const CTL_ELEM_READ: u32 = iowr::<ElemValue>(b'U', 0x12);
pub const CTL_ELEM_WRITE: u32 = iowr::<ElemValue>(b'U', 0x13);
pub const CTL_SUBSCRIBE_EVENTS: u32 = iowr::<i32>(b'U', 0x16);
pub const CTL_TLV_READ: u32 = ioc(DIR_READ | DIR_WRITE, b'U', 0x1a, 8);
pub const CTL_PCM_NEXT_DEVICE: u32 = ior::<i32>(b'U', 0x30);
pub const CTL_PCM_INFO: u32 = iowr::<PcmInfo>(b'U', 0x31);

pub fn cstr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|&b| b == 0).next().unwrap_or(&[])).into_owned()
}

impl ElemId {
    /// An id by interface, device and name; the kernel fills in `numid`.
    pub fn new(iface: i32, device: u32, name: &str) -> ElemId {
        let mut id = ElemId { numid: 0, iface, device, subdevice: 0, name: [0; NAME_LEN], index: 0 };
        let n = name.len().min(NAME_LEN - 1);
        id.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        id
    }

    /// A mixer-interface id by name.
    pub fn named(name: &str) -> ElemId {
        ElemId::new(IFACE_MIXER, 0, name)
    }

    /// An id by number only (the kernel resolves a nonzero `numid` first).
    pub fn by_numid(numid: u32) -> ElemId {
        ElemId { numid, iface: 0, device: 0, subdevice: 0, name: [0; NAME_LEN], index: 0 }
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
    pub fn from_raw(raw: i32) -> Kind {
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
}

/// A control's dB scale from its TLV, in hundredths of a dB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbScale {
    /// dB at the control's minimum value
    pub min_cdb: i32,
    /// dB per step
    pub step_cdb: i32,
    /// the minimum value is a mute, not `min_cdb`
    pub mute_at_min: bool,
}

impl DbScale {
    /// dB at raw value `v` of a control spanning `min..=max`: `(min_dB, max_dB)` for the range.
    pub fn range_db(&self, min: i64, max: i64) -> (f32, f32) {
        let lo = self.min_cdb as f64 / 100.0;
        let hi = (self.min_cdb as i64 + self.step_cdb as i64 * (max - min)) as f64 / 100.0;
        (lo as f32, hi as f32)
    }
}

/// One element with its current value(s).
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub numid: u32,
    pub iface: i32,
    pub device: u32,
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

/// Unpacks `count` values of `kind` from the value union (little-endian).
pub fn unpack_values(kind: Kind, count: usize, raw: &[u64; 128]) -> Vec<i64> {
    match kind {
        Kind::Enum => (0..count.min(128)).map(|i| ((raw[i / 2] >> ((i % 2) * 32)) & 0xffff_ffff) as i64).collect(),
        Kind::Bytes | Kind::Iec958 => (0..count.min(512)).map(|i| ((raw[i / 8] >> ((i % 8) * 8)) & 0xff) as i64).collect(),
        Kind::Int64 => raw[..count.min(64)].iter().map(|&x| x as i64).collect(),
        _ => raw[..count.min(128)].iter().map(|&x| x as i64).collect(),
    }
}

/// Packs `values` of `kind` into the value union.
pub fn pack_values(kind: Kind, values: &[i64], raw: &mut [u64; 128]) {
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
pub fn db_scale(tlv: &[u32; 4]) -> Option<DbScale> {
    (tlv[0] == TLVT_DB_SCALE && tlv[1] >= 8).then(|| DbScale { min_cdb: tlv[2] as i32, step_cdb: (tlv[3] & 0xffff) as i32, mute_at_min: tlv[3] & DB_SCALE_MUTE != 0 })
}

/// The monitor name in an ELD blob (EDID-like data of an HDMI/DP output): a 4-byte header, then 16 fixed bytes
/// of the baseline block, then the name. Byte 4 holds `MNL`, the name length, in its low 5 bits.
pub fn eld_monitor_name(eld: &[u8]) -> Option<String> {
    const FIXED: usize = 20;
    let mnl = (*eld.get(4)? & 0x1f) as usize;
    if mnl == 0 {
        return None;
    }
    let name = eld.get(FIXED..FIXED + mnl)?;
    let s = String::from_utf8_lossy(name).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The PCM device number in an HDMI/DP jack name such as `HDMI/DP,pcm=3 Jack`.
pub fn jack_pcm(name: &str) -> Option<u32> {
    let rest = name.split("pcm=").nth(1)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// A `* Jack` boolean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jack {
    pub numid: u32,
    /// The element name, e.g. `Headphone Jack`.
    pub name: String,
    pub connected: bool,
}

/// An element event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtlEvent {
    pub numid: u32,
    pub name: String,
    pub mask: u32,
}

impl CtlEvent {
    pub fn is_value_change(&self) -> bool {
        self.mask != EVENT_MASK_REMOVE && self.mask & EVENT_MASK_VALUE != 0
    }
}

/// Card identification from `SNDRV_CTL_IOCTL_CARD_INFO`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardData {
    pub card: i32,
    pub id: String,
    pub driver: String,
    pub name: String,
    pub longname: String,
    pub mixername: String,
}

/// One PCM device of a card as `SNDRV_CTL_IOCTL_PCM_INFO` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmData {
    pub device: u32,
    pub id: String,
    pub name: String,
    pub playback: bool,
    pub capture: bool,
}

pub struct Ctl {
    fd: RawFd,
    pub card: u32,
}

impl Ctl {
    pub fn open(card: u32) -> io::Result<Ctl> {
        Ok(Ctl { fd: open_dev(&format!("/dev/snd/controlC{card}"), true)?, card })
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    pub fn card_info(&self) -> io::Result<CardData> {
        // SAFETY: CardInfo is plain data; all-zero is a valid value and the kernel fills it.
        let mut c: CardInfo = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is the struct CTL_CARD_INFO encodes.
        unsafe { ioctl(self.fd, CTL_CARD_INFO, &mut c) }?;
        Ok(CardData { card: c.card, id: cstr(&c.id), driver: cstr(&c.driver), name: cstr(&c.name), longname: cstr(&c.longname), mixername: cstr(&c.mixername) })
    }

    /// The PCM device numbers of the card, in order.
    pub fn pcm_devices(&self) -> io::Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut dev: i32 = -1;
        loop {
            // SAFETY: the request reads and writes one int.
            unsafe { ioctl(self.fd, CTL_PCM_NEXT_DEVICE, &mut dev) }?;
            if dev < 0 || out.len() > 64 {
                return Ok(out);
            }
            out.push(dev as u32);
        }
    }

    /// Info of PCM `device`, both directions (a direction the device lacks reports false).
    pub fn pcm_info(&self, device: u32) -> io::Result<PcmData> {
        let mut data = PcmData { device, id: String::new(), name: String::new(), playback: false, capture: false };
        for stream in [0i32, 1] {
            // SAFETY: PcmInfo is plain data; zero is valid and the request fills it.
            let mut i: PcmInfo = unsafe { std::mem::zeroed() };
            i.device = device;
            i.stream = stream;
            // SAFETY: `i` is the struct CTL_PCM_INFO encodes.
            if unsafe { ioctl(self.fd, CTL_PCM_INFO, &mut i) }.is_ok() {
                if stream == 0 {
                    data.playback = true;
                } else {
                    data.capture = true;
                }
                if data.name.is_empty() {
                    data.id = cstr(&i.id);
                    data.name = cstr(&i.name);
                }
            }
        }
        if !data.playback && !data.capture {
            return Err(io::Error::from_raw_os_error(libc::ENOENT));
        }
        Ok(data)
    }

    /// Every element id, in two calls: count, then that much room.
    fn ids(&self) -> io::Result<Vec<ElemId>> {
        // SAFETY: ElemList is plain data; zero means "no room, just count".
        let mut l: ElemList = unsafe { std::mem::zeroed() };
        // SAFETY: `l` is the struct CTL_ELEM_LIST encodes.
        unsafe { ioctl(self.fd, CTL_ELEM_LIST, &mut l) }?;
        let n = l.count as usize;
        // SAFETY: ElemId is plain data; the kernel overwrites the zeros.
        let mut ids: Vec<ElemId> = vec![unsafe { std::mem::zeroed() }; n];
        l.space = n as u32;
        l.pids = ids.as_mut_ptr();
        // SAFETY: `pids` points at `space` ids that stay alive for the call.
        unsafe { ioctl(self.fd, CTL_ELEM_LIST, &mut l) }?;
        ids.truncate(l.used as usize);
        Ok(ids)
    }

    fn info(&self, id: ElemId, item: u32) -> io::Result<ElemInfo> {
        // SAFETY: ElemInfo is plain data; the kernel fills everything but the id.
        let mut i: ElemInfo = unsafe { std::mem::zeroed() };
        i.id = id;
        // enumerated.item sits at byte 4 of the union
        i.value[0] = (item as u64) << 32;
        // SAFETY: `i` is the struct CTL_ELEM_INFO encodes.
        unsafe { ioctl(self.fd, CTL_ELEM_INFO, &mut i) }?;
        Ok(i)
    }

    fn read(&self, id: ElemId) -> io::Result<ElemValue> {
        // SAFETY: ElemValue is plain data; the kernel fills the values.
        let mut v: ElemValue = unsafe { std::mem::zeroed() };
        v.id = id;
        // SAFETY: `v` is the struct CTL_ELEM_READ encodes.
        unsafe { ioctl(self.fd, CTL_ELEM_READ, &mut v) }?;
        Ok(v)
    }

    fn tlv(&self, numid: u32) -> Option<DbScale> {
        let mut t = Tlv { numid, length: 16, tlv: [0; 4] };
        // SAFETY: `t` has room for the header plus 16 bytes of TLV, as `length` says.
        unsafe { ioctl(self.fd, CTL_TLV_READ, &mut t) }.ok()?;
        db_scale(&t.tlv)
    }

    /// The control behind `id` with its range, items and current values.
    pub fn control(&self, id: ElemId) -> io::Result<Control> {
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
        Ok(Control { numid: id.numid, iface: id.iface, device: id.device, name: cstr(&id.name), kind, count: info.count, min, max, step, items, values, db })
    }

    /// Every control with its current value(s), in numid order. Elements that cannot be read are skipped.
    pub fn list(&self) -> io::Result<Vec<Control>> {
        Ok(self.ids()?.into_iter().filter_map(|id| self.control(id).ok()).collect())
    }

    pub fn get(&self, id: ElemId) -> io::Result<Control> {
        self.control(id)
    }

    /// Writes `values` (one per channel; fewer leave the rest as they are) to the element `numid`.
    pub fn set(&self, c: &Control, values: &[i64]) -> io::Result<()> {
        let mut v = self.read(ElemId::by_numid(c.numid))?;
        pack_values(c.kind, &values[..values.len().min(c.count as usize)], &mut v.value);
        // SAFETY: `v` is the struct CTL_ELEM_WRITE encodes.
        unsafe { ioctl(self.fd, CTL_ELEM_WRITE, &mut v) }
    }

    /// The `* Jack` booleans with their state. Jacks the card cannot sense are not listed by the kernel.
    pub fn jacks(&self) -> io::Result<Vec<Jack>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|c| c.kind == Kind::Bool && c.name.ends_with(" Jack"))
            .map(|c| Jack { numid: c.numid, connected: c.values.first().copied().unwrap_or(0) != 0, name: c.name })
            .collect())
    }

    /// The raw ELD of PCM `device` (HDMI/DP): present only while a sink is connected.
    pub fn eld(&self, device: u32) -> Option<Vec<u8>> {
        let id = ElemId::new(IFACE_PCM, device, "ELD");
        let info = self.info(id, 0).ok()?;
        let n = (info.count as usize).min(512);
        if n == 0 {
            return None;
        }
        let v = self.read(info.id).ok()?;
        Some(unpack_values(Kind::Bytes, n, &v.value).into_iter().map(|b| b as u8).collect())
    }

    /// The connected monitor's name for an HDMI/DP PCM, from the ELD.
    pub fn monitor_name(&self, device: u32) -> Option<String> {
        eld_monitor_name(&self.eld(device)?)
    }

    /// Subscribes (or unsubscribes) this fd to element events.
    pub fn subscribe(&self, on: bool) -> io::Result<()> {
        let mut v: i32 = on as i32;
        // SAFETY: the request reads and writes one int.
        unsafe { ioctl(self.fd, CTL_SUBSCRIBE_EVENTS, &mut v) }
    }

    /// Reads one pending event; `Ok(None)` when there is none (the fd is non-blocking).
    pub fn read_event(&self) -> io::Result<Option<CtlEvent>> {
        let mut ev = std::mem::MaybeUninit::<RawEvent>::zeroed();
        // SAFETY: reads at most size_of::<RawEvent>() bytes into the buffer.
        let r = unsafe { libc::read(self.fd, ev.as_mut_ptr() as *mut libc::c_void, std::mem::size_of::<RawEvent>()) };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock { Ok(None) } else { Err(e) };
        }
        if (r as usize) < std::mem::size_of::<RawEvent>() {
            return Ok(None);
        }
        // SAFETY: the kernel wrote a whole event and zero is valid for every field.
        let ev = unsafe { ev.assume_init() };
        if ev.kind != 0 {
            return Ok(None);
        }
        Ok(Some(CtlEvent { numid: ev.id.numid, name: cstr(&ev.id.name), mask: ev.mask }))
    }
}

impl Drop for Ctl {
    fn drop(&mut self) {
        close_fd(self.fd);
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
        assert_eq!(size_of::<RawEvent>(), 72);
    }

    #[test]
    fn uapi_field_offsets() {
        assert_eq!(offset_of!(ElemId, name), 16);
        assert_eq!(offset_of!(ElemId, index), 60);
        // the list header is offset, space, used, count: space before used
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
        assert_eq!(offset_of!(RawEvent, mask), 4);
        assert_eq!(offset_of!(RawEvent, id), 8);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        assert_eq!(CTL_CARD_INFO, 0x8178_5501);
        assert_eq!(CTL_ELEM_LIST, 0xc050_5510);
        assert_eq!(CTL_ELEM_INFO, 0xc110_5511);
        assert_eq!(CTL_ELEM_READ, 0xc4c8_5512);
        assert_eq!(CTL_ELEM_WRITE, 0xc4c8_5513);
        assert_eq!(CTL_SUBSCRIBE_EVENTS, 0xc004_5516);
        assert_eq!(CTL_TLV_READ, 0xc008_551a);
        assert_eq!(CTL_PCM_NEXT_DEVICE, 0x8004_5530);
        assert_eq!(CTL_PCM_INFO, 0xc120_5531);
    }

    #[test]
    fn ids_are_built_by_interface_and_name() {
        let id = ElemId::named("VBC DAC0 DG Set");
        assert_eq!((id.numid, id.iface, id.index), (0, IFACE_MIXER, 0));
        assert_eq!(cstr(&id.name), "VBC DAC0 DG Set");
        let long = ElemId::named(&"x".repeat(60));
        assert_eq!(cstr(&long.name).len(), 43, "truncated to leave the NUL");
        let eld = ElemId::new(IFACE_PCM, 3, "ELD");
        assert_eq!((eld.iface, eld.device), (3, 3));
        assert_eq!(ElemId::by_numid(7).numid, 7);
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

    #[test]
    fn db_range_spans_the_control() {
        // -63.50 dB at 0, 0.50 dB a step, 127 steps: 0 dB at the top
        let s = DbScale { min_cdb: -6350, step_cdb: 50, mute_at_min: false };
        assert_eq!(s.range_db(0, 127), (-63.5, 0.0));
    }

    #[test]
    fn eld_monitor_name_is_read_from_byte_20() {
        let mut eld = vec![0u8; 20];
        eld[4] = 0x20 | 5; // CEA_EDID_ver 1, MNL 5
        eld.extend_from_slice(b"DELL ");
        eld.extend_from_slice(&[0u8; 8]);
        assert_eq!(eld_monitor_name(&eld).as_deref(), Some("DELL"));
        // MNL 0: no name
        let mut none = vec![0u8; 24];
        none[4] = 0x20;
        assert_eq!(eld_monitor_name(&none), None);
        // truncated blob
        assert_eq!(eld_monitor_name(&eld[..22]), None);
        assert_eq!(eld_monitor_name(&[]), None);
    }

    #[test]
    fn jack_names_carry_the_hdmi_pcm_number() {
        assert_eq!(jack_pcm("HDMI/DP,pcm=3 Jack"), Some(3));
        assert_eq!(jack_pcm("HDMI/DP,pcm=10 Jack"), Some(10));
        assert_eq!(jack_pcm("Headphone Jack"), None);
    }

    #[test]
    fn only_value_changes_count_as_jack_events() {
        let ev = |mask| CtlEvent { numid: 1, name: "x".into(), mask };
        assert!(ev(EVENT_MASK_VALUE).is_value_change());
        assert!(ev(EVENT_MASK_VALUE | EVENT_MASK_INFO).is_value_change());
        assert!(!ev(EVENT_MASK_INFO).is_value_change());
        assert!(!ev(EVENT_MASK_REMOVE).is_value_change(), "removal sets every bit");
    }

    #[test]
    fn cstr_stops_at_nul() {
        assert_eq!(cstr(b"abc\0def"), "abc");
        assert_eq!(cstr(b"abc"), "abc");
    }
}
