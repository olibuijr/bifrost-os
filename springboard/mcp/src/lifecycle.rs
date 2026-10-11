//! `lifecycle` tool: restart, netboot and unattended reinstall of the LAB machine (Midget). Docs: docs/bifrost-mcp.md
//! "Lifecycle", docs/installer.md "Unattended install". Destructive actions are guarded three ways: the machine must be
//! the lab machine (DMI product + lab NIC), `confirm` must equal "<ACTION> <hostname>", and the preset names the disk id.
//! Pure helpers are unit tested; the action itself shells out to efibootmgr, lsblk, findmnt and systemd-run.

use crate::sys;
use crate::tools::{err, s, shq, text};
use serde_json::Value;
use std::time::Duration;

/// DMI `product_name` of the lab machine (Lenovo ThinkCentre M900) and its onboard NIC (iVentoy's client id 0023-24dc-c840).
pub const LAB_PRODUCT: &str = "10FLS2WK00";
pub const LAB_MAC: &str = "00:23:24:dc:c8:40";
pub const DEFAULT_PASSWORD_URL: &str = "http://10.99.0.1:8078/lab/tester.pw";
pub const STASH: &str = "/boot/EFI/BIFROST/unattended.toml";
pub const JOBS: &str = "/var/lib/bifrost-mcp/jobs";

/// The phrase `confirm` must equal: the action in capitals, a space, the machine's hostname ("REINSTALL midget").
pub fn confirm_phrase(action: &str, hostname: &str) -> String {
    format!("{} {}", action.to_uppercase(), hostname.trim())
}

/// The lab machine is identified by hardware, not by a file the previous install could have lost: product name AND NIC.
pub fn is_lab_machine(product: &str, macs: &[String]) -> bool {
    product.trim() == LAB_PRODUCT && macs.iter().any(|m| m.trim().eq_ignore_ascii_case(LAB_MAC))
}

/// The PXE boot entry number ("0006") from `efibootmgr` output; IPv4 rows win over IPv6.
pub fn pxe_entry(efibootmgr: &str) -> Option<String> {
    let mut rows: Vec<&str> = efibootmgr.lines().filter(|l| l.starts_with("Boot") && l.len() > 9 && (l.contains("PXE") || l.contains("IPv4") || l.contains("IPV4"))).collect();
    rows.sort_by_key(|l| (!(l.contains("IPv4") || l.contains("IPV4")), l.to_string()));
    rows.first().map(|l| l[4..8].to_string())
}

/// The preset the netbooted installer reads (see `springboard/src/installer/unattended.rs`). No password in it.
pub struct PresetArgs<'a> {
    pub disk_id: &'a str,
    pub hostname: &'a str,
    pub locale: &'a str,
    pub keyboard: &'a str,
    pub timezone: &'a str,
    pub password_url: &'a str,
    pub now_epoch: u64,
    pub job: &'a str,
}

pub fn preset_toml(p: &PresetArgs) -> String {
    format!(
        "# written by bifrost-mcp lifecycle reinstall; read by bifrost-installer on the netbooted lab ISO\n\
         version = 1\ndisk_id = \"{}\"\nmac = \"{}\"\nhostname = \"{}\"\nlocale = \"{}\"\nkeyboard = \"{}\"\ntimezone = \"{}\"\n\
         username = \"tester\"\nfull_name = \"Tester\"\nautologin = true\nmcp = true\npassword_url = \"{}\"\n\
         created_epoch = {}\nexpires_s = 1800\njob = \"{}\"\n",
        p.disk_id, LAB_MAC, p.hostname, p.locale, p.keyboard, p.timezone, p.password_url, p.now_epoch, p.job
    )
}

/// Values that go into the preset or a shell word must be plain: letters, digits and `._:/@+-`.
pub fn plain(v: &str) -> bool {
    !v.is_empty() && v.len() <= 120 && v.chars().all(|c| c.is_ascii_alphanumeric() || "._:/@+-".contains(c))
}

fn run(cmd: &str, secs: u64) -> Result<String, String> {
    let r = sys::exec(cmd, false, None, Duration::from_secs(secs), None)?;
    if r.code == Some(0) {
        Ok(r.out.trim().to_string())
    } else {
        Err(format!("{cmd}: {}", if r.err.trim().is_empty() { r.out.trim() } else { r.err.trim() }))
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Machine facts the guard needs: (hostname, DMI product, NIC addresses).
fn identity() -> (String, String, Vec<String>) {
    let host = sys::read("/proc/sys/kernel/hostname").trim().to_string();
    let product = sys::read("/sys/class/dmi/id/product_name").trim().to_string();
    let macs = run("cat /sys/class/net/*/address 2>/dev/null", 5).unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
    (host, product, macs)
}

/// Schedules `script` to run detached in `delay` seconds, so the tool call returns before the machine goes down.
fn schedule(job: &str, script: &str, delay: u32) -> Result<(), String> {
    run(&format!("systemd-run --on-active={delay} --unit=bifrost-lifecycle-{job} --collect sh -c {}", shq(script)), 10).map(|_| ())
}

fn record(job: &str, body: &str) {
    let _ = std::fs::create_dir_all(JOBS);
    let _ = std::fs::write(format!("{JOBS}/{job}.json"), body);
}

pub fn lifecycle(a: &Value) -> Result<Value, String> {
    let action = s(a, "action").ok_or("action is required: status | restart | netboot | reinstall")?;
    let (host, product, macs) = identity();
    let job = format!("{}-{action}", now_epoch());
    match action {
        "status" => {
            let boot = run("efibootmgr 2>/dev/null | head -3", 10).unwrap_or_default();
            let ver = sys::read("/usr/share/springboard/version").trim().to_string();
            let up = sys::read("/proc/uptime");
            let stash = std::path::Path::new(STASH).exists();
            let jobs = std::fs::read_dir(JOBS).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            Ok(text(format!(
                "host={host} lab_machine={} uptime={}s springboard={ver} unattended_preset_pending={stash}\njobs={jobs}\n{boot}",
                is_lab_machine(&product, &macs), up.split('.').next().unwrap_or("?")
            )))
        }
        "restart" => {
            record(&job, &format!("{{\"action\":\"restart\",\"at\":{}}}", now_epoch()));
            schedule(&job, "sync; systemctl reboot", 3)?;
            Ok(text(format!("job={job}: rebooting in 3 s; the MCP session drops and reconnects. Check `system overview` uptime afterwards.")))
        }
        "netboot" | "reinstall" => {
            if !is_lab_machine(&product, &macs) {
                return Err(format!("refused: this is not the lab machine (product {product:?}, expected {LAB_PRODUCT} with NIC {LAB_MAC})"));
            }
            let want = confirm_phrase(action, &host);
            if s(a, "confirm") != Some(want.as_str()) {
                return Err(format!("refused: pass confirm=\"{want}\""));
            }
            let efi = run("efibootmgr", 10)?;
            let pxe = pxe_entry(&efi).ok_or("no PXE boot entry in efibootmgr")?;
            let mut note = String::new();
            if action == "reinstall" {
                let fstype = run("findmnt -no FSTYPE /boot", 5).unwrap_or_default();
                if fstype != "vfat" {
                    return Err(format!("refused: /boot is {fstype:?}, not the vfat ESP; cannot stash the preset"));
                }
                let disk_id = run(r#"src=$(findmnt -no SOURCE /boot); pk=$(lsblk -no PKNAME "$src" | head -1); lsblk -dno WWN "/dev/$pk""#, 10)?;
                let tz = run("readlink -f /etc/localtime | sed 's|.*/zoneinfo/||'", 5).unwrap_or_default();
                let zone = if plain(&tz) && !tz.starts_with('/') { tz } else { "Atlantic/Reykjavik".into() };
                let url = s(a, "password_url").unwrap_or(DEFAULT_PASSWORD_URL);
                let locale = s(a, "locale").unwrap_or("en_US.UTF-8");
                let keyboard = s(a, "keyboard").unwrap_or("us");
                for (n, v) in [("disk_id", disk_id.as_str()), ("password_url", url), ("locale", locale), ("keyboard", keyboard), ("hostname", host.as_str())] {
                    if !plain(v) {
                        return Err(format!("refused: {n} {v:?} has unexpected characters"));
                    }
                }
                let toml = preset_toml(&PresetArgs { disk_id: &disk_id, hostname: &host, locale, keyboard, timezone: &zone, password_url: url, now_epoch: now_epoch(), job: &job });
                std::fs::create_dir_all("/boot/EFI/BIFROST").map_err(|e| format!("/boot/EFI/BIFROST: {e}"))?;
                std::fs::write(STASH, toml).map_err(|e| format!("{STASH}: {e}"))?;
                note = format!(" preset written to {STASH} (disk {disk_id}, expires in 30 min);");
            }
            record(&job, &format!("{{\"action\":\"{action}\",\"bootnext\":\"{pxe}\",\"at\":{}}}", now_epoch()));
            schedule(&job, &format!("efibootmgr -n {pxe} >/dev/null; sync; systemctl reboot"), 3)?;
            Ok(text(format!(
                "job={job}:{note} BootNext={pxe} (PXE), rebooting in 3 s into the lab ISO{}. The result is observable on the next boot: ssh/MCP answers again on the live ISO{}.",
                if action == "reinstall" { "; the installer erases the disk, installs, then boots the new system" } else { "" },
                if action == "reinstall" { ", then as the installed system with the tester account" } else { "" }
            )))
        }
        x => Ok(err(format!("unknown action {x:?}: status | restart | netboot | reinstall"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_phrase_is_action_plus_hostname() {
        assert_eq!(confirm_phrase("reinstall", "midget\n"), "REINSTALL midget");
        assert_eq!(confirm_phrase("netboot", "midget"), "NETBOOT midget");
    }

    #[test]
    fn only_the_lab_hardware_passes_the_guard() {
        let mac = vec!["aa:bb:cc:dd:ee:ff".to_string(), "00:23:24:DC:C8:40".to_string()];
        assert!(is_lab_machine("10FLS2WK00\n", &mac));
        assert!(!is_lab_machine("MS-7C37", &mac));
        assert!(!is_lab_machine("10FLS2WK00", &["aa:bb:cc:dd:ee:ff".to_string()]));
        assert!(!is_lab_machine("", &[]));
    }

    #[test]
    fn picks_the_ipv4_pxe_entry() {
        let t = "BootOrder: 0001,0006\nBoot0001* BifrOSt\tHD(1)\nBoot0006* UEFI: IPV4 Intel(R) Ethernet\tPciRoot\nBoot0007* UEFI: IPV6 Intel(R) Ethernet\tPciRoot\n";
        assert_eq!(pxe_entry(t).as_deref(), Some("0006"));
        assert_eq!(pxe_entry("Boot0001* BifrOSt\n"), None);
    }

    #[test]
    fn preset_has_no_password_and_round_trips_fields() {
        let t = preset_toml(&PresetArgs { disk_id: "eui.abc", hostname: "midget", locale: "en_US.UTF-8", keyboard: "us", timezone: "Atlantic/Reykjavik", password_url: DEFAULT_PASSWORD_URL, now_epoch: 42, job: "42-reinstall" });
        assert!(t.contains("disk_id = \"eui.abc\"") && t.contains("username = \"tester\"") && t.contains("created_epoch = 42"));
        assert!(!t.to_lowercase().contains("password =") && t.contains(LAB_MAC));
    }

    #[test]
    fn plain_rejects_shell_and_toml_metacharacters() {
        assert!(plain("eui.e8238fa6bf530001") && plain("http://10.99.0.1:8078/lab/tester.pw"));
        assert!(!plain("a b") && !plain("a\"b") && !plain("$(x)") && !plain(""));
    }

    #[test]
    fn destructive_actions_refuse_without_the_phrase() {
        // not the lab machine in the test environment, so the guard refuses before anything is touched
        let r = lifecycle(&serde_json::json!({"action":"reinstall","confirm":"REINSTALL x"}));
        assert!(r.unwrap_err().starts_with("refused"));
        let r = lifecycle(&serde_json::json!({"action":"netboot"}));
        assert!(r.unwrap_err().starts_with("refused"));
    }
}
