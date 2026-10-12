//! Whole-root recovery for the installer-defined Btrfs layout.
use super::*;
const TOP: &str = "/run/bifrost/recovery-root";
const ESP: &str = "/boot/EFI/BIFROST";
/// The installed kernel package: `linux-rt` (the default since 2026-10-12) when its image is in /boot, else the
/// stock `linux` (older installs, or an install whose package source had no linux-rt).
fn kernel() -> &'static str {
    if Path::new("/boot/vmlinuz-linux-rt").exists() { "linux-rt" } else { "linux" }
}
fn kernel_files() -> [String; 2] {
    let k = kernel();
    [format!("vmlinuz-{k}"), format!("initramfs-{k}.img")]
}
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
    for file in &kernel_files() {
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
    // The running unified image becomes the rollback generation (bifrost-previous.efi), the kernel files above stay
    // for installs that still boot the kernel plus initramfs.
    let running = Path::new(ESP).join("bifrost.efi");
    if running.exists() {
        copy(&running, &Path::new(ESP).join("bifrost-previous.efi"))?;
    }
    save(&Path::new(ESP).join("rollback.txt"), fallback_line(&line).as_bytes())?;
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
    for file in &kernel_files() {
        copy(
            &Path::new(ESP).join("previous").join(file),
            &Path::new("/boot").join(file),
        )?;
    }
    // The previous unified image goes back to bifrost.efi (the boot line below names it).
    let previous_image = Path::new(ESP).join("bifrost-previous.efi");
    if previous_image.exists() {
        copy(&previous_image, &Path::new(ESP).join("bifrost.efi"))?;
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

// ---- Unified kernel image and per-machine initramfs (docs/boot-architecture.md, "Prepared, not probed") ----

/// The unified kernel image the loader starts, relative to the ESP root (the loader's `uki=` token).
const UKI_PATH: &str = "\\EFI\\BIFROST\\bifrost.efi";
/// The previous generation's image, the rollback target.
const UKI_PREVIOUS: &str = "\\EFI\\BIFROST\\bifrost-previous.efi";
/// The mkinitcpio drop-in that replaces the generic MODULES set with the modules this machine used.
const MODULES_DROPIN: &str = "/etc/mkinitcpio.conf.d/bifrost-modules.conf";

/// The boot line's options for the image: every `initrd=` and `uki=` token dropped (the image carries the initramfs
/// and the microcode), everything else kept in order.
pub fn cmdline_options(line: &str) -> String {
    line.split_whitespace()
        .filter(|t| !t.starts_with("initrd=") && !t.starts_with("uki="))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The boot line for the unified image: `uki=` first, then the options.
pub fn uki_line(line: &str) -> String {
    format!("uki={UKI_PATH} {}\n", cmdline_options(line))
}

/// The fallback boot line written to `rollback.txt`: the previous image (or, for an older install, the previous
/// kernel and initramfs), the previous root subvolume, and the recovery marker.
pub fn fallback_line(line: &str) -> String {
    line.split_whitespace()
        .map(|s| {
            if s.starts_with("uki=") {
                format!("uki={UKI_PREVIOUS}")
            } else if s.starts_with("initrd=") {
                format!("initrd=\\EFI\\BIFROST\\previous\\initramfs-{}.img", if s.ends_with("initramfs-linux-rt.img") { "linux-rt" } else { "linux" })
            } else if s.starts_with("rootflags=") {
                "rootflags=subvol=@rollback".to_owned()
            } else {
                s.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        + " bifrost.recovery=1\n"
}

/// Module names from `/proc/modules` text: sorted, unique, plain identifiers only.
pub fn module_names(proc_modules: &str) -> Vec<String> {
    let mut names: Vec<String> = proc_modules
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Directories (under `/lib/modules/<release>/`) whose modules the initramfs must carry: storage, the root
/// filesystem and its crypto, and the display driver for the first KMS frame. mkinitcpio adds each module's
/// `depends` itself. Everything else (sound, radio, input, sensors, watchdogs, kvm) loads later through udev.
const BOOT_MODULE_DIRS: &[&str] = &[
    "kernel/drivers/nvme/",
    "kernel/drivers/ata/",
    "kernel/drivers/scsi/",
    "kernel/drivers/block/",
    "kernel/drivers/md/",
    // virtio_pci and friends: a VM's disk and display sit behind them
    "kernel/drivers/virtio/",
    "kernel/drivers/gpu/",
    "kernel/fs/btrfs/",
    "kernel/crypto/",
    "kernel/arch/x86/crypto/",
    "kernel/lib/",
];

/// Whether a module file (as `modinfo -F filename` prints it) belongs in the initramfs. Built-ins print
/// `(builtin)` and need nothing.
pub fn is_boot_module(path: &str) -> bool {
    let rel = match path.split_once("/kernel/") {
        Some((_, r)) => format!("kernel/{r}"),
        None => path.to_owned(),
    };
    BOOT_MODULE_DIRS.iter().any(|d| rel.starts_with(d))
}

/// The recorded modules reduced to the boot allowlist. `path_of` maps a module to its file.
pub fn boot_modules(names: &[String], path_of: impl Fn(&str) -> Option<String>) -> Vec<String> {
    names.iter().filter(|n| path_of(n).is_some_and(|p| is_boot_module(&p))).cloned().collect()
}

fn module_path(name: &str) -> Option<String> {
    let out = Command::new("modinfo").args(["-F", "filename", name]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The mkinitcpio drop-in for a module list. It overrides the generic `MODULES=` of `/etc/mkinitcpio.conf`.
///
/// It also keeps the image small: the generic `microcode` hook is dropped (the image carries one early cpio for this
/// CPU only, see `cpu_microcode`), `bifrost-firmware` prunes `usr/lib/firmware` to what the drivers requested on the
/// recorded boot, and zstd -19 shrinks the archive (decompression speed is the same at any level).
pub fn modules_conf(names: &[String]) -> String {
    format!(
        "MODULES=({})\nHOOKS=($(printf '%s\\n' \"${{HOOKS[@]}}\" | grep -vx microcode))\nHOOKS+=(bifrost-firmware)\nCOMPRESSION=\"zstd\"\nCOMPRESSION_OPTIONS=(-19)\n",
        names.join(" ")
    )
}

/// Firmware file names the kernel log says were requested (`i915/skl_dmc_ver1_27.bin`), without the compression
/// suffix, sorted and unique. Only relative paths with a firmware extension count.
pub fn firmware_names(dmesg: &str) -> Vec<String> {
    const EXT: &[&str] = &[".bin", ".ucode", ".fw", ".hex", ".sbin", ".mbn", ".elf"];
    let mut names: Vec<String> = dmesg
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| matches!(c, '\'' | '"' | ',' | ';' | ':' | '(' | ')' | '[' | ']')))
        .map(|t| t.strip_suffix(".zst").or_else(|| t.strip_suffix(".xz")).unwrap_or(t))
        .filter(|t| {
            !t.starts_with('/')
                && !t.contains("..")
                && t.contains('/')
                && EXT.iter().any(|e| t.ends_with(e))
                && t.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
        })
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The early cpio entry and the firmware file holding the microcode for the CPU in `cpuinfo` (`/proc/cpuinfo`):
/// Intel ships one file per family-model-stepping, AMD one per family. `None` for anything else.
pub fn cpu_microcode(cpuinfo: &str) -> Option<(&'static str, String)> {
    let field = |k: &str| {
        cpuinfo
            .lines()
            .find(|l| l.split(':').next().is_some_and(|n| n.trim() == k))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
    };
    let num = |k: &str| field(k)?.parse::<u32>().ok();
    match field("vendor_id")?.as_str() {
        "GenuineIntel" => Some((
            "GenuineIntel.bin",
            format!(
                "/usr/lib/firmware/intel-ucode/{:02x}-{:02x}-{:02x}",
                num("cpu family")?,
                num("model")?,
                num("stepping")?
            ),
        )),
        "AuthenticAMD" => {
            let fam = num("cpu family")?;
            Some(("AuthenticAMD.bin", format!("/usr/lib/firmware/amd-ucode/microcode_amd_fam{fam:x}h.bin")))
        }
        _ => None,
    }
}

/// Writes an uncompressed early cpio holding only this CPU's microcode. `None` when it cannot be made; the caller
/// then falls back to the distribution's full `*-ucode.img`.
fn build_microcode_cpio() -> Option<String> {
    let (name, source) = cpu_microcode(&fs::read_to_string("/proc/cpuinfo").ok()?)?;
    let stage = Path::new("/run/bifrost-ucode");
    let _ = fs::remove_dir_all(stage);
    let dir = stage.join("kernel/x86/microcode");
    fs::create_dir_all(&dir).ok()?;
    fs::copy(&source, dir.join(name)).ok()?;
    let out = "/run/bifrost-ucode.cpio";
    let stage_s = stage.to_string_lossy().into_owned();
    run("bsdtar", &["--format", "newc", "-cf", out, "-C", &stage_s, "kernel"]).ok()?;
    let _ = fs::remove_dir_all(stage);
    Some(out.to_owned())
}

/// Whether the mkinitcpio configuration text already embeds microcode in the initramfs (`microcode` in `HOOKS`),
/// in which case passing the `*-ucode.img` files as well would carry it twice.
pub fn hooks_embed_microcode(conf: &str) -> bool {
    conf.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("HOOKS=") || l.starts_with("HOOKS+="))
        .any(|l| l.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|w| w == "microcode"))
}

/// FNV-1a (64 bit) over the sorted, unique hardware ids: changes when a device is added, removed or swapped.
pub fn hardware_fingerprint(ids: &[String]) -> String {
    let mut sorted: Vec<&str> = ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for id in sorted {
        for byte in id.bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// `vendor:device` for every PCI function, and the board name: the input of the hardware fingerprint.
fn hardware_ids() -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(dir) = fs::read_dir("/sys/bus/pci/devices") {
        for entry in dir.flatten() {
            let read = |f: &str| fs::read_to_string(entry.path().join(f)).unwrap_or_default();
            ids.push(format!("{}:{}", read("vendor").trim(), read("device").trim()));
        }
    }
    ids.push(format!(
        "board:{}",
        fs::read_to_string("/sys/class/dmi/id/board_name").unwrap_or_default().trim()
    ));
    ids
}

fn hardware_now() -> String {
    hardware_fingerprint(&hardware_ids())
}

/// The recorded module list, if it was recorded on this hardware. `None` after a hardware change: generic set.
fn recorded_modules() -> Option<Vec<String>> {
    let store = Path::new(STORE);
    let stored = fs::read_to_string(store.join("hardware")).ok()?;
    if stored.trim() != hardware_now() {
        return None;
    }
    let names = boot_modules(&module_names(&fs::read_to_string(store.join("modules")).ok()?), module_path);
    (!names.is_empty()).then_some(names)
}

/// Brings the mkinitcpio drop-in in line with the recorded modules. Returns whether it changed, in which case the
/// initramfs must be rebuilt before the image.
pub fn reconcile_modules() -> io::Result<bool> {
    let wanted = recorded_modules().map(|names| modules_conf(&names));
    let path = Path::new(MODULES_DROPIN);
    match (wanted, fs::read_to_string(path).ok()) {
        (Some(w), Some(c)) if w == c => Ok(false),
        (Some(w), _) => {
            fs::create_dir_all(path.parent().unwrap())?;
            save(path, w.as_bytes())?;
            Ok(true)
        }
        (None, Some(_)) => {
            fs::remove_file(path)?;
            Ok(true)
        }
        (None, None) => Ok(false),
    }
}

/// Records the modules loaded now (called after desktop-ready). The image is rebuilt only when the list or the
/// hardware changed, so a steady machine never touches the previous generation.
pub fn record_modules() -> io::Result<bool> {
    let store = Path::new(STORE);
    let names = module_names(&fs::read_to_string("/proc/modules")?);
    if names.is_empty() {
        return Err(err("No loaded modules to record"));
    }
    let hardware = hardware_now();
    // the firmware the drivers requested this boot; an empty log leaves the recorded list (and the image) alone
    let firmware = Command::new("dmesg")
        .output()
        .map(|o| firmware_names(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let fw_text = firmware.join("\n");
    let fw_changed = !firmware.is_empty()
        && fs::read_to_string(store.join("firmware")).map_or(true, |s| s.trim() != fw_text);
    if fw_changed {
        save(&store.join("firmware"), fw_text.as_bytes())?;
    }
    let same = !fw_changed
        && fs::read_to_string(store.join("modules")).is_ok_and(|s| module_names(&s) == names)
        && fs::read_to_string(store.join("hardware")).is_ok_and(|s| s.trim() == hardware);
    if same {
        // the list is the same, but an update can change how the drop-in is made from it (0.6.3 kept only the boot
        // modules): rebuild only when the drop-in on disk is not the one wanted now
        if reconcile_modules()? {
            run("mkinitcpio", &["-p", kernel()])?;
            build_uki()?;
            return Ok(true);
        }
        return Ok(false);
    }
    if fw_changed && fs::read_to_string(store.join("modules")).is_ok_and(|s| module_names(&s) == names) {
        // modules and drop-in are unchanged, but the firmware list is a build input of the initramfs
        reconcile_modules()?;
        run("mkinitcpio", &["-p", kernel()])?;
        return build_uki().map(|_| true);
    }
    save(&store.join("modules"), names.join("\n").as_bytes())?;
    save(&store.join("hardware"), hardware.as_bytes())?;
    build_boot_image()?;
    Ok(true)
}

/// The initramfs for this machine (rebuilt only when the drop-in changed), then the unified image from it.
pub fn build_boot_image() -> io::Result<()> {
    if reconcile_modules()? {
        run("mkinitcpio", &["-p", kernel()])?;
    }
    build_uki().map(|_| ())
}

fn ucode_images() -> io::Result<Vec<String>> {
    let mut v: Vec<String> = fs::read_dir("/boot")?
        .flatten()
        .map(|e| e.path().to_string_lossy().into_owned())
        .filter(|p| p.ends_with("-ucode.img"))
        .collect();
    v.sort();
    Ok(v)
}

/// Builds `EFI/BIFROST/bifrost.efi` from the kernel, the microcode and the initramfs, with the boot line's options.
/// The old image becomes `bifrost-previous.efi`. `boot.cfg` is rewritten only after the new image exists, so a
/// failed build leaves the last good boot line in place. Returns whether the image changed.
pub fn build_uki() -> io::Result<bool> {
    let esp = Path::new(ESP);
    let line = fs::read_to_string(boot_config())?;
    // One early cpio, this CPU's microcode only. Without the per-machine drop-in the initramfs' own microcode hook
    // already embeds it (never pass the ucode images on top of that); with a drop-in but no cpio, fall back to them.
    let conf = fs::read_to_string("/etc/mkinitcpio.conf").unwrap_or_default();
    let mut microcode: Vec<String> = Vec::new();
    if Path::new(MODULES_DROPIN).exists() {
        match build_microcode_cpio() {
            Some(cpio) => microcode.push(cpio),
            None => microcode = ucode_images()?,
        }
    } else if !hooks_embed_microcode(&conf) {
        microcode = ucode_images()?;
    }
    let out = esp.join("bifrost.efi");
    let tmp = esp.join("bifrost.efi.new");
    let mut args: Vec<String> = vec!["build".into(), format!("--linux=/boot/vmlinuz-{}", kernel())];
    args.extend(microcode.iter().map(|m| format!("--initrd={m}")));
    args.push(format!("--initrd=/boot/initramfs-{}.img", kernel()));
    args.push(format!("--cmdline={}", cmdline_options(&line)));
    args.push(format!("--output={}", tmp.display()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(e) = run("ukify", &refs) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if out.exists() && fs::read(&out)? == fs::read(&tmp)? {
        fs::remove_file(&tmp)?;
        return Ok(false);
    }
    if out.exists() {
        copy(&out, &esp.join("bifrost-previous.efi"))?;
    } else if !esp.join("rollback.txt").exists() {
        // The first image on an older install: the boot line it replaces is the rollback.
        save(&esp.join("rollback.txt"), line.as_bytes())?;
    }
    fs::File::open(&tmp)?.sync_all()?;
    fs::rename(&tmp, &out)?;
    save(&esp.join("boot.cfg"), uki_line(&line).as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod uki_tests {
    use super::*;

    const LINE: &str = "initrd=\\initramfs-linux.img root=UUID=abc rootflags=subvol=@system rw quiet loglevel=3";

    #[test]
    fn uki_line_drops_initramfs_and_leads_with_the_image() {
        assert_eq!(
            uki_line(LINE),
            "uki=\\EFI\\BIFROST\\bifrost.efi root=UUID=abc rootflags=subvol=@system rw quiet loglevel=3\n"
        );
        // already converted: converting again changes nothing
        assert_eq!(uki_line(&uki_line(LINE)), uki_line(LINE));
        assert_eq!(cmdline_options(LINE), "root=UUID=abc rootflags=subvol=@system rw quiet loglevel=3");
    }

    #[test]
    fn fallback_line_points_at_previous_image_and_rollback_subvolume() {
        assert_eq!(
            fallback_line(&uki_line(LINE)),
            "uki=\\EFI\\BIFROST\\bifrost-previous.efi root=UUID=abc rootflags=subvol=@rollback rw quiet loglevel=3 bifrost.recovery=1\n"
        );
        // an install still on the kernel plus initramfs keeps the previous files
        assert!(fallback_line(&LINE.replace("initramfs-linux", "initramfs-linux-rt")).starts_with("initrd=\\EFI\\BIFROST\\previous\\initramfs-linux-rt.img "));
        assert!(fallback_line(LINE).starts_with("initrd=\\EFI\\BIFROST\\previous\\initramfs-linux.img "));
    }

    #[test]
    fn module_names_are_sorted_unique_and_plain() {
        let text = "i915 3072 0 - Live 0x0\nbtrfs 1 0 - Live\nuas 2 0 - Live\ni915 3072 0 - Live\nbad-name 1 0\n";
        assert_eq!(module_names(text), vec!["btrfs", "i915", "uas"]);
        assert!(module_names("").is_empty());
    }

    #[test]
    fn boot_modules_keep_storage_filesystem_and_gpu_only() {
        let p = |n: &str| -> Option<String> {
            Some(match n {
                "nvme" => "/lib/modules/7.2.9-rt3/kernel/drivers/nvme/host/nvme.ko.zst",
                "i915" => "/lib/modules/7.2.9-rt3/kernel/drivers/gpu/drm/i915/i915.ko.zst",
                "btrfs" => "/lib/modules/7.2.9-rt3/kernel/fs/btrfs/btrfs.ko.zst",
                "snd_hda_intel" => "/lib/modules/7.2.9-rt3/kernel/sound/pci/hda/snd-hda-intel.ko.zst",
                "bluetooth" => "/lib/modules/7.2.9-rt3/kernel/net/bluetooth/bluetooth.ko.zst",
                "kvm_intel" => "/lib/modules/7.2.9-rt3/kernel/arch/x86/kvm/kvm-intel.ko.zst",
                "ext4" => "(builtin)",
                _ => return None,
            }
            .to_owned())
        };
        let all: Vec<String> = ["nvme", "i915", "btrfs", "snd_hda_intel", "bluetooth", "kvm_intel", "ext4", "unknown"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(boot_modules(&all, p), vec!["nvme", "i915", "btrfs"]);
        assert!(is_boot_module("/lib/modules/x/kernel/crypto/xxhash_generic.ko.zst"));
        assert!(!is_boot_module("/lib/modules/x/kernel/drivers/usb/storage/uas.ko.zst"));
    }

    #[test]
    fn modules_conf_overrides_the_generic_set() {
        let conf = modules_conf(&module_names("uas 1 0\nbtrfs 1 0\n"));
        assert!(conf.starts_with("MODULES=(btrfs uas)\n"));
        assert!(conf.contains("grep -vx microcode"));
        assert!(conf.contains("HOOKS+=(bifrost-firmware)"));
        assert!(conf.contains("COMPRESSION=\"zstd\""));
    }

    #[test]
    fn firmware_names_come_from_the_kernel_log() {
        let log = "[ 1.0] i915 0000:00:02.0: [drm] Finished loading DMC firmware i915/skl_dmc_ver1_27.bin (v1.27)\n\
                   [ 1.1] i915 0000:00:02.0: [drm] GuC firmware i915/skl_guc_70.1.1.bin.zst: fetch\n\
                   [ 1.2] loading /lib/modules/x/kernel/foo.bin and ../evil/x.bin and nvme0n1";
        assert_eq!(firmware_names(log), vec!["i915/skl_dmc_ver1_27.bin", "i915/skl_guc_70.1.1.bin"]);
    }

    #[test]
    fn microcode_is_picked_for_this_cpu_only() {
        let intel = "vendor_id\t: GenuineIntel\ncpu family\t: 6\nmodel\t\t: 94\nstepping\t: 3\n";
        assert_eq!(
            cpu_microcode(intel),
            Some(("GenuineIntel.bin", "/usr/lib/firmware/intel-ucode/06-5e-03".to_owned()))
        );
        let amd = "vendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 1\n";
        assert_eq!(
            cpu_microcode(amd),
            Some(("AuthenticAMD.bin", "/usr/lib/firmware/amd-ucode/microcode_amd_fam19h.bin".to_owned()))
        );
        assert_eq!(cpu_microcode("vendor_id : Other\n"), None);
    }

    #[test]
    fn microcode_hook_in_hooks_is_detected() {
        assert!(hooks_embed_microcode("HOOKS=(base udev autodetect microcode modconf)\n"));
        assert!(!hooks_embed_microcode("# HOOKS=(microcode)\nHOOKS=(base udev)\n"));
    }

    #[test]
    fn hardware_fingerprint_is_order_free_and_changes_with_hardware() {
        let a = vec!["8086:1234".to_string(), "10de:5678".to_string(), "board:x".to_string()];
        let b = vec!["board:x".to_string(), "8086:1234".to_string(), "10de:5678".to_string()];
        let c = vec!["8086:1234".to_string(), "1002:0001".to_string(), "board:x".to_string()];
        assert_eq!(hardware_fingerprint(&a), hardware_fingerprint(&b));
        assert_ne!(hardware_fingerprint(&a), hardware_fingerprint(&c));
        assert_eq!(hardware_fingerprint(&a).len(), 16);
    }

    #[test]
    fn empty_hardware_has_the_fnv_offset_fingerprint() {
        assert_eq!(hardware_fingerprint(&[]), "cbf29ce484222325");
    }
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
