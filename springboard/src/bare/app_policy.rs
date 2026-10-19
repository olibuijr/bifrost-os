//! Desktop app launch boundary. The shell chooses identity and resources; apps
//! never launch with root credentials or select privileged command arguments.
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub use super::account::Account;

/// A policy failure with a stable domain and code. The functions here still return `io::Result`, so the error travels
/// inside an `io::Error`; [`PolicyError::of`] reads it back for an app or a shell caller that branches on the cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// A manifest permission this shell does not know.
    UnsupportedPermission(String),
    /// Remote sharing needs both the network and the storage permissions.
    RemoteNeedsNetworkAndStorage,
    /// The app is not the remote desktop app, or it does not hold the remote grant.
    RemoteDenied,
    /// The remote server script resolves to a path outside its package.
    RemoteServerEscapesPackage,
    /// The remote service could not be started or stopped.
    RemoteServiceFailed,
    /// The app data directory is a symbolic link, which is refused.
    DataIsSymlink,
    /// The app data directory could not be handed to the app's account.
    DataOwnershipFailed,
    /// The executable has no containing package directory.
    MissingPackageDirectory,
}

impl PolicyError {
    /// The domain every policy error belongs to.
    pub const DOMAIN: &'static str = "bifrost.app-policy";

    /// The domain name, the same for every variant.
    pub fn domain(&self) -> &'static str {
        Self::DOMAIN
    }

    /// The code of this failure inside its domain. Codes are stable: a new failure takes a new number.
    pub fn code(&self) -> i32 {
        match self {
            Self::UnsupportedPermission(_) => 1,
            Self::RemoteNeedsNetworkAndStorage => 2,
            Self::RemoteDenied => 3,
            Self::RemoteServerEscapesPackage => 4,
            Self::RemoteServiceFailed => 5,
            Self::DataIsSymlink => 6,
            Self::DataOwnershipFailed => 7,
            Self::MissingPackageDirectory => 8,
        }
    }

    /// The failure inside `err`, when `err` came from this module. `None` for any other I/O error.
    pub fn of(err: &io::Error) -> Option<&PolicyError> {
        err.get_ref()?.downcast_ref::<PolicyError>()
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPermission(name) => write!(f, "Unsupported app permission: {name}"),
            Self::RemoteNeedsNetworkAndStorage => f.write_str("Remote sharing requires network and storage"),
            Self::RemoteDenied => f.write_str("Remote sharing permission denied"),
            Self::RemoteServerEscapesPackage => f.write_str("Remote server escapes package"),
            Self::RemoteServiceFailed => f.write_str("Remote service request failed"),
            Self::DataIsSymlink => f.write_str("App data must not be a symlink"),
            Self::DataOwnershipFailed => f.write_str("Cannot assign app data ownership"),
            Self::MissingPackageDirectory => f.write_str("Missing app directory"),
        }
    }
}

impl std::error::Error for PolicyError {}

impl From<PolicyError> for io::Error {
    fn from(err: PolicyError) -> io::Error {
        let kind = match err {
            PolicyError::RemoteDenied => io::ErrorKind::PermissionDenied,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, err)
    }
}

/// Every name an entitlement query accepts: the manifest permissions and the first-party entitlements, in the order
/// [`Policy::held`] lists them.
pub const ENTITLEMENT_NAMES: [&str; 19] = [
    "network",
    "storage",
    "audio",
    "camera",
    "screen",
    "notifications",
    "documents",
    "keychain",
    "accessibility",
    "display-config",
    "sensors",
    "contacts",
    "calendars",
    "usb",
    "hid",
    "gpu",
    "remote-desktop",
    bifrost_core::log::ENTITLEMENT,
    SYSTEM_MONITOR_ENTITLEMENT,
];

/// `entitlements = ["system-monitor"]` on a first-party `system-app` (Activity Monitor): the sandbox shares the host PID
/// and network namespaces and sees `/proc` read-only, so the app can list every process and socket of the machine.
pub const SYSTEM_MONITOR_ENTITLEMENT: &str = "system-monitor";

/// First-party app identities allowed to declare capture device permissions.
pub const CAMERA_APP_ID: &str = "is.olibuijr.photobooth.desktop";
pub const SCREEN_APP_ID: &str = "is.olibuijr.colormeter.desktop";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub network: bool,
    pub storage: bool,
    pub audio: bool,
    /// `camera`: binds detected V4L2 capture nodes for Photo Booth.
    pub camera: bool,
    /// `screen`: declaration for Color Meter; use remains consent-gated by shell services.
    pub screen: bool,
    pub remote: bool,
    pub notifications: bool,
    /// `documents`: may ask for files through the shell's open and save panels (`docs/file-portal-design.md`).
    /// It binds nothing: each file arrives as a descriptor for the one file the user chose.
    pub documents: bool,
    /// `keychain`: may store secrets with the shell (`ToShell::Keychain`); it binds nothing.
    pub keychain: bool,
    /// `accessibility`: may post input events and install event taps through the shell's `events` service. It binds
    /// nothing here; the service checks the manifest on every call.
    pub accessibility: bool,
    /// `display-config`: may ask the shell's `display` service for a different display mode. Checked by the service.
    pub display_config: bool,
    /// `sensors`: may read the device sensors (magnetometer, altimeter, motion, pedometer) through the SDK. The
    /// readings come from the shell; the manifest permission is what lets an app ask for them.
    pub sensors: bool,
    /// `contacts`: declares that the app reads and saves the person's contacts through the SDK contacts module. The
    /// manifest permission is the declaration; the contacts service does not gate calls on it yet.
    pub contacts: bool,
    /// `calendars`: declares that the app adds events to the person's calendar (and reads them, when the person allows)
    /// through the SDK calendar module. The declaration only lets the app ask: the calendar service answers nothing
    /// until the person has allowed this app on the access sheet or in Settings. It binds nothing.
    pub calendars: bool,
    /// `usb`: may ask the shell's `usb` service for raw access to a USB device. It binds nothing (no `/dev/bus/usb`
    /// reaches the sandbox); the person picks each device on a sheet, and the shell runs the transfers.
    pub usb: bool,
    /// `hid`: the same for a raw HID device (the shell reads and writes its hidraw node for the app). Keyboards and
    /// security keys are never offered.
    pub hid: bool,
    /// `gpu`: an SDK-drawn app whose engine renders offscreen on the GPU (the Browser's Servo engine) and reads the
    /// frames back into the SDK's shared-memory frames. The sandbox gains the render node, the sysfs paths the Mesa
    /// loader reads, `/run/udev` and the compositor socket (a hidden winit window needs the connection); the window is
    /// still presented from SDK frames. See [`gpu_args`].
    pub gpu: bool,
    /// `entitlements = ["log-store"]` on a `system-app`: the system journal is mounted read-only so
    /// `LogStore::local()` works (Console). Set through `with_entitlements`, never from `permissions`.
    pub log_store: bool,
    /// `entitlements = ["system-monitor"]` on a first-party `system-app`: host PID and network namespaces, host `/proc`
    /// read-only (see [`namespace_args`]). Set through `with_entitlements`, never from `permissions`.
    pub system_monitor: bool,
}
impl Policy {
    /// Applies manifest entitlements; each is honoured only for the package kind that may hold it.
    /// Entitlements are first-party only (`is.olibuijr.*`), like `remote-desktop`: a store package cannot grant
    /// itself the system journal by declaring `kind = "system-app"`.
    pub fn with_entitlements(mut self, id: &str, kind: &str, entitlements: &[String]) -> Self {
        let first_party_system = id.starts_with("is.olibuijr.") && kind == crate::shell::kinds::SYSTEM_APP;
        self.log_store = first_party_system && entitlements.iter().any(|e| e == bifrost_core::log::ENTITLEMENT);
        self.system_monitor = first_party_system && entitlements.iter().any(|e| e == SYSTEM_MONITOR_ENTITLEMENT);
        self
    }
    /// Whether the policy holds the named permission or entitlement, spelled as in the manifest. An unknown name is
    /// held by no one. The query reports what the manifest granted; each service still checks the policy on every call.
    pub fn holds(&self, name: &str) -> bool {
        match name {
            "network" => self.network,
            "storage" => self.storage,
            "audio" => self.audio,
            "camera" => self.camera,
            "screen" => self.screen,
            "notifications" => self.notifications,
            "documents" => self.documents,
            "keychain" => self.keychain,
            "accessibility" => self.accessibility,
            "display-config" => self.display_config,
            "sensors" => self.sensors,
            "contacts" => self.contacts,
            "calendars" => self.calendars,
            "usb" => self.usb,
            "hid" => self.hid,
            "gpu" => self.gpu,
            "remote-desktop" => self.remote,
            SYSTEM_MONITOR_ENTITLEMENT => self.system_monitor,
            other => other == bifrost_core::log::ENTITLEMENT && self.log_store,
        }
    }

    /// The names the policy holds, in [`ENTITLEMENT_NAMES`] order.
    pub fn held(&self) -> Vec<&'static str> {
        ENTITLEMENT_NAMES.into_iter().filter(|name| self.holds(name)).collect()
    }

    pub fn from_permissions(id: &str, permissions: &[String]) -> io::Result<Self> {
        let mut p = Self::default();
        for permission in permissions {
            match permission.as_str() {
                "network" => p.network = true,
                "storage" => p.storage = true,
                "audio" => p.audio = true,
                "camera" if id == CAMERA_APP_ID => p.camera = true,
                "screen" if id == SCREEN_APP_ID => p.screen = true,
                "notifications" => p.notifications = true,
                "documents" => p.documents = true,
                "keychain" => p.keychain = true,
                "accessibility" => p.accessibility = true,
                "display-config" => p.display_config = true,
                "sensors" => p.sensors = true,
                "contacts" => p.contacts = true,
                "calendars" => p.calendars = true,
                "usb" => p.usb = true,
                "hid" => p.hid = true,
                "gpu" => p.gpu = true,
                "remote-desktop" if id == "is.olibuijr.remote.desktop" => p.remote = true,
                _ => return Err(PolicyError::UnsupportedPermission(permission.clone()).into()),
            }
        }
        if p.remote && (!p.network || !p.storage) {
            return Err(PolicyError::RemoteNeedsNetworkAndStorage.into());
        }
        Ok(p)
    }
}
unsafe extern "C" {
    fn geteuid() -> u32;
    fn close_range(first:u32,last:u32,flags:i32)->i32;
    fn setgid(gid: u32) -> i32;
    fn setuid(uid: u32) -> i32;
    fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
}
fn drop_identity(cmd: &mut Command, account: &Account) -> io::Result<()> {
    let (uid, gid) = (account.uid, account.gid);
    // Resolve supplementary groups before fork. pre_exec only performs syscalls.
    let groups = if unsafe { geteuid() } == 0 {
        let text = fs::read_to_string("/etc/group")?;
        let mut groups = vec![gid];
        for line in text.lines() {
            let f: Vec<_> = line.split(':').collect();
            if f.len() == 4 && f[3].split(',').any(|u| u == account.name) {
                if let Ok(g) = f[2].parse::<u32>() {
                    if !groups.contains(&g) {
                        groups.push(g);
                    }
                }
            }
        }
        groups
    } else {
        Vec::new()
    };
    unsafe extern "C" {
        fn setgroups(size: usize, groups: *const u32) -> i32;
    }
    unsafe {
        cmd.pre_exec(move || {
            if geteuid() == 0 {
                if setgroups(groups.len(), groups.as_ptr()) != 0
                    || setgid(gid) != 0
                    || setuid(uid) != 0
                {
                    return Err(io::Error::last_os_error());
                }
            } else if geteuid() != uid {
                return Err(io::Error::other("Wrong app launch identity"));
            }
            // Close inherited device/service handles on exec. AppHost explicitly
            // restores only its authenticated SDK channel after this hook.
            if close_range(3,u32::MAX,4)!=0 {return Err(io::Error::last_os_error());}
            if prctl(38, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}
fn prepare_data(data: &Path, account: &Account) -> io::Result<()> {
    fs::create_dir_all(data)?;
    if fs::symlink_metadata(data)?.file_type().is_symlink() {
        return Err(PolicyError::DataIsSymlink.into());
    }
    if unsafe { geteuid() } == 0 {
        // GNU chown -P does not traverse symlinks. Only this app's data is migrated.
        let status = Command::new("/usr/bin/chown")
            .args(["-R", "-P", "--no-dereference", "--"])
            .arg(format!("{}:{}", account.uid, account.gid))
            .arg(data)
            .status()?;
        if !status.success() {
            return Err(PolicyError::DataOwnershipFailed.into());
        }
    }
    Ok(())
}
/// Prepares an app's data directory and creates the fixed container folders in it (`Documents`, `Library`,
/// `Library/Caches`, `tmp`; see `bifrost_core::container_layout`) before the ownership change, so the folders belong
/// to the app's account. A data directory from before the split can hold a file or a link with one of those names;
/// the folder step is skipped then, and the app keeps launching on the flat directory.
fn prepare_container(data: &Path, account: &Account) -> io::Result<()> {
    fs::create_dir_all(data)?;
    if fs::symlink_metadata(data)?.file_type().is_symlink() {
        return Err(PolicyError::DataIsSymlink.into());
    }
    let _ = bifrost_core::container_layout::ContainerLayout::new(data).ensure();
    prepare_data(data, account)
}
/// The bubblewrap namespace arguments for `policy`. Everything is unshared (`--unshare-all`: user, ipc, pid, net, uts,
/// cgroup) except for an app holding `system-monitor`: it keeps the host PID and network namespaces (bubblewrap has no
/// `--share-pid`, so the other namespaces are unshared one by one) to read every process and `ss` every socket.
/// User, ipc, uts and cgroup stay unshared, and `--cap-drop ALL` plus the app account still apply, so it can signal only
/// what that account owns.
fn namespace_args(policy: Policy) -> Vec<&'static str> {
    if policy.system_monitor {
        vec!["--unshare-user", "--unshare-ipc", "--unshare-uts", "--unshare-cgroup-try"]
    } else {
        vec!["--unshare-all", "--unshare-user"]
    }
}

/// How `/proc` is provided. A fresh procfs belongs to a new PID namespace, so an app in its own gets `--proc`; the host
/// PID namespace of `system-monitor` is read through a read-only bind of the host `/proc` (a user namespace cannot mount
/// procfs for a PID namespace it does not own).
fn proc_args(policy: Policy) -> Vec<&'static str> {
    if policy.system_monitor {
        vec!["--ro-bind", "/proc", "/proc"]
    } else {
        vec!["--proc", "/proc"]
    }
}

/// Construct a fail-closed sandbox. Only the selected package, app data and
/// declared resources are mounted; no system/session bus is exposed to apps.
pub fn command(
    id: &str,
    exec: &Path,
    data: &Path,
    policy: Policy,
    native: bool,
) -> io::Result<Command> {
    let form = bifrost_ui::form_factor::FormFactor::current();
    let account = Account::installed()?;
    prepare_container(data, &account)?;
    let exec = fs::canonicalize(exec)?;
    let package = exec
        .parent()
        .ok_or_else(|| io::Error::from(PolicyError::MissingPackageDirectory))?;
    let mut c = Command::new("/usr/bin/bwrap");
    c.args(["--die-with-parent", "--new-session"])
        .args(namespace_args(policy))
        .args([
        "--cap-drop",
        "ALL",
        "--uid",
    ])
    .arg(account.uid.to_string())
    .arg("--gid")
    .arg(account.gid.to_string());
    if policy.camera {
        for device in camera_capture_devices() {
            // Bind only detected V4L2 capture nodes; no camera directory, USB tree, or unrelated device is exposed.
            c.arg("--dev-bind").arg(&device).arg(&device);
        }
    }
    if policy.network {
        c.arg("--share-net");
    }
    if !native {
        c.arg("--disable-userns");
    }
    c.args(proc_args(policy));
    c.args([
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/etc",
        "/etc",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/bin",
        "/sbin",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib",
        "/lib64",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/run",
        "--dir",
        "/home",
    ]);
    c.arg("--ro-bind")
        .arg(package)
        .arg(package)
        .arg("--bind")
        .arg(data)
        .arg(data);
    if policy.storage {
        c.arg("--bind").arg(&account.home).arg(&account.home);
    } else {
        let private = data.join("home");
        fs::create_dir_all(&private)?;
        prepare_data(&private, &account)?;
        c.arg("--bind").arg(&private).arg(&account.home);
    }
    c.args([
        "--ro-bind",
        "/var/lib/springboard/account",
        "/var/lib/springboard/account",
    ]);
    if policy.audio {
        c.args([
            "--dev-bind-try",
            "/dev/snd",
            "/dev/snd",
            "--bind-try",
            "/var/lib/springboard/run/is.olibuijr.audio.sock",
            "/var/lib/springboard/run/is.olibuijr.audio.sock",
        ]);
    }
    // Unified Logging: every app may write to the journald native socket (world-writable datagram socket).
    c.args(["--ro-bind-try", "/run/systemd/journal/socket", "/run/systemd/journal/socket"]);
    if policy.log_store {
        // Console (`system-app` + `log-store`): read the journal. Group access to the journal files is not granted
        // here; the app account must be in `systemd-journal` (see docs/bifrost-ui.md, Unified Logging).
        c.args(["--ro-bind-try", "/var/log/journal", "/var/log/journal", "--ro-bind-try", "/run/log/journal", "/run/log/journal"]);
    }
    c.env_clear()
        .env("PATH", "/usr/bin")
        .env("HOME", &account.home)
        .env("USER", &account.name)
        .env("LOGNAME", &account.name)
        .env("LANG", "en_US.UTF-8")
        .env("BIFROST_APP_ID", id)
        .env(bifrost_ui::form_factor::ENV, form.as_str())
        .env("BIFROST_DATA", data)
        .env("BIFROST_SHELL_DIR", "/usr/share/springboard/assets");
    if policy.log_store {
        c.env("BIFROST_ENTITLEMENTS", bifrost_core::log::ENTITLEMENT);
    }
    // a desktop has physical keys; a tablet raises the on-screen keyboard for the app's text fields
    if form.is_desktop() {
        c.env("BIFROST_HARDWARE_KEYBOARD", "1");
    }
    if native || policy.gpu {
        let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(io::Error::other)?;
        let display = std::env::var("WAYLAND_DISPLAY").map_err(io::Error::other)?;
        let session = PathBuf::from(&runtime).join("session.json");
        c.args(gpu_args(Path::new(&runtime), &display, native, native && session.is_file())?);
        // Session bus (bifrost-portal: Light/Dark for GTK, Qt and winit apps); the socket is `$XDG_RUNTIME_DIR/bus`.
        let bus = PathBuf::from(&runtime).join("bus");
        if native && bus.exists() {
            c.arg("--bind-try").arg(&bus).arg(&bus);
            c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus.display()));
        }
        c.env("XDG_RUNTIME_DIR", runtime)
            .env("WAYLAND_DISPLAY", display)
            .env("WINIT_UNIX_BACKEND", "wayland");
    }
    // remembered locally so the shell can warm the apps likely to open next (core launch_prefetch)
    bifrost_core::launch_prefetch::note_launch(id, &exec);
    c.arg("--chdir")
        .arg(data)
        .arg("--")
        .arg(exec)
        .stdin(Stdio::null());
    crate::bare::app_manager::configure(&mut c, id)?;
    drop_identity(&mut c, &account)?;
    Ok(c)
}

/// The sandbox arguments that give a client the GPU and the compositor connection.
///
/// `native` (a `window = "native"` app) sees all of `/sys` read-only. An SDK app holding `gpu` gets only what an
/// offscreen GL or Vulkan client needs: the sysfs paths the Mesa loader reads to identify the render node
/// (`/sys/dev/char`, `/sys/devices`). Both get `/dev/dri`, `/run/udev` read-only and the Wayland socket. `session`
/// also mounts `session.json` read-only (native only).
fn gpu_args(runtime: &Path, display: &str, native: bool, session: bool) -> io::Result<Vec<OsString>> {
    if display.is_empty() || display.contains('/') {
        return Err(io::Error::other("Invalid compositor socket name"));
    }
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |parts: &[&OsStr]| args.extend(parts.iter().map(|p| (*p).to_os_string()));
    if native {
        push(&["--ro-bind".as_ref(), "/sys".as_ref(), "/sys".as_ref()]);
    } else {
        for path in ["/sys/dev/char", "/sys/devices", "/sys/bus/pci", "/sys/class/drm"] {
            push(&["--ro-bind-try".as_ref(), path.as_ref(), path.as_ref()]);
        }
    }
    push(&["--ro-bind-try".as_ref(), "/run/udev".as_ref(), "/run/udev".as_ref()]);
    if session {
        let session = runtime.join("session.json");
        push(&["--ro-bind".as_ref(), session.as_ref(), session.as_ref()]);
    }
    let socket = runtime.join(display);
    push(&["--bind".as_ref(), socket.as_ref(), socket.as_ref()]);
    push(&["--dev-bind-try".as_ref(), "/dev/dri".as_ref(), "/dev/dri".as_ref()]);
    Ok(args)
}

/// Enumerated video nodes that can stream one of the formats supported by Photo Booth.
/// Path and file-type checks keep the bind list limited to actual `/dev/videoN` character devices.
fn camera_capture_devices() -> Vec<PathBuf> {
    use std::os::unix::fs::FileTypeExt;

    crate::media::v4l2::probe()
        .into_iter()
        .filter_map(|camera| {
            let path = PathBuf::from(camera.path);
            if !is_video_node_path(&path) || !fs::symlink_metadata(&path).ok()?.file_type().is_char_device() {
                return None;
            }
            Some(path)
        })
        .collect()
}

fn is_video_node_path(path: &Path) -> bool {
    path.parent() == Some(Path::new("/dev"))
        && path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("video"))
            .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
}
/// Permission and identity come from the authenticated app's socket/installed
/// manifest, never from a claimed id or caller-supplied executable path.
pub fn remote_sharing(id: &str, policy: Policy, on: bool, package: &Path) -> io::Result<()> {
    if id != "is.olibuijr.remote.desktop" || !policy.remote {
        return Err(PolicyError::RemoteDenied.into());
    }
    let account = Account::installed()?;
    let mut c = if on {
        let server = fs::canonicalize(package.join("assets/server.py"))?;
        let package = fs::canonicalize(package)?;
        if !server.starts_with(&package) {
            return Err(PolicyError::RemoteServerEscapesPackage.into());
        }
        let mut c = Command::new("/usr/bin/systemd-run");
        c.args([
            "--unit=bifrost-remote",
            "--collect",
            "--property=Restart=on-failure",
            "--property=RestartSec=3s",
            "--property=StartLimitIntervalSec=0",
            "--property=NoNewPrivileges=yes",
            "--property=ProtectSystem=strict",
            "--property=PrivateTmp=yes",
        ]);
        c.arg(format!("--uid={}", account.uid))
            .arg(format!("--gid={}", account.gid))
            .arg(format!(
                "--property=ReadWritePaths={}",
                account.home.display()
            ));
        c.arg("/usr/bin/python")
            .arg(server)
            .arg(account.home.join(".local/share/bifrost-remote"));
        c
    } else {
        let mut c = Command::new("/usr/bin/systemctl");
        c.args(["stop", "bifrost-remote.service"]);
        c
    };
    c.stdin(Stdio::null()).stdout(Stdio::null());
    let status = c.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(PolicyError::RemoteServiceFailed.into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_and_display_permissions_are_recognised_and_bind_nothing() {
        let p = Policy::from_permissions("example", &["accessibility".into(), "display-config".into()]).unwrap();
        assert!(p.accessibility && p.display_config);
        assert!(!p.network && !p.storage && !p.remote);
    }

    #[test]
    fn usb_and_hid_permissions_are_recognised_and_bind_nothing() {
        let p = Policy::from_permissions("example", &["usb".into(), "hid".into()]).unwrap();
        assert!(p.usb && p.hid && p.holds("usb") && p.holds("hid"));
        assert!(!p.network && !p.storage && !p.camera && !p.remote);
        assert_eq!(p.held(), vec!["usb", "hid"]);
        let only_usb = Policy::from_permissions("example", &["usb".into()]).unwrap();
        assert!(!only_usb.holds("hid"), "the two permissions are separate");
    }

    #[test]
    fn sensors_permission_is_recognised_and_binds_nothing() {
        let p = Policy::from_permissions("example", &["sensors".into()]).unwrap();
        assert!(p.sensors && p.holds("sensors"));
        assert!(!p.network && !p.storage && !p.camera && !p.remote);
        assert!(p.held().contains(&"sensors"));
        assert!(!Policy::default().holds("sensors"));
    }

    #[test]
    fn contacts_permission_is_recognised_and_binds_nothing() {
        let p = Policy::from_permissions("example", &["contacts".into()]).unwrap();
        assert!(p.contacts && p.holds("contacts"));
        assert!(!p.network && !p.storage && !p.sensors && !p.remote);
        assert!(p.held().contains(&"contacts"));
        assert!(!Policy::default().holds("contacts"));
    }

    #[test]
    fn calendars_permission_is_recognised_and_binds_nothing() {
        let p = Policy::from_permissions("example", &["calendars".into()]).unwrap();
        assert!(p.calendars && p.holds("calendars"));
        assert!(!p.network && !p.storage && !p.contacts && !p.remote);
        assert!(p.held().contains(&"calendars"));
        assert!(!Policy::default().holds("calendars"));
    }

    #[test]
    fn gpu_permission_is_recognised() {
        let p = Policy::from_permissions("example", &["gpu".into()]).unwrap();
        assert!(p.gpu && p.holds("gpu") && p.held() == vec!["gpu"]);
        assert!(!Policy::default().holds("gpu"));
    }

    #[test]
    fn gpu_args_for_an_sdk_app_are_narrow() {
        let args: Vec<String> = gpu_args(Path::new("/run/user/1000"), "wayland-1", false, false)
            .unwrap()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("--dev-bind-try /dev/dri /dev/dri"));
        assert!(joined.contains("--ro-bind-try /sys/dev/char /sys/dev/char"));
        assert!(joined.contains("--ro-bind-try /sys/devices /sys/devices"));
        assert!(joined.contains("--ro-bind-try /run/udev /run/udev"));
        assert!(joined.contains("--bind /run/user/1000/wayland-1 /run/user/1000/wayland-1"));
        assert!(!joined.contains("--ro-bind /sys /sys"), "SDK apps do not get all of /sys");
        assert!(!joined.contains("session.json"));
    }

    #[test]
    fn gpu_args_for_a_native_app_keep_all_of_sys_and_the_session() {
        let args = gpu_args(Path::new("/run/user/1000"), "wayland-1", true, true).unwrap();
        let joined = args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        assert!(joined.starts_with("--ro-bind /sys /sys --ro-bind-try /run/udev /run/udev"));
        assert!(joined.contains("--ro-bind /run/user/1000/session.json /run/user/1000/session.json"));
        assert!(joined.ends_with("--dev-bind-try /dev/dri /dev/dri"));
        assert!(gpu_args(Path::new("/run/user/1000"), "../x", true, false).is_err());
    }

    #[test]
    fn isolation_permissions_fail_closed() {
        assert!(Policy::from_permissions("example", &["system-update".into()]).is_err());
        assert!(Policy::from_permissions("example", &["camera".into()]).is_err());
        assert!(Policy::from_permissions("example", &["screen".into()]).is_err());
        assert!(Policy::from_permissions(CAMERA_APP_ID, &["screen".into()]).is_err());
        assert!(Policy::from_permissions(SCREEN_APP_ID, &["camera".into()]).is_err());
        let camera = Policy::from_permissions(CAMERA_APP_ID, &["camera".into()]).unwrap();
        assert!(camera.camera && !camera.screen);
        let screen = Policy::from_permissions(SCREEN_APP_ID, &["screen".into()]).unwrap();
        assert!(screen.screen && !screen.camera);
        assert!(Policy::from_permissions("example", &["remote-desktop".into()]).is_err());
        assert!(
            Policy::from_permissions("is.olibuijr.remote.desktop", &["remote-desktop".into()])
                .is_err()
        );
        let p = Policy::from_permissions("example", &["storage".into()]).unwrap();
        assert!(p.storage);
        assert!(!p.network);
        assert!(!p.remote);
        assert!(remote_sharing("example", p, true, Path::new("/missing")).is_err());
        // `documents` binds nothing: it is the picker grant, not the home
        let d = Policy::from_permissions("example", &["documents".into()]).unwrap();
        assert!(d.documents && !d.storage && !d.network);
    }

    #[test]
    fn camera_mount_paths_are_video_nodes_only() {
        assert!(is_video_node_path(Path::new("/dev/video0")));
        assert!(is_video_node_path(Path::new("/dev/video12")));
        for path in ["/dev/video", "/dev/video1x", "/dev/video2/extra", "/dev/v4l/video0", "/dev/dri/card0"] {
            assert!(!is_video_node_path(Path::new(path)), "unexpected camera mount path: {path}");
        }
    }
    #[test]
    fn a_plain_app_unshares_every_namespace_and_gets_its_own_proc() {
        let args = namespace_args(Policy::default());
        assert_eq!(args, ["--unshare-all", "--unshare-user"]);
        assert_eq!(proc_args(Policy::default()), ["--proc", "/proc"]);
    }

    #[test]
    fn system_monitor_shares_host_pid_and_net_and_reads_proc_read_only() {
        let policy = Policy::default().with_entitlements("is.olibuijr.activity.desktop", "system-app", &[SYSTEM_MONITOR_ENTITLEMENT.to_string()]);
        assert!(policy.system_monitor && policy.holds("system-monitor") && policy.held() == vec!["system-monitor"]);
        let args = namespace_args(policy);
        assert!(!args.contains(&"--unshare-all") && !args.contains(&"--unshare-pid") && !args.contains(&"--unshare-net"));
        for kept in ["--unshare-user", "--unshare-ipc", "--unshare-uts", "--unshare-cgroup-try"] {
            assert!(args.contains(&kept), "{kept} stays unshared");
        }
        let proc = proc_args(policy);
        assert_eq!(proc, ["--ro-bind", "/proc", "/proc"], "host /proc, read-only");
    }

    #[test]
    fn system_monitor_is_held_only_by_a_first_party_system_app_that_declares_it() {
        let declared = [SYSTEM_MONITOR_ENTITLEMENT.to_string()];
        let base = Policy::default();
        assert!(base.with_entitlements("is.olibuijr.activity.desktop", "system-app", &declared).system_monitor);
        assert!(!base.with_entitlements("com.example.top", "system-app", &declared).system_monitor, "a store id cannot take it");
        assert!(!base.with_entitlements("is.olibuijr.activity.desktop", "app", &declared).system_monitor, "an ordinary kind cannot take it");
        assert!(!base.with_entitlements("is.olibuijr.activity.desktop", "system-app", &[]).system_monitor, "it must be declared");
        assert!(Policy::from_permissions("is.olibuijr.activity.desktop", &[SYSTEM_MONITOR_ENTITLEMENT.to_string()]).is_err(), "never a permission");
        assert_eq!(namespace_args(base.with_entitlements("com.example.top", "system-app", &declared)), ["--unshare-all", "--unshare-user"]);
    }

    #[test]
    fn isolation_root_accounts_rejected() {
        assert!(Account::parse("root", "root:x:0:0:root:/root:/bin/bash").is_err());
        assert!(Account::parse("bad/name", "").is_err());
        assert_eq!(
            Account::parse("tester", "tester:x:1000:1000::/home/tester:/bin/bash")
                .unwrap()
                .uid,
            1000
        );
    }
}
