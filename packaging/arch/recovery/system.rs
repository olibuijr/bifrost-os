//! Whole-root recovery for the installer-defined Btrfs layout.
use super::*;
const TOP: &str = "/run/bifrost/recovery-root";
const ESP: &str = "/boot/EFI/BIFROST";
fn text(program: &str, args: &[&str]) -> io::Result<String> {
    let out = Command::new(program).args(args).output()?;
    if !out.status.success() {
        return Err(err(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().into())
}
/// The command line the EFI loader boots: the BifrOSt namespace first, the removable path for older installs.
fn boot_config() -> std::path::PathBuf {
    let native = Path::new(ESP).join("boot.cfg");
    if native.exists() { native } else { "/boot/EFI/BOOT/cmdline.txt".into() }
}
pub fn supported() -> bool {
    text("findmnt", &["-nro", "FSTYPE,FSROOT", "/"])
        .is_ok_and(|s| s.split_whitespace().collect::<Vec<_>>() == ["btrfs", "/@system"])
}
fn top() -> io::Result<()> {
    fs::create_dir_all(TOP)?;
    if text("mountpoint", &["-q", TOP]).is_ok() {
        return Ok(());
    }
    let uuid = text("findmnt", &["-nro", "UUID", "/"])?;
    if uuid.is_empty() || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(err("Invalid root UUID"));
    }
    run("mount", &["-o", "subvolid=5", &format!("UUID={uuid}"), TOP])
}
fn bootid() -> io::Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .into())
}
pub fn prepare() -> io::Result<()> {
    let store = Path::new(STORE);
    if matches!(
        state(store).trim(),
        "system-prepared" | "system-probation" | "system-restoring" | "prepared" | "probation"
    ) {
        return Err(err("An update is still pending"));
    }
    for (target, sub) in [
        ("/home", "/@home"),
        (STORE, "/@recovery"),
        ("/var/lib/springboard", "/@state"),
    ] {
        let current = text("findmnt", &["-nro", "FSROOT", "-T", target])?;
        if current != sub {
            return Err(err(
                "User and recovery data must be separate Btrfs subvolumes",
            ));
        }
    }
    top()?;
    let root = Path::new(TOP);
    let prev = root.join("@rollback");
    if prev.exists() {
        run("btrfs", &["subvolume", "delete", prev.to_str().unwrap()])?;
    }
    run(
        "btrfs",
        &[
            "subvolume",
            "snapshot",
            root.join("@system").to_str().unwrap(),
            prev.to_str().unwrap(),
        ],
    )?;
    fs::create_dir_all(store)?;
    fs::create_dir_all(format!("{ESP}/previous"))?;
    for file in ["vmlinuz-linux", "initramfs-linux.img"] {
        copy(
            &Path::new("/boot").join(file),
            &Path::new(ESP).join("previous").join(file),
        )?;
    }
    let line = fs::read_to_string(boot_config())?;
    if !line
        .split_whitespace()
        .any(|s| s == "rootflags=subvol=@system")
    {
        return Err(err("Boot configuration must select @system"));
    }
    save(&store.join("previous-cmdline"), line.as_bytes())?;
    save(&store.join("previous-bootid"), bootid()?.as_bytes())?;
    save(&store.join("previous-version"), installed()?.as_bytes())?;
    let fallback = line
        .split_whitespace()
        .map(|s| {
            if s.starts_with("initrd=") {
                "initrd=\\EFI\\BIFROST\\previous\\initramfs-linux.img"
            } else if s.starts_with("rootflags=") {
                "rootflags=subvol=@rollback"
            } else {
                s
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        + " bifrost.recovery=1\n";
    save(&Path::new(ESP).join("rollback.txt"), fallback.as_bytes())?;
    // A power failure during pacman must boot the previous generation immediately.
    save(&Path::new(ESP).join("pending"), b"1")?;
    phase(store, "system-prepared")
}
/// The staged system is complete: pacman (and its mkinitcpio hook) finished and `ready` cleared `pending`.
/// `system-prepared` is set before pacman runs, so it alone must never trigger the reboot: rebooting while
/// mkinitcpio writes /boot/initramfs-linux.img boots a truncated image and falls back (seen 2026-10-10).
pub fn staged() -> bool {
    fs::read_to_string(Path::new(ESP).join("pending")).is_ok_and(|s| s.trim() == "0")
        && !Path::new("/var/lib/pacman/db.lck").exists()
}
pub fn ready() -> io::Result<()> {
    save(&Path::new(ESP).join("pending"), b"0")
}
pub fn new_boot() -> bool {
    fs::read_to_string(Path::new(STORE).join("previous-bootid"))
        .ok()
        .zip(bootid().ok())
        .is_some_and(|(a, b)| a.trim() != b)
}
pub fn fallback_boot() -> bool {
    fs::read_to_string("/proc/cmdline")
        .is_ok_and(|s| s.split_whitespace().any(|v| v == "bifrost.recovery=1"))
}
pub fn accept() -> io::Result<()> {
    fs::remove_file(Path::new(ESP).join("pending"))?;
    phase(Path::new(STORE), "accepted")
}
pub fn restore() -> io::Result<()> {
    let store = Path::new(STORE);
    save(
        &store.join("last-fallback"),
        fs::read_to_string("/proc/cmdline")?.as_bytes(),
    )?;
    phase(store, "system-restoring")?;
    top()?;
    let root = Path::new(TOP);
    let rollback = root.join("@rollback");
    if !rollback.exists() {
        return Err(err("No complete system snapshot"));
    }
    let candidate = root.join("@restored");
    if candidate.exists() {
        run(
            "btrfs",
            &["subvolume", "delete", candidate.to_str().unwrap()],
        )?;
    }
    run(
        "btrfs",
        &[
            "subvolume",
            "snapshot",
            rollback.to_str().unwrap(),
            candidate.to_str().unwrap(),
        ],
    )?;
    if root.join("@system").exists() {
        let name = format!(
            "@failed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        fs::rename(root.join("@system"), root.join(name))?;
    }
    fs::rename(candidate, root.join("@system"))?;
    for file in ["vmlinuz-linux", "initramfs-linux.img"] {
        copy(
            &Path::new(ESP).join("previous").join(file),
            &Path::new("/boot").join(file),
        )?;
    }
    let previous = fs::read(store.join("previous-cmdline"))?;
    // The loader prefers EFI/BIFROST/boot.cfg, so restore whichever copies exist.
    for path in [Path::new(ESP).join("boot.cfg"), "/boot/EFI/BOOT/cmdline.txt".into()] {
        if path.exists() {
            save(&path, &previous)?;
        }
    }
    // Keep @rollback intact until the marker is cleared, including across power loss.
    if Path::new(ESP).join("pending").exists() {
        fs::remove_file(Path::new(ESP).join("pending"))?;
    }
    phase(store, "rolled-back")?;
    run("sync", &[])?;
    run("systemctl", &["reboot"])
}

pub fn fallback_reset() -> io::Result<()> {
    save(&Path::new(ESP).join("pending"), b"1")?;
    unsafe extern "C" {
        fn sync();
        fn reboot(cmd: i32) -> i32;
    }
    let _ = Command::new("systemctl").arg("reboot").status();
    std::thread::sleep(Duration::from_secs(3));
    unsafe {
        sync();
        if reboot(0x01234567) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
