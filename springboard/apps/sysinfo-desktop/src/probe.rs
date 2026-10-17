//! The hardware and software facts the shared `bifrost_core::sysinfo` report does not carry: processor topology and
//! caches, memory modules, video memory, USB power, mounted volumes, network addressing, secure boot, app details and
//! the hardware classes the report has no list for (audio, Bluetooth, cameras, printers, Thunderbolt, NVMe, SATA).
//!
//! Every reader takes the directory or file it reads, and the parsers are pure functions over text or bytes, so the
//! tests run on fixtures. `Hardware::collect` reads the real locations once.

use bifrost_core::sysinfo::UsbDevice;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Reads a small text file, trimmed. A missing or unreadable file gives "".
fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Sorted entry names of a directory; empty when it is missing.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

/// The last component of the target of a symbolic link ("" when it is not one).
fn link_name(path: &Path) -> String {
    fs::read_link(path).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
}

// ---------------------------------------------------------------- processor

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheInfo {
    pub level: u32,
    /// "Data", "Instruction" or "Unified".
    pub kind: String,
    pub size_kb: u64,
}

impl CacheInfo {
    /// "L1 data 32 KB", "L3 16 MB".
    pub fn label(&self) -> String {
        let kind = match self.kind.as_str() {
            "Data" => " data",
            "Instruction" => " instruction",
            _ => "",
        };
        format!("L{}{kind} {}", self.level, kb_label(self.size_kb))
    }
}

/// A size given in kilobytes: whole megabytes from 1024 up when they divide evenly.
pub fn kb_label(kb: u64) -> String {
    if kb >= 1024 && kb % 1024 == 0 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{kb} KB")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuDetail {
    /// Physical cores; 0 when the kernel does not report the topology (some ARM kernels).
    pub cores: usize,
    pub packages: usize,
    pub caches: Vec<CacheInfo>,
    /// The slowest and the fastest core right now, MHz; 0 when `/proc/cpuinfo` has no clock.
    pub now_min_mhz: u32,
    pub now_max_mhz: u32,
    /// The highest clock the processor supports (cpufreq), MHz; 0 when unknown.
    pub rated_max_mhz: u32,
    /// The clock of each logical processor right now, as (processor number, MHz), from cpufreq. Empty when cpufreq is
    /// not there.
    pub core_clocks_mhz: Vec<(u32, u32)>,
    /// Performance and efficiency cores, counted as physical cores. None when every core is alike or the kernel does
    /// not say which is which.
    pub core_kinds: Option<(usize, usize)>,
}

/// The physical cores and packages in `/proc/cpuinfo` text: the distinct `(physical id, core id)` pairs. Logical
/// processors that share a core (hyper-threads) count once.
pub fn parse_cpu_topology(text: &str) -> (usize, usize) {
    let mut cores: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut packages: BTreeSet<u32> = BTreeSet::new();
    let mut physical: Option<u32> = None;
    let mut core: Option<u32> = None;
    let mut started = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" => {
                if started {
                    note_core(physical, core, &mut cores, &mut packages);
                }
                started = true;
                physical = None;
                core = None;
            }
            "physical id" => physical = value.parse().ok(),
            "core id" => core = value.parse().ok(),
            _ => {}
        }
    }
    if started {
        note_core(physical, core, &mut cores, &mut packages);
    }
    (cores.len(), packages.len())
}

fn note_core(physical: Option<u32>, core: Option<u32>, cores: &mut BTreeSet<(u32, u32)>, packages: &mut BTreeSet<u32>) {
    if let (Some(p), Some(c)) = (physical, core) {
        cores.insert((p, c));
        packages.insert(p);
    }
}

/// The `cpu MHz` of every logical processor in `/proc/cpuinfo` text, rounded to whole MHz.
pub fn parse_cpuinfo_mhz(text: &str) -> Vec<u32> {
    text.lines()
        .filter_map(|l| {
            let (key, value) = l.split_once(':')?;
            (key.trim() == "cpu MHz").then(|| value.trim().parse::<f32>().ok()).flatten().map(|v| v.round() as u32)
        })
        .collect()
}

/// A cache size as sysfs writes it: "32K", "1024K", "8M", "1G".
pub fn parse_cache_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let unit = text.chars().last()?;
    let n: u64 = text[..text.len() - unit.len_utf8()].parse().ok()?;
    match unit {
        'K' | 'k' => Some(n),
        'M' | 'm' => Some(n * 1024),
        'G' | 'g' => Some(n * 1024 * 1024),
        _ => text.parse().ok().map(|bytes: u64| bytes / 1024),
    }
}

/// The caches of one processor (`.../cpu0/cache/index*`), by level and kind.
pub fn read_caches(dir: &Path) -> Vec<CacheInfo> {
    let mut caches: Vec<CacheInfo> = entries(dir)
        .into_iter()
        .filter(|n| n.starts_with("index"))
        .filter_map(|n| {
            let p = dir.join(n);
            let level = read_trim(&p.join("level")).parse().ok()?;
            let size_kb = parse_cache_size(&read_trim(&p.join("size")))?;
            Some(CacheInfo { level, kind: read_trim(&p.join("type")), size_kb })
        })
        .collect();
    caches.sort_by(|a, b| a.level.cmp(&b.level).then(a.kind.cmp(&b.kind)));
    caches.dedup();
    caches
}

/// A CPU list as the kernel writes it in sysfs ("0-7,16,18-19") in processor order. Malformed parts are left out.
pub fn parse_cpu_list(text: &str) -> Vec<u32> {
    let mut out: BTreeSet<u32> = BTreeSet::new();
    for part in text.trim().split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((first, last)) => {
                if let (Ok(first), Ok(last)) = (first.trim().parse::<u32>(), last.trim().parse::<u32>()) {
                    if first <= last && last - first < 4096 {
                        out.extend(first..=last);
                    }
                }
            }
            None => {
                if let Ok(n) = part.parse::<u32>() {
                    out.insert(n);
                }
            }
        }
    }
    out.into_iter().collect()
}

/// The processor numbers under `cpu_root` (`cpuN` directories), with the value of `file` in each one trimmed.
fn per_cpu(cpu_root: &Path, file: &str) -> Vec<(u32, String)> {
    entries(cpu_root)
        .into_iter()
        .filter_map(|n| {
            let index: u32 = n.strip_prefix("cpu")?.parse().ok()?;
            let value = read_trim(&cpu_root.join(&n).join(file));
            (!value.is_empty()).then_some((index, value))
        })
        .collect()
}

/// The clock of each processor now: `cpuN/cpufreq/scaling_cur_freq` (kHz) as (processor, MHz).
pub fn read_core_clocks(cpu_root: &Path) -> Vec<(u32, u32)> {
    per_cpu(cpu_root, "cpufreq/scaling_cur_freq")
        .into_iter()
        .filter_map(|(index, khz)| khz.parse::<u32>().ok().map(|khz| (index, khz / 1000)))
        .collect()
}

/// The physical cores among the processors `cpus`: the distinct (package, core) pairs from each processor's
/// `topology` directory. A processor without a core number counts as a core of its own.
pub fn count_physical_cores(cpu_root: &Path, cpus: &[u32]) -> usize {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for c in cpus {
        let topology = cpu_root.join(format!("cpu{c}")).join("topology");
        let core = read_trim(&topology.join("core_id"));
        let key = if core.is_empty() { (String::new(), format!("cpu{c}")) } else { (read_trim(&topology.join("physical_package_id")), core) };
        seen.insert(key);
    }
    seen.len()
}

/// The performance and efficiency cores. Intel hybrid processors list each kind under `devices/cpu_core` and
/// `devices/cpu_atom`. Other processors give each core a `cpu_capacity`: the cores with the largest capacity are the
/// performance cores. None for one kind of core, or when neither source says.
pub fn read_core_kinds(devices: &Path, cpu_root: &Path) -> Option<(usize, usize)> {
    let perf = parse_cpu_list(&read_trim_text(&devices.join("cpu_core/cpus")));
    let eff = parse_cpu_list(&read_trim_text(&devices.join("cpu_atom/cpus")));
    if !perf.is_empty() && !eff.is_empty() {
        return Some((count_physical_cores(cpu_root, &perf), count_physical_cores(cpu_root, &eff)));
    }
    let capacities: Vec<(u32, u32)> = per_cpu(cpu_root, "cpu_capacity").into_iter().filter_map(|(i, c)| c.parse::<u32>().ok().map(|c| (i, c))).collect();
    let top = capacities.iter().map(|(_, c)| *c).max()?;
    let low = capacities.iter().map(|(_, c)| *c).min()?;
    if top == low {
        return None;
    }
    let perf: Vec<u32> = capacities.iter().filter(|(_, c)| *c == top).map(|(i, _)| *i).collect();
    let eff: Vec<u32> = capacities.iter().filter(|(_, c)| *c != top).map(|(i, _)| *i).collect();
    Some((count_physical_cores(cpu_root, &perf), count_physical_cores(cpu_root, &eff)))
}

pub fn read_cpu(cpuinfo_text: &str, cache_dir: &Path, cpufreq_dir: &Path, cpu_root: &Path, devices: &Path) -> CpuDetail {
    let (cores, packages) = parse_cpu_topology(cpuinfo_text);
    let clocks = parse_cpuinfo_mhz(cpuinfo_text);
    let rated_khz: u32 = read_trim(&cpufreq_dir.join("cpuinfo_max_freq")).parse().unwrap_or(0);
    CpuDetail {
        cores,
        packages,
        caches: read_caches(cache_dir),
        now_min_mhz: clocks.iter().copied().min().unwrap_or(0),
        now_max_mhz: clocks.iter().copied().max().unwrap_or(0),
        rated_max_mhz: rated_khz / 1000,
        core_clocks_mhz: read_core_clocks(cpu_root),
        core_kinds: read_core_kinds(devices, cpu_root),
    }
}

// ---------------------------------------------------------------- memory

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dimm {
    /// The slot name the firmware gives ("DIMM A1", "ChannelA-DIMM0").
    pub locator: String,
    /// 0 for an empty slot.
    pub size_mb: u64,
    pub kind: String,
    pub speed_mts: u32,
    pub configured_mts: u32,
    pub manufacturer: String,
    pub part: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemDetail {
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub dimms: Vec<Dimm>,
}

fn meminfo_bytes(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_ascii_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kb| kb * 1024)
}

/// MemAvailable and the swap sizes of `/proc/meminfo` text, in bytes (the module list is not part of it).
pub fn parse_meminfo(text: &str) -> MemDetail {
    MemDetail { available: meminfo_bytes(text, "MemAvailable"), swap_total: meminfo_bytes(text, "SwapTotal"), swap_free: meminfo_bytes(text, "SwapFree"), dimms: Vec::new() }
}

/// The memory type name for an SMBIOS memory device type code.
pub fn memory_type_name(code: u8) -> &'static str {
    match code {
        0x12 => "DDR2",
        0x13 => "DDR2 FB-DIMM",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x20 => "HBM",
        0x21 => "HBM2",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x24 => "HBM3",
        _ => "",
    }
}

/// The memory devices (SMBIOS structure type 17) in a raw structure table such as `/sys/firmware/dmi/tables/DMI`.
/// Parsing stops at the end-of-table structure or when the data runs out; a truncated structure is left out.
pub fn parse_smbios_memory(table: &[u8]) -> Vec<Dimm> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 4 <= table.len() {
        let kind = table[at];
        let len = table[at + 1] as usize;
        if len < 4 || at + len > table.len() {
            break;
        }
        let formatted = &table[at..at + len];
        // the string set follows the formatted area: NUL-terminated strings, closed by an empty one
        let mut p = at + len;
        let mut strings: Vec<String> = Vec::new();
        if table.get(p) == Some(&0) && table.get(p + 1) == Some(&0) {
            p += 2;
        } else {
            while p < table.len() {
                let start = p;
                while p < table.len() && table[p] != 0 {
                    p += 1;
                }
                strings.push(String::from_utf8_lossy(&table[start..p]).trim().to_string());
                p += 1;
                if table.get(p) == Some(&0) {
                    p += 1;
                    break;
                }
            }
        }
        if kind == 127 {
            break;
        }
        if kind == 17 {
            if let Some(d) = smbios_dimm(formatted, &strings) {
                out.push(d);
            }
        }
        at = p;
    }
    out
}

fn smbios_dimm(f: &[u8], strings: &[String]) -> Option<Dimm> {
    let u16_at = |o: usize| f.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let string = |o: usize| -> String {
        match f.get(o) {
            Some(&i) if i > 0 => strings.get(i as usize - 1).cloned().unwrap_or_default(),
            _ => String::new(),
        }
    };
    let raw = u16_at(0x0C)?;
    let size_mb = match raw {
        0 | 0xFFFF => 0,
        0x7FFF => f.get(0x1C..0x20).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64),
        r if r & 0x8000 != 0 => (r & 0x7FFF) as u64 / 1024,
        r => r as u64,
    };
    Some(Dimm {
        locator: string(0x10),
        size_mb,
        kind: f.get(0x12).map_or("", |c| memory_type_name(*c)).to_string(),
        speed_mts: u16_at(0x15).filter(|s| *s != 0xFFFF).unwrap_or(0) as u32,
        configured_mts: u16_at(0x20).filter(|s| *s != 0xFFFF).unwrap_or(0) as u32,
        manufacturer: string(0x17),
        part: string(0x1A),
    })
}

// ---------------------------------------------------------------- graphics memory

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuMemory {
    /// The PCI address of the graphics device ("0000:01:00.0").
    pub address: String,
    pub vram_bytes: u64,
}

/// The video memory of each DRM card (`cardN/device/mem_info_vram_total`), keyed by the PCI address behind it.
pub fn read_gpu_memory(drm: &Path) -> Vec<GpuMemory> {
    entries(drm)
        .into_iter()
        .filter(|n| n.starts_with("card") && !n.contains('-'))
        .filter_map(|n| {
            let device = drm.join(&n).join("device");
            let vram_bytes: u64 = read_trim(&device.join("mem_info_vram_total")).parse().ok()?;
            Some(GpuMemory { address: link_name(&device), vram_bytes })
        })
        .collect()
}

// ---------------------------------------------------------------- USB

/// Where a port sits: the bus number and the hub path below it. The root hub `usb3` is `(3, [])`, the device `3-1.2`
/// is `(3, [1, 2])`; a name that is neither sorts last.
pub fn usb_position(port: &str) -> (u32, Vec<u32>) {
    if let Some(bus) = port.strip_prefix("usb") {
        return (bus.parse().unwrap_or(u32::MAX), Vec::new());
    }
    match port.split_once('-') {
        Some((bus, path)) => (bus.parse().unwrap_or(u32::MAX), path.split('.').filter_map(|p| p.parse().ok()).collect()),
        None => (u32::MAX, Vec::new()),
    }
}

/// The devices in tree order (bus by bus, a hub before what hangs off it) with their depth: a root hub is 0, a device on
/// it 1, a device behind a hub 2.
pub fn usb_tree(devices: &[UsbDevice]) -> Vec<(usize, &UsbDevice)> {
    let mut list: Vec<((u32, Vec<u32>), &UsbDevice)> = devices.iter().map(|d| (usb_position(&d.port), d)).collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list.into_iter().map(|((_, path), d)| (path.len(), d)).collect()
}

/// The hub a device hangs from: "3-1.2" under "3-1", "3-1" under the root hub "usb3". "" for a root hub.
pub fn usb_parent(port: &str) -> String {
    if port.starts_with("usb") {
        return String::new();
    }
    match port.rsplit_once('.') {
        Some((parent, _)) => parent.to_string(),
        None => port.split_once('-').map(|(bus, _)| format!("usb{bus}")).unwrap_or_default(),
    }
}

/// `bMaxPower` of every USB device, as "500 mA"; a port without one is left out.
pub fn read_usb_power(dir: &Path) -> Vec<(String, String)> {
    entries(dir)
        .into_iter()
        .filter(|n| !n.contains(':'))
        .filter_map(|n| {
            let text = read_trim(&dir.join(&n).join("bMaxPower"));
            let ma = text.trim_end_matches("mA").trim();
            (!ma.is_empty() && ma.bytes().all(|b| b.is_ascii_digit())).then(|| (n, format!("{ma} mA")))
        })
        .collect()
}

/// Printers on USB: the devices that carry an interface of class 07 (printer), named by their product string.
pub fn read_usb_printers(dir: &Path) -> Vec<Printer> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for name in entries(dir) {
        let Some((port, _)) = name.split_once(':') else { continue };
        if read_trim(&dir.join(&name).join("bInterfaceClass")) != "07" || !seen.insert(port.to_string()) {
            continue;
        }
        let product = read_trim(&dir.join(port).join("product"));
        out.push(Printer { name: if product.is_empty() { format!("USB printer {port}") } else { product }, uri: format!("usb:{port}"), state: "Connected".to_string() });
    }
    out
}

// ---------------------------------------------------------------- volumes

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    pub device: String,
    pub mount: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountEntry {
    pub device: String,
    pub mount: String,
    pub fs: String,
}

/// `\040` and friends in a `/proc/mounts` field (the kernel writes a space as `\040`).
pub fn unescape_mount(field: &str) -> String {
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mounts of real storage in `/proc/mounts` text: block devices (not loop devices) and network file systems. A
/// device mounted twice at the same place is listed once.
pub fn parse_mounts(text: &str) -> Vec<MountEntry> {
    let mut out: Vec<MountEntry> = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(device), Some(mount), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
        let network = matches!(fs, "nfs" | "nfs4" | "cifs" | "smb3" | "fuse.sshfs");
        let block = device.starts_with("/dev/") && !device.starts_with("/dev/loop");
        if !(block || network) {
            continue;
        }
        let entry = MountEntry { device: unescape_mount(device), mount: unescape_mount(mount), fs: fs.to_string() };
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out
}

pub fn read_volumes(mounts_text: &str) -> Vec<Volume> {
    parse_mounts(mounts_text)
        .into_iter()
        .map(|m| {
            let capacity = bifrost_core::volume::capacity(Path::new(&m.mount)).ok();
            Volume { total: capacity.map_or(0, |c| c.total), free: capacity.map_or(0, |c| c.available), device: m.device, mount: m.mount, fs: m.fs }
        })
        .collect()
}

// ---------------------------------------------------------------- network

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetDetail {
    pub name: String,
    /// Backed by a device; false for bridges, tunnels and other virtual links.
    pub physical: bool,
    /// Link speed in Mb/s; 0 when the link reports none (Wi-Fi, a down link).
    pub speed_mbps: u32,
    pub ipv4: Vec<String>,
    pub gateway: String,
    /// Wi-Fi link quality (0 to 100) and signal level in dBm.
    pub signal: Option<(u32, i32)>,
}

/// A hexadecimal address of `/proc/net/route` as a number in network order: `0101A8C0` is 192.168.1.1.
pub fn route_addr(hex: &str) -> Option<u32> {
    u32::from_str_radix(hex, 16).ok().map(u32::swap_bytes)
}

pub fn ipv4_string(addr: u32) -> String {
    let b = addr.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

pub fn parse_ipv4(text: &str) -> Option<u32> {
    let mut parts = text.trim().split('.');
    let mut bytes = [0u8; 4];
    for b in bytes.iter_mut() {
        *b = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then(|| u32::from_be_bytes(bytes))
}

/// The device and the gateway address of the default route in `/proc/net/route` text.
pub fn parse_gateway(text: &str) -> Option<(String, String)> {
    text.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            [dev, dest, gateway, _, _, _, _, mask, ..] if *dest == "00000000" && *mask == "00000000" => Some((dev.to_string(), ipv4_string(route_addr(gateway)?))),
            _ => None,
        }
    })
}

/// The on-link networks in `/proc/net/route` text: device, network and mask (network order). Default routes are left out.
pub fn parse_route_networks(text: &str) -> Vec<(String, u32, u32)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.as_slice() {
                [dev, dest, _, _, _, _, _, mask, ..] => {
                    let (dest, mask) = (route_addr(dest)?, route_addr(mask)?);
                    (mask != 0).then(|| (dev.to_string(), dest, mask))
                }
                _ => None,
            }
        })
        .collect()
}

/// The addresses this machine owns in `/proc/net/fib_trie` text (the `/32 host LOCAL` entries), without loopback.
pub fn parse_fib_local(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(addr) = line.strip_prefix("|--") {
            last = addr.trim().to_string();
        } else if line.starts_with("/32 host LOCAL") && !last.is_empty() && !last.starts_with("127.") && !out.contains(&last) {
            out.push(last.clone());
        }
    }
    out
}

/// Pairs each local address with the device whose on-link network holds it (the narrowest network wins).
pub fn assign_ipv4(addrs: &[String], networks: &[(String, u32, u32)]) -> Vec<(String, String)> {
    addrs
        .iter()
        .filter_map(|a| {
            let n = parse_ipv4(a)?;
            networks
                .iter()
                .filter(|(_, dest, mask)| n & mask == *dest)
                .max_by_key(|(_, _, mask)| mask.count_ones())
                .map(|(dev, _, _)| (dev.clone(), a.clone()))
        })
        .collect()
}

/// The name servers of `resolv.conf` text, in order.
pub fn parse_resolv(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("nameserver").map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()
}

/// Wi-Fi link quality and level from `/proc/net/wireless` text: device, quality (0 to 100) and dBm.
pub fn parse_wireless(text: &str) -> Vec<(String, u32, i32)> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (dev, rest) = line.split_once(':')?;
            let f: Vec<&str> = rest.split_whitespace().collect();
            let num = |s: &str| s.trim_end_matches('.').parse::<f32>().ok();
            let quality = num(*f.get(1)?)?;
            let level = num(*f.get(2)?)?;
            // quality is reported out of 70 by most drivers
            Some((dev.trim().to_string(), ((quality / 70.0) * 100.0).round().clamp(0.0, 100.0) as u32, level as i32))
        })
        .collect()
}

pub fn read_net(net: &Path, proc_net: &Path) -> Vec<NetDetail> {
    let route = read_trim_text(&proc_net.join("route"));
    let addrs = assign_ipv4(&parse_fib_local(&read_trim_text(&proc_net.join("fib_trie"))), &parse_route_networks(&route));
    let gateway = parse_gateway(&route);
    let wireless = parse_wireless(&read_trim_text(&proc_net.join("wireless")));
    entries(net)
        .into_iter()
        .filter(|n| n != "lo")
        .map(|name| {
            let path = net.join(&name);
            NetDetail {
                physical: path.join("device").exists(),
                speed_mbps: read_trim(&path.join("speed")).parse::<i64>().ok().filter(|s| *s > 0).map_or(0, |s| s as u32),
                ipv4: addrs.iter().filter(|(dev, _)| *dev == name).map(|(_, a)| a.clone()).collect(),
                gateway: gateway.iter().filter(|(dev, _)| *dev == name).map(|(_, g)| g.clone()).next().unwrap_or_default(),
                signal: wireless.iter().find(|(dev, _, _)| *dev == name).map(|(_, q, l)| (*q, *l)),
                name,
            }
        })
        .collect()
}

/// The name servers: `/etc/resolv.conf`, or, when that only names the local stub resolver, the upstream list the
/// resolver keeps.
pub fn read_dns(resolv: &Path, upstream: &Path) -> Vec<String> {
    let servers = parse_resolv(&read_trim_text(resolv));
    if !servers.is_empty() && servers.iter().all(|s| s.starts_with("127.")) {
        let real = parse_resolv(&read_trim_text(upstream));
        if !real.is_empty() {
            return real;
        }
    }
    servers
}

fn read_trim_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

// ---------------------------------------------------------------- operating system

/// A field of `os-release` text ("BUILD_ID"), unquoted.
pub fn os_field(text: &str, key: &str) -> String {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string()).unwrap_or_default()
}

/// Secure boot from the EFI variable `SecureBoot-…`: four attribute bytes, then one data byte. `efi` is whether the
/// machine booted through EFI at all.
pub fn secure_boot_state(efi: bool, variable: Option<&[u8]>) -> &'static str {
    if !efi {
        return "Not supported (legacy BIOS boot)";
    }
    match variable.and_then(|v| v.get(4)) {
        Some(1) => "Enabled",
        Some(0) => "Disabled",
        _ => "Unknown",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppDetail {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Days since 1970-01-01 of the first install; 0 when the registry does not say.
    pub installed_day: i64,
}

impl AppDetail {
    /// "System", "Preinstalled" or "User" for the registry's kind.
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "system" | "service" => "System",
            "system-app" => "Preinstalled",
            _ => "User",
        }
    }
}

/// The apps in `apps/installed.json` text.
pub fn parse_registry(text: &str) -> Vec<AppDetail> {
    let Ok(j) = bifrost_core::json::parse(text) else { return Vec::new() };
    j.get("apps")
        .and_then(|a| a.as_array())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| {
            Some(AppDetail {
                id: a.str_field("id")?,
                name: a.str_field("name").unwrap_or_default(),
                kind: a.str_field("kind").unwrap_or_default(),
                installed_day: a.get("purchased").and_then(|p| p.as_u64()).unwrap_or(0) as i64,
            })
        })
        .collect()
}

/// A day number (days since 1970-01-01) as `YYYY-MM-DD`.
pub fn day_label(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------------------------------------------------------------- audio, Bluetooth, cameras, printers, Thunderbolt

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioCard {
    pub index: u32,
    pub id: String,
    pub driver: String,
    pub name: String,
    pub detail: String,
    pub codec: String,
}

/// The sound cards in `/proc/asound/cards` text: a line "` 0 [PCH            ]: HDA-Intel - HDA Intel PCH`", then an
/// indented description line.
pub fn parse_asound_cards(text: &str) -> Vec<AudioCard> {
    let mut out: Vec<AudioCard> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let head = t.split_whitespace().next().unwrap_or("");
        if let (Ok(index), Some(open), Some(close)) = (head.parse::<u32>(), t.find('['), t.find(']')) {
            if open < close {
                let rest = t[close + 1..].trim_start_matches(':').trim();
                let (driver, name) = rest.split_once(" - ").unwrap_or(("", rest));
                out.push(AudioCard { index, id: t[open + 1..close].trim().to_string(), driver: driver.trim().to_string(), name: name.trim().to_string(), ..Default::default() });
                continue;
            }
        }
        if let Some(last) = out.last_mut() {
            if last.detail.is_empty() {
                last.detail = t.to_string();
            }
        }
    }
    out
}

pub fn read_audio(asound: &Path) -> Vec<AudioCard> {
    let mut cards = parse_asound_cards(&read_trim_text(&asound.join("cards")));
    for c in cards.iter_mut() {
        let codec = read_trim_text(&asound.join(format!("card{}", c.index)).join("codec#0"));
        c.codec = codec.lines().next().and_then(|l| l.strip_prefix("Codec:")).map(|s| s.trim().to_string()).unwrap_or_default();
    }
    cards
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BluetoothController {
    pub name: String,
    pub bus: String,
    pub driver: String,
    /// Whether radio is switched off (a software or hardware block); None when no switch is listed.
    pub blocked: Option<bool>,
    /// Connections open now.
    pub links: usize,
}

pub fn read_bluetooth(dir: &Path, rfkill: &Path) -> Vec<BluetoothController> {
    let names = entries(dir);
    let blocked = entries(rfkill)
        .into_iter()
        .map(|n| rfkill.join(n))
        .find(|p| read_trim(&p.join("type")) == "bluetooth")
        .map(|p| read_trim(&p.join("soft")) == "1" || read_trim(&p.join("hard")) == "1");
    names
        .iter()
        .filter(|n| n.starts_with("hci") && !n.contains(':'))
        .map(|n| {
            let bus = match link_name(&dir.join(n).join("device/subsystem")).as_str() {
                "usb" => "USB".to_string(),
                "" => String::new(),
                other => other.to_string(),
            };
            let prefix = format!("{n}:");
            BluetoothController { name: n.clone(), bus, driver: link_name(&dir.join(n).join("device/driver")), blocked, links: names.iter().filter(|e| e.starts_with(&prefix)).count() }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Camera {
    pub node: String,
    pub name: String,
    pub driver: String,
}

/// The video capture devices: the first node of each device under `/sys/class/video4linux` (the later ones carry metadata).
pub fn read_cameras(dir: &Path) -> Vec<Camera> {
    entries(dir)
        .into_iter()
        .filter(|n| n.starts_with("video"))
        .filter_map(|n| {
            let p = dir.join(&n);
            if read_trim(&p.join("index")).parse::<u32>().unwrap_or(0) != 0 {
                return None;
            }
            Some(Camera { node: format!("/dev/{n}"), name: read_trim(&p.join("name")), driver: link_name(&p.join("device/driver")) })
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    pub state: String,
}

/// The printers in CUPS `printers.conf` text (`<Printer name>` blocks with `DeviceURI` and `State` lines).
pub fn parse_printers_conf(text: &str) -> Vec<Printer> {
    let mut out = Vec::new();
    let mut current: Option<Printer> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("<Printer ").or_else(|| line.strip_prefix("<DefaultPrinter ")).and_then(|r| r.strip_suffix('>')) {
            current = Some(Printer { name: name.trim().to_string(), ..Default::default() });
        } else if line.starts_with("</") {
            out.extend(current.take());
        } else if let Some(p) = current.as_mut() {
            if let Some(v) = line.strip_prefix("DeviceURI ") {
                p.uri = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("State ") {
                p.state = v.trim().to_string();
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThunderboltDevice {
    pub name: String,
    pub vendor: String,
    pub authorized: String,
    pub generation: String,
}

pub fn read_thunderbolt(dir: &Path) -> Vec<ThunderboltDevice> {
    entries(dir)
        .into_iter()
        .filter(|n| !n.starts_with("domain"))
        .filter_map(|n| {
            let p = dir.join(&n);
            let name = read_trim(&p.join("device_name"));
            if name.is_empty() {
                return None;
            }
            let authorized = match read_trim(&p.join("authorized")).as_str() {
                "1" | "2" => "Authorized".to_string(),
                "0" => "Not authorized".to_string(),
                _ => String::new(),
            };
            Some(ThunderboltDevice { name, vendor: read_trim(&p.join("vendor_name")), authorized, generation: read_trim(&p.join("generation")) })
        })
        .collect()
}

// ---------------------------------------------------------------- NVMe and SATA

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NvmeDrive {
    pub name: String,
    pub model: String,
    pub firmware: String,
    pub state: String,
    pub transport: String,
}

pub fn read_nvme(dir: &Path) -> Vec<NvmeDrive> {
    entries(dir)
        .into_iter()
        .filter(|n| n.starts_with("nvme"))
        .map(|n| {
            let p = dir.join(&n);
            NvmeDrive { model: read_trim(&p.join("model")), firmware: read_trim(&p.join("firmware_rev")), state: read_trim(&p.join("state")), transport: read_trim(&p.join("transport")), name: n }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SataPort {
    /// "ata1".
    pub port: String,
    /// The negotiated link speed ("6.0 Gbps"); "" when unknown.
    pub speed: String,
    pub attached: bool,
}

/// The SATA links (`ata_link/linkN`) with their negotiated speed and whether a device answers on them.
pub fn read_sata(links: &Path, devices: &Path) -> Vec<SataPort> {
    let mut ports: Vec<(u32, SataPort)> = entries(links)
        .into_iter()
        .filter_map(|n| {
            let id: u32 = n.strip_prefix("link")?.parse().ok()?;
            let speed = read_trim(&links.join(&n).join("sata_spd"));
            let speed = if speed.starts_with('<') { String::new() } else { speed };
            Some((id, SataPort { port: format!("ata{id}"), speed, attached: devices.join(format!("dev{id}.0")).exists() }))
        })
        .collect();
    ports.sort_by_key(|(id, _)| *id);
    ports.into_iter().map(|(_, p)| p).collect()
}

// ---------------------------------------------------------------- fonts, logs, firewall

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFamily {
    pub name: String,
    pub mono: bool,
    pub variable: bool,
}

/// The families the desktop can draw from the system and the user's font folders.
pub fn read_fonts(dirs: &[PathBuf]) -> Vec<FontFamily> {
    bifrost_render::fontprefs::installed(dirs).into_iter().map(|f| FontFamily { name: f.family, mono: f.mono, variable: f.variable }).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogEntry {
    pub name: String,
    pub bytes: u64,
}

fn tree_bytes(path: &Path, depth: usize, budget: &mut usize) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() || depth == 0 {
        return 0;
    }
    let mut total = 0;
    for name in entries(path) {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        total += tree_bytes(&path.join(name), depth - 1, budget);
    }
    total
}

/// What is under `/var/log` and how much space each entry takes, largest first (at most `limit` entries). The journal
/// directory counts as one entry with everything in it.
pub fn read_logs(dir: &Path, limit: usize) -> Vec<LogEntry> {
    let mut out: Vec<LogEntry> = entries(dir)
        .into_iter()
        .map(|name| {
            let mut budget = 5_000;
            LogEntry { bytes: tree_bytes(&dir.join(&name), 3, &mut budget), name }
        })
        .collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Firewall {
    /// "nftables", "iptables" or "" when no packet filter is loaded.
    pub backend: String,
    pub tables: Vec<String>,
    pub configs: Vec<String>,
}

/// The packet filter the kernel has loaded. The rule set itself needs administrator rights, so only the backend, the
/// iptables tables and the configuration the machine ships are known here.
pub fn read_firewall(proc_net: &Path, sys_module: &Path, etc: &Path) -> Firewall {
    let mut tables: Vec<String> = ["ip_tables_names", "ip6_tables_names"].iter().flat_map(|f| read_trim_text(&proc_net.join(f)).lines().map(|l| l.trim().to_string()).collect::<Vec<_>>()).filter(|t| !t.is_empty()).collect();
    tables.sort();
    tables.dedup();
    let backend = if sys_module.join("nf_tables").exists() {
        "nftables"
    } else if !tables.is_empty() {
        "iptables"
    } else {
        ""
    };
    let configs = ["nftables.conf", "ufw", "firewalld"].iter().filter(|name| etc.join(name).exists()).map(|name| name.to_string()).collect();
    Firewall { backend: backend.to_string(), tables, configs }
}

// ---------------------------------------------------------------- card readers, SCSI, Fibre Channel, SAS, optical, diagnostics

/// Whether `name` is `prefix` followed by a number ("host0", "sr1"), not `prefix` alone.
fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// A memory card slot: its host controller, and the card in it when one is inserted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardSlot {
    pub host: String,
    pub driver: String,
    /// The card's name; "" for an empty slot.
    pub card: String,
    /// "SD", "MMC" or "SDIO", as the card reports it.
    pub kind: String,
    /// The card's manufacturer ID, as the card gives it.
    pub manufacturer: String,
}

/// The memory card slots under `mmc_root` (the `mmcN` host folders) and the card in each.
pub fn read_card_readers(mmc_root: &Path) -> Vec<CardSlot> {
    entries(mmc_root)
        .into_iter()
        .filter(|n| numbered(n, "mmc"))
        .map(|host| {
            let dir = mmc_root.join(&host);
            let card_dir = entries(&dir).into_iter().find(|n| n.starts_with(&format!("{host}:")));
            let (card, kind, manufacturer) = match card_dir {
                Some(c) => {
                    let p = dir.join(c);
                    (read_trim(&p.join("name")), read_trim(&p.join("type")), read_trim(&p.join("manfid")))
                }
                None => Default::default(),
            };
            CardSlot { driver: link_name(&dir.join("device/driver")), host, card, kind, manufacturer }
        })
        .collect()
}

/// A SCSI host adapter. `transport` is "Fibre Channel" or "SAS" for an adapter those classes list, "" for the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScsiHost {
    pub name: String,
    pub proc_name: String,
    pub driver: String,
    pub transport: String,
}

/// The SCSI host adapters under `scsi_root` (`hostN`). An adapter that has an entry under `fc_root` or `sas_root` is
/// named by that transport.
pub fn read_scsi_hosts(scsi_root: &Path, fc_root: &Path, sas_root: &Path) -> Vec<ScsiHost> {
    entries(scsi_root)
        .into_iter()
        .filter(|n| numbered(n, "host"))
        .map(|name| {
            let transport = if fc_root.join(&name).exists() {
                "Fibre Channel"
            } else if sas_root.join(&name).exists() {
                "SAS"
            } else {
                ""
            };
            let dir = scsi_root.join(&name);
            ScsiHost { proc_name: read_trim(&dir.join("proc_name")), driver: link_name(&dir.join("device/driver")), transport: transport.to_string(), name }
        })
        .collect()
}

/// A Fibre Channel port (`fc_host`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FcPort {
    pub host: String,
    /// The name the driver gives the port; "" when it gives none.
    pub symbolic: String,
    pub port_name: String,
    pub node_name: String,
    /// "Online", "Linkdown" and the like.
    pub state: String,
    pub port_type: String,
    pub speed: String,
}

pub fn read_fc_ports(fc_root: &Path) -> Vec<FcPort> {
    entries(fc_root)
        .into_iter()
        .filter(|n| numbered(n, "host"))
        .map(|host| {
            let p = fc_root.join(&host);
            FcPort {
                symbolic: read_trim(&p.join("symbolic_name")),
                port_name: read_trim(&p.join("port_name")),
                node_name: read_trim(&p.join("node_name")),
                state: read_trim(&p.join("port_state")),
                port_type: read_trim(&p.join("port_type")),
                speed: read_trim(&p.join("speed")),
                host,
            }
        })
        .collect()
}

/// A SAS phy: one link of a SAS host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SasPhy {
    /// "phy-0:1".
    pub name: String,
    /// The negotiated rate ("6.0 Gbit"); "" when the phy has no link.
    pub link_rate: String,
    /// "end device", "edge expander" or "none".
    pub device: String,
    pub address: String,
}

pub fn read_sas_phys(phy_root: &Path) -> Vec<SasPhy> {
    entries(phy_root)
        .into_iter()
        .filter(|n| n.starts_with("phy-"))
        .map(|name| {
            let p = phy_root.join(&name);
            SasPhy {
                link_rate: read_trim(&p.join("negotiated_linkrate")),
                device: read_trim(&p.join("device_type")),
                address: read_trim(&p.join("sas_address")),
                name,
            }
        })
        .collect()
}

/// An optical drive (`/sys/block/srN`), with the disc it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpticalDrive {
    pub device: String,
    pub vendor: String,
    pub model: String,
    pub revision: String,
    /// The size of the disc in bytes; 0 when no disc is in the drive.
    pub media_bytes: u64,
}

pub fn read_optical_drives(block_root: &Path) -> Vec<OpticalDrive> {
    entries(block_root)
        .into_iter()
        .filter(|n| numbered(n, "sr"))
        .map(|device| {
            let p = block_root.join(&device);
            let sectors: u64 = read_trim(&p.join("size")).parse().unwrap_or(0);
            OpticalDrive {
                vendor: read_trim(&p.join("device/vendor")),
                model: read_trim(&p.join("device/model")),
                revision: read_trim(&p.join("device/rev")),
                media_bytes: sectors * 512,
                device,
            }
        })
        .collect()
}

/// A crash report or a core dump saved on this computer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub name: String,
    pub bytes: u64,
    /// Days since 1970-01-01 of its last change.
    pub day: i64,
}

/// The files in the report folders `dirs`, newest first, at most `limit`. A folder that is missing or not readable by
/// this user holds nothing.
pub fn read_diagnostics(dirs: &[PathBuf], limit: usize) -> Vec<DiagnosticReport> {
    let mut out = Vec::new();
    for dir in dirs {
        for name in entries(dir) {
            let Ok(meta) = fs::metadata(dir.join(&name)) else { continue };
            if !meta.is_file() {
                continue;
            }
            let secs = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            out.push(DiagnosticReport { name, bytes: meta.len(), day: (secs / 86_400) as i64 });
        }
    }
    out.sort_by(|a, b| b.day.cmp(&a.day).then(a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

// ---------------------------------------------------------------- installations, printer software

/// One package change from the package manager's log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installation {
    /// "2026-10-01 12:00".
    pub when: String,
    /// "installed", "upgraded", "removed", "downgraded" or "reinstalled".
    pub action: String,
    pub package: String,
    /// The version (for an upgrade, "old -> new"); "" when the log gives none.
    pub version: String,
}

/// The package changes in a pacman log (`[2026-10-01T12:00:00+0000] [ALPM] installed foo (1.0-1)` lines), newest first,
/// at most `limit`.
pub fn parse_pacman_log(text: &str, limit: usize) -> Vec<Installation> {
    let mut out: Vec<Installation> = text
        .lines()
        .filter_map(|line| {
            let (stamp, rest) = line.strip_prefix('[')?.split_once(']')?;
            let (action, rest) = rest.strip_prefix(" [ALPM] ")?.split_once(' ')?;
            if !matches!(action, "installed" | "upgraded" | "removed" | "downgraded" | "reinstalled") {
                return None;
            }
            let (package, version) = rest.split_once(' ').unwrap_or((rest, ""));
            let version = version.trim().trim_start_matches('(').trim_end_matches(')').to_string();
            let when = stamp.get(..16).unwrap_or(stamp).replace('T', " ");
            Some(Installation { when, action: action.to_string(), package: package.to_string(), version })
        })
        .collect();
    out.reverse();
    out.truncate(limit);
    out
}

pub fn read_installations(log: &Path, limit: usize) -> Vec<Installation> {
    parse_pacman_log(&read_trim_text(log), limit)
}

/// Vendor and device names from a `pci.ids` database (the hwdata file the PCI tools read).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PciNames {
    vendors: BTreeMap<u32, String>,
    devices: BTreeMap<(u32, u32), String>,
}

impl PciNames {
    /// The vendor's name, when the database has it.
    pub fn vendor(&self, id: u32) -> Option<String> {
        self.vendors.get(&id).cloned()
    }

    /// The device's name under its vendor, when the database has it.
    pub fn device(&self, vendor: u32, id: u32) -> Option<String> {
        self.devices.get(&(vendor, id)).cloned()
    }
}

/// `"8086  Intel Corporation"` as (0x8086, "Intel Corporation").
fn split_pci_id(text: &str) -> Option<(u32, String)> {
    let (id, name) = text.split_once("  ")?;
    Some((u32::from_str_radix(id.trim(), 16).ok()?, name.trim().to_string()))
}

/// The vendor and device names in `pci.ids` text. Vendor lines start at column 0, device lines under them start with one
/// tab (subsystem lines with two are skipped), and the class list at the "C " line ends the names.
pub fn parse_pci_ids(text: &str) -> PciNames {
    let mut names = PciNames::default();
    let mut vendor: Option<u32> = None;
    for line in text.lines() {
        if line.starts_with("C ") {
            break;
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with("\t\t") {
            continue;
        }
        if let Some(device) = line.strip_prefix('\t') {
            if let (Some(v), Some((id, name))) = (vendor, split_pci_id(device)) {
                names.devices.insert((v, id), name);
            }
            continue;
        }
        vendor = split_pci_id(line).map(|(id, name)| {
            names.vendors.insert(id, name);
            id
        });
    }
    names
}

/// The names from the first `pci.ids` that is present (none is an empty table, and the names fall back to the IDs).
pub fn read_pci_names(paths: &[&Path]) -> PciNames {
    paths.iter().find_map(|p| fs::read_to_string(p).ok()).map(|text| parse_pci_ids(&text)).unwrap_or_default()
}

/// A package the pacman local database lists as installed (`/var/lib/pacman/local/<name-version>/desc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PacmanPackage {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Unix seconds of the install (`%INSTALLDATE%`); 0 when the file does not say.
    pub installed_epoch: i64,
    /// The installed size in bytes (`%SIZE%`).
    pub size: u64,
}

/// The value under a `%KEY%` header of a pacman `desc` file: the lines after the header, up to the blank line.
fn desc_value(text: &str, key: &str) -> Option<String> {
    let header = format!("%{key}%");
    let mut lines = text.lines();
    lines.find(|l| l.trim() == header)?;
    Some(lines.next().unwrap_or("").trim().to_string())
}

/// One package from the text of a pacman `desc` file. No name means no package.
pub fn parse_pacman_desc(text: &str) -> Option<PacmanPackage> {
    let name = desc_value(text, "NAME").filter(|n| !n.is_empty())?;
    Some(PacmanPackage {
        name,
        version: desc_value(text, "VERSION").unwrap_or_default(),
        description: desc_value(text, "DESC").unwrap_or_default(),
        installed_epoch: desc_value(text, "INSTALLDATE").and_then(|v| v.parse().ok()).unwrap_or(0),
        size: desc_value(text, "SIZE").and_then(|v| v.parse().ok()).unwrap_or(0),
    })
}

/// The installed packages in a pacman local database directory, newest install first (then by name), at most `limit`.
/// The database is read as files; pacman is not run.
pub fn read_pacman_packages(local: &Path, limit: usize) -> Vec<PacmanPackage> {
    let mut out: Vec<PacmanPackage> = entries(local).iter().filter_map(|dir| parse_pacman_desc(&read_trim_text(&local.join(dir).join("desc")))).collect();
    out.sort_by(|a, b| b.installed_epoch.cmp(&a.installed_epoch).then_with(|| a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

/// A printer driver file (a PPD) installed for the printing system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrinterDriver {
    pub file: String,
    pub manufacturer: String,
    pub model: String,
}

/// The value of a PPD keyword line such as `*ModelName: "LaserJet"`, without its quotes.
fn ppd_value(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|rest| {
            let (_, after_open) = rest.split_once('"')?;
            let (value, _) = after_open.split_once('"')?;
            Some(value.trim().to_string())
        })
        .unwrap_or_default()
}

/// The first bytes of a file as text. The keywords a driver is named by come first, so a driver file is read only to
/// there.
fn read_head(path: &Path, limit: u64) -> String {
    use std::io::Read;
    let Ok(file) = fs::File::open(path) else { return String::new() };
    let mut buf = Vec::new();
    let _ = file.take(limit).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// The printer drivers in `dirs`, by model name, at most `limit`.
pub fn read_printer_drivers(dirs: &[PathBuf], limit: usize) -> Vec<PrinterDriver> {
    let mut out = Vec::new();
    for dir in dirs {
        for name in entries(dir).into_iter().filter(|n| n.ends_with(".ppd")) {
            let text = read_head(&dir.join(&name), 8192);
            out.push(PrinterDriver { manufacturer: ppd_value(&text, "*Manufacturer:"), model: ppd_value(&text, "*ModelName:"), file: name });
        }
    }
    out.sort_by(|a, b| a.model.cmp(&b.model).then(a.file.cmp(&b.file)));
    out.truncate(limit);
    out
}

// ---------------------------------------------------------------- everything

/// Everything above, read once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hardware {
    pub cpu: CpuDetail,
    pub memory: MemDetail,
    pub gpu_memory: Vec<GpuMemory>,
    pub usb_power: Vec<(String, String)>,
    pub volumes: Vec<Volume>,
    pub net: Vec<NetDetail>,
    pub dns: Vec<String>,
    pub secure_boot: String,
    pub build_id: String,
    pub apps: Vec<AppDetail>,
    pub audio: Vec<AudioCard>,
    pub bluetooth: Vec<BluetoothController>,
    pub cameras: Vec<Camera>,
    pub printers: Vec<Printer>,
    pub thunderbolt: Vec<ThunderboltDevice>,
    pub nvme: Vec<NvmeDrive>,
    pub sata: Vec<SataPort>,
    pub fonts: Vec<FontFamily>,
    pub logs: Vec<LogEntry>,
    pub firewall: Firewall,
    pub card_readers: Vec<CardSlot>,
    pub scsi: Vec<ScsiHost>,
    pub fc_ports: Vec<FcPort>,
    pub sas_phys: Vec<SasPhy>,
    pub optical: Vec<OpticalDrive>,
    pub diagnostics: Vec<DiagnosticReport>,
    pub installs: Vec<Installation>,
    pub packages: Vec<PacmanPackage>,
    pub pci_names: PciNames,
    pub printer_drivers: Vec<PrinterDriver>,
}

impl Hardware {
    pub fn collect() -> Hardware {
        let p = Path::new;
        let cpuinfo = read_trim_text(p("/proc/cpuinfo"));
        let mut memory = parse_meminfo(&read_trim_text(p("/proc/meminfo")));
        memory.dimms = parse_smbios_memory(&fs::read("/sys/firmware/dmi/tables/DMI").unwrap_or_default());
        let efi = p("/sys/firmware/efi").exists();
        let secure_var = fs::read("/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c").ok();
        let mut printers = parse_printers_conf(&read_trim_text(p("/etc/cups/printers.conf")));
        for usb in read_usb_printers(p("/sys/bus/usb/devices")) {
            if !printers.iter().any(|known| known.name == usb.name) {
                printers.push(usb);
            }
        }
        let mut font_dirs = vec![PathBuf::from("/usr/share/fonts")];
        if let Some(home) = std::env::var_os("HOME") {
            font_dirs.push(Path::new(&home).join(".local/share/fonts"));
        }
        Hardware {
            cpu: read_cpu(&cpuinfo, p("/sys/devices/system/cpu/cpu0/cache"), p("/sys/devices/system/cpu/cpu0/cpufreq"), p("/sys/devices/system/cpu"), p("/sys/devices")),
            memory,
            gpu_memory: read_gpu_memory(p("/sys/class/drm")),
            usb_power: read_usb_power(p("/sys/bus/usb/devices")),
            volumes: read_volumes(&read_trim_text(p("/proc/mounts"))),
            net: read_net(p("/sys/class/net"), p("/proc/net")),
            dns: read_dns(p("/etc/resolv.conf"), p("/run/systemd/resolve/resolv.conf")),
            secure_boot: secure_boot_state(efi, secure_var.as_deref()).to_string(),
            build_id: os_field(&read_trim_text(p("/etc/os-release")), "BUILD_ID"),
            apps: parse_registry(&read_trim_text(p("/var/lib/springboard/apps/installed.json"))),
            audio: read_audio(p("/proc/asound")),
            bluetooth: read_bluetooth(p("/sys/class/bluetooth"), p("/sys/class/rfkill")),
            cameras: read_cameras(p("/sys/class/video4linux")),
            printers,
            thunderbolt: read_thunderbolt(p("/sys/bus/thunderbolt/devices")),
            nvme: read_nvme(p("/sys/class/nvme")),
            sata: read_sata(p("/sys/class/ata_link"), p("/sys/class/ata_device")),
            fonts: read_fonts(&font_dirs),
            logs: read_logs(p("/var/log"), 20),
            firewall: read_firewall(p("/proc/net"), p("/sys/module"), p("/etc")),
            card_readers: read_card_readers(p("/sys/class/mmc_host")),
            scsi: read_scsi_hosts(p("/sys/class/scsi_host"), p("/sys/class/fc_host"), p("/sys/class/sas_host")),
            fc_ports: read_fc_ports(p("/sys/class/fc_host")),
            sas_phys: read_sas_phys(p("/sys/class/sas_phy")),
            optical: read_optical_drives(p("/sys/block")),
            diagnostics: read_diagnostics(&[PathBuf::from("/var/crash"), PathBuf::from("/var/lib/systemd/coredump")], 50),
            installs: read_installations(p("/var/log/pacman.log"), 300),
            packages: read_pacman_packages(p("/var/lib/pacman/local"), 2000),
            pci_names: read_pci_names(&[p("/usr/share/hwdata/pci.ids"), p("/usr/share/misc/pci.ids")]),
            printer_drivers: read_printer_drivers(&[PathBuf::from("/usr/share/cups/model"), PathBuf::from("/usr/share/ppd")], 300),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = "processor\t: 0\nphysical id\t: 0\ncore id\t\t: 0\ncpu MHz\t\t: 1800.004\n\nprocessor\t: 1\nphysical id\t: 0\ncore id\t\t: 0\ncpu MHz\t\t: 3200.5\n\nprocessor\t: 2\nphysical id\t: 0\ncore id\t\t: 1\ncpu MHz\t\t: 2000.0\n";

    #[test]
    fn hyper_threads_share_a_physical_core() {
        assert_eq!(parse_cpu_topology(CPUINFO), (2, 1));
        assert_eq!(parse_cpu_topology("processor : 0\nprocessor : 1\n"), (0, 0), "no ids, no topology");
        assert_eq!(parse_cpu_topology(""), (0, 0));
    }

    /// A fresh folder under the temporary directory, for a fixture tree.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sysinfo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn card_slots_name_the_card_in_each_host() {
        let dir = scratch("mmc");
        put(&dir.join("mmc0/mmc0:0001/name"), "SD64G\n");
        put(&dir.join("mmc0/mmc0:0001/type"), "SD\n");
        put(&dir.join("mmc0/mmc0:0001/manfid"), "0x000003\n");
        fs::create_dir_all(dir.join("mmc1")).unwrap();
        let slots = read_card_readers(&dir);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], CardSlot { host: "mmc0".into(), driver: String::new(), card: "SD64G".into(), kind: "SD".into(), manufacturer: "0x000003".into() });
        assert_eq!(slots[1].host, "mmc1");
        assert!(slots[1].card.is_empty(), "an empty slot has no card");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scsi_adapters_are_named_by_their_transport() {
        let dir = scratch("scsi");
        let (scsi, fc, sas) = (dir.join("scsi"), dir.join("fc"), dir.join("sas"));
        for (host, name) in [("host0", "ahci"), ("host1", "mpt3sas"), ("host2", "sas_host")] {
            put(&scsi.join(host).join("proc_name"), &format!("{name}\n"));
        }
        fs::create_dir_all(fc.join("host1")).unwrap();
        fs::create_dir_all(sas.join("host2")).unwrap();
        let hosts = read_scsi_hosts(&scsi, &fc, &sas);
        let transports: Vec<(&str, &str)> = hosts.iter().map(|h| (h.name.as_str(), h.transport.as_str())).collect();
        assert_eq!(transports, vec![("host0", ""), ("host1", "Fibre Channel"), ("host2", "SAS")]);
        assert_eq!(hosts[0].proc_name, "ahci");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fibre_channel_ports_carry_state_and_speed() {
        let dir = scratch("fcport");
        let host = dir.join("host1");
        put(&host.join("symbolic_name"), "HBA 1\n");
        put(&host.join("port_state"), "Online\n");
        put(&host.join("speed"), "8 Gbit\n");
        put(&host.join("port_name"), "0x21000024ff\n");
        put(&host.join("node_name"), "0x20000024ff\n");
        put(&host.join("port_type"), "N_Port\n");
        let ports = read_fc_ports(&dir);
        assert_eq!(ports.len(), 1);
        assert_eq!((ports[0].host.as_str(), ports[0].symbolic.as_str(), ports[0].state.as_str(), ports[0].speed.as_str()), ("host1", "HBA 1", "Online", "8 Gbit"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sas_phys_show_their_rate_and_whether_they_have_a_link() {
        let dir = scratch("sas");
        put(&dir.join("phy-1:0/negotiated_linkrate"), "6.0 Gbit\n");
        put(&dir.join("phy-1:0/device_type"), "end device\n");
        put(&dir.join("phy-1:0/sas_address"), "0x5000c5004\n");
        fs::create_dir_all(dir.join("phy-1:1")).unwrap();
        let phys = read_sas_phys(&dir);
        assert_eq!(phys.len(), 2);
        assert_eq!((phys[0].link_rate.as_str(), phys[0].device.as_str()), ("6.0 Gbit", "end device"));
        assert!(phys[1].link_rate.is_empty(), "no negotiated rate: no link");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn optical_drives_report_the_disc_size_only_when_a_disc_is_in() {
        let dir = scratch("optical");
        put(&dir.join("sr0/size"), "0\n");
        put(&dir.join("sr0/device/model"), "DVD-RW\n");
        put(&dir.join("sr0/device/vendor"), "HL-DT-ST\n");
        put(&dir.join("sr1/size"), "8388608\n");
        put(&dir.join("sda/size"), "1000\n");
        let drives = read_optical_drives(&dir);
        assert_eq!(drives.iter().map(|d| d.device.as_str()).collect::<Vec<_>>(), vec!["sr0", "sr1"], "disks are not optical drives");
        assert_eq!((drives[0].model.as_str(), drives[0].media_bytes), ("DVD-RW", 0));
        assert_eq!(drives[1].media_bytes, 4_294_967_296);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diagnostics_list_the_newest_files_first_and_skip_missing_folders() {
        let dir = scratch("diag");
        put(&dir.join("a.crash"), "abc");
        put(&dir.join("b.core"), "12345");
        let reports = read_diagnostics(&[dir.clone(), dir.join("missing")], 10);
        assert_eq!(reports.len(), 2);
        assert!(reports.iter().any(|r| r.name == "b.core" && r.bytes == 5));
        assert!(reports.iter().all(|r| r.day > 19_000), "a file written now is dated after 2022");
        assert_eq!(read_diagnostics(&[dir.clone()], 1).len(), 1, "the limit holds");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_package_log_gives_the_changes_newest_first() {
        let log = "[2026-09-30T08:00:00+0000] [PACMAN] Running 'pacman -Syu'\n[2026-09-30T08:01:00+0000] [ALPM] upgraded linux (6.1.1-1 -> 6.1.2-1)\n[2026-10-01T12:00:00+0000] [ALPM] installed foo (1.0-1)\n[2026-10-02T09:30:00+0000] [ALPM] removed bar (0.9-2)\nnot a line\n";
        let changes = parse_pacman_log(log, 10);
        assert_eq!(changes.len(), 3, "the pacman lines that are not package changes are left out");
        assert_eq!(changes[0], Installation { when: "2026-10-02 09:30".into(), action: "removed".into(), package: "bar".into(), version: "0.9-2".into() });
        assert_eq!(changes[2].version, "6.1.1-1 -> 6.1.2-1");
        assert_eq!(parse_pacman_log(log, 1).len(), 1, "the limit holds");
        assert!(parse_pacman_log("", 10).is_empty());
    }

    #[test]
    fn pci_ids_name_vendors_and_their_devices() {
        let text = "# comment\n8086  Intel Corporation\n\t3ea0  UHD Graphics\n\t\t1028 0a3e  Subsystem\n10de  NVIDIA Corporation\n\t2504  GA106 [GeForce RTX 3060]\nC 03  Display controller\n\t00  VGA\n";
        let names = parse_pci_ids(text);
        assert_eq!(names.vendor(0x8086).as_deref(), Some("Intel Corporation"));
        assert_eq!(names.device(0x8086, 0x3ea0).as_deref(), Some("UHD Graphics"));
        assert_eq!(names.device(0x10de, 0x2504).as_deref(), Some("GA106 [GeForce RTX 3060]"));
        assert_eq!(names.device(0x8086, 0x2504), None, "a device is named under its own vendor only");
        assert_eq!(names.vendor(0x1234), None);
        assert!(read_pci_names(&[Path::new("/nonexistent/pci.ids")]) == PciNames::default());
    }

    #[test]
    fn a_pacman_desc_gives_name_version_install_time_and_size() {
        let desc = "%NAME%\nbash\n\n%VERSION%\n5.2.37-1\n\n%DESC%\nThe GNU Bourne Again shell\n\n%INSTALLDATE%\n1700000000\n\n%SIZE%\n1234567\n";
        let p = parse_pacman_desc(desc).unwrap();
        assert_eq!(p, PacmanPackage { name: "bash".into(), version: "5.2.37-1".into(), description: "The GNU Bourne Again shell".into(), installed_epoch: 1_700_000_000, size: 1_234_567 });
        assert_eq!(parse_pacman_desc("%VERSION%\n1\n").as_ref(), None, "a desc with no name is not a package");
        let bare = parse_pacman_desc("%NAME%\nfoo\n").unwrap();
        assert_eq!((bare.version.as_str(), bare.installed_epoch, bare.size), ("", 0, 0), "missing fields read as empty or zero");
    }

    #[test]
    fn installed_packages_are_read_from_the_local_database_newest_first() {
        let dir = scratch("pacman-local");
        for (name, when) in [("alpha-1-1", "100"), ("beta-2-1", "300"), ("gamma-3-1", "300")] {
            let pkg = dir.join(name);
            fs::create_dir_all(&pkg).unwrap();
            let pkg_name = name.rsplitn(3, '-').nth(2).unwrap();
            put(&pkg.join("desc"), &format!("%NAME%\n{pkg_name}\n\n%INSTALLDATE%\n{when}\n"));
        }
        fs::create_dir_all(dir.join("ALPM_DB_VERSION")).unwrap();
        put(&dir.join("ALPM_DB_VERSION").join("desc"), "not a package\n");
        let all = read_pacman_packages(&dir, 10);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["beta", "gamma", "alpha"], "newest install first, then by name");
        assert_eq!(read_pacman_packages(&dir, 1).len(), 1, "the limit holds");
        assert!(read_pacman_packages(&dir.join("missing"), 10).is_empty());
    }

    #[test]
    fn printer_drivers_name_their_maker_and_model() {
        let text = "*PPD-Adobe: \"4.3\"\n*Manufacturer: \"HP\"\n*ModelName: \"HP LaserJet 4\"\n*NickName: \"x\"\n";
        assert_eq!(ppd_value(text, "*Manufacturer:"), "HP");
        assert_eq!(ppd_value(text, "*ModelName:"), "HP LaserJet 4");
        assert_eq!(ppd_value(text, "*Missing:"), "");
        let dir = scratch("ppd");
        put(&dir.join("hp4.ppd"), text);
        put(&dir.join("notes.txt"), "not a driver");
        let drivers = read_printer_drivers(&[dir.clone(), dir.join("missing")], 10);
        assert_eq!(drivers, vec![PrinterDriver { file: "hp4.ppd".into(), manufacturer: "HP".into(), model: "HP LaserJet 4".into() }]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cpu_lists_expand_ranges_in_order() {
        assert_eq!(parse_cpu_list("0-3,8,10-11\n"), vec![0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("5,5,2"), vec![2, 5], "duplicates collapse, order is numeric");
        assert!(parse_cpu_list("").is_empty());
        assert!(parse_cpu_list("junk").is_empty());
        assert!(parse_cpu_list("9-3").is_empty(), "a backwards range is left out");
    }

    /// A processor directory tree under a fresh temporary directory: `cpu_root` holds `cpuN` folders, `devices` the
    /// hybrid core lists.
    fn cpu_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sysinfo-cpu-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cpu_root = dir.join("cpu");
        let devices = dir.join("devices");
        fs::create_dir_all(&cpu_root).unwrap();
        fs::create_dir_all(&devices).unwrap();
        (dir, cpu_root, devices)
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn each_processor_reports_its_clock_now() {
        let (dir, cpu_root, _) = cpu_fixture("clocks");
        put(&cpu_root.join("cpu0/cpufreq/scaling_cur_freq"), "3200000\n");
        put(&cpu_root.join("cpu1/cpufreq/scaling_cur_freq"), "800000\n");
        fs::create_dir_all(cpu_root.join("cpu2/cpuidle")).unwrap();
        assert_eq!(read_core_clocks(&cpu_root), vec![(0, 3200), (1, 800)], "a processor without cpufreq is left out");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hybrid_processors_split_performance_and_efficiency_cores() {
        let (dir, cpu_root, devices) = cpu_fixture("hybrid");
        put(&devices.join("cpu_core/cpus"), "0-3\n");
        put(&devices.join("cpu_atom/cpus"), "4-5\n");
        // two threads per performance core (0 and 1 share core 0, 2 and 3 share core 1), one per efficiency core
        for (cpu, core) in [(0, 0), (1, 0), (2, 1), (3, 1), (4, 2), (5, 3)] {
            put(&cpu_root.join(format!("cpu{cpu}/topology/core_id")), &format!("{core}\n"));
            put(&cpu_root.join(format!("cpu{cpu}/topology/physical_package_id")), "0\n");
        }
        assert_eq!(read_core_kinds(&devices, &cpu_root), Some((2, 2)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capacity_marks_the_performance_cores_of_other_processors() {
        let (dir, cpu_root, devices) = cpu_fixture("capacity");
        for (cpu, capacity) in [(0, 1024), (1, 1024), (2, 512), (3, 512)] {
            put(&cpu_root.join(format!("cpu{cpu}/cpu_capacity")), &format!("{capacity}\n"));
        }
        assert_eq!(read_core_kinds(&devices, &cpu_root), Some((2, 2)), "no topology: each processor is a core");
        put(&cpu_root.join("cpu3/cpu_capacity"), "1024\n");
        assert_eq!(read_core_kinds(&devices, &cpu_root), Some((3, 1)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_kind_of_core_reports_no_split() {
        let (dir, cpu_root, devices) = cpu_fixture("uniform");
        for cpu in 0..4 {
            put(&cpu_root.join(format!("cpu{cpu}/cpu_capacity")), "1024\n");
        }
        assert_eq!(read_core_kinds(&devices, &cpu_root), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clocks_are_read_per_logical_processor() {
        assert_eq!(parse_cpuinfo_mhz(CPUINFO), vec![1800, 3201, 2000]);
    }

    #[test]
    fn cache_sizes_read_in_kilobytes() {
        assert_eq!(parse_cache_size("32K"), Some(32));
        assert_eq!(parse_cache_size("8M"), Some(8192));
        assert_eq!(parse_cache_size("1G"), Some(1_048_576));
        assert_eq!(parse_cache_size(""), None);
        assert_eq!(parse_cache_size("x"), None);
        assert_eq!(kb_label(32), "32 KB");
        assert_eq!(kb_label(16_384), "16 MB");
        assert_eq!(kb_label(1536), "1536 KB");
        assert_eq!(CacheInfo { level: 1, kind: "Data".into(), size_kb: 32 }.label(), "L1 data 32 KB");
        assert_eq!(CacheInfo { level: 3, kind: "Unified".into(), size_kb: 16_384 }.label(), "L3 16 MB");
    }

    #[test]
    fn meminfo_gives_available_memory_and_swap() {
        let m = parse_meminfo("MemTotal: 100 kB\nMemAvailable:    2048 kB\nSwapTotal: 4096 kB\nSwapFree: 1024 kB\n");
        assert_eq!((m.available, m.swap_total, m.swap_free), (2048 * 1024, 4096 * 1024, 1024 * 1024));
        assert_eq!(parse_meminfo("").available, 0);
    }

    /// One type 17 structure: 8 GB DDR4 at 3200 MT/s in "DIMM A1", then an empty slot, then the end marker.
    fn smbios_fixture() -> Vec<u8> {
        let mut dimm = vec![0u8; 0x22];
        dimm[0] = 17;
        dimm[1] = 0x22;
        dimm[0x0C..0x0E].copy_from_slice(&8192u16.to_le_bytes());
        dimm[0x10] = 1; // locator
        dimm[0x12] = 0x1A; // DDR4
        dimm[0x15..0x17].copy_from_slice(&3200u16.to_le_bytes());
        dimm[0x17] = 2; // manufacturer
        dimm[0x1A] = 3; // part number
        dimm[0x20..0x22].copy_from_slice(&2933u16.to_le_bytes());
        let mut table = dimm;
        table.extend_from_slice(b"DIMM A1\0Samsung\0M471A1K43\0\0");
        let mut empty = vec![0u8; 0x22];
        empty[0] = 17;
        empty[1] = 0x22;
        empty[0x10] = 1;
        table.extend(empty);
        table.extend_from_slice(b"DIMM B1\0\0");
        table.extend_from_slice(&[127, 4, 0, 0, 0, 0]);
        table
    }

    #[test]
    fn smbios_memory_devices_are_decoded() {
        let d = parse_smbios_memory(&smbios_fixture());
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], Dimm { locator: "DIMM A1".into(), size_mb: 8192, kind: "DDR4".into(), speed_mts: 3200, configured_mts: 2933, manufacturer: "Samsung".into(), part: "M471A1K43".into() });
        assert_eq!((d[1].locator.as_str(), d[1].size_mb), ("DIMM B1", 0));
    }

    #[test]
    fn a_truncated_smbios_table_stops_cleanly() {
        let t = smbios_fixture();
        assert!(parse_smbios_memory(&t[..10]).is_empty());
        assert!(parse_smbios_memory(&[]).is_empty());
    }

    #[test]
    fn usb_ports_sort_into_a_tree() {
        let dev = |port: &str| UsbDevice { port: port.to_string(), ..Default::default() };
        let devices = vec![dev("2-1"), dev("1-1.2"), dev("usb2"), dev("1-1"), dev("usb1"), dev("1-2")];
        let tree: Vec<(usize, &str)> = usb_tree(&devices).into_iter().map(|(d, u)| (d, u.port.as_str())).collect();
        assert_eq!(tree, vec![(0, "usb1"), (1, "1-1"), (2, "1-1.2"), (1, "1-2"), (0, "usb2"), (1, "2-1")]);
        assert_eq!(usb_position("1-10.3"), (1, vec![10, 3]));
    }

    #[test]
    fn a_usb_device_names_its_hub() {
        assert_eq!(usb_parent("3-1.2"), "3-1");
        assert_eq!(usb_parent("3-1"), "usb3");
        assert_eq!(usb_parent("usb3"), "");
    }

    #[test]
    fn mounts_keep_real_storage_and_decode_escapes() {
        let text = "proc /proc proc rw 0 0\n/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/loop3 /snap/x squashfs ro 0 0\n/dev/sdb1 /run/media/me/My\\040Disk vfat rw 0 0\nserver:/share /mnt/share nfs4 rw 0 0\n/dev/nvme0n1p2 / ext4 rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n";
        let m = parse_mounts(text);
        assert_eq!(m.iter().map(|e| e.mount.as_str()).collect::<Vec<_>>(), vec!["/", "/run/media/me/My Disk", "/mnt/share"]);
        assert_eq!(unescape_mount("a\\040b"), "a b");
        assert_eq!(unescape_mount("plain"), "plain");
    }

    const ROUTE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\nwlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\nwlan0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";

    #[test]
    fn the_gateway_and_networks_come_from_the_route_table() {
        assert_eq!(route_addr("0101A8C0"), Some(0xC0A8_0101));
        assert_eq!(ipv4_string(0xC0A8_0101), "192.168.1.1");
        assert_eq!(parse_gateway(ROUTE), Some(("wlan0".to_string(), "192.168.1.1".to_string())));
        assert_eq!(parse_route_networks(ROUTE), vec![("wlan0".to_string(), 0xC0A8_0100, 0xFFFF_FF00)]);
        assert_eq!(parse_ipv4("192.168.1.5"), Some(0xC0A8_0105));
        assert_eq!(parse_ipv4("192.168.1"), None);
    }

    #[test]
    fn local_addresses_are_assigned_to_their_network() {
        let trie = "Main:\n  +-- 0.0.0.0/0 3 0 5\n     |-- 192.168.1.0\n        /24 link UNICAST\n     |-- 192.168.1.5\n        /32 host LOCAL\n     |-- 127.0.0.1\n        /32 host LOCAL\n     |-- 10.9.9.9\n        /32 host LOCAL\n";
        let addrs = parse_fib_local(trie);
        assert_eq!(addrs, vec!["192.168.1.5".to_string(), "10.9.9.9".to_string()]);
        let nets = parse_route_networks(ROUTE);
        assert_eq!(assign_ipv4(&addrs, &nets), vec![("wlan0".to_string(), "192.168.1.5".to_string())], "10.9.9.9 is on no listed network");
    }

    #[test]
    fn dns_servers_and_wifi_signal_are_parsed() {
        assert_eq!(parse_resolv("# c\nnameserver 1.1.1.1\nsearch lan\nnameserver  9.9.9.9\n"), vec!["1.1.1.1", "9.9.9.9"]);
        let wireless = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n face | tstat   link level noise  nwid  crypt   frag  retry   misc | beacon | 22\nwlan0: 0000   70.  -40.  -256        0      0      0      0      0        0\n";
        assert_eq!(parse_wireless(wireless), vec![("wlan0".to_string(), 100, -40)]);
    }

    #[test]
    fn os_fields_and_secure_boot_read_plainly() {
        assert_eq!(os_field("NAME=\"Test\"\nBUILD_ID=\"2026.10\"\n", "BUILD_ID"), "2026.10");
        assert_eq!(os_field("NAME=x", "BUILD_ID"), "");
        assert_eq!(secure_boot_state(true, Some(&[6, 0, 0, 0, 1])), "Enabled");
        assert_eq!(secure_boot_state(true, Some(&[6, 0, 0, 0, 0])), "Disabled");
        assert_eq!(secure_boot_state(true, None), "Unknown");
        assert_eq!(secure_boot_state(false, None), "Not supported (legacy BIOS boot)");
    }

    #[test]
    fn the_registry_gives_names_kinds_and_install_days() {
        let text = r#"{"apps":[{"id":"is.o.notes","name":"Notes","version":"1.0.0","kind":"system-app","purchased":20000},{"id":"x.y","name":"Y","version":"2"}]}"#;
        let apps = parse_registry(text);
        assert_eq!(apps.len(), 2);
        assert_eq!((apps[0].name.as_str(), apps[0].kind_label(), apps[0].installed_day), ("Notes", "Preinstalled", 20000));
        assert_eq!((apps[1].kind_label(), apps[1].installed_day), ("User", 0));
        assert!(parse_registry("not json").is_empty());
    }

    #[test]
    fn day_numbers_read_as_dates() {
        assert_eq!(day_label(0), "1970-01-01");
        assert_eq!(day_label(20_000), "2024-10-04");
        assert_eq!(day_label(19_782), "2024-02-29", "the leap day");
        assert_eq!(day_label(19_784), "2024-03-02", "after a leap day");
    }

    #[test]
    fn sound_cards_are_parsed_with_their_description() {
        let text = " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n                      HDA Intel PCH at 0xf7e10000 irq 130\n 1 [NVidia         ]: HDA-Intel - HDA NVidia\n                      HDA NVidia at 0xf7080000 irq 17\n";
        let cards = parse_asound_cards(text);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0], AudioCard { index: 0, id: "PCH".into(), driver: "HDA-Intel".into(), name: "HDA Intel PCH".into(), detail: "HDA Intel PCH at 0xf7e10000 irq 130".into(), codec: String::new() });
        assert_eq!(cards[1].index, 1);
        assert!(parse_asound_cards("--- no soundcards ---\n").is_empty());
    }

    #[test]
    fn cups_printers_are_parsed() {
        let text = "# Printer configuration file\n<Printer Office>\nInfo Office\nDeviceURI ipp://10.0.0.5/ipp/print\nState Idle\n</Printer>\n<DefaultPrinter Lab>\nDeviceURI usb://X\nState Stopped\n</Printer>\n";
        let p = parse_printers_conf(text);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], Printer { name: "Office".into(), uri: "ipp://10.0.0.5/ipp/print".into(), state: "Idle".into() });
        assert_eq!(p[1].name, "Lab");
    }

    #[test]
    fn firewall_notes_the_backend_the_kernel_has() {
        let dir = std::env::temp_dir().join(format!("sysinfo-firewall-{}", std::process::id()));
        let (proc_net, module, etc) = (dir.join("proc_net"), dir.join("module"), dir.join("etc"));
        fs::create_dir_all(module.join("nf_tables")).unwrap();
        fs::create_dir_all(&proc_net).unwrap();
        fs::create_dir_all(etc.join("ufw")).unwrap();
        fs::write(proc_net.join("ip_tables_names"), "filter\nnat\n").unwrap();
        let f = read_firewall(&proc_net, &module, &etc);
        assert_eq!(f, Firewall { backend: "nftables".into(), tables: vec!["filter".into(), "nat".into()], configs: vec!["ufw".into()] });
        let none = read_firewall(&dir.join("missing"), &dir.join("missing"), &dir.join("missing"));
        assert_eq!(none, Firewall::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sata_links_carry_speed_and_presence() {
        let dir = std::env::temp_dir().join(format!("sysinfo-sata-{}", std::process::id()));
        let (links, devices) = (dir.join("ata_link"), dir.join("ata_device"));
        fs::create_dir_all(links.join("link1")).unwrap();
        fs::create_dir_all(links.join("link2")).unwrap();
        fs::create_dir_all(devices.join("dev1.0")).unwrap();
        fs::write(links.join("link1/sata_spd"), "6.0 Gbps\n").unwrap();
        fs::write(links.join("link2/sata_spd"), "<unknown>\n").unwrap();
        let ports = read_sata(&links, &devices);
        assert_eq!(ports, vec![SataPort { port: "ata1".into(), speed: "6.0 Gbps".into(), attached: true }, SataPort { port: "ata2".into(), speed: String::new(), attached: false }]);
        let _ = fs::remove_dir_all(&dir);
    }
}
