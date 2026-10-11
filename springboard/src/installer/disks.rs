//! Disks and partitions for the installer: parsing `parted -sm … print free`, classifying what is on a
//! disk, and planning where BifrOSt goes (erase the whole disk, or use its largest free region).

use std::process::Command;

pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;
/// Smallest root partition the installer will create.
pub const MIN_ROOT: u64 = 12 * GIB;
/// The EFI partition holds the kernel and initramfs too, so an existing one must be at least this big to be reused.
pub const MIN_REUSE_ESP: u64 = 400 * MIB;
pub const NEW_ESP: u64 = 512 * MIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Esp,
    Linux,
    Windows,
    Mac,
    Swap,
    Other,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub index: u32,
    pub path: String,
    pub start: u64,
    pub size: u64,
    pub fs: String,
    pub label: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub size: u64,
    pub model: String,
    /// "gpt", "msdos" or "" for a blank disk
    pub table: String,
    pub parts: Vec<Part>,
}

impl Disk {
    pub fn name(&self) -> String {
        if self.model.trim().is_empty() { self.path.clone() } else { self.model.trim().to_string() }
    }

    pub fn largest_free(&self) -> Option<&Part> {
        self.parts.iter().filter(|p| p.kind == Kind::Free).max_by_key(|p| p.size)
    }

    pub fn reusable_esp(&self) -> Option<&Part> {
        self.parts.iter().find(|p| p.kind == Kind::Esp && p.size >= MIN_REUSE_ESP && p.fs.contains("fat"))
    }

    /// Data already on the disk (everything that is not free space).
    pub fn used(&self) -> u64 {
        self.parts.iter().filter(|p| p.kind != Kind::Free).map(|p| p.size).sum()
    }

    /// Can BifrOSt be installed into free space without touching anything else?
    pub fn can_use_free(&self) -> bool {
        self.table == "gpt" && self.largest_free().is_some_and(|f| f.size >= MIN_ROOT + if self.reusable_esp().is_some() { 0 } else { NEW_ESP })
    }
}

fn kind_of(fs: &str, label: &str, flags: &str) -> Kind {
    let (fs, label, flags) = (fs.to_lowercase(), label.to_lowercase(), flags.to_lowercase());
    if flags.contains("esp") || label.contains("efi system") {
        Kind::Esp
    } else if fs == "free" {
        Kind::Free
    } else if fs.contains("swap") {
        Kind::Swap
    } else if fs.contains("ntfs") || fs.contains("exfat") || label.contains("basic data") || label.contains("microsoft") {
        Kind::Windows
    } else if fs.contains("hfs") || fs.contains("apfs") {
        Kind::Mac
    } else if fs.contains("ext") || fs.contains("btrfs") || fs.contains("xfs") || fs.contains("f2fs") || label.contains("linux") {
        Kind::Linux
    } else {
        Kind::Other
    }
}

fn bytes(s: &str) -> u64 {
    s.trim_end_matches('B').parse().unwrap_or(0)
}

/// Parses `parted -sm <disk> unit B print free`.
pub fn parse_parted(text: &str) -> Option<Disk> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && *l != "BYT;");
    let head: Vec<&str> = lines.next()?.trim_end_matches(';').split(':').collect();
    if head.len() < 6 {
        return None;
    }
    let path = head[0].to_string();
    let mut disk = Disk { path: path.clone(), size: bytes(head[1]), model: head.get(6).copied().unwrap_or("").to_string(), table: if head[5] == "unknown" { String::new() } else { head[5].to_string() }, parts: vec![] };
    let sep = if path.chars().last().is_some_and(|c| c.is_ascii_digit()) { "p" } else { "" };
    for l in lines {
        let f: Vec<&str> = l.trim_end_matches(';').split(':').collect();
        if f.len() < 5 {
            continue;
        }
        let (idx, start, size) = (f[0].parse::<u32>().unwrap_or(0), bytes(f[1]), bytes(f[3]));
        let (fs, label, flags) = (f[4], f.get(5).copied().unwrap_or(""), f.get(6).copied().unwrap_or(""));
        let kind = kind_of(fs, label, flags);
        // slivers of alignment padding are not free space a user can use
        if kind == Kind::Free && size < 8 * MIB {
            continue;
        }
        disk.parts.push(Part { index: idx, path: if kind == Kind::Free { String::new() } else { format!("{path}{sep}{idx}") }, start, size, fs: fs.to_string(), label: label.to_string(), kind });
    }
    Some(disk)
}

/// The disk that holds the running system's root (never offered for erasing).
fn root_disk() -> Option<String> {
    let src = Command::new("findmnt").args(["-nro", "SOURCE", "/"]).output().ok()?;
    let src = String::from_utf8_lossy(&src.stdout).trim().split('[').next()?.to_string();
    let pk = Command::new("lsblk").args(["-dnro", "PKNAME", &src]).output().ok()?;
    let pk = String::from_utf8_lossy(&pk.stdout).trim().to_string();
    (!pk.is_empty()).then(|| format!("/dev/{pk}"))
}

/// Disks from a fixture file (BIFROST_DISK_FIXTURE): `parted -sm` outputs separated by lines of `---`.
fn fixture(path: &str) -> Vec<Disk> {
    std::fs::read_to_string(path).unwrap_or_default().split("\n---\n").filter_map(parse_parted).collect()
}

/// Whole disks the installer may offer: block devices of type disk, not the installer medium or the running root.
pub fn probe() -> Vec<Disk> {
    if let Ok(f) = std::env::var("BIFROST_DISK_FIXTURE") {
        return fixture(&f);
    }
    let run = |cmd: &str, args: &[&str]| Command::new(cmd).args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let out = run("lsblk", &["-dpnro", "PATH,TYPE,RM,RO"]);
    let live = run("findmnt", &["-nro", "SOURCE", "/run/archiso/bootmnt"]).trim().to_string();
    let root = root_disk();
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4 && f[1] == "disk" && f[3] == "0" && !f[0].contains("/loop") && !f[0].contains("/zram") && !f[0].contains("/sr") && !f[0].contains("/nbd") && !f[0].contains("/ram")).then(|| f[0].to_string())
        })
        .filter(|p| (live.is_empty() || !live.starts_with(p.as_str())) && root.as_deref() != Some(p.as_str()))
        .map(|p| {
            let mut d = parse_parted(&run("parted", &["-sm", &p, "unit", "B", "print", "free"]))
                .unwrap_or_else(|| Disk { path: p.clone(), size: 0, model: String::new(), table: String::new(), parts: vec![] });
            if d.size == 0 {
                d.size = run("lsblk", &["-bdnro", "SIZE", &p]).trim().parse().unwrap_or(0);
            }
            if d.model.is_empty() {
                d.model = run("lsblk", &["-dnro", "MODEL", &p]).trim().to_string();
            }
            d
        })
        .filter(|d| d.size >= 8 << 30)
        .collect()
}

/// How BifrOSt is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Wipe the disk: new GPT, a 512 MiB EFI partition and the rest as root.
    Erase,
    /// Keep everything; use the largest free region.
    FreeSpace,
}

/// One block of the disk in the layout bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub size: u64,
    pub label: String,
    pub kind: Kind,
    /// part of the new BifrOSt install
    pub new: bool,
}

/// The disk as it will look after installing with `mode` (for the preview bar).
pub fn preview(disk: &Disk, mode: Mode) -> Vec<Segment> {
    match mode {
        Mode::Erase => vec![
            Segment { size: NEW_ESP, label: "EFI".into(), kind: Kind::Esp, new: true },
            Segment { size: disk.size.saturating_sub(NEW_ESP + MIB), label: "BifrOSt".into(), kind: Kind::Linux, new: true },
        ],
        Mode::FreeSpace => {
            let region = disk.largest_free().map(|f| f.start);
            let need_esp = disk.reusable_esp().is_none();
            let mut v = vec![];
            for p in &disk.parts {
                if Some(p.start) == region && p.kind == Kind::Free {
                    if need_esp {
                        v.push(Segment { size: NEW_ESP, label: "EFI".into(), kind: Kind::Esp, new: true });
                    }
                    v.push(Segment { size: p.size.saturating_sub(if need_esp { NEW_ESP } else { 0 }), label: "BifrOSt".into(), kind: Kind::Linux, new: true });
                } else {
                    v.push(Segment { size: p.size, label: if p.label.is_empty() { p.fs.clone() } else { p.label.clone() }, kind: p.kind, new: false });
                }
            }
            v
        }
    }
}

/// The commands that partition the disk, as (program, args). Partition paths for the new root and EFI
/// partitions are found afterwards by `new_partitions`.
pub fn partition_commands(disk: &Disk, mode: Mode) -> Vec<Vec<String>> {
    let p = disk.path.clone();
    let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match mode {
        Mode::Erase => vec![{
            let mut v = c(&["parted", "-s", &p, "mklabel", "gpt", "mkpart", "ESP", "fat32", "1MiB", "513MiB", "set", "1", "esp", "on", "mkpart", "BifrOSt", "btrfs", "513MiB", "100%"]);
            v.shrink_to_fit();
            v
        }],
        Mode::FreeSpace => {
            let Some(f) = disk.largest_free() else { return vec![] };
            let (s, e) = (f.start.div_ceil(MIB), (f.start + f.size) / MIB - 1);
            if disk.reusable_esp().is_some() {
                vec![c(&["parted", "-s", &p, "mkpart", "BifrOSt", "btrfs", &format!("{s}MiB"), &format!("{e}MiB")])]
            } else {
                vec![
                    c(&["parted", "-s", &p, "mkpart", "ESP", "fat32", &format!("{s}MiB"), &format!("{}MiB", s + 512)]),
                    c(&["parted", "-s", &p, "mkpart", "BifrOSt", "btrfs", &format!("{}MiB", s + 512), &format!("{e}MiB")]),
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUAL: &str = "BYT;\n/dev/nvme0n1:1000204886016B:nvme:512:512:gpt:WD PC SN740:;\n1:17408B:1048575B:1031168B:free;\n1:1048576B:538968063B:537919488B:fat32:EFI System Partition:boot, esp;\n2:538968064B:107374182399B:106835214336B:ntfs:Basic data partition:msftdata;\n1:107374182400B:1000204869119B:892830686720B:free;\n";
    const EMPTY: &str = "BYT;\n/dev/vda:17179869184B:virtio:512:512:unknown:Virtio Block Device:;\n";

    #[test]
    fn parses_a_windows_disk_with_free_space() {
        let d = parse_parted(DUAL).unwrap();
        assert_eq!((d.path.as_str(), d.table.as_str(), d.model.as_str()), ("/dev/nvme0n1", "gpt", "WD PC SN740"));
        let kinds: Vec<Kind> = d.parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [Kind::Esp, Kind::Windows, Kind::Free], "the 1 MiB alignment sliver is dropped");
        assert_eq!(d.parts[0].path, "/dev/nvme0n1p1");
        assert_eq!(d.reusable_esp().unwrap().index, 1);
        assert!(d.can_use_free());
        assert_eq!(d.largest_free().unwrap().size, 892_830_686_720);
    }

    #[test]
    fn blank_disk_has_no_table_and_cannot_use_free_space() {
        let d = parse_parted(EMPTY).unwrap();
        assert_eq!((d.table.as_str(), d.size, d.parts.len()), ("", 17179869184, 0));
        assert!(!d.can_use_free());
        assert_eq!(d.path, "/dev/vda");
    }

    #[test]
    fn erase_preview_and_commands() {
        let d = parse_parted(DUAL).unwrap();
        let v = preview(&d, Mode::Erase);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|s| s.new));
        let cmds = partition_commands(&d, Mode::Erase);
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].windows(2).any(|w| w == ["mklabel", "gpt"]));
    }

    #[test]
    fn free_space_install_keeps_existing_partitions_and_reuses_a_big_esp() {
        let d = parse_parted(DUAL).unwrap();
        let v = preview(&d, Mode::FreeSpace);
        assert_eq!(v.iter().filter(|s| s.new).count(), 1, "only a root partition is added; the 512 MiB ESP is reused");
        assert_eq!(v.iter().filter(|s| !s.new).count(), 2);
        let cmds = partition_commands(&d, Mode::FreeSpace);
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].iter().any(|a| a == "102400MiB"), "starts at the free region: {:?}", cmds[0]);
        assert!(!cmds.iter().any(|c| c.contains(&"mklabel".to_string())), "never relabels a disk it keeps");
    }

    #[test]
    fn a_small_esp_is_not_reused_and_a_new_one_is_made() {
        let small = DUAL.replace("537919488B:fat32", "100663296B:fat32");
        let d = parse_parted(&small).unwrap();
        assert!(d.reusable_esp().is_none());
        assert_eq!(partition_commands(&d, Mode::FreeSpace).len(), 2);
        assert_eq!(preview(&d, Mode::FreeSpace).iter().filter(|s| s.new).count(), 2);
    }
}
