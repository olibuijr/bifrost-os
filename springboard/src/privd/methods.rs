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
        "time.set_timezone" => set_timezone(str_param(p, "name")?),
        "time.set_ntp" => match p.get("on") {
            Some(Json::Bool(on)) => run("timedatectl", &["set-ntp", if *on { "true" } else { "false" }]).map(|_| obj(vec![])),
            _ => Err(PErr::bad("on must be a boolean")),
        },
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

fn power(verb: &str) -> Result<Json, PErr> {
    if verb != "suspend" {
        // SAFETY: sync takes no arguments.
        unsafe { sync() };
    }
    run("systemctl", &[verb]).map(|_| obj(vec![]))
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
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(not(target_arch = "x86_64"))]
const O_NOFOLLOW: i32 = 0o100000;

/// Writes `text` to `file` without following a symlink at the final component (root writes here).
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

#[cfg(test)]
mod tests {
    use super::*;

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
