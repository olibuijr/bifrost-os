//! The root methods. Fixed argv, validated parameters, paths canonicalised against allow-listed roots.
//! `RootExec` is also what `privd_client::Local` runs when the shell is itself root (host tests, installer).

use super::server::Exec;
use super::{obj, s, PErr, STATE_DIR};
use crate::shell::{pacman_update, store::install};
use bifrost_core::json::Json;
use std::path::Path;
use std::process::Command;

extern "C" {
    fn sync();
}

pub struct RootExec;

impl Exec for RootExec {
    fn call(&self, method: &str, p: &Json) -> Result<Json, PErr> {
        call(method, p)
    }
}

fn str_param<'a>(p: &'a Json, name: &str) -> Result<&'a str, PErr> {
    p.get(name).and_then(Json::as_str).ok_or_else(|| PErr::bad(format!("missing {name}")))
}

fn run(prog: &str, args: &[&str]) -> Result<(), PErr> {
    let out = Command::new(prog).args(args).output().map_err(|e| PErr::internal(format!("{prog}: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(PErr::internal(format!("{prog} {}: {}", args.join(" "), err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim())))
}

pub fn call(method: &str, p: &Json) -> Result<Json, PErr> {
    match method {
        "system.ping" => Ok(obj(vec![("version", s(env!("CARGO_PKG_VERSION"))), ("proto", Json::Num(super::PROTO as f64))])),
        "power.shutdown" => power("poweroff"),
        "power.restart" => power("reboot"),
        "power.sleep" => power("suspend"),
        "power.wake_alarm" => wake_alarm(p),
        "time.set_timezone" => set_timezone(str_param(p, "name")?),
        "time.set_ntp" => match p.get("on") {
            Some(Json::Bool(on)) => run("timedatectl", &["set-ntp", if *on { "true" } else { "false" }]).map(|_| obj(vec![])),
            _ => Err(PErr::bad("on must be a boolean")),
        },
        "storage.policy_set" => match p.get("empty_trash_after_30_days") {
            Some(Json::Bool(on)) => storage_policy_set(*on),
            _ => Err(PErr::bad("empty_trash_after_30_days must be a boolean")),
        },
        "network.dns_set" => network_dns_set(p),
        "dhcp6.serve" => dhcp6_serve(p),
        "startup_disk.list" => startup_disk_list(),
        "startup_disk.set" => startup_disk_set(p),
        "locale.set" => set_locale(str_param(p, "lang")?, str_param(p, "region")?),
        "update.check" => update_check(),
        "update.apply" => {
            let kind = str_param(p, "kind")?;
            if !matches!(kind, "os" | "shell") {
                return Err(PErr::bad("kind must be os or shell"));
            }
            // the guard writes the recovery state; the shell exits 3 afterwards
            pacman_update::install_local().map(|_| obj(vec![])).map_err(PErr::internal)
        }
        "update.rollback" => pacman_update::rollback_local().map(|_| obj(vec![])).map_err(PErr::internal),
        "update.status" => {
            let out = Command::new("/usr/bin/bifrost-update-guard").arg("status").output().map_err(|e| PErr::internal(e.to_string()))?;
            Ok(obj(vec![("status", s(String::from_utf8_lossy(&out.stdout).trim()))]))
        }
        "store.install" => store_install(p),
        "package.remove" => package_remove(str_param(p, "name")?),
        "package.install_file" => package_install_file(str_param(p, "path")?),
        "package.install_repo" => package_install_repo(p.get("names")),
        bifrost_core::usb_approval::M_USB_AUTHORIZE => usb_authorize(Path::new(bifrost_core::usb_approval::SYSFS_USB_DEVICES), str_param(p, "port")?, p.get("allow")),
        "store.uninstall" => {
            let id = str_param(p, "id")?;
            if !install::valid_id(id) {
                return Err(PErr::bad("invalid app id"));
            }
            install::uninstall_local(Path::new(STATE_DIR), id).map(|_| obj(vec![])).map_err(PErr::internal)
        }
        "system.boot_attempt" => boot_attempt(str_param(p, "event")?),
        "disk.loop_attach" => {
            let read_only = matches!(p.get("read_only"), Some(Json::Bool(true)));
            super::loopdev::attach(str_param(p, "path")?, read_only)
        }
        "disk.loop_detach" => super::loopdev::detach(str_param(p, "device")?),
        "backup.restore_apply" => match crate::shell::backup::apply_pending(Path::new(STATE_DIR)) {
            None => Ok(obj(vec![("files", Json::Num(0.0))])),
            Some(Ok(n)) => Ok(obj(vec![("files", Json::Num(n as f64))])),
            Some(Err(e)) => Err(PErr::internal(e.to_string())),
        },
        m if super::disk::handles(m) => super::disk::call(m, p),
        m if super::endpoint_security::handles(m) => super::endpoint_security::call(m, p),
        _ => super::accounts::call(method, p).unwrap_or_else(|| Err(PErr::bad(format!("unknown method {method}")))),
    }
}

/// Removes one package with pacman. The shell confirms the removal with the user first; privd checks the name and
/// refuses the shell and the base system (`pacman_update::remove_args`) before pacman runs.
fn package_remove(name: &str) -> Result<Json, PErr> {
    let args = pacman_update::remove_args(name).map_err(PErr::bad)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("pacman", &args).map(|_| obj(vec![]))
}

/// `usb.authorize {port, allow}`: writes a device's `authorized` attribute for the shell's accessory approval. Only a
/// device port (`1-2`, `1-2.4`) is written: a root hub (`usbN`) carries every device below it, and anything that is not a
/// port name is refused before a path is built (`usb_approval::authorized_path`).
fn usb_authorize(devices_root: &Path, port: &str, allow: Option<&Json>) -> Result<Json, PErr> {
    use bifrost_core::usb_approval::{enforce, Decision};
    let Some(Json::Bool(allow)) = allow else { return Err(PErr::bad("allow must be a boolean")) };
    if port.starts_with("usb") {
        return Err(PErr::bad("not a USB device port"));
    }
    let decision = if *allow { Decision::Allow } else { Decision::Deny };
    match enforce(devices_root, port, decision) {
        Ok(_) => Ok(obj(vec![])),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Err(PErr::bad("not a USB device port")),
        Err(e) => Err(PErr::internal(format!("usb {port}: {e}"))),
    }
}

/// The largest package archive privd copies into staging, in bytes.
const MAX_PACKAGE_BYTES: u64 = 4 << 30;

/// Installs the package archive at `path` with `pacman -U`. The shell asks the person first; privd checks the name like a
/// package archive, opens the file without following a link at the end, and copies it into a folder only root can write,
/// so pacman installs the bytes that were checked and not a path the user can swap meanwhile.
fn package_install_file(path: &str) -> Result<Json, PErr> {
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let src_path = Path::new(path);
    if !src_path.is_absolute() || src_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(PErr::bad("the package path must be absolute"));
    }
    let file = src_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if !pacman_update::is_package_file(file) {
        return Err(PErr::bad("not a package file"));
    }
    let mut src = std::fs::OpenOptions::new().read(true).custom_flags(O_NOFOLLOW).open(src_path).map_err(|e| PErr::bad(format!("{file}: {e}")))?;
    let meta = src.metadata().map_err(|e| PErr::internal(e.to_string()))?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_PACKAGE_BYTES {
        return Err(PErr::bad("not a usable package file"));
    }
    let staging = Path::new(STATE_DIR).join("staging");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&staging).map_err(|e| PErr::internal(format!("staging: {e}")))?;
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).map_err(|e| PErr::internal(format!("staging: {e}")))?;
    let staged = staging.join(file);
    let copied = (|| -> std::io::Result<()> {
        let mut dst = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).custom_flags(O_NOFOLLOW).open(&staged)?;
        std::io::copy(&mut (&mut src).take(MAX_PACKAGE_BYTES), &mut dst)?;
        dst.sync_all()
    })();
    let result = match copied {
        Ok(()) => {
            let args = pacman_update::install_file_args(&staged.to_string_lossy());
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run("pacman", &args).map(|_| obj(vec![]))
        }
        Err(e) => Err(PErr::internal(format!("staging {file}: {e}"))),
    };
    let _ = std::fs::remove_file(&staged);
    result
}

/// Installs packages from the sync databases with `pacman -S --needed`. The names are checked like a removal's.
fn package_install_repo(names: Option<&Json>) -> Result<Json, PErr> {
    let names: Vec<String> = match names {
        Some(Json::Arr(items)) if items.iter().all(|j| j.as_str().is_some()) => items.iter().filter_map(|j| j.as_str().map(str::to_string)).collect(),
        _ => return Err(PErr::bad("names must be a list of strings")),
    };
    let args = pacman_update::install_repo_args(&names).map_err(PErr::bad)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("pacman", &args).map(|_| obj(vec![]))
}

fn power(verb: &str) -> Result<Json, PErr> {
    if verb != "suspend" {
        // SAFETY: sync takes no arguments.
        unsafe { sync() };
    }
    run("systemctl", &[verb]).map(|_| obj(vec![]))
}

/// Arms the real-time clock to wake the machine at `at` (Unix seconds), or clears the alarm when `at` is absent or 0.
/// The time is checked against the clock first (`bifrost_core::wake_schedule::validate_alarm`). The kernel refuses a new
/// value while an old one is armed, so the old alarm is cleared before the new one is written.
fn wake_alarm(p: &Json) -> Result<Json, PErr> {
    use bifrost_core::wake_schedule::{alarm_value, validate_alarm, CLEAR_ALARM, RTC_WAKEALARM};
    let write = |text: &str| std::fs::write(RTC_WAKEALARM, text).map_err(|e| PErr::internal(format!("{RTC_WAKEALARM}: {e}")));
    let at = match p.get("at") {
        None | Some(Json::Null) => 0,
        Some(Json::Num(n)) if n.is_finite() && n.fract() == 0.0 => *n as i64,
        _ => return Err(PErr::bad("at must be a whole number of seconds")),
    };
    let checked = if at == 0 {
        None
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Some(validate_alarm(at, now).map_err(|e| PErr::bad(format!("{e:?}")))?)
    };
    write(CLEAR_ALARM)?;
    if let Some(at) = checked {
        write(&alarm_value(at))?;
    }
    Ok(obj(vec![]))
}

/// A zone name that is a plain relative path to a regular file under /usr/share/zoneinfo.
pub fn valid_zone(name: &str) -> bool {
    let chars = !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-' | b'/'));
    if !chars || name.starts_with('/') || name.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return false;
    }
    let root = Path::new("/usr/share/zoneinfo");
    match (root.join(name).canonicalize(), root.canonicalize()) {
        (Ok(f), Ok(r)) => f.starts_with(r) && f.is_file(),
        _ => false,
    }
}

fn set_timezone(name: &str) -> Result<Json, PErr> {
    if !valid_zone(name) {
        return Err(PErr::bad("unknown time zone"));
    }
    run("timedatectl", &["set-timezone", name]).map(|_| obj(vec![]))
}

/// `ll` or `ll_CC`: two lowercase letters, optionally an underscore and two uppercase ones.
pub fn locale_shape(name: &str) -> bool {
    let b = name.as_bytes();
    match b.len() {
        2 => b.iter().all(u8::is_ascii_lowercase),
        5 => b[..2].iter().all(u8::is_ascii_lowercase) && b[2] == b'_' && b[3..].iter().all(u8::is_ascii_uppercase),
        _ => false,
    }
}

/// A locale that is a shape `locale_shape` accepts and whose glibc source is installed (so it can be generated).
pub fn valid_locale(name: &str) -> bool {
    locale_shape(name) && Path::new("/usr/share/i18n/locales").join(name).is_file()
}

/// `localectl set-locale`: `lang` for LANG and `region` for the formats (time, numbers, money, paper, measures).
fn set_locale(lang: &str, region: &str) -> Result<Json, PErr> {
    if !valid_locale(lang) || !valid_locale(region) {
        return Err(PErr::bad("unknown locale"));
    }
    let (l, r) = (format!("{lang}.UTF-8"), format!("{region}.UTF-8"));
    let mut assigns = vec![format!("LANG={l}")];
    assigns.extend(["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER"].iter().map(|k| format!("{k}={r}")));
    let mut args = vec!["set-locale"];
    args.extend(assigns.iter().map(String::as_str));
    run("localectl", &args).map(|_| obj(vec![]))
}

fn update_check() -> Result<Json, PErr> {
    let f = pacman_update::check_local().map_err(PErr::internal)?;
    let opt = |v: Option<String>| v.map(Json::Str).unwrap_or(Json::Null);
    Ok(obj(vec![
        ("installed", opt(f.installed)),
        ("version", opt(f.update.as_ref().map(|u| u.version.clone()))),
        ("size", Json::Num(f.update.as_ref().map_or(0, |u| u.size) as f64)),
        ("notes", opt(f.update.and_then(|u| u.notes))),
    ]))
}

/// `store.install {id}`: the only parameter is the app id. privd downloads the catalogue itself and installs
/// the catalogued package against the catalogue's SHA-256, so nothing the user can write (a staged file, a
/// client-supplied hash) is ever opened or trusted by root. Package extraction rejects `..`, absolute paths
/// and every non-file entry (symlinks, hard links, devices), see `install::read_tar`.
fn store_install(p: &Json) -> Result<Json, PErr> {
    let id = str_param(p, "id")?;
    if !install::valid_id(id) {
        return Err(PErr::bad("invalid app id"));
    }
    let app = install::install_store_app(Path::new(STATE_DIR), id).map_err(PErr::internal)?;
    if app.id != id {
        let _ = install::uninstall_local(Path::new(STATE_DIR), &app.id);
        return Err(PErr::bad("package identity mismatch"));
    }
    Ok(obj(vec![("id", s(app.id)), ("version", s(app.version)), ("name", s(app.name))]))
}

#[cfg(target_arch = "x86_64")]
pub(super) const O_NOFOLLOW: i32 = 0o400000;
#[cfg(not(target_arch = "x86_64"))]
pub(super) const O_NOFOLLOW: i32 = 0o100000;

/// Writes `text` to `file` without following a symlink at the final component (root writes here).
/// The Storage policy file that bifrostd's purge reads (`purge::POLICY_PATH`, in the bifrostd crate).
pub const STORAGE_POLICY_FILE: &str = "/etc/bifrost/storage.conf";

/// The text of the Storage policy file: whether the Trash empties itself after 30 days.
pub fn storage_policy_text(empty_trash: bool) -> String {
    format!("empty_trash_after_30_days = {empty_trash}\n")
}

/// Writes the Storage policy (`storage.policy_set`) as root, no-follow, so a link planted in `/etc/bifrost` is refused.
fn storage_policy_set(empty_trash: bool) -> Result<Json, PErr> {
    let dir = Path::new(STORAGE_POLICY_FILE).parent().unwrap_or(Path::new("/etc/bifrost"));
    std::fs::create_dir_all(dir).map_err(|e| PErr::internal(e.to_string()))?;
    write_nofollow(STORAGE_POLICY_FILE, &storage_policy_text(empty_trash)).map_err(|e| PErr::internal(e.to_string()))?;
    Ok(obj(vec![("empty_trash_after_30_days", Json::Bool(empty_trash))]))
}

pub fn write_nofollow(file: &str, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).custom_flags(O_NOFOLLOW).open(file)?;
    f.write_all(text.as_bytes())
}

/// The crash-loop latch (`boot_attempts`), owned by root so the user session cannot clear it.
fn boot_attempt(event: &str) -> Result<Json, PErr> {
    use crate::bare::takeover;
    let file = format!("{STATE_DIR}/boot_attempts");
    match event {
        "begin" => {
            let age = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
            let failed = takeover::current_attempts(&std::fs::read_to_string(&file).unwrap_or_default(), age);
            match takeover::decide(failed) {
                takeover::Boot::CrashLoop(n) => Ok(obj(vec![("attempt", Json::Num(n as f64)), ("crash_loop", Json::Bool(true))])),
                takeover::Boot::TakeOver(n) => {
                    write_nofollow(&file, &n.to_string()).map_err(|e| PErr::internal(e.to_string()))?;
                    Ok(obj(vec![("attempt", Json::Num(n as f64)), ("crash_loop", Json::Bool(false))]))
                }
            }
        }
        "stable" => {
            write_nofollow(&file, "0").map_err(|e| PErr::internal(e.to_string()))?;
            Ok(obj(vec![("attempt", Json::Num(0.0)), ("crash_loop", Json::Bool(false))]))
        }
        _ => Err(PErr::bad("event must be begin or stable")),
    }
}

/// The DNS servers the user set in Settings > Network (`network.dns_set`), kept as a systemd-resolved drop-in.
pub const DNS_PREFS_FILE: &str = "/etc/systemd/resolved.conf.d/10-bifrost-dns.conf";

/// The most servers one list may carry.
const MAX_DNS_SERVERS: usize = 4;

/// True for a literal IPv4 or IPv6 address: no name, no port, no zone.
pub fn valid_dns_server(addr: &str) -> bool {
    addr.parse::<std::net::IpAddr>().is_ok()
}

/// The drop-in text: a `[Resolve]` section with one `DNS=` line, the servers in the order given.
pub fn dns_text(servers: &[String]) -> String {
    format!("[Resolve]\nDNS={}\n", servers.join(" "))
}

/// Writes the DNS servers (`network.dns_set`, a list of addresses; an empty list removes the drop-in) and has
/// systemd-resolved reload them. Every address is checked before any file is touched.
/// Starts or stops the DHCPv6 server on one interface (`dhcp6.serve`): `interface`, `on`, and for `on` the segment's
/// `prefix` (a /64 network address) and optional `dns` list. The settings are checked by `bifrost_core::dhcp6_server`
/// before anything is written; the configuration goes under `/run/bifrost/dhcp6` and `bifrost-dhcp6@<interface>` runs it.
fn dhcp6_serve(p: &Json) -> Result<Json, PErr> {
    use bifrost_core::dhcp6_server::{valid_interface, Dhcp6Server, RUN_DIR};
    let interface = str_param(p, "interface")?;
    if !valid_interface(interface) {
        return Err(PErr::bad("interface is not a valid interface name"));
    }
    let on = match p.get("on") {
        Some(Json::Bool(b)) => *b,
        _ => return Err(PErr::bad("on must be a boolean")),
    };
    let unit = format!("bifrost-dhcp6@{interface}.service");
    if !on {
        run("systemctl", &["stop", &unit])?;
        match std::fs::remove_file(format!("{RUN_DIR}/{interface}.conf")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(PErr::internal(e.to_string())),
        }
        return Ok(obj(vec![("interface", s(interface)), ("on", Json::Bool(false))]));
    }
    let prefix = str_param(p, "prefix")?.parse().map_err(|_| PErr::bad("prefix must be an IPv6 address"))?;
    let mut server = Dhcp6Server::new(interface, prefix).map_err(|e| PErr::bad(format!("{e:?}")))?;
    if let Some(list) = p.get("dns") {
        let items = list.as_array().ok_or_else(|| PErr::bad("dns must be a list"))?;
        for item in items {
            let addr = item.as_str().and_then(|a| a.parse().ok()).ok_or_else(|| PErr::bad("each dns entry must be an IPv6 address"))?;
            server.dns.push(addr);
        }
    }
    let conf = server.dnsmasq_conf().map_err(|e| PErr::bad(format!("{e:?}")))?;
    std::fs::create_dir_all(RUN_DIR).map_err(|e| PErr::internal(e.to_string()))?;
    write_nofollow(&server.conf_path(), &conf).map_err(|e| PErr::internal(e.to_string()))?;
    run("systemctl", &["restart", &unit])?;
    Ok(obj(vec![("interface", s(interface)), ("on", Json::Bool(true))]))
}

/// The firmware boot table, from `efibootmgr` (`startup_disk.list`): the entries in boot order with the current, next and
/// default one marked.
fn startup_disk_table() -> Result<bifrost_core::startup_disk::BootTable, PErr> {
    let out = Command::new("efibootmgr").output().map_err(|e| PErr::internal(format!("efibootmgr: {e}")))?;
    if !out.status.success() {
        return Err(PErr::internal("efibootmgr failed (is this a UEFI boot?)"));
    }
    Ok(bifrost_core::startup_disk::BootTable::parse(&String::from_utf8_lossy(&out.stdout)))
}

fn startup_disk_list() -> Result<Json, PErr> {
    let t = startup_disk_table()?;
    let default = t.default_entry().map(|e| e.num);
    let rows = t
        .in_boot_order()
        .into_iter()
        .map(|e| {
            obj(vec![
                ("id", s(e.id())),
                ("label", s(e.label.as_str())),
                ("active", Json::Bool(e.active)),
                ("current", Json::Bool(t.current == Some(e.num))),
                ("next", Json::Bool(t.next == Some(e.num))),
                ("default", Json::Bool(default == Some(e.num))),
            ])
        })
        .collect();
    Ok(obj(vec![("entries", Json::Arr(rows))]))
}

/// Makes a listed, active entry the default start, or the next start only with `once` (`startup_disk.set`).
fn startup_disk_set(p: &Json) -> Result<Json, PErr> {
    let id = str_param(p, "id")?;
    let once = match p.get("once") {
        None => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(PErr::bad("once must be a boolean")),
    };
    if bifrost_core::startup_disk::BootTable::default().boot_once_argv(id) == Err(bifrost_core::startup_disk::ChooseError::BadNumber) {
        return Err(PErr::bad("id must be four hex digits"));
    }
    let table = startup_disk_table()?;
    let argv = if once { table.boot_once_argv(id) } else { table.set_default_argv(id) }.map_err(|e| PErr::bad(format!("{e:?}")))?;
    let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    run(&argv[0], &args)?;
    Ok(obj(vec![("id", s(id.to_ascii_uppercase())), ("once", Json::Bool(once))]))
}

fn network_dns_set(p: &Json) -> Result<Json, PErr> {
    let items = p.get("servers").and_then(Json::as_array).ok_or_else(|| PErr::bad("servers must be a list"))?;
    let mut servers = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(addr) if valid_dns_server(addr) => servers.push(addr.to_string()),
            _ => return Err(PErr::bad("each server must be an IPv4 or IPv6 address")),
        }
    }
    if servers.len() > MAX_DNS_SERVERS {
        return Err(PErr::bad(format!("at most {MAX_DNS_SERVERS} servers")));
    }
    if servers.is_empty() {
        match std::fs::remove_file(DNS_PREFS_FILE) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(PErr::internal(e.to_string())),
        }
    } else {
        let dir = Path::new(DNS_PREFS_FILE).parent().unwrap_or(Path::new("/etc/systemd/resolved.conf.d"));
        std::fs::create_dir_all(dir).map_err(|e| PErr::internal(e.to_string()))?;
        write_nofollow(DNS_PREFS_FILE, &dns_text(&servers)).map_err(|e| PErr::internal(e.to_string()))?;
    }
    run("systemctl", &["try-reload-or-restart", "systemd-resolved"])?;
    Ok(obj(vec![("servers", Json::Arr(servers.into_iter().map(s).collect()))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_installs_refuse_bad_paths_and_names_before_anything_runs() {
        let file = |p: &str| call("package.install_file", &obj(vec![("path", s(p))]));
        for bad in ["relative/a-1-1-any.pkg.tar.zst", "/tmp/../etc/a-1-1-any.pkg.tar.zst", "/tmp/notes.txt", "/tmp/a-1-1-any.tar.gz", ""] {
            assert_eq!(file(bad).unwrap_err().code, "bad_params", "{bad}");
        }
        assert_eq!(file("/nonexistent-dir/a-1-1-any.pkg.tar.zst").unwrap_err().code, "bad_params", "a file that cannot be opened");
        assert_eq!(call("package.install_file", &obj(vec![])).unwrap_err().code, "bad_params");
        let repo = |names: Json| call("package.install_repo", &obj(vec![("names", names)]));
        assert_eq!(repo(Json::Arr(vec![s("--overwrite=*")])).unwrap_err().code, "bad_params");
        assert_eq!(repo(Json::Arr(vec![])).unwrap_err().code, "bad_params");
        assert_eq!(repo(Json::Arr(vec![Json::Bool(true)])).unwrap_err().code, "bad_params");
        assert_eq!(repo(s("htop")).unwrap_err().code, "bad_params");
        assert_eq!(call("package.install_repo", &obj(vec![])).unwrap_err().code, "bad_params");
    }

    #[test]
    fn usb_authorize_writes_a_device_port_and_refuses_hubs_and_paths() {
        let root = std::env::temp_dir().join(format!("privd-usb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("1-2")).unwrap();
        std::fs::create_dir_all(root.join("usb1")).unwrap();
        std::fs::write(root.join("1-2").join("authorized"), "0").unwrap();
        std::fs::write(root.join("usb1").join("authorized"), "1").unwrap();
        let yes = Json::Bool(true);
        usb_authorize(&root, "1-2", Some(&yes)).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("1-2").join("authorized")).unwrap(), "1");
        usb_authorize(&root, "1-2", Some(&Json::Bool(false))).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("1-2").join("authorized")).unwrap(), "0");
        for bad in ["usb1", "../x", "1-2/../../etc", "1-2:1.0", ""] {
            assert_eq!(usb_authorize(&root, bad, Some(&yes)).unwrap_err().code, "bad_params", "{bad}");
        }
        assert_eq!(std::fs::read_to_string(root.join("usb1").join("authorized")).unwrap(), "1", "a root hub is never written");
        assert_eq!(usb_authorize(&root, "1-2", Some(&Json::Str("1".into()))).unwrap_err().code, "bad_params");
        assert_eq!(usb_authorize(&root, "1-2", None).unwrap_err().code, "bad_params");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dns_servers_must_be_plain_addresses() {
        for good in ["1.1.1.1", "9.9.9.9", "2606:4700:4700::1111", "::1"] {
            assert!(valid_dns_server(good), "{good}");
        }
        for bad in ["", "one.one.one.one", "1.1.1.1;reboot", "1.1.1.1:53", "fe80::1%eth0", "256.1.1.1", "1.1.1.1 8.8.8.8"] {
            assert!(!valid_dns_server(bad), "{bad}");
        }
    }

    #[test]
    fn dns_drop_in_lists_the_servers_in_order() {
        assert_eq!(dns_text(&["1.1.1.1".into(), "2606:4700::1111".into()]), "[Resolve]\nDNS=1.1.1.1 2606:4700::1111\n");
        assert_eq!(DNS_PREFS_FILE, "/etc/systemd/resolved.conf.d/10-bifrost-dns.conf");
    }

    #[test]
    fn network_dns_set_refuses_bad_lists_before_any_file_or_command() {
        let dns = |servers: Json| call("network.dns_set", &obj(vec![("servers", servers)]));
        assert_eq!(dns(Json::Arr(vec![s("8.8.8.8;reboot")])).unwrap_err().code, "bad_params");
        assert_eq!(dns(Json::Arr(vec![s("dns.example")])).unwrap_err().code, "bad_params");
        assert_eq!(dns(Json::Arr(vec![Json::Bool(true)])).unwrap_err().code, "bad_params");
        assert_eq!(dns(Json::Arr((0..5).map(|_| s("1.1.1.1")).collect())).unwrap_err().code, "bad_params");
        assert_eq!(dns(s("1.1.1.1")).unwrap_err().code, "bad_params");
        assert_eq!(call("network.dns_set", &obj(vec![])).unwrap_err().code, "bad_params");
    }

    #[test]
    fn dhcp6_serve_refuses_bad_settings_before_any_file_or_command() {
        let serve = |f: Vec<(&str, Json)>| call("dhcp6.serve", &obj(f)).unwrap_err().code;
        assert_eq!(serve(vec![("interface", s("eth0\ninterface=lo")), ("on", Json::Bool(true))]), "bad_params");
        assert_eq!(serve(vec![("interface", s("eth1"))]), "bad_params");
        assert_eq!(serve(vec![("interface", s("eth1")), ("on", Json::Bool(true))]), "bad_params", "no prefix");
        assert_eq!(serve(vec![("interface", s("eth1")), ("on", Json::Bool(true)), ("prefix", s("fd00::5"))]), "bad_params");
        assert_eq!(serve(vec![("interface", s("eth1")), ("on", Json::Bool(true)), ("prefix", s("fd00::")), ("dns", s("fd00::1"))]), "bad_params");
        assert_eq!(serve(vec![("interface", s("eth1")), ("on", Json::Bool(true)), ("prefix", s("fd00::")), ("dns", Json::Arr(vec![s("fe80::1")]))]), "bad_params");
    }

    #[test]
    fn startup_disk_set_refuses_a_bad_id_before_any_command() {
        let set = |id: Json| call("startup_disk.set", &obj(vec![("id", id)]));
        assert_eq!(set(s("0003;reboot")).unwrap_err().code, "bad_params");
        assert_eq!(set(s("3")).unwrap_err().code, "bad_params");
        assert_eq!(set(Json::Bool(true)).unwrap_err().code, "bad_params");
        assert_eq!(call("startup_disk.set", &obj(vec![])).unwrap_err().code, "bad_params");
        assert_eq!(call("startup_disk.set", &obj(vec![("id", s("0003")), ("once", s("yes"))])).unwrap_err().code, "bad_params");
    }

    #[test]
    fn zone_names_are_plain_relative_paths() {
        for bad in ["", "/etc/passwd", "../../etc/passwd", "Europe/../../x", "a b", "Europe//Oslo", "UTC;reboot"] {
            assert!(!valid_zone(bad), "{bad}");
        }
    }

    #[test]
    fn locale_names_have_the_glibc_shape() {
        assert!(locale_shape("is") && locale_shape("is_IS") && locale_shape("en_US"));
        for bad in ["", "i", "is_is", "IS", "is-IS", "is_IS.UTF-8", "../etc", "is_IS;reboot", "is_I"] {
            assert!(!locale_shape(bad), "{bad}");
        }
    }

    #[test]
    fn storage_policy_text_is_the_key_bifrostd_reads() {
        assert_eq!(storage_policy_text(true), "empty_trash_after_30_days = true\n");
        assert_eq!(storage_policy_text(false), "empty_trash_after_30_days = false\n");
        assert_eq!(STORAGE_POLICY_FILE, "/etc/bifrost/storage.conf");
    }

    #[test]
    fn storage_policy_set_refuses_a_missing_or_non_boolean_flag() {
        assert_eq!(call("storage.policy_set", &obj(vec![])).unwrap_err().code, "bad_params");
        assert_eq!(call("storage.policy_set", &obj(vec![("empty_trash_after_30_days", s("yes"))])).unwrap_err().code, "bad_params");
    }

    #[test]
    fn locale_set_refuses_bad_names_before_localectl_runs() {
        assert_eq!(call("locale.set", &obj(vec![("lang", s("../x")), ("region", s("is_IS"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("locale.set", &obj(vec![("lang", s("xx_YY")), ("region", s("is_IS"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("locale.set", &obj(vec![("lang", s("is_IS"))])).unwrap_err().code, "bad_params");
    }

    #[test]
    fn latch_file_is_not_written_through_a_symlink() {
        let base = std::env::temp_dir().join(format!("privd-latch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let target = base.join("victim");
        std::fs::write(&target, "keep").unwrap();
        let link = base.join("boot_attempts");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_nofollow(link.to_str().unwrap(), "0").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        let plain = base.join("plain");
        write_nofollow(plain.to_str().unwrap(), "2").unwrap();
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "2");
    }

    #[test]
    fn bad_params_are_refused_before_any_command_runs() {
        let none = Json::Obj(vec![]);
        assert_eq!(call("update.apply", &obj(vec![("kind", s("bios"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("time.set_ntp", &none).unwrap_err().code, "bad_params");
        assert_eq!(call("time.set_timezone", &obj(vec![("name", s("../x"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("store.uninstall", &obj(vec![("id", s("../x"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("package.remove", &obj(vec![("name", s("base"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("package.remove", &obj(vec![("name", s("--root=/"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("system.boot_attempt", &obj(vec![("event", s("x"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("store.install", &obj(vec![("path", s("/etc/passwd")), ("sha256", s("00"))])).unwrap_err().code, "bad_params");
        assert_eq!(call("store.install", &obj(vec![("id", s("../x"))])).unwrap_err().code, "bad_params");
    }
}
