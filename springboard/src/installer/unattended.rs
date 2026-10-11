//! Unattended ("preset") install for the lab machine. The running system's MCP server (`lifecycle reinstall`) writes
//! `EFI/BIFROST/unattended.toml` on its ESP and netboots the lab ISO; the live installer finds the file on the disk it
//! is about to erase, checks it against the hardware, installs without a display and boots the new system.
//! Format and rationale: docs/installer.md "Unattended install". The password is not in the file (see `password_url`).

use super::{disks, keymaps, plan};
use std::process::Command;

pub const STASH_PATH: &str = "EFI/BIFROST/unattended.toml";
pub const STASH_COPY: &str = "/run/bifrost-unattended.toml";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub version: u32,
    /// Stable id of the only disk that may be erased (`lsblk -dno WWN`, e.g. `eui.e8238fa6...`).
    pub disk_id: String,
    /// NIC address that must be present; a different machine never matches.
    pub mac: String,
    pub hostname: String,
    pub locale: String,
    /// Keyboard layout id as in `keymaps` ("us", "is", ...).
    pub keyboard: String,
    pub timezone: String,
    pub username: String,
    pub full_name: String,
    pub autologin: bool,
    pub mcp: bool,
    /// Where the installer fetches the account password (titan's lab server). Never stored in the repo or the preset.
    pub password_url: String,
    pub created_epoch: u64,
    pub expires_s: u64,
    pub job: String,
}

/// Minimal TOML subset: `key = "string"`, `key = 123`, `key = true`, `#` comments, blank lines. Unknown keys are an error
/// (a typo must not silently fall back to a default on a disk-erasing path).
pub fn parse(text: &str) -> Result<Preset, String> {
    let mut p = Preset { expires_s: 1800, autologin: true, mcp: true, ..Default::default() };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').ok_or(format!("line {}: expected key = value", n + 1))?;
        let (k, v) = (k.trim(), v.trim());
        let sv = || -> Result<String, String> {
            let inner = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).ok_or(format!("line {}: {k} must be a quoted string", n + 1))?;
            if inner.contains('"') || inner.contains('\\') {
                return Err(format!("line {}: {k}: quotes and backslashes are not allowed", n + 1));
            }
            Ok(inner.to_string())
        };
        let nv = || v.parse::<u64>().map_err(|_| format!("line {}: {k} must be a number", n + 1));
        let bv = || match v {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("line {}: {k} must be true or false", n + 1)),
        };
        match k {
            "version" => p.version = nv()? as u32,
            "disk_id" => p.disk_id = sv()?,
            "mac" => p.mac = sv()?.to_lowercase(),
            "hostname" => p.hostname = sv()?,
            "locale" => p.locale = sv()?,
            "keyboard" => p.keyboard = sv()?,
            "timezone" => p.timezone = sv()?,
            "username" => p.username = sv()?,
            "full_name" => p.full_name = sv()?,
            "autologin" => p.autologin = bv()?,
            "mcp" => p.mcp = bv()?,
            "password_url" => p.password_url = sv()?,
            "created_epoch" => p.created_epoch = nv()?,
            "expires_s" => p.expires_s = nv()?,
            "job" => p.job = sv()?,
            _ => return Err(format!("line {}: unknown key {k}", n + 1)),
        }
    }
    if p.version != 1 {
        return Err(format!("unsupported preset version {}", p.version));
    }
    for (name, v) in [("disk_id", &p.disk_id), ("mac", &p.mac), ("hostname", &p.hostname), ("username", &p.username), ("password_url", &p.password_url)] {
        if v.is_empty() {
            return Err(format!("{name} is required"));
        }
    }
    if !plan::valid_username(&p.username) {
        return Err(format!("invalid username {:?}", p.username));
    }
    if !p.hostname.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("invalid hostname {:?}", p.hostname));
    }
    if !p.timezone.is_empty() && !plan::valid_timezone(&p.timezone) {
        return Err(format!("invalid timezone {:?}", p.timezone));
    }
    if !p.password_url.starts_with("http://10.99.0.1:") && !p.password_url.starts_with("http://192.168.") {
        return Err("password_url must point at the lab host".into());
    }
    Ok(p)
}

/// The guard: auto mode proceeds only when the disk carrying the preset is the disk the preset names, the lab NIC is
/// present, and the preset is fresh. `macs` are the addresses of this machine.
pub fn check(p: &Preset, actual_disk_id: &str, macs: &[String], now_epoch: u64) -> Result<(), String> {
    if actual_disk_id.trim().is_empty() || !p.disk_id.eq_ignore_ascii_case(actual_disk_id.trim()) {
        return Err(format!("disk id mismatch: preset {:?}, disk {:?}; refusing auto mode", p.disk_id, actual_disk_id.trim()));
    }
    if !macs.iter().any(|m| m.eq_ignore_ascii_case(&p.mac)) {
        return Err(format!("NIC {} not present; this is not the lab machine", p.mac));
    }
    if now_epoch > p.created_epoch.saturating_add(p.expires_s) {
        return Err("preset expired".into());
    }
    if p.created_epoch > now_epoch.saturating_add(3600) {
        return Err("preset is from the future (clock?)".into());
    }
    Ok(())
}

/// Boot entry number of the `BifrOSt` row in `efibootmgr` output ("Boot0003* BifrOSt ...").
pub fn bifrost_entry(efibootmgr: &str) -> Option<String> {
    efibootmgr.lines().find(|l| l.starts_with("Boot") && l.len() > 9 && l.contains("BifrOSt")).map(|l| l[4..8].to_string())
}

fn out(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// Looks for a preset on the ESP of every candidate disk. Returns the text and the disk it was found on.
pub fn find_stash() -> Option<(String, String)> {
    // a restarted installer (lab sync) reuses the copy taken before the first attempt
    if let Ok(t) = std::fs::read_to_string(STASH_COPY) {
        let disk = std::fs::read_to_string(format!("{STASH_COPY}.disk")).unwrap_or_default();
        return Some((t, disk.trim().to_string()));
    }
    for d in disks::probe() {
        for l in out("lsblk", &["-nrpo", "PATH,FSTYPE", &d.path]).lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 2 || f[1] != "vfat" {
                continue;
            }
            let _ = std::fs::create_dir_all("/run/bifrost-esp");
            if !Command::new("mount").args(["-o", "ro", "-t", "vfat", f[0], "/run/bifrost-esp"]).status().map(|s| s.success()).unwrap_or(false) {
                continue;
            }
            let t = std::fs::read_to_string(format!("/run/bifrost-esp/{STASH_PATH}"));
            let _ = Command::new("umount").arg("/run/bifrost-esp").status();
            if let Ok(t) = t {
                let _ = std::fs::write(STASH_COPY, &t);
                let _ = std::fs::write(format!("{STASH_COPY}.disk"), &d.path);
                return Some((t, d.path.clone()));
            }
        }
    }
    None
}

/// Runs the whole unattended install for `text` found on `disk_path`. Ok(()) after the new system is installed and
/// BootNext points at it (the caller reboots). Every refusal happens before the disk is touched.
pub fn run(text: &str, disk_path: &str) -> Result<(), String> {
    let p = parse(text)?;
    let disk = disks::probe().into_iter().find(|d| d.path == disk_path).ok_or(format!("no disk {disk_path}"))?;
    let id = out("lsblk", &["-dno", "WWN", &disk.path]);
    let macs: Vec<String> = out("sh", &["-c", "cat /sys/class/net/*/address 2>/dev/null"]).split_whitespace().map(str::to_lowercase).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    check(&p, &id, &macs, now)?;
    let pw = out("curl", &["-fsS", "--max-time", "10", &p.password_url]);
    let pw = pw.trim();
    if pw.len() < 8 {
        return Err("could not fetch the account password from the lab host".into());
    }
    let layout = keymaps::layout_index(if p.keyboard.is_empty() { "us" } else { &p.keyboard })
        .and_then(|i| keymaps::layouts().nth(i))
        .or_else(|| keymaps::layouts().next())
        .ok_or("no keyboard layouts")?;
    let cfg = plan::Config {
        disk,
        mode: disks::Mode::Erase,
        locale: if p.locale.is_empty() { "en_US.UTF-8".into() } else { p.locale.clone() },
        layout,
        full_name: if p.full_name.is_empty() { p.username.clone() } else { p.full_name.clone() },
        username: p.username.clone(),
        password: pw.to_string(),
        hostname: p.hostname.clone(),
        timezone: p.timezone.clone(),
    };
    let progress: plan::Shared = Default::default();
    plan::install(&cfg, &progress);
    if let Some(e) = progress.lock().unwrap().failed.clone() {
        return Err(format!("install failed: {e}"));
    }
    let _ = std::fs::remove_file(STASH_COPY);
    match bifrost_entry(&out("efibootmgr", &[])) {
        Some(n) => {
            let _ = Command::new("efibootmgr").args(["-n", &n]).status();
        }
        None => eprintln!("bifrost-installer: no BifrOSt boot entry found; leaving BootOrder to the firmware"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "version = 1\ndisk_id = \"eui.e8238fa6bf530001001b448b4a52a8bb\"\nmac = \"00:23:24:DC:C8:40\"\nhostname = \"midget\"\n\
        username = \"tester\"\npassword_url = \"http://10.99.0.1:8078/lab/tester.pw\"\ncreated_epoch = 1000\nexpires_s = 600\n# c\ntimezone = \"Atlantic/Reykjavik\"\n";

    #[test]
    fn parses_the_preset_and_applies_defaults() {
        let p = parse(GOOD).unwrap();
        assert_eq!((p.username.as_str(), p.mac.as_str(), p.expires_s), ("tester", "00:23:24:dc:c8:40", 600));
        assert!(p.autologin && p.mcp);
    }

    #[test]
    fn rejects_bad_presets() {
        assert!(parse("version = 2\n").is_err());
        assert!(parse(&GOOD.replace("hostname = \"midget\"\n", "")).is_err());
        assert!(parse(&format!("{GOOD}bogus = 1\n")).unwrap_err().contains("unknown key"));
        assert!(parse(&GOOD.replace("\"tester\"", "\"../x\"")).is_err());
        assert!(parse(&GOOD.replace("10.99.0.1", "evil.example")).is_err());
        assert!(parse(&GOOD.replace("Atlantic/Reykjavik", "../../etc")).is_err());
    }

    #[test]
    fn guard_refuses_wrong_disk_nic_and_stale_presets() {
        let p = parse(GOOD).unwrap();
        let macs = vec!["00:23:24:dc:c8:40".to_string()];
        assert!(check(&p, "EUI.E8238FA6BF530001001B448B4A52A8BB", &macs, 1100).is_ok());
        assert!(check(&p, "eui.other", &macs, 1100).unwrap_err().contains("disk id mismatch"));
        assert!(check(&p, "", &macs, 1100).is_err());
        assert!(check(&p, &p.disk_id, &["aa:bb:cc:dd:ee:ff".to_string()], 1100).unwrap_err().contains("not the lab machine"));
        assert!(check(&p, &p.disk_id, &macs, 1601).unwrap_err().contains("expired"));
    }

    #[test]
    fn finds_the_bifrost_boot_entry() {
        let t = "BootCurrent: 0001\nBootOrder: 0009,0006\nBoot0006* UEFI: IPV4 Intel\nBoot0009* BifrOSt\tHD(1,GPT)\n";
        assert_eq!(bifrost_entry(t).as_deref(), Some("0009"));
        assert_eq!(bifrost_entry("Boot0006* UEFI: IPV4\n"), None);
    }
}
