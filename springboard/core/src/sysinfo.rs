//! System information: the hardware and software facts that About This Computer and the System Information app show.
//! Everything is read from /proc, /sys, /etc/os-release and the installed-app directory. Each reader takes its root
//! path, so the tests run on fixture trees and the parsers stay pure. `Roots::system()` names the real locations.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The real locations the readers use. Tests build their own.
#[derive(Clone, Debug)]
pub struct Roots {
    pub cpuinfo: PathBuf,
    pub meminfo: PathBuf,
    pub osrelease: PathBuf,
    pub os_release: PathBuf,
    pub dmi: PathBuf,
    pub pci: PathBuf,
    pub usb: PathBuf,
    pub drm: PathBuf,
    pub net: PathBuf,
    pub proc_net: PathBuf,
    pub power: PathBuf,
    pub block: PathBuf,
    pub apps: PathBuf,
}

impl Roots {
    pub fn system() -> Roots {
        Roots {
            cpuinfo: "/proc/cpuinfo".into(),
            meminfo: "/proc/meminfo".into(),
            osrelease: "/proc/sys/kernel/osrelease".into(),
            os_release: "/etc/os-release".into(),
            dmi: "/sys/class/dmi/id".into(),
            pci: "/sys/bus/pci/devices".into(),
            usb: "/sys/bus/usb/devices".into(),
            drm: "/sys/class/drm".into(),
            net: "/sys/class/net".into(),
            proc_net: "/proc/net".into(),
            power: "/sys/class/power_supply".into(),
            block: "/sys/block".into(),
            apps: "/var/lib/springboard/apps".into(),
        }
    }
}

/// Reads a small text file, trimmed. Missing or unreadable files give "" (some DMI fields need root).
fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Sorted entry names of a directory; empty when it is missing.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

/// A hexadecimal number as sysfs writes it ("0x8086" or "8086").
pub fn parse_hex(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

// ---------------------------------------------------------------- processor and memory

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cpu {
    /// The name the processor reports ("Intel(R) Core(TM) i7-…"), or the ARM "Hardware" line.
    pub model: String,
    pub vendor: String,
    /// Logical processors (one per "processor" stanza).
    pub logical: usize,
    /// The first reported clock, MHz; 0 when the kernel does not report one.
    pub mhz: f32,
}

pub fn parse_cpuinfo(text: &str) -> Cpu {
    let mut cpu = Cpu::default();
    let mut hardware = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" => cpu.logical += 1,
            "model name" if cpu.model.is_empty() => cpu.model = value.to_string(),
            "vendor_id" if cpu.vendor.is_empty() => cpu.vendor = value.to_string(),
            "cpu MHz" if cpu.mhz == 0.0 => cpu.mhz = value.parse().unwrap_or(0.0),
            "Hardware" if hardware.is_empty() => hardware = value.to_string(),
            _ => {}
        }
    }
    if cpu.model.is_empty() {
        cpu.model = hardware;
    }
    cpu
}

/// MemTotal in bytes from /proc/meminfo text.
pub fn parse_mem_total(text: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_ascii_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kb| kb * 1024)
}

// ---------------------------------------------------------------- firmware

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Firmware {
    pub vendor: String,
    pub product: String,
    pub version: String,
    pub board: String,
    pub bios: String,
    /// The form factor's name ("Laptop", "Desktop"), from the chassis type code.
    pub chassis: String,
    pub serial: String,
}

/// The chassis name for an SMBIOS chassis type code.
pub fn chassis_name(code: u32) -> &'static str {
    match code {
        3 | 4 | 5 | 6 | 7 => "Desktop",
        8 | 9 | 10 | 14 => "Laptop",
        11 => "Handheld",
        13 | 15 | 16 => "All in One",
        17 | 23 | 25 | 28 => "Server",
        30 | 31 | 32 => "Tablet",
        _ => "",
    }
}

pub fn read_firmware(dir: &Path) -> Firmware {
    let chassis_code = read_trim(&dir.join("chassis_type")).parse::<u32>().unwrap_or(0);
    Firmware {
        vendor: read_trim(&dir.join("sys_vendor")),
        product: read_trim(&dir.join("product_name")),
        version: read_trim(&dir.join("product_version")),
        board: read_trim(&dir.join("board_name")),
        bios: read_trim(&dir.join("bios_version")),
        chassis: chassis_name(chassis_code).to_string(),
        serial: read_trim(&dir.join("product_serial")),
    }
}

// ---------------------------------------------------------------- PCI

#[derive(Clone, Debug, PartialEq)]
pub struct PciDevice {
    /// The bus address, "0000:00:02.0".
    pub address: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// The 24-bit class code: base class in bits 16..24.
    pub class: u32,
    /// The bound kernel driver, "" when none is bound.
    pub driver: String,
}

impl PciDevice {
    pub fn base_class(&self) -> u32 {
        (self.class >> 16) & 0xff
    }

    /// The display name of the class ("Display controller").
    pub fn class_name(&self) -> &'static str {
        pci_class_name(self.base_class())
    }

    /// The vendor's name for the vendors this system meets most, or the hexadecimal ID.
    pub fn vendor_name(&self) -> String {
        vendor_name(self.vendor_id).map_or_else(|| format!("Vendor {:04x}", self.vendor_id), str::to_string)
    }
}

/// The PCI base class names (PCI Local Bus Specification base class codes).
pub fn pci_class_name(base: u32) -> &'static str {
    match base {
        0x00 => "Unclassified",
        0x01 => "Storage controller",
        0x02 => "Network controller",
        0x03 => "Display controller",
        0x04 => "Multimedia controller",
        0x05 => "Memory controller",
        0x06 => "Bridge",
        0x07 => "Communication controller",
        0x08 => "System peripheral",
        0x09 => "Input device",
        0x0b => "Processor",
        0x0c => "Serial bus controller",
        0x0d => "Wireless controller",
        0x0f => "Satellite controller",
        0x10 => "Encryption controller",
        0x11 => "Signal processing controller",
        _ => "Other device",
    }
}

/// A few vendor IDs by name. Unknown IDs show as hexadecimal.
pub fn vendor_name(id: u32) -> Option<&'static str> {
    Some(match id {
        0x8086 => "Intel",
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x10ec => "Realtek",
        0x14e4 => "Broadcom",
        0x168c => "Qualcomm Atheros",
        0x15b3 => "Mellanox",
        0x1b21 => "ASMedia",
        0x1af4 => "Red Hat (virtio)",
        0x144d => "Samsung",
        _ => return None,
    })
}

/// Reads `/sys/bus/pci/devices`, sorted by address. Devices without a readable vendor are skipped.
pub fn read_pci(dir: &Path) -> Vec<PciDevice> {
    entries(dir)
        .into_iter()
        .filter_map(|address| {
            let path = dir.join(&address);
            let vendor_id = parse_hex(&read_trim(&path.join("vendor")))?;
            let device_id = parse_hex(&read_trim(&path.join("device"))).unwrap_or(0);
            let class = parse_hex(&read_trim(&path.join("class"))).unwrap_or(0);
            let driver = fs::read_link(path.join("driver")).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            Some(PciDevice { address, vendor_id, device_id, class, driver })
        })
        .collect()
}

// ---------------------------------------------------------------- USB

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsbDevice {
    /// The port path, "1-1.2" (the sysfs name).
    pub port: String,
    pub vendor_id: u32,
    pub product_id: u32,
    pub product: String,
    pub manufacturer: String,
    /// Negotiated speed, Mb/s (sysfs writes 1.5 for low speed, which reads as 1); 0 when unknown.
    pub speed_mbps: u32,
}

/// Reads `/sys/bus/usb/devices`. Interfaces (names with ":") are skipped; only devices with an ID count.
pub fn read_usb(dir: &Path) -> Vec<UsbDevice> {
    entries(dir)
        .into_iter()
        .filter(|name| !name.contains(':'))
        .filter_map(|port| {
            let path = dir.join(&port);
            let vendor_id = parse_hex(&read_trim(&path.join("idVendor")))?;
            Some(UsbDevice {
                vendor_id,
                product_id: parse_hex(&read_trim(&path.join("idProduct"))).unwrap_or(0),
                product: read_trim(&path.join("product")),
                manufacturer: read_trim(&path.join("manufacturer")),
                speed_mbps: read_trim(&path.join("speed")).parse::<f32>().map_or(0, |v| v as u32),
                port,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- displays

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Display {
    /// The connector, "eDP-1" or "HDMI-A-1".
    pub connector: String,
    pub connected: bool,
    /// The preferred mode, "1920x1080"; "" when no mode is listed.
    pub mode: String,
}

/// Reads the display connectors under `/sys/class/drm` (entries named `cardN-CONNECTOR`).
pub fn read_displays(dir: &Path) -> Vec<Display> {
    entries(dir)
        .into_iter()
        .filter_map(|name| {
            let (_, connector) = name.split_once('-')?;
            if !name.starts_with("card") {
                return None;
            }
            let path = dir.join(&name);
            let status = read_trim(&path.join("status"));
            if status.is_empty() {
                return None;
            }
            let mode = fs::read_to_string(path.join("modes")).ok().and_then(|m| m.lines().next().map(|l| l.trim().to_string())).unwrap_or_default();
            Some(Display { connector: connector.to_string(), connected: status == "connected", mode })
        })
        .collect()
}

// ---------------------------------------------------------------- network

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetInterface {
    pub name: String,
    pub mac: String,
    /// "up", "down" or "unknown".
    pub state: String,
    pub wireless: bool,
    /// Global and site-local IPv6 addresses, as eight groups of hex (link-local ones are left out).
    pub addresses: Vec<String>,
    /// Whether this interface carries the default route.
    pub default_route: bool,
}

/// Reads `/sys/class/net` for the links, and `proc_net` (`/proc/net`) for their addresses and the default route.
/// The loopback device is left out.
pub fn read_net(dir: &Path, proc_net: &Path) -> Vec<NetInterface> {
    let v6 = parse_if_inet6(&read_trim_text(&proc_net.join("if_inet6")));
    let default = parse_default_route(&read_trim_text(&proc_net.join("route")));
    entries(dir)
        .into_iter()
        .filter(|name| name != "lo")
        .map(|name| {
            let path = dir.join(&name);
            let wireless = path.join("wireless").exists() || path.join("phy80211").exists();
            let addresses = v6.iter().filter(|(dev, _)| *dev == name).map(|(_, addr)| addr.clone()).collect();
            let default_route = default.as_deref() == Some(name.as_str());
            NetInterface { mac: read_trim(&path.join("address")), state: read_trim(&path.join("operstate")), wireless, addresses, default_route, name }
        })
        .collect()
}

/// `/proc/net/if_inet6` lines: "<32 hex digits> <index> <prefix> <scope> <flags> <device>". Returns (device, address)
/// for every address that is not link-local (scope 0x20).
pub fn parse_if_inet6(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (hex, scope, dev) = match fields.as_slice() {
                [hex, _, _, scope, _, dev] => (*hex, *scope, *dev),
                _ => return None,
            };
            if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || scope == "20" {
                return None;
            }
            let groups: Vec<String> = (0..8)
                .map(|i| {
                    let group = hex[i * 4..i * 4 + 4].trim_start_matches('0');
                    if group.is_empty() { "0".to_string() } else { group.to_string() }
                })
                .collect();
            Some((dev.to_string(), groups.join(":")))
        })
        .collect()
}

/// The device of the default route in `/proc/net/route` (destination and mask both 00000000).
pub fn parse_default_route(text: &str) -> Option<String> {
    text.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            [dev, dest, _, _, _, _, _, mask, ..] if *dest == "00000000" && *mask == "00000000" => Some(dev.to_string()),
            _ => None,
        }
    })
}

// ---------------------------------------------------------------- power

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerSource {
    pub name: String,
    /// "Battery", "Mains", "USB" and so on, from the sysfs type.
    pub kind: String,
    pub capacity_percent: Option<u32>,
    /// "Charging", "Discharging", "Full" and so on.
    pub status: String,
    /// The energy the battery holds when full, and when new (µWh in sysfs; kept as Wh here).
    pub full_wh: f32,
    pub design_wh: f32,
    pub cycles: Option<u32>,
    pub model: String,
}

impl PowerSource {
    /// Health: full capacity as a share of the design capacity, 0..=100. None without both values.
    pub fn health_percent(&self) -> Option<u32> {
        if self.design_wh <= 0.0 || self.full_wh <= 0.0 {
            return None;
        }
        Some(((self.full_wh / self.design_wh) * 100.0).round().min(100.0) as u32)
    }
}

fn micro_to_unit(text: &str) -> f32 {
    text.trim().parse::<f64>().map_or(0.0, |v| (v / 1_000_000.0) as f32)
}

/// Reads `/sys/class/power_supply`.
pub fn read_power(dir: &Path) -> Vec<PowerSource> {
    entries(dir)
        .into_iter()
        .filter_map(|name| {
            let path = dir.join(&name);
            let kind = read_trim(&path.join("type"));
            if kind.is_empty() {
                return None;
            }
            let energy = |file: &str| micro_to_unit(&read_trim(&path.join(file)));
            Some(PowerSource {
                capacity_percent: read_trim(&path.join("capacity")).parse().ok(),
                status: read_trim(&path.join("status")),
                full_wh: energy("energy_full"),
                design_wh: energy("energy_full_design"),
                cycles: read_trim(&path.join("cycle_count")).parse().ok().filter(|c| *c > 0),
                model: read_trim(&path.join("model_name")),
                kind,
                name,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- storage

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disk {
    pub name: String,
    pub model: String,
    pub size_bytes: u64,
    pub removable: bool,
    pub rotational: bool,
    /// The partition names ("nvme0n1p1"), in order.
    pub partitions: Vec<String>,
}

/// Reads `/sys/block`, leaving out virtual devices (loop, RAM, zram, device-mapper).
pub fn read_disks(dir: &Path) -> Vec<Disk> {
    entries(dir)
        .into_iter()
        .filter(|name| !["loop", "ram", "zram", "dm-", "md"].iter().any(|p| name.starts_with(p)))
        .map(|name| {
            let path = dir.join(&name);
            // sysfs counts 512-byte sectors whatever the device's block size
            let sectors: u64 = read_trim(&path.join("size")).parse().unwrap_or(0);
            let partitions = entries(&path).into_iter().filter(|p| p.starts_with(&name) && path.join(p).join("partition").exists()).collect();
            Disk {
                model: read_trim(&path.join("device/model")),
                size_bytes: sectors * 512,
                removable: read_trim(&path.join("removable")) == "1",
                rotational: read_trim(&path.join("queue/rotational")) == "1",
                partitions,
                name,
            }
        })
        .collect()
}

// ---------------------------------------------------------------- software

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstalledApp {
    pub id: String,
    /// The newest installed version.
    pub version: String,
}

/// PRETTY_NAME and VERSION_ID from os-release text.
pub fn parse_os_release(text: &str) -> (String, String) {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    (field("PRETTY_NAME"), field("VERSION_ID"))
}

/// Compares dotted version strings by their numbers ("1.10.0" is newer than "1.9.2").
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| v.split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<u64>>();
    let (pa, pb) = (parts(a), parts(b));
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let ord = pa.get(i).copied().unwrap_or(0).cmp(&pb.get(i).copied().unwrap_or(0));
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.cmp(b)
}

/// The installed apps: `<root>/<id>/<version>/`, one entry per id with its newest version. Dot-directories are skipped.
pub fn read_installed_apps(dir: &Path) -> Vec<InstalledApp> {
    entries(dir)
        .into_iter()
        .filter(|id| !id.starts_with('.'))
        .filter_map(|id| {
            let versions = entries(&dir.join(&id));
            let version = versions.into_iter().filter(|v| !v.starts_with('.')).max_by(|a, b| compare_versions(a, b))?;
            Some(InstalledApp { id, version })
        })
        .collect()
}

// ---------------------------------------------------------------- the report

/// Everything the System Information app shows, read once.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub firmware: Firmware,
    pub cpu: Cpu,
    pub memory_bytes: u64,
    pub pci: Vec<PciDevice>,
    pub usb: Vec<UsbDevice>,
    pub displays: Vec<Display>,
    pub net: Vec<NetInterface>,
    pub power: Vec<PowerSource>,
    pub disks: Vec<Disk>,
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
    pub apps: Vec<InstalledApp>,
}

impl Report {
    pub fn collect(roots: &Roots) -> Report {
        let (os_name, os_version) = parse_os_release(&read_trim_text(&roots.os_release));
        Report {
            firmware: read_firmware(&roots.dmi),
            cpu: parse_cpuinfo(&read_trim_text(&roots.cpuinfo)),
            memory_bytes: parse_mem_total(&read_trim_text(&roots.meminfo)),
            pci: read_pci(&roots.pci),
            usb: read_usb(&roots.usb),
            displays: read_displays(&roots.drm),
            net: read_net(&roots.net, &roots.proc_net),
            power: read_power(&roots.power),
            disks: read_disks(&roots.block),
            os_name,
            os_version,
            kernel: read_trim(&roots.osrelease),
            apps: read_installed_apps(&roots.apps),
        }
    }

    /// The graphics devices (PCI display class).
    pub fn graphics(&self) -> Vec<&PciDevice> {
        self.pci.iter().filter(|d| d.base_class() == 0x03).collect()
    }
}

fn read_trim_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// The short facts About This Computer shows, read from the real system once per process.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub model: String,
    pub processor: String,
    pub memory_bytes: u64,
    pub graphics: Vec<String>,
    pub serial: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
}

impl Summary {
    pub fn from_report(r: &Report) -> Summary {
        let model = if r.firmware.product.is_empty() { r.firmware.board.clone() } else { r.firmware.product.clone() };
        Summary {
            model,
            processor: r.cpu.model.clone(),
            memory_bytes: r.memory_bytes,
            graphics: r.graphics().iter().map(|d| format!("{} {}", d.vendor_name(), d.class_name())).collect(),
            serial: r.firmware.serial.clone(),
            os_name: r.os_name.clone(),
            os_version: r.os_version.clone(),
            kernel: r.kernel.clone(),
        }
    }

    /// The summary of this machine. Read on first use, then kept for the process.
    pub fn this_machine() -> &'static Summary {
        static SUMMARY: OnceLock<Summary> = OnceLock::new();
        SUMMARY.get_or_init(|| Summary::from_report(&Report::collect(&Roots::system())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Core(TM) i7-8565U\ncpu MHz\t\t: 1800.000\n\nprocessor\t: 1\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Core(TM) i7-8565U\ncpu MHz\t\t: 1800.000\n";

    #[test]
    fn cpuinfo_counts_logical_processors_and_takes_the_first_name() {
        let cpu = parse_cpuinfo(CPUINFO);
        assert_eq!(cpu.logical, 2);
        assert_eq!(cpu.model, "Intel(R) Core(TM) i7-8565U");
        assert_eq!(cpu.vendor, "GenuineIntel");
        assert_eq!(cpu.mhz, 1800.0);
    }

    #[test]
    fn arm_cpuinfo_uses_the_hardware_line() {
        let cpu = parse_cpuinfo("processor : 0\nHardware : BCM2835\n");
        assert_eq!(cpu.model, "BCM2835");
        assert_eq!(cpu.logical, 1);
        assert_eq!(parse_cpuinfo("").logical, 0);
    }

    #[test]
    fn memory_total_is_bytes() {
        assert_eq!(parse_mem_total("MemTotal:       16384000 kB\nMemFree: 1 kB\n"), 16_384_000 * 1024);
        assert_eq!(parse_mem_total("MemFree: 1 kB\n"), 0);
    }

    #[test]
    fn chassis_codes_map_to_form_factors() {
        assert_eq!(chassis_name(9), "Laptop");
        assert_eq!(chassis_name(3), "Desktop");
        assert_eq!(chassis_name(13), "All in One");
        assert_eq!(chassis_name(99), "");
    }

    #[test]
    fn pci_class_and_vendor_names() {
        let gpu = PciDevice { address: "0000:00:02.0".into(), vendor_id: 0x8086, device_id: 0x3ea0, class: 0x030000, driver: "i915".into() };
        assert_eq!(gpu.base_class(), 0x03);
        assert_eq!(gpu.class_name(), "Display controller");
        assert_eq!(gpu.vendor_name(), "Intel");
        let odd = PciDevice { vendor_id: 0x1234, ..gpu.clone() };
        assert_eq!(odd.vendor_name(), "Vendor 1234");
        assert_eq!(parse_hex("0x10de"), Some(0x10de));
        assert_eq!(parse_hex("zz"), None);
    }

    #[test]
    fn os_release_fields() {
        let (name, version) = parse_os_release("NAME=\"BifrOSt\"\nPRETTY_NAME=\"BifrOSt 0.6\"\nVERSION_ID=0.6.5\n");
        assert_eq!(name, "BifrOSt 0.6");
        assert_eq!(version, "0.6.5");
        assert_eq!(parse_os_release(""), (String::new(), String::new()));
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("1.10.0", "1.9.2"), Ordering::Greater);
        assert_eq!(compare_versions("2.0", "2.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.9", "0.10"), Ordering::Less);
    }

    #[test]
    fn ipv6_addresses_skip_link_local_and_compress_zero_groups() {
        let text = "fe800000000000000000000000000001 02 40 20 80 eth0\n\
                    20010db8000000000000000000000042 02 40 00 80 eth0\n\
                    20010db8000000010000000000000007 03 40 00 80 wlan0\n";
        assert_eq!(
            parse_if_inet6(text),
            vec![
                ("eth0".to_string(), "2001:db8:0:0:0:0:0:42".to_string()),
                ("wlan0".to_string(), "2001:db8:0:1:0:0:0:7".to_string()),
            ]
        );
        assert!(parse_if_inet6("not a table\n").is_empty());
    }

    #[test]
    fn the_default_route_names_its_device() {
        let text = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                    wlan0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n\
                    eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        assert_eq!(parse_default_route(text).as_deref(), Some("eth0"));
        assert_eq!(parse_default_route("Iface\tDestination\n"), None);
    }

    #[test]
    fn health_is_full_over_design() {
        let b = PowerSource { full_wh: 45.0, design_wh: 50.0, ..Default::default() };
        assert_eq!(b.health_percent(), Some(90));
        assert_eq!(PowerSource::default().health_percent(), None);
        assert_eq!(micro_to_unit("50000000"), 50.0);
    }

    #[test]
    fn firmware_reads_the_dmi_files() {
        let dir = std::env::temp_dir().join(format!("bifrost-sysinfo-dmi-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sys_vendor"), "Framework\n").unwrap();
        fs::write(dir.join("product_name"), "Laptop 13\n").unwrap();
        fs::write(dir.join("chassis_type"), "10\n").unwrap();
        let fw = read_firmware(&dir);
        assert_eq!(fw.vendor, "Framework");
        assert_eq!(fw.product, "Laptop 13");
        assert_eq!(fw.chassis, "Laptop");
        assert_eq!(fw.serial, "", "a missing DMI file reads as empty");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_apps_take_the_newest_version() {
        let dir = std::env::temp_dir().join(format!("bifrost-sysinfo-apps-{}", std::process::id()));
        for v in ["1.9.0", "1.10.0"] {
            fs::create_dir_all(dir.join("is.olibuijr.notes.desktop").join(v)).unwrap();
        }
        fs::create_dir_all(dir.join(".staging")).unwrap();
        let apps = read_installed_apps(&dir);
        assert_eq!(apps, vec![InstalledApp { id: "is.olibuijr.notes.desktop".into(), version: "1.10.0".into() }]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_prefers_the_product_name_over_the_board() {
        let mut r = Report::default();
        r.firmware.board = "B550".into();
        assert_eq!(Summary::from_report(&r).model, "B550");
        r.firmware.product = "Tower".into();
        assert_eq!(Summary::from_report(&r).model, "Tower");
    }
}
