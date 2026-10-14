//! Facts the shared `bifrost_core::sysinfo` report does not carry: the host name, uptime, boot volume, loaded kernel
//! modules, startup items and the language (and, in `probe`, the hardware detail). The parsers are pure functions over text, so they are tested on the host;
//! `Extras::collect` only reads the files and hands the text to them.

use crate::probe::Hardware;
use bifrost_app::displays::DisplayInfo;
use std::path::Path;

/// One loaded kernel module (a line of `/proc/modules`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub size_bytes: u64,
    pub used_by: usize,
}

/// A display as the display service describes it, for the Displays category: its name, refresh rate and scale. The
/// rate and scale are whole numbers so the facts compare exactly: the rate in hertz, the scale in percent (200 is 2x).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenFacts {
    pub name: String,
    pub refresh_hz: u32,
    pub scale_percent: u32,
}

impl ScreenFacts {
    pub fn new(name: &str, refresh_hz: f32, scale: f32) -> ScreenFacts {
        ScreenFacts { name: name.to_string(), refresh_hz: refresh_hz.round().max(0.0) as u32, scale_percent: (scale * 100.0).round().max(0.0) as u32 }
    }

    pub fn from_info(info: &DisplayInfo) -> ScreenFacts {
        ScreenFacts::new(&info.name, info.refresh_hz(), info.mode.scale())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub hostname: String,
    pub uptime_secs: u64,
    /// The device and file system mounted at `/`, such as `/dev/nvme0n1p2 (ext4)`.
    pub boot_volume: String,
    /// "UEFI" when the firmware booted through EFI, otherwise "Legacy BIOS".
    pub boot_mode: String,
    pub modules: Vec<Module>,
    /// Display names of the autostart entries.
    pub startup: Vec<String>,
    /// The `LANG` value, such as `is_IS.UTF-8`.
    pub locale: String,
    /// The detail `probe` reads: processor caches, memory modules, volumes, network addressing and the rest.
    pub hw: Hardware,
    /// The displays the display service lists, with their refresh rate and scale. Set by the window when the list
    /// arrives, not by `collect`, so a fresh read keeps the list the window already has.
    pub screens: Vec<ScreenFacts>,
}

impl Extras {
    pub fn collect() -> Extras {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let mut startup = startup_items(Path::new("/etc/xdg/autostart"));
        if let Some(home) = std::env::var_os("HOME") {
            startup.extend(startup_items(&Path::new(&home).join(".config/autostart")));
        }
        startup.sort();
        startup.dedup();
        Extras {
            hostname: read("/proc/sys/kernel/hostname").trim().to_string(),
            uptime_secs: parse_uptime(&read("/proc/uptime")),
            boot_volume: parse_boot_volume(&read("/proc/mounts")),
            boot_mode: if Path::new("/sys/firmware/efi").exists() { "UEFI".to_string() } else { "Legacy BIOS".to_string() },
            modules: parse_modules(&read("/proc/modules")),
            startup,
            locale: std::env::var("LANG").unwrap_or_default(),
            hw: Hardware::collect(),
            screens: Vec::new(),
        }
    }
}

/// The whole seconds of `/proc/uptime` ("12345.67 98765.43").
pub fn parse_uptime(text: &str) -> u64 {
    text.split_whitespace().next().and_then(|s| s.split('.').next()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// "3 days, 4 hours, 5 minutes", with the larger units left out while they are zero.
pub fn uptime_label(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    let unit = |n: u64, one: &str| if n == 1 { format!("{n} {one}") } else { format!("{n} {one}s") };
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(unit(days, "day"));
    }
    if days > 0 || hours > 0 {
        parts.push(unit(hours, "hour"));
    }
    parts.push(unit(minutes, "minute"));
    parts.join(", ")
}

/// Lines of `/proc/modules`: "name size refcount deps state address". Sorted by name.
pub fn parse_modules(text: &str) -> Vec<Module> {
    let mut out: Vec<Module> = text
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let name = f.next()?.to_string();
            let size_bytes = f.next()?.parse().ok()?;
            let used_by = f.next()?.parse().ok()?;
            Some(Module { name, size_bytes, used_by })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The device and file system of the `/` line of `/proc/mounts`; the last such line wins, as in the kernel's view.
pub fn parse_boot_volume(text: &str) -> String {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (dev, mount, fs) = (f.next()?, f.next()?, f.next()?);
            (mount == "/").then(|| format!("{dev} ({fs})"))
        })
        .last()
        .unwrap_or_default()
}

/// The names of the `.desktop` entries in an autostart folder: the `Name=` line, else the file stem.
fn startup_items(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "desktop"))
        .map(|e| {
            let stem = e.path().file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            desktop_name(&std::fs::read_to_string(e.path()).unwrap_or_default()).unwrap_or(stem)
        })
        .collect()
}

/// The `Name=` value of a desktop entry's main group.
pub fn desktop_name(text: &str) -> Option<String> {
    text.lines()
        .take_while(|l| !l.starts_with('[') || l.starts_with("[Desktop Entry]"))
        .find_map(|l| l.strip_prefix("Name="))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
}

/// Splits `is_IS.UTF-8` into language, region and encoding. Missing parts are empty.
pub fn split_locale(locale: &str) -> (String, String, String) {
    let (rest, encoding) = locale.split_once('.').unwrap_or((locale, ""));
    let (language, region) = rest.split_once('_').unwrap_or((rest, ""));
    let encoding = encoding.split('@').next().unwrap_or("");
    (language.to_string(), region.to_string(), encoding.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_reads_whole_seconds_and_labels_the_units() {
        assert_eq!(parse_uptime("12345.67 98765.43\n"), 12345);
        assert_eq!(parse_uptime(""), 0);
        assert_eq!(uptime_label(59), "0 minutes");
        assert_eq!(uptime_label(3_600 + 120), "1 hour, 2 minutes");
        assert_eq!(uptime_label(86_400 * 3 + 3_600 * 4 + 300), "3 days, 4 hours, 5 minutes");
    }

    #[test]
    fn modules_are_parsed_and_sorted() {
        let text = "snd 100 2 - Live 0x0\nac97 4096 0 - Live 0x1\nbad line\n";
        let m = parse_modules(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], Module { name: "ac97".into(), size_bytes: 4096, used_by: 0 });
        assert_eq!(m[1].name, "snd");
    }

    #[test]
    fn boot_volume_is_the_root_mount() {
        let text = "proc /proc proc rw 0 0\n/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/nvme0n1p1 /boot vfat rw 0 0\n";
        assert_eq!(parse_boot_volume(text), "/dev/nvme0n1p2 (ext4)");
        assert_eq!(parse_boot_volume("proc /proc proc rw 0 0"), "");
    }

    #[test]
    fn desktop_name_reads_the_main_group_only() {
        assert_eq!(desktop_name("[Desktop Entry]\nName=Sync\nExec=x\n[Desktop Action a]\nName=Other"), Some("Sync".into()));
        assert_eq!(desktop_name("[Desktop Entry]\nExec=x"), None);
    }

    #[test]
    fn screen_facts_round_to_whole_hertz_and_percent() {
        let s = ScreenFacts::new("eDP-1", 59.94, 1.25);
        assert_eq!((s.name.as_str(), s.refresh_hz, s.scale_percent), ("eDP-1", 60, 125));
        assert_eq!(ScreenFacts::new("x", -1.0, 0.0), ScreenFacts { name: "x".into(), refresh_hz: 0, scale_percent: 0 });
    }

    #[test]
    fn locale_splits_into_language_region_and_encoding() {
        assert_eq!(split_locale("is_IS.UTF-8"), ("is".into(), "IS".into(), "UTF-8".into()));
        assert_eq!(split_locale("C"), ("C".into(), String::new(), String::new()));
        assert_eq!(split_locale(""), (String::new(), String::new(), String::new()));
    }
}
