//! External display control over DDC/CI (VESA Monitor Control Command Set): reading and setting a monitor's own
//! brightness and contrast through the I2C channel in its video cable. A laptop panel has a backlight in sysfs
//! (`bare::power`); an external monitor has none, so its brightness slider has to talk to the monitor itself.
//!
//! Three layers, the first two pure:
//! - the wire format: a set-feature packet, a get-feature request and the reader of the get-feature reply, with the
//!   checksums the protocol demands, and the percent/raw scaling of a feature value;
//! - [`get_vcp`], [`set_vcp`] and [`set_brightness_percent`] over any [`Bus`], so the exchange is tested with a scripted bus;
//! - discovery ([`external_bus_numbers`], reading `/sys/class/drm/card*-*/ddc`) and [`I2cDev`], the real bus on
//!   `/dev/i2c-N` (the monitor answers at slave address 0x37). [`set_external_brightness`] runs it for every
//!   connected external connector. A failure on one monitor never stops the others, and a machine without the
//!   i2c-dev module or without access to it simply reports zero monitors changed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::raw::{c_int, c_ulong};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Duration;

/// The 7-bit I2C address a monitor answers DDC/CI at.
pub const DISPLAY_ADDR: u16 = 0x37;
/// Source address the host puts first in every packet it sends.
const HOST_SRC: u8 = 0x51;
/// The monitor's address as the 8-bit write address; the protocol starts the checksum of a request with it.
const WRITE_ADDR8: u8 = 0x6E;
/// A reply starts with the monitor's 8-bit address and its checksum starts from the virtual host address.
const REPLY_SRC: u8 = 0x6E;
const REPLY_SEED: u8 = 0x50;
const OP_GET: u8 = 0x01;
const OP_GET_REPLY: u8 = 0x02;
const OP_SET: u8 = 0x03;
/// The length of a get-feature reply, from the source address to the checksum.
pub const GET_REPLY_LEN: usize = 11;
/// The pause the protocol asks for between a request and the next transfer, in milliseconds.
const PAUSE_MS: u64 = 50;

/// VCP feature code of the luminance (brightness).
pub const VCP_BRIGHTNESS: u8 = 0x10;
/// VCP feature code of the contrast.
pub const VCP_CONTRAST: u8 = 0x12;

/// What went wrong with an exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bus refused a transfer (the kind of the I/O error).
    Io(io::ErrorKind),
    /// The reply was shorter than a get-feature reply.
    Short,
    /// The reply's checksum did not match its bytes.
    BadChecksum,
    /// The reply is not a get-feature reply (a null message: the monitor was not ready, or another opcode).
    NotAReply,
    /// The monitor said it does not support this feature.
    Unsupported,
    /// The reply is for another feature than the one asked for.
    WrongFeature,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e.kind())
    }
}

/// One feature's value as a monitor reports it: the largest value it accepts and the present one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vcp {
    pub code: u8,
    pub max: u16,
    pub current: u16,
}

impl Vcp {
    /// The present value as a whole percent of the maximum (0 when the monitor reports a maximum of 0).
    pub fn percent(&self) -> u8 {
        to_percent(self.current, self.max)
    }
}

/// XOR of `bytes` starting from `seed`: the checksum of the protocol.
fn xor(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |a, b| a ^ b)
}

/// The bytes to write to the monitor to set feature `code` to `value` (the slave address is not part of them).
pub fn set_packet(code: u8, value: u16) -> [u8; 7] {
    let mut p = [HOST_SRC, 0x80 | 4, OP_SET, code, (value >> 8) as u8, value as u8, 0];
    p[6] = xor(WRITE_ADDR8, &p[..6]);
    p
}

/// The bytes to write to the monitor to ask for feature `code`.
pub fn get_packet(code: u8) -> [u8; 5] {
    let mut p = [HOST_SRC, 0x80 | 2, OP_GET, code, 0];
    p[4] = xor(WRITE_ADDR8, &p[..4]);
    p
}

/// Reads a get-feature reply for feature `code`.
pub fn parse_get_reply(reply: &[u8], code: u8) -> Result<Vcp, Error> {
    if reply.len() < GET_REPLY_LEN {
        return Err(Error::Short);
    }
    let r = &reply[..GET_REPLY_LEN];
    if r[0] != REPLY_SRC || r[1] != (0x80 | 8) || r[2] != OP_GET_REPLY {
        return Err(Error::NotAReply);
    }
    if xor(REPLY_SEED, &r[..GET_REPLY_LEN - 1]) != r[GET_REPLY_LEN - 1] {
        return Err(Error::BadChecksum);
    }
    if r[3] != 0 {
        return Err(Error::Unsupported);
    }
    if r[4] != code {
        return Err(Error::WrongFeature);
    }
    Ok(Vcp { code, max: u16::from_be_bytes([r[6], r[7]]), current: u16::from_be_bytes([r[8], r[9]]) })
}

/// A percent (clamped to 100) as a raw value of a feature whose maximum is `max`, rounded to the nearest.
pub fn to_raw(percent: u8, max: u16) -> u16 {
    let p = percent.min(100) as u32;
    ((p * max as u32 + 50) / 100) as u16
}

/// A raw value as a whole percent of `max`, rounded to the nearest and never above 100.
pub fn to_percent(current: u16, max: u16) -> u8 {
    if max == 0 {
        return 0;
    }
    ((current.min(max) as u32 * 100 + max as u32 / 2) / max as u32) as u8
}

/// The channel to one monitor: plain writes and reads at its slave address.
pub trait Bus {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Reads up to `buf.len()` bytes and answers how many arrived.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// Waits `ms` milliseconds between two transfers. A scripted bus overrides it to return at once.
    fn pause(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Asks the monitor for feature `code`.
pub fn get_vcp(bus: &mut dyn Bus, code: u8) -> Result<Vcp, Error> {
    bus.write(&get_packet(code))?;
    bus.pause(PAUSE_MS);
    let mut buf = [0u8; GET_REPLY_LEN];
    let n = bus.read(&mut buf)?;
    parse_get_reply(&buf[..n], code)
}

/// Sets feature `code` to the raw `value`. The protocol has no answer to a set, so the caller reads the feature
/// back when it needs to know it took.
pub fn set_vcp(bus: &mut dyn Bus, code: u8, value: u16) -> Result<(), Error> {
    bus.write(&set_packet(code, value))?;
    bus.pause(PAUSE_MS);
    Ok(())
}

/// Sets the brightness to `percent` of what the monitor accepts: reads the maximum, then writes the scaled value.
/// Answers the feature as written.
pub fn set_brightness_percent(bus: &mut dyn Bus, percent: u8) -> Result<Vcp, Error> {
    let now = get_vcp(bus, VCP_BRIGHTNESS)?;
    let value = to_raw(percent, now.max);
    set_vcp(bus, VCP_BRIGHTNESS, value)?;
    Ok(Vcp { current: value, ..now })
}

/// Connector kinds that are the machine's own panel or not a real output, so they have no external monitor on them.
const NOT_EXTERNAL: [&str; 4] = ["eDP", "LVDS", "DSI", "Virtual"];

/// The I2C adapter numbers of the connected external connectors under a DRM sysfs root (`/sys/class/drm`), sorted and
/// without repeats. A connector counts when its `status` reads `connected`, its kind is not an internal panel and its
/// `ddc` link points at an `i2c-N` adapter.
pub fn external_bus_numbers(drm_root: &Path) -> Vec<u32> {
    let Ok(rd) = fs::read_dir(drm_root) else { return Vec::new() };
    let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let Some(rest) = name.strip_prefix("card") else { continue };
        let Some((_, kind)) = rest.split_once('-') else { continue };
        if NOT_EXTERNAL.iter().any(|k| kind.starts_with(k)) {
            continue;
        }
        let dir = drm_root.join(&name);
        if fs::read_to_string(dir.join("status")).map(|s| s.trim() != "connected").unwrap_or(true) {
            continue;
        }
        if let Some(n) = adapter_number(&dir.join("ddc")) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.sort_unstable();
    out
}

/// The N of the `i2c-N` adapter a connector's `ddc` entry stands for: the name its link points at, else the
/// `i2c-dev/i2c-N` entry inside the directory.
fn adapter_number(ddc: &Path) -> Option<u32> {
    let from_name = |s: &str| s.strip_prefix("i2c-")?.parse::<u32>().ok();
    if let Some(n) = fs::read_link(ddc).ok().and_then(|t| t.file_name().and_then(|f| from_name(&f.to_string_lossy()))) {
        return Some(n);
    }
    fs::read_dir(ddc.join("i2c-dev")).ok()?.flatten().find_map(|e| from_name(&e.file_name().to_string_lossy()))
}

const I2C_SLAVE: c_ulong = 0x0703;

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// The real bus: `/dev/i2c-N` with the slave address set to the monitor's.
pub struct I2cDev {
    file: File,
}

impl I2cDev {
    /// Opens adapter `n` for the monitor at [`DISPLAY_ADDR`].
    pub fn open(n: u32) -> io::Result<I2cDev> {
        let file = OpenOptions::new().read(true).write(true).open(format!("/dev/i2c-{n}"))?;
        // SAFETY: `file` is an open descriptor and I2C_SLAVE takes the address as its integer argument.
        let rc = unsafe { ioctl(file.as_raw_fd(), I2C_SLAVE, DISPLAY_ADDR as c_ulong) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(I2cDev { file })
    }
}

impl Bus for I2cDev {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

/// Sets the brightness of every connected external monitor to `percent`. Answers how many took it.
pub fn set_external_brightness(percent: u8) -> usize {
    external_bus_numbers(Path::new("/sys/class/drm"))
        .into_iter()
        .filter(|&n| I2cDev::open(n).map_err(Error::from).and_then(|mut bus| set_brightness_percent(&mut bus, percent)).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::fs::symlink;

    /// A bus that records writes and answers reads from a script.
    #[derive(Default)]
    struct Script {
        writes: Vec<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
        pauses: usize,
    }

    impl Bus for Script {
        fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let r = self.replies.pop_front().ok_or(io::ErrorKind::TimedOut)?;
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            Ok(n)
        }
        fn pause(&mut self, _ms: u64) {
            self.pauses += 1;
        }
    }

    /// A get-feature reply with a correct checksum.
    fn reply(result: u8, code: u8, max: u16, cur: u16) -> Vec<u8> {
        let mut r = vec![0x6E, 0x88, 0x02, result, code, 0x00, (max >> 8) as u8, max as u8, (cur >> 8) as u8, cur as u8];
        let sum = r.iter().fold(0x50u8, |a, b| a ^ b);
        r.push(sum);
        r
    }

    #[test]
    fn the_set_packet_has_the_protocol_bytes_and_checksum() {
        assert_eq!(set_packet(0x10, 50), [0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9A]);
        assert_eq!(set_packet(0x12, 0x0102)[4..6], [0x01, 0x02]);
    }

    #[test]
    fn the_get_request_has_the_protocol_bytes_and_checksum() {
        assert_eq!(get_packet(0x10), [0x51, 0x82, 0x01, 0x10, 0xAC]);
    }

    #[test]
    fn a_good_reply_gives_the_maximum_and_the_present_value() {
        let r = reply(0, 0x10, 100, 50);
        assert_eq!(r[10], 0xF2);
        let v = parse_get_reply(&r, 0x10).unwrap();
        assert_eq!(v, Vcp { code: 0x10, max: 100, current: 50 });
        assert_eq!(v.percent(), 50);
    }

    #[test]
    fn a_bad_reply_is_refused_for_its_reason() {
        let mut r = reply(0, 0x10, 100, 50);
        assert_eq!(parse_get_reply(&r[..5], 0x10), Err(Error::Short));
        r[10] ^= 1;
        assert_eq!(parse_get_reply(&r, 0x10), Err(Error::BadChecksum));
        assert_eq!(parse_get_reply(&reply(1, 0x10, 0, 0), 0x10), Err(Error::Unsupported));
        assert_eq!(parse_get_reply(&reply(0, 0x12, 100, 5), 0x10), Err(Error::WrongFeature));
        let mut null = vec![0x6E, 0x80];
        null.resize(GET_REPLY_LEN, 0);
        assert_eq!(parse_get_reply(&null, 0x10), Err(Error::NotAReply));
    }

    #[test]
    fn percent_and_raw_round_to_the_nearest_and_stay_in_range() {
        assert_eq!(to_raw(0, 255), 0);
        assert_eq!(to_raw(100, 255), 255);
        assert_eq!(to_raw(50, 255), 128);
        assert_eq!(to_raw(250, 80), 80);
        assert_eq!(to_percent(128, 255), 50);
        assert_eq!(to_percent(300, 100), 100);
        assert_eq!(to_percent(5, 0), 0);
    }

    #[test]
    fn get_writes_the_request_waits_and_reads_the_reply() {
        let mut bus = Script::default();
        bus.replies.push_back(reply(0, 0x10, 100, 30));
        let v = get_vcp(&mut bus, VCP_BRIGHTNESS).unwrap();
        assert_eq!(v.current, 30);
        assert_eq!(bus.writes, vec![get_packet(0x10).to_vec()]);
        assert_eq!(bus.pauses, 1);
    }

    #[test]
    fn a_silent_monitor_is_an_io_error() {
        let mut bus = Script::default();
        assert_eq!(get_vcp(&mut bus, VCP_BRIGHTNESS), Err(Error::Io(io::ErrorKind::TimedOut)));
    }

    #[test]
    fn setting_brightness_reads_the_maximum_then_writes_the_scaled_value() {
        let mut bus = Script::default();
        bus.replies.push_back(reply(0, 0x10, 200, 10));
        let v = set_brightness_percent(&mut bus, 25).unwrap();
        assert_eq!(v, Vcp { code: 0x10, max: 200, current: 50 });
        assert_eq!(bus.writes.len(), 2);
        assert_eq!(bus.writes[1], set_packet(0x10, 50).to_vec());
    }

    fn connector(root: &Path, name: &str, status: &str, adapter: Option<&str>) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("status"), format!("{status}\n")).unwrap();
        if let Some(a) = adapter {
            symlink(format!("../../../i2c-{a}"), dir.join("ddc")).unwrap();
        }
    }

    #[test]
    fn discovery_takes_connected_external_connectors_with_an_adapter() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        connector(r, "card0-eDP-1", "connected", Some("1"));
        connector(r, "card0-DP-1", "connected", Some("7"));
        connector(r, "card0-HDMI-A-1", "connected", Some("5"));
        connector(r, "card0-DP-2", "disconnected", Some("8"));
        connector(r, "card0-DP-3", "connected", None);
        connector(r, "card1-HDMI-A-1", "connected", Some("5"));
        fs::create_dir_all(r.join("renderD128")).unwrap();
        assert_eq!(external_bus_numbers(r), vec![5, 7]);
    }

    #[test]
    fn discovery_falls_back_to_the_i2c_dev_entry_and_a_missing_root_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        connector(r, "card0-DP-1", "connected", None);
        fs::create_dir_all(r.join("card0-DP-1/ddc/i2c-dev/i2c-9")).unwrap();
        assert_eq!(external_bus_numbers(r), vec![9]);
        assert!(external_bus_numbers(&r.join("absent")).is_empty());
    }
}
