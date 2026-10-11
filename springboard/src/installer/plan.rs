//! The installation itself, in Rust: config-file contents as pure functions (tested), and the executor
//! that partitions, runs pacstrap and configures the new system. Only disk and package tools
//! (parted, mkfs.*, pacstrap, arch-chroot) are external programs.

use super::disks::{partition_commands, Disk, Mode};
use super::keymaps::Layout;
use std::fmt::Write as _;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// The live system's lab NIC: (MAC, address/prefix) of the interface with a 10.99.0.x address, if any.
fn lab_link() -> Option<(String, String)> {
    let out = std::process::Command::new("ip").args(["-o", "-4", "addr", "show"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (iface, cidr) = text.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let cidr = f.get(3)?;
        let iface = f.get(1)?;
        cidr.starts_with("10.99.0.").then(|| (iface.to_string(), cidr.to_string()))
    })?;
    let mac = std::fs::read_to_string(format!("/sys/class/net/{iface}/address")).ok()?.trim().to_string();
    Some((mac, cidr))
}

/// A NetworkManager keyfile that pins the lab address on the NIC with this MAC (gateway titan, 10.99.0.1).
fn lab_nmconnection(mac: &str, cidr: &str) -> String {
    format!("[connection]\nid=lab\ntype=ethernet\nautoconnect=true\n\n[ethernet]\nmac-address={mac}\n\n[ipv4]\nmethod=manual\naddress1={cidr},10.99.0.1\ndns=1.1.1.1;\n\n[ipv6]\nmethod=ignore\n")
}

/// Everything the user chose.
#[derive(Clone, Debug)]
pub struct Config {
    pub disk: Disk,
    pub mode: Mode,
    pub locale: String,
    pub layout: Layout,
    pub full_name: String,
    pub username: String,
    pub password: String,
    pub hostname: String,
    /// IANA zone name ("Atlantic/Reykjavik"); empty keeps the image default (UTC).
    pub timezone: String,
    /// The lab's offline package mirror; pacstrap installs from it alone. Empty uses the internet.
    pub mirror: String,
    /// Appearance mode key as in `bifrost_render::appearance::Mode` ("light", "dark", "auto").
    pub appearance: String,
    /// Accent colour key as in `bifrost_render::appearance::Accent` ("blue", "graphite", "multicolour", ...).
    pub accent: String,
    /// Wallpaper id (`wallpapers/<id>.raw` in the assets); empty keeps the packaged default.
    pub wallpaper: String,
}

pub const DEFAULT_APPEARANCE: &str = "dark";
pub const DEFAULT_ACCENT: &str = "blue";
/// The shell's wallpaper pref (see `bin/shell.rs`: `prefs.get("wallpaper")`).
pub const WALLPAPER_PREF: &str = "wallpaper";

/// A wallpaper id the shell can map: a plain file stem, never a path.
pub fn valid_wallpaper_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The shell's `settings.ini` with the look chosen at install time merged into `existing` (other keys are kept). The
/// values are normalised through the shell's own parsers, so a bad value becomes the default instead of a bad file.
pub fn settings_ini(cfg: &Config, existing: &str) -> String {
    use crate::shell::settings::{ACCENT_PREF, APPEARANCE_PREF};
    use bifrost_render::appearance::{Accent, Mode};
    let ours = [APPEARANCE_PREF, ACCENT_PREF, WALLPAPER_PREF];
    let mut out: String = existing
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| l.split_once('=').map_or(true, |(k, _)| !ours.contains(&k.trim())))
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("{APPEARANCE_PREF}={}\n", Mode::parse(&cfg.appearance).key()));
    out.push_str(&format!("{ACCENT_PREF}={}\n", Accent::parse(&cfg.accent).key()));
    if valid_wallpaper_id(&cfg.wallpaper) {
        out.push_str(&format!("{WALLPAPER_PREF}={}\n", cfg.wallpaper));
    }
    out
}

/// The pacman configuration pacstrap uses with a lab mirror: that one repository and nothing else, the way the Omarchy
/// ISO installs from its bundled mirror (`pacman-offline.conf`). The installed system keeps the stock configuration.
pub fn offline_pacman_conf(mirror: &str) -> String {
    format!(
        "[options]\nHoldPkg = pacman glibc\nArchitecture = auto\nParallelDownloads = 4\nSigLevel = Required DatabaseOptional\n\
         LocalFileSigLevel = Optional\n\n[offline]\nSigLevel = Never\nServer = {mirror}\n"
    )
}

pub const STAGES: [&str; 4] = ["Preparing the disk", "Installing the system", "Setting up your account and apps", "Finishing"];
/// Progress range of each stage (percent): the system install dominates.
const RANGE: [(u8, u8); 4] = [(0, 4), (4, 82), (82, 97), (97, 100)];
/// What the full bundle occupies on the target, for progress while pacstrap runs.
pub const EXPECTED_BYTES: f64 = 2.4e9;

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub stage: usize,
    pub percent: u8,
    pub message: String,
    pub done: bool,
    pub failed: Option<String>,
    pub log: Vec<String>,
    pub started: Option<std::time::Instant>,
}

impl Progress {
    /// Whole minutes left, once enough has been done to estimate. Not before 15 %: the first percents are disk setup and
    /// pacman's database sync with no bytes written, so a straight line from them said "9 min" for a 100 s lab install.
    pub fn eta_minutes(&self) -> Option<u64> {
        let el = self.started?.elapsed().as_secs_f64();
        (self.percent >= 15 && !self.done).then(|| ((el / self.percent as f64 * (100.0 - self.percent as f64) / 60.0 - 0.05).ceil().max(1.0)) as u64)
    }
}

pub type Shared = Arc<Mutex<Progress>>;

// ---- file contents -----------------------------------------------------------------------------

pub fn locale_conf(locale: &str) -> String {
    format!("LANG={locale}\n")
}

/// The line to uncomment in /etc/locale.gen.
pub fn locale_gen_line(locale: &str) -> String {
    let charset = locale.rsplit('.').next().unwrap_or("UTF-8");
    format!("{locale} {charset}")
}

pub fn vconsole_conf(layout: &Layout) -> String {
    format!("KEYMAP={}\n", layout.console_keymap)
}

pub fn wayland_keyboard_conf(layout: &Layout) -> String {
    format!("XKB_DEFAULT_LAYOUT={}\nXKB_DEFAULT_VARIANT={}\n",layout.xkb_layout,layout.xkb_variant)
}

pub fn x11_keyboard_conf(layout: &Layout) -> String {
    let mut s = String::from("Section \"InputClass\"\n    Identifier \"keyboard\"\n    MatchIsKeyboard \"on\"\n");
    let _ = writeln!(s, "    Option \"XkbLayout\" \"{}\"", layout.xkb_layout);
    if !layout.xkb_variant.is_empty() {
        let _ = writeln!(s, "    Option \"XkbVariant\" \"{}\"", layout.xkb_variant);
    }
    s.push_str("EndSection\n");
    s
}

pub fn account_file(full_name: &str, username: &str) -> String {
    format!("full_name={full_name}\nusername={username}\n")
}

/// CPU core kept off the scheduler for the audio writer (B5). None until measured on Midget: set it to
/// e.g. Some("7") to add isolcpus, nohz_full and rcu_nocbs for that core. The audio writer then needs
/// SPRINGBOARD_AUDIO_CPUS to pin itself there, or it would run on the other cores.
pub const AUDIO_CORE: Option<&str> = None;

/// Installed kernel command line. i915 framebuffer compression and PSR are off: on Skylake (HD 530) they produced
/// sheared, repeated stripes across the top of the EFI frame and the first frames after the handoff.
/// threadirqs (B1) runs interrupt handlers as kernel threads, so the sound card and xHCI IRQ threads can be
/// raised above the rest of the system with chrt.
pub const INITRAMFS_HOOKS: &str = "HOOKS=(base udev autodetect microcode modconf kms keyboard block filesystems bifrost-mounts)";

/// An empty systemd.tty.rows.console= makes PID 1 skip its ANSI size query on /dev/console. Nothing answers
/// that query behind the boot frame, so PID 1 otherwise waits out its 333 ms reply timeout every boot.
pub fn cmdline_txt(root_uuid: &str) -> String {
    cmdline_with(root_uuid, AUDIO_CORE)
}

fn cmdline_with(root_uuid: &str, audio_core: Option<&str>) -> String {
    let isolate = audio_core.map(|c| format!(" isolcpus={c} nohz_full={c} rcu_nocbs={c}")).unwrap_or_default();
    format!("initrd=\\initramfs-linux.img root=UUID={root_uuid} rootflags=subvol=@system rw quiet loglevel=3 rd.udev.log_level=3 rd.systemd.show_status=false systemd.show_status=false rd.luks=0 rd.lvm=0 rd.md=0 rd.dm=0 vt.global_cursor_default=0 systemd.tty.rows.console= i915.enable_fbc=0 i915.enable_psr=0 threadirqs{isolate}\n")
}

/// HDA codec power saving off (B2): the kernel's default suspends the codec after 10 s idle, and the wake-up
/// clicks and costs a buffer's worth of latency at the start of every sound.
pub fn audio_modprobe() -> &'static str {
    "options snd_hda_intel power_save=0\n"
}

/// Boot-time raise of the sound card and xHCI interrupt threads (B1): SCHED_FIFO 50, so an audio or input
/// interrupt is not queued behind the shell's own work. Needs threadirqs on the command line.
pub fn irq_priority_unit() -> &'static str {
    "[Unit]\nDescription=Raise the audio and USB interrupt threads\nAfter=systemd-modules-load.service\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=/bin/sh -c 'for d in /proc/[0-9]*; do c=$(cat $d/comm 2>/dev/null) || continue; case \"$c\" in irq/*xhci*|irq/*snd*|irq/*hda*|irq/*HDA*) chrt -f -p 50 ${d#/proc/} 2>/dev/null;; esac; done'\n\n[Install]\nWantedBy=multi-user.target\n"
}

/// sshd on the installed system: key or password logins for the user, never for root.
pub fn sshd_dropin() -> &'static str {
    "PermitRootLogin no\nPermitEmptyPasswords no\nMaxAuthTries 4\nX11Forwarding no\n"
}

pub fn os_release() -> &'static str {
    "NAME=\"BifrOSt\"\nPRETTY_NAME=\"BifrOSt\"\nID=bifrost\nID_LIKE=arch\nHOME_URL=\"https://bifrost.olibuijr.com\"\n"
}

/// Graphics and hardware-video packages for the GPUs found in `lspci -nn` output (`vendor:device` ids).
/// Everything gets Mesa (OpenGL; its VA-API and VDPAU drivers for AMD are part of the package now) and the VA-API tools.
pub fn gpu_packages(lspci: &str) -> Vec<&'static str> {
    let mut v = vec!["mesa", "vulkan-icd-loader", "libva-utils", "ffmpeg"];
    let display = |l: &&str| l.contains("VGA compatible") || l.contains("3D controller") || l.contains("Display controller");
    for l in lspci.lines().filter(display) {
        let l = l.to_lowercase();
        let add: &[&str] = if l.contains("[8086:") || l.contains("intel corporation") {
            &["vulkan-intel", "intel-media-driver"]
        } else if l.contains("[1002:") || l.contains("advanced micro devices") || l.contains("[amd/ati]") {
            &["vulkan-radeon"]
        } else if l.contains("[10de:") || l.contains("nvidia") {
            &["nvidia-open", "nvidia-utils", "libva-nvidia-driver"]
        } else {
            &[]
        };
        for p in add {
            if !v.contains(p) {
                v.push(p);
            }
        }
    }
    v
}

/// System snapshots preserve separate user/app state, recovery records, caches and logs.
pub const BTRFS_LAYOUT: &[(&str,&str)] = &[("@system","/"),("@home","/home"),("@state","/var/lib/springboard"),("@recovery","/var/lib/bifrost-recovery"),("@cache","/var/cache"),("@logs","/var/log")];
/// Runtime dependencies are shared with both Arch package routes.
pub const RUNTIME_PACKAGES: &str = include_str!("../../../packaging/arch/runtime-packages.txt");
pub const BASE_PACKAGES: &[&str] = &[
    "btrfs-progs",
    "base",
    "linux",
    "linux-firmware",
    "networkmanager",
    "openssh",
    "libglvnd",
    "alsa-utils",
    "git",
    "efibootmgr",
    "sudo",
];
pub fn packages() -> Vec<&'static str> {
    let mut packages = BASE_PACKAGES.to_vec();
    for package in RUNTIME_PACKAGES.split_whitespace() {
        if !packages.contains(&package) { packages.push(package); }
    }
    packages
}


/// Account validation shared by the UI and the executor.
/// A zone name that is safe to put in a path: `Area/City`, letters, digits, `_`, `-`, `+`, no `..`. Empty means "not set".
pub fn valid_timezone(z: &str) -> bool {
    !z.is_empty() && !z.contains("..") && !z.starts_with('/') && z.len() <= 64
        && z.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
}

pub fn valid_username(u: &str) -> bool {
    let mut c = u.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase() || f == '_') && u.len() <= 32 && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn account_error(full_name: &str, username: &str, pw: &str, pw2: &str) -> Option<&'static str> {
    if full_name.trim().is_empty() {
        Some("Enter your name.")
    } else if !valid_username(username) {
        Some("Account name: lowercase letters, digits, - and _, starting with a letter.")
    } else if pw.is_empty() {
        Some("Choose a password.")
    } else if pw != pw2 {
        Some("The passwords do not match.")
    } else {
        None
    }
}

// ---- executor ----------------------------------------------------------------------------------

struct Run<'a> {
    p: &'a Shared,
}

impl Run<'_> {
    fn set(&self, stage: usize, percent: u8, msg: &str) {
        // always in the journal: a dozen lines per install, and the only trace a lab run has of where it is
        eprintln!("bifrost-installer: [{percent}%] stage {stage}: {msg}");
        let mut p = self.p.lock().unwrap();
        p.stage = stage;
        p.percent = percent.max(p.percent);
        p.message = msg.to_string();
        p.log.push(format!("== {msg}"));
    }

    fn log(&self, s: String) {
        if std::env::var_os("BIFROST_TRACE").is_some() {
            eprintln!("bifrost-installer: {s}");
        }
        let mut p = self.p.lock().unwrap();
        p.log.push(s);
        if p.log.len() > 400 {
            p.log.drain(..100);
        }
    }

    /// Runs a program, logging its output; Err carries the failing command and its tail of output.
    fn cmd(&self, args: &[&str]) -> Result<String, String> {
        self.cmd_in(args, None)
    }

    fn cmd_in(&self, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        self.log(format!("$ {}", args.iter().map(|a| if a.contains("PASS") { "…" } else { a }).collect::<Vec<_>>().join(" ")));
        let mut c = Command::new(args[0]);
        c.args(&args[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child = c.spawn().map_err(|e| format!("{}: {e}", args[0]))?;
        if let (Some(input), Some(mut si)) = (stdin, child.stdin.take()) {
            let _ = si.write_all(input.as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        for l in text.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev() {
            self.log(l.to_string());
        }
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!("{} failed ({}): {}", args[0], out.status, text.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")))
        }
    }

    fn write(&self, path: &str, contents: &str) -> Result<(), String> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(path, contents).map_err(|e| format!("{path}: {e}"))
    }

    fn chroot(&self, args: &[&str]) -> Result<String, String> {
        let mut v = vec!["arch-chroot", "/mnt"];
        v.extend_from_slice(args);
        self.cmd(&v)
    }
}

/// Files the installer needs from the live medium. The boot loader goes to the ESP after pacstrap. The shell's assets and
/// installer media are not in the `springboard-shell` update package (it is built without assets), so they come from here.
/// The shell, its units and session come from the package, which is installed last so it wins any overlap.
/// They are copied into RAM first: if the medium is a network CD that drops out half way, the install can still finish.
const LIVE_FILES: &[&str] = &["/usr/share/bifrost/splash.efi"];
const LIVE_DIRS: &[&str] = &["/usr/share/springboard/assets", "/usr/share/bifrost"];
const STAGE: &str = "/run/bifrost-stage";

pub fn stage_live_files() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_dir_all(STAGE);
    std::fs::create_dir_all(STAGE).map_err(|e| e.to_string())?;
    std::fs::set_permissions(STAGE, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    for f in LIVE_FILES.iter().chain(LIVE_DIRS) {
        let dest = format!("{STAGE}{f}");
        std::fs::create_dir_all(std::path::Path::new(&dest).parent().unwrap()).map_err(|e| e.to_string())?;
        let source=if std::path::Path::new(f).is_dir() { std::fs::create_dir_all(&dest).map_err(|e|e.to_string())?; format!("{f}/.") } else { (*f).to_string() };
        let st = Command::new("cp").args(["-a", &source, &dest]).status().map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(format!("cannot read {f} from the installer medium"));
        }
    }
    Ok(())
}

/// Runs the whole installation. Updates `progress` as it goes; on error `failed` is set.
pub fn install(cfg: &Config, progress: &Shared) {
    progress.lock().unwrap().started = Some(std::time::Instant::now());
    let r = Run { p: progress };
    let res = (|| -> Result<(), String> {
        r.set(0, 0, "Reading the installer files");
        stage_live_files()?;
        r.cmd(&["btrfs","--version"])?;
        r.cmd(&["modprobe", "btrfs"])?;
        r.set(0, 1, "Partitioning the disk");
        let dev = cfg.disk.path.as_str();
        // A previous attempt that failed part way leaves its subvolumes mounted under /mnt, and parted then refuses to
        // rewrite the table ("Partition(s) are being used"). Release the target first; errors just mean "not mounted".
        let _ = r.cmd(&["sh", "-c", &format!("umount -R /mnt 2>/dev/null; for p in $(lsblk -nrpo PATH {dev}); do umount \"$p\" 2>/dev/null; done; swapoff -a 2>/dev/null; true")]);
        let before = list_parts(&r, dev);
        for c in partition_commands(&cfg.disk, cfg.mode) {
            let args: Vec<&str> = c.iter().map(String::as_str).collect();
            r.cmd(&args)?;
        }
        let _ = r.cmd(&["udevadm", "settle"]);
        let after = list_parts(&r, dev);
        // Erase rewrites the table, so the new partitions reuse the old names: take every partition then.
        let mut new: Vec<&String> = after.iter().filter(|p| p.as_str() != dev && (cfg.mode == Mode::Erase || !before.contains(p))).collect();
        new.sort_by_key(|p| (p.len(), p.to_string()));
        let (efi, root): (String, String) = match (cfg.mode, new.as_slice()) {
            (Mode::Erase, [e, rt, ..]) | (Mode::FreeSpace, [e, rt, ..]) => ((*e).clone(), (*rt).clone()),
            (Mode::FreeSpace, [rt]) => (cfg.disk.reusable_esp().map(|p| p.path.clone()).ok_or("no EFI partition to reuse")?, (*rt).clone()),
            _ => return Err("the new partitions did not appear".into()),
        };
        let new_esp = !(cfg.mode == Mode::FreeSpace && new.len() == 1);
        r.set(0, 3, "Formatting");
        if new_esp {
            r.cmd(&["mkfs.fat", "-F32", &efi])?;
        }
        r.cmd(&["mkfs.btrfs", "-f", "-L", "BifrOSt", &root])?;
        r.cmd(&["mount", "-t", "btrfs", &root, "/mnt"])?;
        for (subvol, _) in BTRFS_LAYOUT {r.cmd(&["btrfs","subvolume","create",&format!("/mnt/{subvol}")])?;}
        r.cmd(&["umount","/mnt"])?;
        r.cmd(&["mount","-t","btrfs","-o","subvol=@system,compress=zstd:3,noatime",&root,"/mnt"])?;
        for (subvol, target) in BTRFS_LAYOUT.iter().skip(1) {
            let mount=format!("/mnt{target}");std::fs::create_dir_all(&mount).map_err(|e|e.to_string())?;
            r.cmd(&["mount","-t","btrfs","-o",&format!("subvol={subvol},compress=zstd:3,noatime"),&root,&mount])?;
        }
        std::fs::create_dir_all("/mnt/boot").map_err(|e| e.to_string())?;
        r.cmd(&["mount", &efi, "/mnt/boot"])?;

        r.set(1, 4, "Installing the system and apps");
        let lspci = r.cmd(&["lspci", "-nn"]).unwrap_or_default();
        let gpu = gpu_packages(&lspci);
        r.log(format!("graphics stack: {}", gpu.join(" ")));
        if !cfg.mirror.is_empty() {
            // every package from the lab's offline mirror (the live system's own configuration is kept aside)
            if !std::path::Path::new("/etc/pacman.conf.online").exists() {
                let _ = std::fs::copy("/etc/pacman.conf", "/etc/pacman.conf.online");
            }
            std::fs::write("/etc/pacman.conf", offline_pacman_conf(&cfg.mirror)).map_err(|e| format!("/etc/pacman.conf: {e}"))?;
            r.log(format!("packages from the lab mirror {}", cfg.mirror));
        } else if let Ok(conf) = std::fs::read_to_string("/etc/pacman.conf") {
            // more parallel downloads than the stock 5 (the link, not the CPU, is the limit)
            let _ = std::fs::write("/etc/pacman.conf", conf.replace("ParallelDownloads = 5", "ParallelDownloads = 12"));
        }
        let mut args = vec!["pacstrap", "-K", "/mnt"];
        args.extend(packages());
        args.extend(gpu);
        let watcher = {
            let p = progress.clone();
            std::thread::spawn(move || {
                while !p.lock().unwrap().done && p.lock().unwrap().stage == 1 {
                    if let Some(used) = used_bytes("/mnt") {
                        let frac = (used / EXPECTED_BYTES).min(1.0);
                        let mut g = p.lock().unwrap();
                        g.percent = g.percent.max((RANGE[1].0 as f64 + (RANGE[1].1 - RANGE[1].0) as f64 * frac) as u8);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(700));
                }
            })
        };
        let res = r.cmd(&args);
        progress.lock().unwrap().stage = 2;
        let _ = watcher.join();
        res?;

        r.set(2, 83, "Configuring the system");
        let sys = |p: &str| format!("/mnt{p}");
        let fstab = r.cmd(&["genfstab", "-U", "/mnt"])?;
        r.write(&sys("/etc/fstab"), &fstab)?;
        r.write(&sys("/etc/hostname"), &format!("{}\n", cfg.hostname))?;
        if valid_timezone(&cfg.timezone) {
            r.chroot(&["ln", "-sf", &format!("/usr/share/zoneinfo/{}", cfg.timezone), "/etc/localtime"])?;
        }
        r.write(&sys("/etc/locale.conf"), &locale_conf(&cfg.locale))?;
        let gen_line = locale_gen_line(&cfg.locale);
        let lg = std::fs::read_to_string(sys("/etc/locale.gen")).unwrap_or_default();
        let lg = lg.lines().map(|l| if l.trim_start_matches('#').trim() == gen_line { gen_line.clone() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        r.write(&sys("/etc/locale.gen"), &(lg + "\n"))?;
        r.chroot(&["locale-gen"])?;
        r.write(&sys("/etc/vconsole.conf"), &vconsole_conf(&cfg.layout))?;
        r.write(&sys("/etc/X11/xorg.conf.d/00-keyboard.conf"), &x11_keyboard_conf(&cfg.layout))?;
        r.write(&sys("/etc/bifrost/keyboard.env"), &wayland_keyboard_conf(&cfg.layout))?;
        r.write(&sys("/usr/lib/os-release"), os_release())?;
        r.write(&sys("/etc/ssh/sshd_config.d/10-bifrost.conf"), sshd_dropin())?;
        // a lab ISO (it carries /etc/bifrost/lab.token) leaves the lab's root key on the installed system, so the next test
        // iteration can reach it; a release ISO has no token and installs nothing of the sort
        if std::path::Path::new("/etc/bifrost/lab.token").exists() {
            if let Ok(keys) = std::fs::read_to_string("/root/.ssh/authorized_keys") {
                std::fs::create_dir_all(sys("/root/.ssh")).map_err(|e| e.to_string())?;
                r.write(&sys("/root/.ssh/authorized_keys"), &keys)?;
                let _ = r.cmd(&["chmod", "700", &sys("/root/.ssh")]);
                let _ = r.cmd(&["chmod", "600", &sys("/root/.ssh/authorized_keys")]);
                // sshd keeps the first value it reads, so this must sort before 10-bifrost.conf. Password logins stay on
                // for the account; only root is key-only.
                r.write(&sys("/etc/ssh/sshd_config.d/05-lab.conf"), "PermitRootLogin prohibit-password\n")?;
                r.log("lab access: root key installed".to_string());
            }
            // The lab link is point-to-point with titan and its DHCP (iVentoy) only answers PXE boots, so the installed
            // system keeps the address the live system had on that NIC as a static profile, matched by MAC.
            if let Some((mac, cidr)) = lab_link() {
                let dir = sys("/etc/NetworkManager/system-connections");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let file = format!("{dir}/lab.nmconnection");
                r.write(&file, &lab_nmconnection(&mac, &cidr))?;
                let _ = r.cmd(&["chmod", "600", &file]);
                r.log(format!("lab access: static {cidr} on {mac}"));
            }
        }

        // the update channel: System Update installs `springboard-shell` from this repo (a lab ISO points at titan's lab repo)
        {
            let lab = std::path::Path::new("/etc/bifrost/lab.token").exists();
            let server = if lab { "http://192.168.1.12:8079/bifrost/$arch" } else { "https://bifrost.olibuijr.com/arch/$arch" };
            let conf = std::fs::read_to_string(sys("/etc/pacman.conf")).unwrap_or_default();
            if !conf.contains("[bifrost]") {
                r.write(&sys("/etc/pacman.conf"), &format!("{conf}\n[bifrost]\nSigLevel = Optional TrustAll\nServer = {server}\n"))?;
            }
        }

        r.set(2, 86, "Installing the desktop");
        // installer media only: splash.efi and wayland/ are owned by the package, and pacman refuses to overwrite files it does not own
        for (src, dst) in [("/usr/share/springboard/assets", "/usr/share/springboard/"), ("/usr/share/bifrost/installer", "/usr/share/bifrost/"), ("/usr/share/bifrost/video", "/usr/share/bifrost/")] {
            std::fs::create_dir_all(sys(dst)).map_err(|e| e.to_string())?;
            r.cmd(&["cp", "-a", &format!("{STAGE}{src}"), &sys(dst)])?;
        }
        // the live medium can be older than the packed assets the lab sync brought: drop what the packed set no longer has
        prune_assets(std::path::Path::new(&sys("/usr/share/springboard/assets"))).map_err(|e| format!("assets: {e}"))?;
        // The package owns the shell, units, session and apps. Installing it here, from the same
        // repo System Update uses, keeps one source of truth and no fake pacman registration.
        r.chroot(&["pacman", "-Sy", "--noconfirm", "springboard-shell"])?;

        r.set(2, 89, "Installing default apps from App Store");
        install_default_apps(&r, &cfg.mirror)?;
        r.set(2, 90, "Creating your account");
        let name = if cfg.full_name.is_empty() { &cfg.username } else { &cfg.full_name };
        r.chroot(&["useradd", "-m", "-G", "wheel,audio,video,input,storage", "-s", "/bin/bash", "-c", name, &cfg.username])?;
        r.cmd_in(&["arch-chroot", "/mnt", "chpasswd"], Some(&format!("{}:{}\n", cfg.username, cfg.password)))?;
        r.write(&sys("/etc/sudoers.d/10-wheel"), "%wheel ALL=(ALL:ALL) ALL\n")?;
        let _ = r.cmd(&["chmod", "440", &sys("/etc/sudoers.d/10-wheel")]);
        // lab ISO: the account gets the lab key too, so the user can log in over ssh (and sudo) without root
        if std::path::Path::new("/etc/bifrost/lab.token").exists() {
            if let Ok(keys) = std::fs::read_to_string("/root/.ssh/authorized_keys") {
                let dir = format!("/home/{}/.ssh", cfg.username);
                std::fs::create_dir_all(sys(&dir)).map_err(|e| e.to_string())?;
                r.write(&sys(&format!("{dir}/authorized_keys")), &keys)?;
                r.chroot(&["chown", "-R", &format!("{0}:{0}", cfg.username), &dir])?;
                r.chroot(&["chmod", "700", &dir])?;
                r.chroot(&["chmod", "600", &format!("{dir}/authorized_keys")])?;
            }
        }
        let _ = r.chroot(&["passwd", "-l", "root"]);
        r.write(&sys("/var/lib/springboard/account"), &account_file(name, &cfg.username))?;
        // the look chosen in the installer (merged with any settings the image already carries)
        {
            let path = sys("/var/lib/springboard/settings.ini");
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            r.write(&path, &settings_ini(cfg, &existing))?;
        }
        // a fresh install starts with no crash-loop history (the counter is kept in @state)
        let _ = r.chroot(&["rm", "-f", "/var/lib/springboard/boot_attempts"]);

        r.set(2, 93, "Setting up the startup screen");
        let uuid = r.cmd(&["blkid", "-s", "UUID", "-o", "value", &root])?.trim().to_string();
        r.cmd(&["cp", &format!("{STAGE}/usr/share/bifrost/splash.efi"), &sys("/boot/EFI/BOOT/BOOTx64.efi")]).or_else(|_| {
            std::fs::create_dir_all(sys("/boot/EFI/BOOT")).map_err(|e| e.to_string())?;
            r.cmd(&["cp", &format!("{STAGE}/usr/share/bifrost/splash.efi"), &sys("/boot/EFI/BOOT/BOOTx64.efi")])
        })?;
        r.write(&sys("/boot/EFI/BOOT/cmdline.txt"), &cmdline_txt(&uuid))?;
        // Stable BifrOSt namespace for the native EFI loader. Keep the BOOT
        // copy above because recovery tooling and older USB media still read it.
        std::fs::create_dir_all(sys("/boot/EFI/BIFROST")).map_err(|e| e.to_string())?;
        r.cmd(&["cp", &format!("{STAGE}/usr/share/bifrost/splash.efi"), &sys("/boot/EFI/BIFROST/BOOTx64.efi")])?;
        r.write(&sys("/boot/EFI/BIFROST/boot.cfg"), &cmdline_txt(&uuid))?;
        // The firmware entry names the BifrOSt loader; EFI/BOOT stays as the removable-media fallback.
        let _ = r.cmd(&["efibootmgr", "-c", "-d", dev, "-p", &part_number(&efi), "-L", "BifrOSt", "-l", "\\EFI\\BIFROST\\BOOTx64.efi"]);
        let mc = std::fs::read_to_string(sys("/etc/mkinitcpio.conf")).unwrap_or_default();
        let mc = mc.lines().map(|l| {
            // busybox init mounts the root at once; the systemd initrd held sysroot.mount ~0.9 s behind its
            // mount-event rate limit (boot-bench). btrfs needs no fsck and the console is never shown.
            if l.starts_with("HOOKS=") { INITRAMFS_HOOKS.to_string() } else if l.starts_with("MODULES=(") { let mut line=l.to_string(); for module in ["virtio_gpu", "btrfs"] { if !line.contains(module) { line=line.replacen("MODULES=(", &format!("MODULES=({module} "), 1); } } line } else { l.to_string() }
        }).collect::<Vec<_>>().join("\n");
        r.write(&sys("/etc/mkinitcpio.conf"), &(mc + "\n"))?;
        // the package ships the hook; an older package from the update repo may not, so keep a local copy then
        if !std::path::Path::new(&sys("/usr/lib/initcpio/install/bifrost-mounts")).exists() {
            r.write(&sys("/etc/initcpio/install/bifrost-mounts"), include_str!("../../../packaging/arch/initcpio/install/bifrost-mounts"))?;
            r.write(&sys("/etc/initcpio/hooks/bifrost-mounts"), include_str!("../../../packaging/arch/initcpio/hooks/bifrost-mounts"))?;
        }
        r.cmd(&["install","-Dm755","/mnt/usr/bin/bifrost-update-guard","/mnt/var/lib/bifrost-recovery/engine"])?;
        r.chroot(&["systemctl", "mask", "getty@tty1.service"])?;
        r.write(&sys("/etc/systemd/system.conf.d/bifrost-watchdog.conf"),"[Manager]\nRuntimeWatchdogSec=60s\nRebootWatchdogSec=120s\n")?;
        r.write(&sys("/etc/modprobe.d/bifrost-audio.conf"), audio_modprobe())?;
        // Write the unit before enabling it. systemctl enable resolves the
        // unit immediately inside the target root and otherwise aborts the
        // install with status 1 on a fresh system.
        r.write(&sys("/etc/systemd/system/bifrost-irq-priority.service"), irq_priority_unit())?;
        r.chroot(&["systemctl", "enable", "bifrost-update-guard.service", "springboard.service", "NetworkManager.service", "sshd.service", "seatd.service", "power-profiles-daemon.service", "bifrost-irq-priority.service", "bifrost-mcp.service"])?;
        r.chroot(&["systemctl", "set-default", "graphical.target"])?;
        // one initramfs: the fallback image is a second full build nobody boots on a fresh install
        r.write(&sys("/etc/mkinitcpio.d/linux.preset"), "ALL_config=\"/etc/mkinitcpio.conf\"\nALL_kver=\"/boot/vmlinuz-linux\"\nPRESETS=('default')\ndefault_image=\"/boot/initramfs-linux.img\"\n")?;
        r.chroot(&["mkinitcpio", "-p", "linux"])?;

        r.set(3, 98, "Finishing");
        let _ = r.cmd(&["sync"]);
        let _ = r.cmd(&["fuser", "-k", "-m", "/mnt"]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        r.cmd(&["umount", "-R", "/mnt"]).or_else(|_| r.cmd(&["umount", "-R", "-l", "/mnt"]))?;
        Ok(())
    })();
    let mut p = progress.lock().unwrap();
    match res {
        Ok(()) => {
            p.percent = 100;
            p.message = "BifrOSt is installed".into();
            p.done = true;
        }
        Err(e) => {
            eprintln!("bifrost-installer: FAILED: {e}");
            p.failed = Some(e)
        }
    }
}

const DEFAULT_APPS: [&str; 2] = ["is.olibuijr.browser.desktop", "is.olibuijr.files.desktop"];

/// A store package on the lab server next to the offline mirror (tools/lab-push.sh links its `store/`), or None outside
/// the lab and for anything but a plain `/store/...` path.
pub fn lab_package_url(mirror: &str, package: &str) -> Option<String> {
    let base = mirror.strip_suffix("/mirror/").filter(|b| b.starts_with("http://"))?;
    (package.starts_with("/store/") && !package.contains("..")).then(|| format!("{base}{package}"))
}

/// The default apps, through the App Store. A lab install takes each package's checksum from the store's HTTPS catalogue
/// but the bytes from the lab server: the store's 54 MB browser came at 2 MB/s through the public tunnel (25 s of a
/// still progress bar), and the lab link is gigabit. Any miss falls back to the App Store download.
fn install_default_apps(r: &Run, mirror: &str) -> Result<(), String> {
    use crate::shell::store::{api, net};
    let catalogue = if mirror.is_empty() {
        vec![]
    } else {
        r.cmd(&["curl", "-fsS", "--max-time", "20", &net::absolute("/api/v1/pc/apps.json")])
            .ok()
            .and_then(|t| api::parse_catalog(&t).ok())
            .unwrap_or_default()
    };
    for id in DEFAULT_APPS {
        let lab = catalogue.iter().find(|a| a.id == id).and_then(|a| Some((lab_package_url(mirror, &a.package)?, a.sha256.to_ascii_lowercase())));
        let local = lab.and_then(|(url, sha)| {
            let file = format!("/mnt/tmp/{id}.aap");
            r.cmd(&["curl", "-fsS", "--max-time", "60", "-o", &file, &url]).ok()?;
            let sum = r.cmd(&["sha256sum", &file]).ok()?;
            if sum.split_whitespace().next() != Some(sha.as_str()) {
                r.log(format!("{id}: lab package checksum mismatch; using the App Store"));
                let _ = std::fs::remove_file(&file);
                return None;
            }
            Some(file)
        });
        match local {
            Some(file) => {
                let res = r.chroot(&["/usr/bin/springboard", "--install-package", file.trim_start_matches("/mnt")]);
                let _ = std::fs::remove_file(&file);
                if res.is_err() {
                    r.chroot(&["/usr/bin/springboard", "--install-store-app", id])?;
                }
            }
            None => {
                r.chroot(&["/usr/bin/springboard", "--install-store-app", id])?;
            }
        }
    }
    Ok(())
}

fn part_number(path: &str) -> String {
    path.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect()
}

fn list_parts(r: &Run, dev: &str) -> Vec<String> {
    r.cmd(&["lsblk", "-nrpo", "PATH", dev]).map(|t| t.lines().filter(|l| *l != dev).map(String::from).collect()).unwrap_or_default()
}

fn used_bytes(mount: &str) -> Option<f64> {
    let o = Command::new("df").args(["-B1", "--output=used", mount]).output().ok()?;
    String::from_utf8_lossy(&o.stdout).lines().nth(1)?.trim().parse().ok()
}

/// The UI fonts the packed asset set has at its top level (`tools/pack.py`); the font library lives in `fonts/`.
const PACKED_FONTS: [&str; 3] = ["SFUI.ttf", "SFUIMono.ttf", "Symbols.ttf"];

/// Removes copied asset files the packed set no longer has: top-level fonts other than `PACKED_FONTS`, and stills
/// (and their thumbnails) whose id `wallpapers/index.ini` does not list. Without an index the stills are kept.
fn prune_assets(dir: &std::path::Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let font = [".ttf", ".otf", ".ttc"].iter().any(|x| name.to_ascii_lowercase().ends_with(x));
        if font && e.file_type()?.is_file() && !PACKED_FONTS.contains(&name.as_str()) {
            std::fs::remove_file(e.path())?;
        }
    }
    let stills = dir.join("wallpapers");
    let Ok(index) = std::fs::read_to_string(stills.join("index.ini")) else { return Ok(()) };
    let ids: Vec<&str> = index.lines().filter_map(|l| l.split_once('=').map(|(k, _)| k.trim())).collect();
    for e in std::fs::read_dir(&stills)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".thumb.raw").or_else(|| name.strip_suffix(".raw")) else { continue };
        if !ids.contains(&id) {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn prune_assets_keeps_only_the_packed_set() {
        let dir = std::env::temp_dir().join(format!("prune-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("wallpapers")).unwrap();
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        for f in ["SFUI.ttf", "SFUIMono.ttf", "Symbols.ttf", "Old-Symbols.ttf", "Stub.otf", "fonts/Geist-Variable.ttf", "wallpaper.raw",
                  "wallpapers/kirkjufell_dusk.raw", "wallpapers/kirkjufell_dusk.thumb.raw", "wallpapers/desert_day.raw", "wallpapers/desert_day.thumb.raw"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::fs::write(dir.join("wallpapers/index.ini"), "kirkjufell_dusk=Kirkjufell Dusk\n").unwrap();
        super::prune_assets(&dir).unwrap();
        let mut left: Vec<String> = walk(&dir, &dir);
        left.sort();
        assert_eq!(left, ["SFUI.ttf", "SFUIMono.ttf", "Symbols.ttf", "fonts/Geist-Variable.ttf", "wallpaper.raw", "wallpapers/index.ini", "wallpapers/kirkjufell_dusk.raw", "wallpapers/kirkjufell_dusk.thumb.raw"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn walk(root: &std::path::Path, d: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(d).unwrap().flatten().flat_map(|e| {
            if e.path().is_dir() { walk(root, &e.path()) } else { vec![e.path().strip_prefix(root).unwrap().to_string_lossy().into_owned()] }
        }).collect()
    }

    use super::*;
    use crate::installer::keymaps::{layout_index, layouts};

    #[test]
    fn lab_profile_pins_the_address_on_the_lab_nic() {
        let k = lab_nmconnection("00:23:24:dc:c8:40", "10.99.0.200/24");
        assert!(k.contains("mac-address=00:23:24:dc:c8:40\n"));
        assert!(k.contains("method=manual\naddress1=10.99.0.200/24,10.99.0.1\n"));
        assert!(k.contains("[ipv6]\nmethod=ignore"));
    }

    fn layout(id: &str) -> Layout {
        layouts().nth(layout_index(id).unwrap()).unwrap()
    }

    fn look_config(appearance: &str, accent: &str, wallpaper: &str) -> Config {
        Config {
            disk: crate::installer::disks::parse_parted("BYT;\n/dev/vda:17179869184B:virtio:512:512:unknown:Virtio Block Device:;\n").unwrap(),
            mode: Mode::Erase, locale: "en_US.UTF-8".into(), layout: layout("us"), full_name: "Ada".into(), username: "ada".into(),
            password: String::new(), hostname: "bifrost".into(), timezone: String::new(), mirror: String::new(),
            appearance: appearance.into(), accent: accent.into(), wallpaper: wallpaper.into(),
        }
    }

    #[test]
    fn settings_ini_carries_the_chosen_look() {
        let s = settings_ini(&look_config("light", "purple", "gullfoss_canyon"), "");
        assert_eq!(s, "appearance_mode=light\naccent_color=purple\nwallpaper=gullfoss_canyon\n");
    }

    #[test]
    fn settings_ini_merges_and_normalises() {
        let s = settings_ini(&look_config("neon", "nope", "../etc"), "# keep\nliquid_glass=0.5\n\nappearance_mode=auto\nwallpaper=old\n");
        assert!(s.contains("liquid_glass=0.5\n") && s.contains("# keep\n"));
        assert!(s.contains("appearance_mode=dark\n") && s.contains("accent_color=blue\n"));
        assert!(!s.contains("wallpaper=") && !s.contains("auto") && s.matches("appearance_mode").count() == 1);
        assert!(valid_wallpaper_id("kirkjufell_dusk") && !valid_wallpaper_id("a/b") && !valid_wallpaper_id(""));
    }

    #[test]
    fn timezone_names() {
        assert!(valid_timezone("Atlantic/Reykjavik") && valid_timezone("Etc/GMT+1"));
        assert!(!valid_timezone("") && !valid_timezone("../etc/passwd") && !valid_timezone("/etc/passwd") && !valid_timezone("a b"));
    }

    #[test]
    fn config_files() {
        assert_eq!(locale_conf("is_IS.UTF-8"), "LANG=is_IS.UTF-8\n");
        assert_eq!(locale_gen_line("is_IS.UTF-8"), "is_IS.UTF-8 UTF-8");
        assert_eq!(vconsole_conf(&layout("is")), "KEYMAP=is-latin1\n");
        let x = x11_keyboard_conf(&layout("us-intl"));
        assert!(x.contains("\"XkbLayout\" \"us\"") && x.contains("\"XkbVariant\" \"intl\""));
        assert!(!x11_keyboard_conf(&layout("de")).contains("XkbVariant"));
        assert_eq!(wayland_keyboard_conf(&layout("us-intl")), "XKB_DEFAULT_LAYOUT=us\nXKB_DEFAULT_VARIANT=intl\n");
        assert_eq!(account_file("Ada Lovelace", "ada"), "full_name=Ada Lovelace\nusername=ada\n");
        assert!(sshd_dropin().contains("PermitRootLogin no"));
        let c = cmdline_txt("abcd");
        assert!(c.starts_with("initrd=\\initramfs-linux.img root=UUID=abcd rootflags=subvol=@system rw quiet") && c.ends_with('\n'));
        assert!(!c.contains("plymouth") && !c.contains(" splash"));
        assert!(!INITRAMFS_HOOKS.contains("systemd") && !INITRAMFS_HOOKS.contains("fsck") && INITRAMFS_HOOKS.contains(" kms "));
        assert!(c.contains(" threadirqs") && !c.contains("isolcpus"));
        assert!(c.contains(" systemd.tty.rows.console= "));
        let iso = cmdline_with("abcd", Some("7"));
        assert!(iso.contains(" threadirqs isolcpus=7 nohz_full=7 rcu_nocbs=7\n"));
        assert_eq!(audio_modprobe(), "options snd_hda_intel power_save=0\n");
        assert!(irq_priority_unit().contains("chrt -f -p 50") && irq_priority_unit().contains("irq/*xhci*"));
    }

    #[test]
    fn username_and_password_rules() {
        assert!(valid_username("ada") && valid_username("_x-1") && !valid_username("Ada") && !valid_username("1a") && !valid_username(""));
        assert!(!valid_username(&"a".repeat(33)));
        assert_eq!(account_error("", "ada", "x", "x"), Some("Enter your name."));
        assert_eq!(account_error("Ada", "ada", "x", "y"), Some("The passwords do not match."));
        assert_eq!(account_error("Ada", "ada", "", ""), Some("Choose a password."));
        assert_eq!(account_error("Ada", "ada", "x", "x"), None);
    }

    #[test]
    fn eta_needs_progress_first() {
        let mut p = Progress { percent: 2, started: Some(std::time::Instant::now()), ..Default::default() };
        assert_eq!(p.eta_minutes(), None);
        p.percent = 4;
        p.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(22));
        assert_eq!(p.eta_minutes(), None, "disk setup and the db sync alone would extrapolate to 9 min");
        p.percent = 50;
        p.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(120));
        assert_eq!(p.eta_minutes(), Some(2));
        p.done = true;
        assert_eq!(p.eta_minutes(), None);
    }

    #[test]
    fn lab_installs_fetch_store_packages_from_the_lab_server() {
        let pkg = "/store/is.olibuijr.files.desktop/is.olibuijr.files.desktop-1.15.1.aap";
        assert_eq!(
            lab_package_url("http://10.99.0.1:8078/mirror/", pkg).as_deref(),
            Some("http://10.99.0.1:8078/store/is.olibuijr.files.desktop/is.olibuijr.files.desktop-1.15.1.aap")
        );
        assert_eq!(lab_package_url("", pkg), None, "no lab mirror: the App Store as usual");
        assert_eq!(lab_package_url("http://10.99.0.1:8078/mirror/", "https://elsewhere/x.aap"), None, "only store paths");
        assert_eq!(lab_package_url("http://10.99.0.1:8078/mirror/", "/store/../etc/x.aap"), None);
    }

    #[test]
    fn an_offline_mirror_is_the_only_repository_pacstrap_sees() {
        let c = offline_pacman_conf("http://10.99.0.1:8078/mirror/");
        assert!(c.contains("[offline]") && c.contains("Server = http://10.99.0.1:8078/mirror/"));
        assert!(!c.contains("[core]") && !c.contains("[extra]"), "nothing comes from the internet");
        assert!(c.contains("SigLevel = Never"), "the lab mirror is unsigned (a local repo-add index)");
    }

    #[test]
    fn gpu_packages_follow_the_hardware() {
        let intel = "00:02.0 VGA compatible controller [0300]: Intel Corporation HD Graphics 530 [8086:1912] (rev 06)";
        assert!(gpu_packages(intel).contains(&"intel-media-driver") && gpu_packages(intel).contains(&"vulkan-intel"));
        let amd = "06:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Navi 22 [1002:73df]";
        assert!(gpu_packages(amd).contains(&"vulkan-radeon") && !gpu_packages(amd).contains(&"nvidia-open"));
        let nv = "01:00.0 VGA compatible controller [0300]: NVIDIA Corporation GA102 [10de:2216]";
        assert!(gpu_packages(nv).contains(&"nvidia-open") && gpu_packages(nv).contains(&"libva-nvidia-driver"));
        let both = format!("{intel}\n{nv}");
        let p = gpu_packages(&both);
        assert!(p.contains(&"intel-media-driver") && p.contains(&"nvidia-utils"), "hybrid laptops get both stacks");
        assert_eq!(gpu_packages("00:1f.3 Audio device: Intel [8086:a170]"), ["mesa", "vulkan-icd-loader", "libva-utils", "ffmpeg"], "non-display devices add nothing");
    }

    #[test]
    fn partition_numbers() {
        assert_eq!(part_number("/dev/nvme0n1p1"), "1");
        assert_eq!(part_number("/dev/sda12"), "12");
    }
}
