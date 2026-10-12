//! Keeps labwc's server-side title bars (third-party apps) in step with the system Light/Dark appearance.
//! Copies the matching BifrOSt theme variant into the session user's `~/.local/share/themes/BifrOSt/labwc/`
//! and sends labwc SIGHUP (reconfigure). A no-op when labwc is not running (host or dev).
use std::os::unix::fs::{chown, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

const SRC: &str = "/usr/share/bifrost/wayland/themes";
static APPLIED: AtomicU8 = AtomicU8::new(0); // 0 none, 1 light, 2 dark, 3 gave up (logged once)

/// Milliseconds (since the first call, plus one) of the last `/proc` scan for labwc; 0 = never scanned.
static LAST_SCAN_MS: AtomicU64 = AtomicU64::new(0);
const RESCAN_MS: u64 = 5_000;

/// True when a `/proc` scan for labwc may run now: at most one per `RESCAN_MS`.
fn scan_due() -> bool {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let now = START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1;
    let last = LAST_SCAN_MS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < RESCAN_MS {
        return false;
    }
    LAST_SCAN_MS.store(now, Ordering::Relaxed);
    true
}

pub fn follow_scheme() {
    let want = if bifrost_render::appearance::target_is_light() { 1 } else { 2 };
    let have = APPLIED.load(Ordering::Relaxed);
    if have == want || have == 3 {
        return;
    }
    let Some(pid) = labwc_pid() else {
        return; // not under labwc (yet): retry on a later tick
    };
    let variant = if want == 1 { "light" } else { "dark" };
    match apply(variant, pid) {
        Ok(()) => APPLIED.store(want, Ordering::Relaxed),
        Err(e) => {
            eprintln!("labwc theme: {e}");
            APPLIED.store(3, Ordering::Relaxed);
        }
    }
}

fn labwc_pid() -> Option<u32> {
    if let Some(p) = std::env::var("LABWC_PID").ok().and_then(|v| v.parse().ok()) {
        return Some(p);
    }
    if !scan_due() {
        return None;
    }
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
        (comm.trim() == "labwc").then_some(pid)
    })
}

/// (uid, gid, home) of the session account named in `/var/lib/springboard/account`.
fn session_user() -> Option<(u32, u32, String)> {
    let account = std::fs::read_to_string("/var/lib/springboard/account").ok()?;
    let name = account.lines().find_map(|l| l.strip_prefix("username="))?.trim().to_string();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() >= 6 && f[0] == name {
            Some((f[2].parse().ok()?, f[3].parse().ok()?, f[5].to_string()))
        } else {
            None
        }
    })
}

fn apply(variant: &str, pid: u32) -> std::io::Result<()> {
    let err = |m: &str| std::io::Error::new(std::io::ErrorKind::NotFound, m.to_string());
    let (uid, gid, home) = session_user().ok_or_else(|| err("no session user"))?;
    let dest = PathBuf::from(&home).join(".local/share/themes/BifrOSt/labwc");
    std::fs::create_dir_all(&dest)?;
    // Directories we may have created as root: hand them to the user.
    for up in 0..4 {
        if let Some(d) = dest.ancestors().nth(up) {
            let _ = chown(d, Some(uid), Some(gid));
        }
    }
    for f in std::fs::read_dir(format!("{SRC}/{variant}/labwc"))?.flatten() {
        let tmp = dest.join(format!(".{}.tmp", f.file_name().to_string_lossy()));
        std::fs::copy(f.path(), &tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        chown(&tmp, Some(uid), Some(gid))?;
        std::fs::rename(&tmp, dest.join(f.file_name()))?;
    }
    // SIGHUP makes labwc reload config and theme; spawn only, never wait.
    std::process::Command::new("kill").args(["-HUP", &pid.to_string()]).spawn()?;
    Ok(())
}
