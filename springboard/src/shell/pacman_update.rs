//! Software Update on an Arch desktop: the shell is the `springboard-shell` package, so the
//! installed version comes from `pacman -Q` and updates from pacman instead of the OTA server.
//!
//! Checking is read-only. Installing invokes the independent recovery guard,
//! which preserves the shell package and rejects base-system upgrades until a
//! whole-root recovery path is available. The UI confirms before installation.

use super::update::Manifest;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

pub const PACKAGE: &str = "springboard-shell";

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Found {
    /// the installed package version ("0.1.0-1"), when the package is installed
    pub installed: Option<String>,
    pub update: Option<Manifest>,
}

/// "springboard-shell 0.1.0-1" (`pacman -Q`) to "0.1.0-1".
pub fn parse_installed(out: &str) -> Option<String> {
    let mut f = out.split_whitespace();
    (f.next()? == PACKAGE).then(|| f.next().map(String::from)).flatten()
}

/// The new version from `checkupdates` or `pacman -Qu` output, whose lines read
/// "springboard-shell 0.1.0-1 -> 0.2.0-1".
pub fn parse_update(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[0] == PACKAGE && f[2] == "->").then(|| f[3].to_string())
    })
}

/// "Download Size  : 12.30 MiB" from `pacman -Si` in bytes; 0 when absent.
pub fn parse_download_size(si: &str) -> u64 {
    let Some(line) = si.lines().find(|l| l.starts_with("Download Size")) else { return 0 };
    let mut f = line.split(':').nth(1).unwrap_or("").split_whitespace();
    let (Some(n), Some(unit)) = (f.next().and_then(|n| n.parse::<f64>().ok()), f.next()) else { return 0 };
    let mult = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return 0,
    };
    (n * mult) as u64
}

/// The "Description" and "Version" style value of a `pacman -Si` field.
fn field(si: &str, name: &str) -> Option<String> {
    si.lines().find(|l| l.starts_with(name))?.split_once(':').map(|(_, v)| v.trim().to_string())
}

fn manifest(version: String, si: &str) -> Manifest {
    Manifest { version, build: None, sha256: String::new(), size: parse_download_size(si), url: String::new(), notes: field(si, "Description"), changelog: vec![] }
}

fn run(cmd: &str, args: &[&str]) -> Result<(i32, String), String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    Ok((out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned()))
}

fn on_path(name: &str) -> bool {
    std::env::var("PATH").is_ok_and(|p| p.split(':').any(|d| std::path::Path::new(d).join(name).is_file()))
}

/// Asks pacman what is installed and whether the package has an update; blocking, read-only.
pub fn check() -> Result<Found, String> {
    if !on_path("pacman") {
        return Err("pacman is not installed".into());
    }
    let installed = run("pacman", &["-Q", PACKAGE]).ok().and_then(|(_, o)| parse_installed(&o));
    if installed.is_none() {
        return Err(format!("{PACKAGE} is not installed as a package"));
    }
    // checkupdates exits 2 when nothing is pending, 0 with a list
    // `checkupdates` needs fakeroot; without it (or `checkupdates`) a root shell refreshes the sync databases itself
    let pending = if on_path("checkupdates") && on_path("fakeroot") {
        run("checkupdates", &[])?.1
    } else {
        if is_root() {
            let _ = run("pacman", &["-Sy", "--noconfirm"]);
        }
        run("pacman", &["-Qu", PACKAGE])?.1
    };
    let update = parse_update(&pending).map(|v| manifest(v, &run("pacman", &["-Si", PACKAGE]).map(|r| r.1).unwrap_or_default()));
    Ok(Found { installed, update })
}

/// Whether this process is root (the OS shell is; a session shell is not).
fn is_root() -> bool {
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// The command line of the install, as (program, args).
pub fn install_command(root: bool) -> (&'static str, Vec<&'static str>) {
    if root {
        ("/usr/bin/bifrost-update-guard", vec!["install"])
    } else {
        ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "install"])
    }
}

/// The last non-empty line of pacman's output, as the failure text.
fn last_line(s: &str) -> String {
    s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("pacman failed").trim().to_string()
}

/// The command line that starts the rollback, as (program, args). It only starts a separate unit,
/// so it returns at once; the shell is then stopped by the rollback itself.
pub fn rollback_command(root: bool) -> (&'static str, Vec<&'static str>) {
    if root {
        ("/usr/bin/bifrost-update-guard", vec!["rollback-start"])
    } else {
        ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "rollback-start"])
    }
}

fn run_guard(prog: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(prog).args(args).output().map_err(|e| format!("{prog}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let (err, stdout) = (String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
        // the page shows one line; the journal keeps the whole run for diagnosis
        eprintln!("springboard: {prog} {} failed ({}):\n{stdout}{err}", args.join(" "), out.status);
        Err(last_line(if err.trim().is_empty() { &stdout } else { &err }))
    }
}

/// Upgrades the system including the shell package; blocking. The caller has asked the user.
pub fn install() -> Result<(), String> {
    let (prog, args) = install_command(is_root());
    run_guard(prog, &args)
}

/// Starts the rollback to the previous system; the caller has asked the user.
pub fn rollback() -> Result<(), String> {
    let (prog, args) = rollback_command(is_root());
    run_guard(prog, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_version_is_the_second_word() {
        assert_eq!(parse_installed("springboard-shell 0.1.0-1\n").as_deref(), Some("0.1.0-1"));
        assert_eq!(parse_installed("other 1-1"), None);
        assert_eq!(parse_installed(""), None);
    }

    #[test]
    fn update_lines_name_the_new_version() {
        let out = "linux 7.2.4-1 -> 7.2.5-1\nspringboard-shell 0.1.0-1 -> 0.2.0-1\n";
        assert_eq!(parse_update(out).as_deref(), Some("0.2.0-1"));
        assert_eq!(parse_update("linux 1-1 -> 2-1\n"), None);
        assert_eq!(parse_update(""), None);
    }

    #[test]
    fn download_size_converts_units() {
        assert_eq!(parse_download_size("Name : x\nDownload Size  : 12.00 MiB\nInstalled Size : 1 MiB"), 12 * 1024 * 1024);
        assert_eq!(parse_download_size("Download Size  : 512.00 KiB"), 512 * 1024);
        assert_eq!(parse_download_size("Name : x"), 0);
        assert_eq!(parse_download_size("Download Size  : 1 parsec"), 0);
    }

    #[test]
    fn manifest_carries_size_and_description() {
        let m = manifest("0.2.0-1".into(), "Description    : SpringBoard shell\nDownload Size  : 2.00 MiB\n");
        assert_eq!((m.version.as_str(), m.size, m.notes.as_deref()), ("0.2.0-1", 2 * 1024 * 1024, Some("SpringBoard shell")));
    }

    #[test]
    fn rollback_starts_the_guard_unit_directly_or_through_sudo() {
        assert_eq!(rollback_command(true), ("/usr/bin/bifrost-update-guard", vec!["rollback-start"]));
        assert_eq!(rollback_command(false), ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "rollback-start"]));
    }

    #[test]
    fn root_installs_directly_and_others_through_sudo() {
        assert_eq!(install_command(true), ("/usr/bin/bifrost-update-guard", vec!["install"]));
        assert_eq!(install_command(false), ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "install"]));
    }

    #[test]
    fn failure_text_is_the_last_line() {
        assert_eq!(last_line("a\n\nerror: failed to init transaction\n\n"), "error: failed to init transaction");
        assert_eq!(last_line(""), "pacman failed");
    }
}
