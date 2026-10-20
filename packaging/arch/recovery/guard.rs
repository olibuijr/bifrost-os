//! Independent, statically linked shell-package recovery. No application data rollback.
use std::{
    fs, io,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::net::UnixStream,
    os::unix::fs::MetadataExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
mod system;
const STORE: &str = "/var/lib/bifrost-recovery";
// The shell's control socket (root and the installed account only); there is no TCP control port.
const CONTROL_SOCKET: &str = "/run/bifrost/shell-control.sock";
const PACMAN_LOG: &str = "/var/log/pacman.log";
const PACMAN_CACHE: &str = "/var/cache/pacman/pkg";
/// A prepared update that has not reached its pacman transaction (or finished it) after this long is abandoned.
const PREPARED_TIMEOUT: Duration = Duration::from_secs(900);
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
/// Whether a process named `comm` runs (reads /proc/<pid>/comm).
fn process_named(comm: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())
            && fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == comm)
    })
}
/// A pacman transaction is running: its database lock exists and a pacman process holds it (a lock left behind by a
/// killed pacman does not count).
fn pacman_active() -> bool {
    Path::new("/var/lib/pacman/db.lck").exists() && process_named("pacman")
}
/// A prepared update is abandoned when it has waited `PREPARED_TIMEOUT` without pacman running and without having
/// reached its goal (`completed`: the shell binary was replaced, or the system update was staged).
fn prepared_abandoned(age: Duration, pacman_busy: bool, completed: bool) -> bool {
    !pacman_busy && !completed && age >= PREPARED_TIMEOUT
}
/// Whether the `prepared` or `system-prepared` state in `store` is abandoned (see `prepared_abandoned`). `root` is the
/// tree whose shell binary a shell-only update replaces.
fn abandoned_now(root: &Path, store: &Path, pacman_busy: bool) -> bool {
    let completed = match state(store).trim() {
        "system-prepared" => system::pending_cleared(),
        "prepared" => {
            let old = fs::read_to_string(store.join("previous-inode")).unwrap_or_default();
            fs::metadata(root.join("usr/bin/springboard"))
                .is_ok_and(|m| format!("{}:{}", m.dev(), m.ino()) != old.trim())
        }
        _ => return false,
    };
    let age = fs::metadata(store.join("state"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .unwrap_or_default();
    prepared_abandoned(age, pacman_busy, completed)
}
/// Gives up a prepared update that never changed the system: the boot marker goes, the phase returns to idle.
fn abandon(store: &Path) -> io::Result<()> {
    abandon_at(store, &system::pending_path())
}
fn abandon_at(store: &Path, pending: &Path) -> io::Result<()> {
    let _ = fs::remove_file(pending);
    phase(store, "accepted")
}
/// Whether a failed `pacman -Su` may have changed the system. `before` and `after` are `pacman -Q` listings (`None`
/// when one could not be read, which counts as changed); the log is what pacman wrote during the run.
fn transaction_touched(before: Option<&str>, after: Option<&str>, log: &str) -> bool {
    match (before, after) {
        (Some(b), Some(a)) if b == a => {}
        _ => return true,
    }
    log.lines().any(|l| {
        l.contains("could not extract")
            || l.contains("problem occurred while")
            || ["upgraded ", "installed ", "removed ", "downgraded ", "reinstalled "]
                .iter()
                .any(|w| l.contains(&format!("[ALPM] {w}")))
    })
}
fn log_len() -> u64 {
    fs::metadata(PACMAN_LOG).map(|m| m.len()).unwrap_or(0)
}
fn log_since(from: u64) -> String {
    let Ok(mut f) = fs::File::open(PACMAN_LOG) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if f.seek(SeekFrom::Start(from)).is_err() || f.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
fn safe_version(v: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+_:-".contains(&b))
}
fn prepare(root: &Path, store: &Path, version: &str) -> io::Result<()> {
    prepare_with(root, store, version, pacman_active(), &system::pending_path())
}
fn prepare_with(root: &Path, store: &Path, version: &str, pacman_busy: bool, pending: &Path) -> io::Result<()> {
    if !safe_version(version) {
        return Err(err("Invalid package version"));
    }
    if state(store).trim() == "prepared" && abandoned_now(root, store, pacman_busy) {
        // an earlier install() never reached (or never finished) its pacman transaction
        abandon_at(store, pending)?;
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
    // Files the failed package added (in its file list, not in the previous one) would otherwise stay on disk with no
    // owner, and the next update would abort on them. Read the lists before the database entry is replaced.
    let old_files = fs::read_to_string(snap.join("database/files")).unwrap_or_default();
    let mut new_files = String::new();
    if let Ok(entries) = fs::read_dir(&db) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("springboard-shell-") && name != wanted && e.path().is_dir() {
                new_files.push_str(&fs::read_to_string(e.path().join("files")).unwrap_or_default());
                new_files.push('\n');
            }
        }
    }
    for rel in added_files(&old_files, &new_files) {
        let path = root.join(&rel);
        // files and links only: a directory may hold the user's own files
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_dir()) {
            let _ = fs::remove_file(&path);
        }
    }
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
    if root == Path::new("/") {
        // The nine files above are the shell's own; the previous package also carries every other binary and unit.
        // Put them back from the package cache when it is still there (best effort: the database entry and the nine
        // files are already restored, so a missing cache entry only leaves the newer helpers in place).
        match cached_package(Path::new(PACMAN_CACHE), v.trim()) {
            Some(file) => {
                if let Err(e) = run("pacman", &["-U", "--noconfirm", file.to_str().unwrap_or_default()]) {
                    eprintln!("BifrOSt recovery: could not reinstall the previous package from the cache: {e}");
                }
            }
            None => eprintln!("BifrOSt recovery: the previous package is not in the cache; only the shell was restored"),
        }
    }
    phase(store, "rolled-back")
}
/// The paths in a pacman `files` list: after the `%FILES%` header, one relative path per line, directories end in `/`.
/// Only plain relative paths are returned.
fn listed_files(text: &str) -> Vec<String> {
    let mut on = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('%') && line.ends_with('%') {
            on = line == "%FILES%";
        } else if on
            && !line.is_empty()
            && !line.ends_with('/')
            && !line.starts_with('/')
            && !line.split('/').any(|c| c == ".." || c == ".")
        {
            out.push(line.to_owned());
        }
    }
    out
}
/// The files in `new` that `old` does not list: what the failed package added.
fn added_files(old: &str, new: &str) -> Vec<String> {
    let old: std::collections::HashSet<String> = listed_files(old).into_iter().collect();
    listed_files(new).into_iter().filter(|f| !old.contains(f)).collect()
}
/// The cached package file of `version` among directory entries: `springboard-shell-<version>-<arch>.pkg.tar.<ext>`
/// (an epoch's `:` is written `.` in file names), never a signature.
fn cached_package_name(version: &str, names: &[String]) -> Option<String> {
    let versions = [version.to_owned(), version.replace(':', ".")];
    let mut found: Vec<&String> = names
        .iter()
        .filter(|n| {
            !n.ends_with(".sig")
                && n.contains(".pkg.tar")
                && versions.iter().any(|v| n.starts_with(&format!("springboard-shell-{v}-")))
        })
        .collect();
    found.sort();
    found.first().map(|n| (*n).clone())
}
fn cached_package(dir: &Path, version: &str) -> Option<std::path::PathBuf> {
    let names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    cached_package_name(version, &names).map(|n| dir.join(n))
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
fn state_is_complete(text: &str) -> bool {
    text.contains("screen=") && text.contains("screen_on=")
}
fn probe_at(socket: &Path) -> bool {
    let Ok(mut s) = UnixStream::connect(socket) else {
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
    state_is_complete(&String::from_utf8_lossy(&buf[..n]))
}
fn probe() -> bool {
    if !Path::new("/run/bifrost/user/wayland-0").exists() {
        return false;
    }
    probe_at(Path::new(CONTROL_SOCKET))
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
                } else if abandoned_now(Path::new("/"), store, pacman_active()) {
                    abandon(store)?;
                    println!("BifrOSt update abandoned: it never reached its pacman transaction");
                    return Ok(());
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
                } else if abandoned_now(Path::new("/"), store, pacman_active()) {
                    abandon(store)?;
                    println!("BifrOSt update abandoned: it never reached its pacman transaction");
                    return Ok(());
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
/// Gives up the update `install` prepared, after a failure that changed nothing.
fn give_up(store: &Path) {
    if let Err(e) = abandon(store) {
        eprintln!("BifrOSt recovery: could not clear the prepared update: {e}");
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
    // Download everything first: a missing file, a dropped link or a full cache fails here, before the system is
    // prepared, and costs nothing (no reboot, no state to clear).
    run("pacman", &["-Suw", "--noconfirm", "springboard-shell"])?;
    let store = Path::new(STORE);
    if full {
        system::prepare()?;
    } else {
        prepare(Path::new("/"), store, &installed()?)?;
    }
    if let Err(e) = run("systemctl", &["start", "bifrost-update-guard.service"]) {
        // nobody watches this update: do not leave it prepared
        give_up(store);
        return Err(e);
    }
    let before = output(&["-Q"]).ok();
    let log_from = log_len();
    if let Err(e) = run("pacman", &["-Su", "--noconfirm", "--overwrite", "/usr/bin/springboard,/usr/lib/systemd/system/springboard.service,/usr/lib/springboard/seed-data,/usr/bin/bifrost-update-guard,/usr/lib/systemd/system/bifrost-update-guard.service,/usr/share/springboard/system-assets/*", "springboard-shell"]) {
        let after = output(&["-Q"]).ok();
        if transaction_touched(before.as_deref(), after.as_deref(), &log_since(log_from)) {
            // the transaction started: the system may be half updated, recover it
            if full {
                system::fallback_reset()?;
            } else {
                restore(Path::new("/"), store)?;
            }
            return Err(e);
        }
        // nothing changed: keep the running system and report the failure
        give_up(store);
        return Err(err(&format!("The update failed before any package changed; the running system is untouched ({e})")));
    }
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
    fn the_probe_asks_the_shell_over_its_unix_socket() {
        use std::os::unix::net::UnixListener;
        assert_eq!(CONTROL_SOCKET, "/run/bifrost/shell-control.sock");
        let dir = std::env::temp_dir().join(format!("bifrost-probe-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("c.sock");
        assert!(!probe_at(&sock), "no socket, no shell");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 16];
            let n = conn.read(&mut buf).unwrap();
            conn.write_all(b"version=1\nscreen=1280x800\nscreen_on=1\n").unwrap();
            buf[..n].to_vec()
        });
        assert!(probe_at(&sock));
        assert_eq!(server.join().unwrap(), b"state\n");
        assert!(state_is_complete("screen=1x1\nscreen_on=0\n"));
        assert!(!state_is_complete("version=1\n"));
        let _ = fs::remove_dir_all(&dir);
    }
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
    fn temp_base(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("bifrost-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }
    fn age_state(store: &Path, secs: u64) {
        let f = fs::OpenOptions::new().write(true).open(store.join("state")).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(secs)).unwrap();
    }
    fn shell_root(base: &Path, version: &str, files: Option<&str>) -> std::path::PathBuf {
        let root = base.join("root");
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::write(root.join("usr/bin/springboard"), b"good").unwrap();
        let db = root.join("var/lib/pacman/local").join(format!("springboard-shell-{version}"));
        fs::create_dir_all(&db).unwrap();
        fs::write(db.join("desc"), b"old-db").unwrap();
        if let Some(f) = files {
            fs::write(db.join("files"), f).unwrap();
        }
        root
    }

    #[test]
    fn a_prepared_update_that_never_reaches_pacman_times_out_to_idle() {
        let long = PREPARED_TIMEOUT;
        // waiting too long, no pacman, goal not reached: abandoned
        assert!(prepared_abandoned(long, false, false));
        // still young, pacman running, or already done: kept
        assert!(!prepared_abandoned(Duration::from_secs(5), false, false));
        assert!(!prepared_abandoned(long * 4, true, false));
        assert!(!prepared_abandoned(long * 4, false, true));

        let base = temp_base("stale-prepared");
        let root = shell_root(&base, "1.0-1", None);
        let store = base.join("state");
        let pending = base.join("pending");
        prepare_with(&root, &store, "1.0-1", false, &pending).unwrap();
        fs::write(&pending, b"1").unwrap();
        // fresh: the next install is refused
        assert!(prepare_with(&root, &store, "1.0-1", false, &pending).is_err());
        assert_eq!(state(&store), "prepared");
        // old, no pacman: the next install resets it and goes ahead
        age_state(&store, PREPARED_TIMEOUT.as_secs() + 60);
        assert!(abandoned_now(&root, &store, false));
        assert!(!abandoned_now(&root, &store, true), "a running pacman keeps it");
        prepare_with(&root, &store, "1.0-1", false, &pending).unwrap();
        assert_eq!(state(&store), "prepared");
        assert!(!pending.exists(), "the boot marker of the abandoned update is gone");
        // a replaced shell binary means the transaction ran: never abandoned, however old
        // (the new file exists before the rename, so it cannot reuse the old inode)
        fs::write(root.join("usr/bin/springboard.new"), b"new").unwrap();
        fs::rename(root.join("usr/bin/springboard.new"), root.join("usr/bin/springboard")).unwrap();
        age_state(&store, PREPARED_TIMEOUT.as_secs() + 60);
        assert!(!abandoned_now(&root, &store, false));
        abandon_at(&store, &pending).unwrap();
        assert_eq!(state(&store), "accepted");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_failure_before_the_transaction_changes_nothing_and_does_not_reboot() {
        let q = "bash 5.2-1\nlinux 7.0-1\nspringboard-shell 1.0-1\n";
        let log = "[2026-10-20] [PACMAN] Running 'pacman -Su'\n[2026-10-20] [ALPM] error: failed to retrieve some files\n";
        assert!(!transaction_touched(Some(q), Some(q), log));
        assert!(!transaction_touched(Some(q), Some(q), ""));
        // a package changed
        assert!(transaction_touched(Some(q), Some("bash 5.2-2\nlinux 7.0-1\nspringboard-shell 1.0-1\n"), log));
        // an unreadable listing counts as changed
        assert!(transaction_touched(None, Some(q), log));
        assert!(transaction_touched(Some(q), None, log));
        // the log says a package was written or an extraction failed
        assert!(transaction_touched(Some(q), Some(q), "[ALPM] upgraded linux (7.0-1 -> 7.1-1)\n"));
        assert!(transaction_touched(Some(q), Some(q), "error: could not extract usr/bin/x (Disk full)\n"));
        assert!(transaction_touched(Some(q), Some(q), "error: problem occurred while upgrading linux\n"));
    }

    #[test]
    fn restore_removes_the_files_the_failed_package_added() {
        let old_files = "%FILES%\nusr/\nusr/bin/\nusr/bin/springboard\n";
        let new_files = "%FILES%\nusr/\nusr/bin/\nusr/bin/springboard\nusr/bin/new-helper\nusr/lib/bifrost/services/\nusr/lib/bifrost/services/extra\n\n%BACKUP%\netc/x\t0\n";
        assert_eq!(added_files(old_files, new_files), vec!["usr/bin/new-helper", "usr/lib/bifrost/services/extra"]);
        // paths that could leave the tree are never listed
        assert_eq!(listed_files("%FILES%\n../etc/passwd\n/etc/shadow\nusr/../x\nusr/ok\n"), vec!["usr/ok"]);

        let base = temp_base("restore-added");
        let root = shell_root(&base, "1.0-1", Some(old_files));
        let store = base.join("state");
        prepare(&root, &store, "1.0-1").unwrap();
        // the failed update: a new database entry, a new helper, a new service binary, the user's own file in a shared dir
        let new_db = root.join("var/lib/pacman/local/springboard-shell-2.0-1");
        fs::create_dir_all(&new_db).unwrap();
        fs::write(new_db.join("files"), new_files).unwrap();
        fs::write(root.join("usr/bin/springboard"), b"broken").unwrap();
        fs::write(root.join("usr/bin/new-helper"), b"new").unwrap();
        fs::create_dir_all(root.join("usr/lib/bifrost/services")).unwrap();
        fs::write(root.join("usr/lib/bifrost/services/extra"), b"new").unwrap();
        fs::write(root.join("usr/lib/bifrost/services/user-own"), b"mine").unwrap();
        restore(&root, &store).unwrap();
        assert_eq!(fs::read(root.join("usr/bin/springboard")).unwrap(), b"good");
        assert!(!root.join("usr/bin/new-helper").exists());
        assert!(!root.join("usr/lib/bifrost/services/extra").exists());
        assert_eq!(fs::read(root.join("usr/lib/bifrost/services/user-own")).unwrap(), b"mine");
        assert!(!new_db.exists());
        assert!(root.join("var/lib/pacman/local/springboard-shell-1.0-1/files").is_file());
        assert_eq!(state(&store), "rolled-back");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_previous_package_is_found_in_the_cache() {
        let names: Vec<String> = [
            "springboard-shell-1.0-1-x86_64.pkg.tar.zst.sig",
            "springboard-shell-1.0-10-x86_64.pkg.tar.zst",
            "springboard-shell-1.0-1-x86_64.pkg.tar.zst",
            "springboard-shell-2.0-1-x86_64.pkg.tar.zst",
            "other-1.0-1-any.pkg.tar.zst",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(cached_package_name("1.0-1", &names).as_deref(), Some("springboard-shell-1.0-1-x86_64.pkg.tar.zst"));
        assert_eq!(cached_package_name("3.0-1", &names), None);
        // an epoch's colon is a dot in the file name
        let epoch = vec!["springboard-shell-1.2.0-1-x86_64.pkg.tar.zst".to_string()];
        assert_eq!(cached_package_name("1:2.0-1", &epoch).as_deref(), Some("springboard-shell-1.2.0-1-x86_64.pkg.tar.zst"));
    }

    #[test]
    fn recovery_rejects_bad_metadata() {
        assert!(!safe_version("../root"));
        assert!(!safe_version(""));
        assert!(safe_version("0.4.24.1791456741-1"));
    }
}
