//! Desktop app launch boundary. The shell chooses identity and resources; apps
//! never launch with root credentials or select privileged command arguments.
use std::{
    fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub use super::account::Account;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub network: bool,
    pub storage: bool,
    pub audio: bool,
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
    /// `entitlements = ["log-store"]` on a `system-app`: the system journal is mounted read-only so
    /// `LogStore::local()` works (Console). Set through `with_entitlements`, never from `permissions`.
    pub log_store: bool,
}
impl Policy {
    /// Applies manifest entitlements; each is honoured only for the package kind that may hold it.
    /// Entitlements are first-party only (`is.olibuijr.*`), like `remote-desktop`: a store package cannot grant
    /// itself the system journal by declaring `kind = "system-app"`.
    pub fn with_entitlements(mut self, id: &str, kind: &str, entitlements: &[String]) -> Self {
        self.log_store = id.starts_with("is.olibuijr.") && kind == crate::shell::kinds::SYSTEM_APP && entitlements.iter().any(|e| e == bifrost_core::log::ENTITLEMENT);
        self
    }
    pub fn from_permissions(id: &str, permissions: &[String]) -> io::Result<Self> {
        let mut p = Self::default();
        for permission in permissions {
            match permission.as_str() {
                "network" => p.network = true,
                "storage" => p.storage = true,
                "audio" => p.audio = true,
                "notifications" => p.notifications = true,
                "documents" => p.documents = true,
                "keychain" => p.keychain = true,
                "accessibility" => p.accessibility = true,
                "display-config" => p.display_config = true,
                "remote-desktop" if id == "is.olibuijr.remote.desktop" => p.remote = true,
                _ => {
                    return Err(io::Error::other(format!(
                        "Unsupported app permission: {permission}"
                    )))
                }
            }
        }
        if p.remote && (!p.network || !p.storage) {
            return Err(io::Error::other(
                "Remote sharing requires network and storage",
            ));
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
        return Err(io::Error::other("App data must not be a symlink"));
    }
    if unsafe { geteuid() } == 0 {
        // GNU chown -P does not traverse symlinks. Only this app's data is migrated.
        let status = Command::new("/usr/bin/chown")
            .args(["-R", "-P", "--no-dereference", "--"])
            .arg(format!("{}:{}", account.uid, account.gid))
            .arg(data)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("Cannot assign app data ownership"));
        }
    }
    Ok(())
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
    prepare_data(data, &account)?;
    let exec = fs::canonicalize(exec)?;
    let package = exec
        .parent()
        .ok_or_else(|| io::Error::other("Missing app directory"))?;
    let mut c = Command::new("/usr/bin/bwrap");
    c.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--unshare-user",
        "--cap-drop",
        "ALL",
        "--uid",
    ])
    .arg(account.uid.to_string())
    .arg("--gid")
    .arg(account.gid.to_string());
    if policy.network {
        c.arg("--share-net");
    }
    if !native {
        c.arg("--disable-userns");
    }
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
        "--proc",
        "/proc",
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
    if native {
        c.args(["--ro-bind", "/sys", "/sys", "--ro-bind-try", "/run/udev", "/run/udev"]);
        let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(io::Error::other)?;
        let display = std::env::var("WAYLAND_DISPLAY").map_err(io::Error::other)?;
        if display.contains('/') {
            return Err(io::Error::other("Invalid compositor socket name"));
        }
        let socket = PathBuf::from(&runtime).join(&display);
        let session = PathBuf::from(&runtime).join("session.json");
        if session.is_file() {
            c.arg("--ro-bind").arg(&session).arg(&session);
        }
        c.arg("--bind")
            .arg(&socket)
            .arg(&socket)
            .args(["--dev-bind-try", "/dev/dri", "/dev/dri"]);
        // Session bus (bifrost-portal: Light/Dark for GTK, Qt and winit apps); the socket is `$XDG_RUNTIME_DIR/bus`.
        let bus = PathBuf::from(&runtime).join("bus");
        if bus.exists() {
            c.arg("--bind-try").arg(&bus).arg(&bus);
            c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus.display()));
        }
        c.env("XDG_RUNTIME_DIR", runtime)
            .env("WAYLAND_DISPLAY", display)
            .env("WINIT_UNIX_BACKEND", "wayland");
    }
    c.arg("--chdir")
        .arg(data)
        .arg("--")
        .arg(exec)
        .stdin(Stdio::null());
    crate::bare::app_manager::configure(&mut c, id)?;
    drop_identity(&mut c, &account)?;
    Ok(c)
}
/// Permission and identity come from the authenticated app's socket/installed
/// manifest, never from a claimed id or caller-supplied executable path.
pub fn remote_sharing(id: &str, policy: Policy, on: bool, package: &Path) -> io::Result<()> {
    if id != "is.olibuijr.remote.desktop" || !policy.remote {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Remote sharing permission denied",
        ));
    }
    let account = Account::installed()?;
    let mut c = if on {
        let server = fs::canonicalize(package.join("assets/server.py"))?;
        let package = fs::canonicalize(package)?;
        if !server.starts_with(&package) {
            return Err(io::Error::other("Remote server escapes package"));
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
        Err(io::Error::other("Remote service request failed"))
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
    fn isolation_permissions_fail_closed() {
        assert!(Policy::from_permissions("example", &["system-update".into()]).is_err());
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
