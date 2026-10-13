//! Independent, statically linked shell-package recovery. No application data rollback.
use std::{
    fs, io,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::MetadataExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
mod system;
const STORE: &str = "/var/lib/bifrost-recovery";
const FILES: &[&str] = &[
    "usr/bin/bifrost-open-url",
    "usr/share/applications/bifrost-browser.desktop",
    "etc/xdg/mimeapps.list",
    "usr/bin/springboard",
    "usr/lib/bifrost/session",
    "usr/lib/bifrost/session-env",
    "usr/bin/bifrost-apps",
    "usr/lib/springboard/seed-data",
    "usr/lib/systemd/system/springboard.service",
];
unsafe extern "C" {
    fn geteuid() -> u32;
}
fn err(s: &str) -> io::Error {
    io::Error::other(s)
}
fn save(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()
}
fn copy(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst.parent().unwrap())?;
    let tmp = dst.with_extension("bifrost-restore");
    fs::copy(src, &tmp)?;
    fs::set_permissions(&tmp, fs::metadata(src)?.permissions())?;
    fs::File::open(&tmp)?.sync_all()?;
    fs::rename(tmp, dst)?;
    fs::File::open(dst.parent().unwrap())?.sync_all()
}
fn tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            tree(&e.path(), &dst.join(e.file_name()))?;
        } else if e.file_type()?.is_file() {
            copy(&e.path(), &dst.join(e.file_name()))?;
        } else {
            return Err(err("Unexpected symlink in package database"));
        }
    }
    Ok(())
}
fn state(store: &Path) -> String {
    fs::read_to_string(store.join("state")).unwrap_or_default()
}
fn phase(store: &Path, value: &str) -> io::Result<()> {
    save(&store.join("state"), value.as_bytes())
}
fn safe_version(v: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+_:-".contains(&b))
}
fn prepare(root: &Path, store: &Path, version: &str) -> io::Result<()> {
    if !safe_version(version) {
        return Err(err("Invalid package version"));
    }
    if matches!(state(store).trim(), "prepared" | "probation" | "restoring") {
        return Err(err("A previous update has not finished probation"));
    }
    fs::create_dir_all(store)?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(store, fs::Permissions::from_mode(0o700))?;
    }
    let snap = store.join("previous");
    if snap.exists() {
        fs::remove_dir_all(&snap)?;
    }
    fs::create_dir_all(&snap)?;
    for file in FILES {
        let src = root.join(file);
        if src.is_file() {
            copy(&src, &snap.join(file))?;
        } else if *file == "usr/bin/springboard" {
            return Err(err("No installed shell to preserve"));
        }
    }
    let dbname = format!("springboard-shell-{version}");
    tree(
        &root.join("var/lib/pacman/local").join(&dbname),
        &snap.join("database"),
    )?;
    let meta = fs::metadata(root.join("usr/bin/springboard"))?;
    save(&store.join("previous-version"), version.as_bytes())?;
    save(
        &store.join("previous-inode"),
        format!("{}:{}", meta.dev(), meta.ino()).as_bytes(),
    )?;
    phase(store, "prepared")
}
fn restore(root: &Path, store: &Path) -> io::Result<()> {
    let v = fs::read_to_string(store.join("previous-version"))?;
    if !safe_version(v.trim()) {
        return Err(err("Invalid recovery metadata"));
    }
    let snap = store.join("previous");
    if !snap.join("usr/bin/springboard").is_file() || !snap.join("database/desc").is_file() {
        return Err(err("Incomplete recovery snapshot"));
    }
    phase(store, "restoring")?;
    for file in FILES {
        let src = snap.join(file);
        if src.is_file() {
            copy(&src, &root.join(file))?;
        }
    }
    let db = root.join("var/lib/pacman/local");
    let wanted = format!("springboard-shell-{}", v.trim());
    let staging = db.join("bifrost-recovery-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    tree(&snap.join("database"), &staging)?;
    for e in fs::read_dir(&db)? {
        let e = e?;
        if e.file_name()
            .to_string_lossy()
            .starts_with("springboard-shell-")
            && e.file_type()?.is_dir()
        {
            fs::remove_dir_all(e.path())?;
        }
    }
    fs::rename(staging, db.join(wanted))?;
    phase(store, "rolled-back")
}
fn output(args: &[&str]) -> io::Result<String> {
    let o = Command::new("pacman").args(args).output()?;
    if !o.status.success() {
        return Err(err(&String::from_utf8_lossy(&o.stderr)));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}
fn installed() -> io::Result<String> {
    let text = output(&["-Q", "springboard-shell"])?;
    text.split_whitespace()
        .nth(1)
        .map(str::to_owned)
        .ok_or_else(|| err("No installed shell package"))
}
fn run(program: &str, args: &[&str]) -> io::Result<()> {
    if !Command::new(program).args(args).status()?.success() {
        return Err(err("Command failed"));
    }
    Ok(())
}
fn running(inode: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            && fs::metadata(e.path().join("exe"))
                .is_ok_and(|m| format!("{}:{}", m.dev(), m.ino()) == inode)
    })
}
fn probe() -> bool {
    if !Path::new("/run/bifrost/user/wayland-0").exists() {
        return false;
    }
    let Ok(mut s) = TcpStream::connect_timeout(
        &"127.0.0.1:7777".parse::<SocketAddr>().unwrap(),
        Duration::from_secs(1),
    ) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
    if s.write_all(b"state\n").is_err() {
        return false;
    }
    let mut buf = [0; 8192];
    let Ok(n) = s.read(&mut buf) else {
        return false;
    };
    let text = String::from_utf8_lossy(&buf[..n]);
    text.contains("screen=") && text.contains("screen_on=")
}
fn watch() -> io::Result<()> {
    let store = Path::new(STORE);
    let mut start = None;
    let mut healthy = None;
    let mut beat = String::new();
    let mut advanced = Instant::now();
    if system::fallback_boot() || state(store).trim() == "system-restoring" {
        return system::restore();
    }
    loop {
        match state(store).trim() {
            "system-prepared" => {
                if system::new_boot() {
                    phase(store, "system-probation")?;
                    start = Some(Instant::now());
                }
            }
            "prepared" => {
                let old = fs::read_to_string(store.join("previous-inode"))?;
                // Download/install alone must not restart or reject the user's running old session.
                if !running(old.trim())
                    && !Path::new("/var/lib/pacman/db.lck").exists()
                    && fs::metadata("/usr/bin/springboard")
                        .is_ok_and(|m| format!("{}:{}", m.dev(), m.ino()) != old.trim())
                {
                    phase(store, "probation")?;
                    start = Some(Instant::now());
                }
            }
            "probation" | "system-probation" => {
                let started = *start.get_or_insert_with(Instant::now);
                let new = fs::metadata("/usr/bin/springboard")
                    .map(|m| format!("{}:{}", m.dev(), m.ino()))
                    .unwrap_or_default();
                let next = fs::read_to_string("/var/lib/springboard/heartbeat").unwrap_or_default();
                if !next.is_empty() && next != beat {
                    beat = next;
                    advanced = Instant::now();
                }
                if running(&new) && probe() && advanced.elapsed() < Duration::from_secs(15) {
                    let since = healthy.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_secs(30) {
                        if state(store).trim() == "system-probation" {
                            system::accept()?;
                        } else {
                            phase(store, "accepted")?;
                        }
                        println!("BifrOSt update accepted after 30s responsive session probation");
                        return Ok(());
                    }
                } else {
                    healthy = None;
                }
                if started.elapsed() >= Duration::from_secs(90) {
                    if state(store).trim() == "system-probation" {
                        return system::fallback_reset();
                    }
                    // Recover independently of the possibly broken session service.
                    let _ = run("systemctl", &["stop", "springboard.service"]);
                    restore(Path::new("/"), store)?;
                    let _ = run("systemctl", &["daemon-reload"]);
                    let _ = run("systemctl", &["reset-failed", "springboard.service"]);
                    run("systemctl", &["start", "springboard.service"])?;
                    println!("BifrOSt update rolled back after failed session probation");
                    return Ok(());
                }
            }
            "restoring" => {
                restore(Path::new("/"), store)?;
                let _ = run("systemctl", &["daemon-reload"]);
                run("systemctl", &["restart", "springboard.service"])?;
                return Ok(());
            }
            _ => return Ok(()),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
fn install() -> io::Result<()> {
    run("pacman", &["-Sy", "--noconfirm"])?;
    let plan = output(&["-Sup", "--print-format", "%n", "springboard-shell"])?;
    let names: Vec<_> = plan
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let full = system::supported();
    if !full && names.iter().any(|n| *n != "springboard-shell") {
        return Err(err("Base-system upgrades require a recoverable root snapshot. Managed update refused; no packages changed."));
    }
    if names.is_empty() {
        println!("System package is up to date");
        return Ok(());
    }
    if full {
        system::prepare()?;
    } else {
        prepare(Path::new("/"), Path::new(STORE), &installed()?)?;
    }
    run("systemctl", &["start", "bifrost-update-guard.service"])?;
    if let Err(e)=run("pacman",&["-Su","--noconfirm","--overwrite","/usr/bin/springboard,/usr/lib/systemd/system/springboard.service,/usr/lib/springboard/seed-data,/usr/bin/bifrost-update-guard,/usr/lib/systemd/system/bifrost-update-guard.service","springboard-shell"]){if full {system::fallback_reset()?;} else {restore(Path::new("/"),Path::new(STORE))?;}return Err(e);}
    // the first frame's caches for the new shell, before the reboot (boot-08); best effort, the shell rebuilds on a miss
    let _ = Command::new("/usr/bin/springboard").arg("--first-frame-cache").status();
    if full {
        system::ready()?;
        println!("System update staged; reboot required for kernel/root probation");
    }
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let op = args.get(1).map(String::as_str).unwrap_or("status");
    let result = if op == "status" {
        println!(
            "phase={}\nprevious={}",
            state(Path::new(STORE)).trim(),
            fs::read_to_string(Path::new(STORE).join("previous-version"))
                .unwrap_or_default()
                .trim()
        );
        Ok(())
    } else if unsafe { geteuid() } != 0 {
        Err(err("Recovery operations require root"))
    } else {
        match op {
            "prepare" => {
                if system::supported() {
                    system::prepare()
                } else {
                    installed().and_then(|v| prepare(Path::new("/"), Path::new(STORE), &v))
                }
            }
            "watch" => watch(),
            "install" => install(),
            // Runs the rollback in its own unit: the shell's service is stopped during the rollback,
            // and a child of that service would be killed with it.
            "rollback-start" => run("systemctl", &["start", "--no-block", "bifrost-rollback.service"]),
            "apply" => {
                if state(Path::new(STORE)).trim() == "system-prepared" {
                    // wait for pacman and the initramfs to finish before rebooting into them
                    let deadline = Instant::now() + Duration::from_secs(600);
                    loop {
                        if system::staged() {
                            break run("sync", &[]).and_then(|_| run("systemctl", &["reboot"]));
                        }
                        if Instant::now() >= deadline || state(Path::new(STORE)).trim() != "system-prepared" {
                            break Err(err("The update is not staged yet (pacman still running); not rebooting"));
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                } else {
                    run("systemctl", &["restart", "springboard.service"])
                }
            }
            // Unified kernel image and per-machine initramfs (docs/boot-architecture.md): pacman's hook and the installer
            // build it, the record unit (after desktop-ready) rebuilds it when the loaded modules change.
            "uki" => system::build_boot_image(),
            "record-modules" => system::record_modules().map(|_| ()),
            "rollback" if Path::new(STORE).join("previous-bootid").exists() => system::restore(),
            "rollback" => {
                let _ = run("systemctl", &["stop", "springboard.service"]);
                restore(Path::new("/"), Path::new(STORE)).and_then(|_| {
                    let _ = run("systemctl", &["daemon-reload"]);
                    run("systemctl", &["restart", "springboard.service"])
                })
            }
            _ => Err(err(
                "Usage: bifrost-update-guard status|prepare|install|rollback-start|watch|rollback",
            )),
        }
    };
    if let Err(e) = result {
        eprintln!("BifrOSt recovery: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_restores_files_and_database() {
        let base =
            std::env::temp_dir().join(format!("bifrost-recovery-test-{}", std::process::id()));
        let root = base.join("root");
        let store = base.join("state");
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::write(root.join("usr/bin/springboard"), b"good").unwrap();
        fs::create_dir_all(root.join("var/lib/pacman/local/springboard-shell-1.0-1")).unwrap();
        fs::write(
            root.join("var/lib/pacman/local/springboard-shell-1.0-1/desc"),
            b"old-db",
        )
        .unwrap();
        fs::create_dir_all(root.join("home/tester")).unwrap();
        fs::write(root.join("home/tester/document"), b"user-data").unwrap();
        prepare(&root, &store, "1.0-1").unwrap();
        assert!(prepare(&root, &store, "1.0-1").is_err());
        fs::write(root.join("usr/bin/springboard"), b"broken").unwrap();
        fs::create_dir_all(root.join("var/lib/pacman/local/springboard-shell-2.0-1")).unwrap();
        restore(&root, &store).unwrap();
        assert_eq!(fs::read(root.join("usr/bin/springboard")).unwrap(), b"good");
        assert_eq!(
            fs::read(root.join("home/tester/document")).unwrap(),
            b"user-data"
        );
        assert!(!root
            .join("var/lib/pacman/local/springboard-shell-2.0-1")
            .exists());
        assert_eq!(state(&store), "rolled-back");
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn recovery_rejects_bad_metadata() {
        assert!(!safe_version("../root"));
        assert!(!safe_version(""));
        assert!(safe_version("0.4.24.1791456741-1"));
    }
}
