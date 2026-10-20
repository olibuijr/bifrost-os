//! Opening a downloaded program: the shell side of `bifrost_core::launch_policy`.
//!
//! When the person opens a program from Files that arrived by a download or an attachment (it carries the quarantine
//! record the receiving app wrote, `user.bifrost.quarantine`), the shell reads the record, hashes the file, and asks
//! the policy: allowed, ask first, or blocked. A blocked program gets the "cannot be opened" alert, and Settings >
//! Privacy & Security then offers Open Anyway for it, which is remembered per path and per hash of the code (a
//! replaced program is asked about again). The setting ("Allow apps from") is the `allow_apps_from` pref; the
//! remembered programs and the program last blocked are small files in the data folder.
//!
//! BifrOSt does not sign code yet, so every program here has the unsigned identity: only its hash is known. Nothing
//! leaves this computer.

use super::list::{Group, Item, Tint};
use super::pages::{Action, Pick};
use super::prefs::Prefs;
use bifrost_core::codereq::Identity;
use bifrost_core::launch_policy::{Candidate, Policy, Setting, Source, Verdict};
use bifrost_core::quarantine::{Quarantine, QuarantineKind};
use bifrost_core::sha256::{self, Sha256};
use std::ffi::{CString, OsStr};
use std::io::Read;
use std::os::raw::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The pref that keeps the Allow Apps From setting.
pub const SETTING_PREF: &str = "allow_apps_from";
/// The data-folder file with the programs the person said to open anyway (`Policy::overrides_to_text`).
pub const OVERRIDES_FILE: &str = "launch_overrides.txt";
/// The data-folder file with the program last blocked: its path, then the sentence the alert showed.
pub const BLOCKED_FILE: &str = "launch_blocked.txt";
/// The extended attribute the receiving app writes (`bifrost_app::quarantine::QUARANTINE_XATTR`).
pub const QUARANTINE_XATTR: &str = "user.bifrost.quarantine";

/// The choices of Settings > Privacy & Security > Allow Apps From, in the order shown.
pub const SETTINGS: [(&str, Setting); 3] = [("Store", Setting::StoreOnly), ("Store and Identified Developers", Setting::StoreAndIdentified), ("Anywhere", Setting::Anywhere)];

extern "C" {
    fn lgetxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: usize) -> isize;
    fn geteuid() -> u32;
}

/// What the shell does that the tests replace: reading the record and running the program.
#[derive(Clone, Copy)]
pub struct Hooks {
    pub record: fn(&Path) -> Option<Quarantine>,
    pub run: fn(&Path) -> std::io::Result<()>,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks { record: record_of, run: run_program }
    }
}

/// True for a file that runs: an AppImage-like program by name, or any regular file with an execute bit.
pub fn is_program(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    let appimage = path.extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("appimage"));
    appimage || meta.permissions().mode() & 0o111 != 0
}

/// Reads `flags;timestamp;agent;uuid` (numbers in hex), the value the receiving app wrote. Every record counts as a
/// download from outside; the flag bits carry no more meaning than that here.
pub fn parse_record(value: &str) -> Option<Quarantine> {
    let mut parts = value.trim().splitn(4, ';');
    let (_flags, time, agent) = (parts.next()?, parts.next()?, parts.next()?);
    let time = u64::from_str_radix(time, 16).ok()?;
    Some(Quarantine::new(QuarantineKind::WebDownload, agent, agent, time))
}

/// The quarantine record on `path`, or None when the file has none (or the record is damaged).
pub fn record_of(path: &Path) -> Option<Quarantine> {
    let (p, n) = (CString::new(path.as_os_str().as_bytes()).ok()?, CString::new(QUARANTINE_XATTR).ok()?);
    let mut buf = [0u8; 512];
    // SAFETY: both strings are live NUL-terminated buffers and `buf` is a live buffer of the length passed.
    let got = unsafe { lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr() as *mut c_void, buf.len()) };
    if got < 0 {
        return None;
    }
    parse_record(std::str::from_utf8(&buf[..got as usize]).ok()?)
}

/// The identity of an unsigned program: the SHA-256 of its contents as the code hash.
pub fn identity_of(path: &Path) -> std::io::Result<Identity> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(Identity { code_directory_hash: Some(sha256::hex(&hash.finish())), ..Identity::default() })
}

/// The identity a downloaded program runs under: its own cgroup, data folder and prefetch record.
pub const PROGRAM_ID: &str = "is.olibuijr.downloaded-program";

/// The command that starts `path` the way an installed native app starts: as the session account (never as the shell's
/// root), with no new privileges, no inherited descriptors and the app sandbox around it, holding the storage
/// permission only (the home folder, no network). It fails when that cannot be set up.
#[cfg(feature = "desktop-wayland")]
fn sandboxed_command(path: &Path) -> std::io::Result<std::process::Command> {
    use crate::bare::app_policy::{command, Policy};
    let data = super::apphost::root().join("appdata").join(PROGRAM_ID);
    command(PROGRAM_ID, path, &data, Policy { storage: true, ..Policy::default() }, true)
}

/// Builds without the desktop sandbox cannot confine a program.
#[cfg(not(feature = "desktop-wayland"))]
fn sandboxed_command(_path: &Path) -> std::io::Result<std::process::Command> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no sandbox in this build"))
}

/// True when this process runs as root (the session service does).
fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { geteuid() == 0 }
}

/// The command for `path`: sandboxed as the session account. A root shell never starts a program any other way, so
/// when the sandbox cannot be built the program is refused. A shell that is not root (a development host) starts it
/// as its own user in its own folder.
fn program_command(path: &Path) -> std::io::Result<std::process::Command> {
    match sandboxed_command(path) {
        Ok(cmd) => Ok(cmd),
        Err(e) if running_as_root() => Err(e),
        Err(_) => {
            let mut cmd = std::process::Command::new(path);
            if let Some(dir) = path.parent() {
                cmd.current_dir(dir);
            }
            Ok(cmd)
        }
    }
}

/// Starts the program built by `build` detached, with no input or output, and reaps it when it ends. Nothing is
/// started when `build` fails.
fn start(path: &Path, build: impl FnOnce(&Path) -> std::io::Result<std::process::Command>) -> std::io::Result<()> {
    let mut cmd = build(path)?;
    let mut child = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    // wait for it on its own thread so a finished program does not stay a zombie of the shell
    let _ = std::thread::Builder::new().name("launch-reap".into()).spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Starts the program detached with no input or output, as the session account inside the app sandbox (see
/// `program_command`).
pub fn run_program(path: &Path) -> std::io::Result<()> {
    start(path, program_command)
}

/// The policy: the setting from the prefs and the remembered programs from the data folder.
pub fn load_policy(prefs: &Prefs, dir: &Path) -> Policy {
    let setting = prefs.get(SETTING_PREF).and_then(Setting::parse).unwrap_or(Setting::StoreAndIdentified);
    Policy::new(setting).with_overrides_text(&std::fs::read_to_string(dir.join(OVERRIDES_FILE)).unwrap_or_default())
}

/// Keeps the remembered programs.
pub fn save_overrides(policy: &Policy, dir: &Path) -> std::io::Result<()> {
    let tmp = dir.join(format!("{OVERRIDES_FILE}.tmp"));
    std::fs::write(&tmp, policy.overrides_to_text())?;
    std::fs::rename(&tmp, dir.join(OVERRIDES_FILE))
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The verdict for a received program.
pub fn assess(policy: &Policy, path: &Path, record: Quarantine, identity: &Identity) -> Verdict {
    let shown = path.to_string_lossy();
    let name = name_of(path);
    let verdict = policy.assess(&Candidate { path: &shown, name: &name, source: Source::Received(record), identity, signature_intact: true });
    verdict
}

/// Remembers that the person said to open this program (this code at this path) anyway.
pub fn grant(policy: &mut Policy, path: &Path, identity: &Identity) {
    let shown = path.to_string_lossy();
    let name = name_of(path);
    policy.open_anyway(&Candidate { path: &shown, name: &name, source: Source::Local, identity, signature_intact: true });
}

/// Remembers the program that was blocked last, with the sentence the alert showed.
pub fn write_blocked(dir: &Path, path: &Path, sentence: &str) {
    let _ = std::fs::write(dir.join(BLOCKED_FILE), format!("{}\n{}\n", path.to_string_lossy().replace('\n', " "), sentence.replace('\n', " ")));
}

/// The program that was blocked last and the sentence, if one is waiting for Open Anyway.
pub fn read_blocked(dir: &Path) -> Option<(PathBuf, String)> {
    let text = std::fs::read_to_string(dir.join(BLOCKED_FILE)).ok()?;
    let mut lines = text.lines();
    let path = PathBuf::from(lines.next()?);
    path.is_absolute().then(|| (path, lines.next().unwrap_or("").to_string()))
}

/// Forgets the blocked program.
pub fn clear_blocked(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(BLOCKED_FILE));
}

/// Open Anyway on the blocked program: its code is hashed again, remembered, and the block is cleared. Returns false
/// when nothing is waiting or the file cannot be read.
pub fn open_anyway(dir: &Path, prefs: &Prefs) -> bool {
    let Some((path, _)) = read_blocked(dir) else { return false };
    let Ok(identity) = identity_of(&path) else { return false };
    let mut policy = load_policy(prefs, dir);
    grant(&mut policy, &path, &identity);
    if save_overrides(&policy, dir).is_err() {
        return false;
    }
    clear_blocked(dir);
    true
}

/// The groups added to Settings > Privacy: the blocked program with Open Anyway (first, only while one waits), then
/// the Allow Apps From choices.
pub fn groups(setting: Setting, blocked: Option<&(PathBuf, String)>) -> Vec<Group> {
    let mut out = Vec::new();
    if let Some((path, sentence)) = blocked {
        out.push(
            Group::new(vec![Item::new(format!("\"{}\"", name_of(path))).value("Blocked").tint(Tint::Dim), Item::new("Open Anyway").tint(Tint::Blue).action(Action::OpenAnyway)])
                .header("SECURITY")
                .footer(&format!("{sentence} Choose Open Anyway, then open it again from Files.")),
        );
    }
    out.push(
        Group::new(vec![{
            let now = SETTINGS.iter().position(|(_, s)| *s == setting).unwrap_or(0);
            Item::new("Allow Apps From").popup(SETTINGS.iter().map(|(label, _)| label.to_string()).collect(), now, Action::Pick(Pick::AllowAppsFrom(now)))
        }])
            .footer("Programs downloaded from the internet or received in a message are checked against this when you open them from Files."),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::launch_policy::Reason;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("launch-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn program(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn record() -> Quarantine {
        Quarantine::new(QuarantineKind::WebDownload, "Browser", "Browser", 100)
    }

    #[test]
    fn programs_are_found_by_their_execute_bit_or_their_name() {
        let dir = temp("is-program");
        let exe = program(&dir, "tool", "#!/bin/sh\n");
        let plain = dir.join("notes.txt");
        std::fs::write(&plain, "x").unwrap();
        let image = dir.join("Editor.AppImage");
        std::fs::write(&image, "x").unwrap();
        assert!(is_program(&exe) && is_program(&image));
        assert!(!is_program(&plain) && !is_program(&dir) && !is_program(&dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_record_the_sdk_writes_is_read_back() {
        let r = parse_record("0040;6553f100;BifrOSt Browser;A1B2").unwrap();
        assert_eq!((r.agent_name.as_str(), r.time), ("BifrOSt Browser", 0x6553_f100));
        assert!(parse_record("0040;zz;agent;u").is_none() && parse_record("junk").is_none());
    }

    #[test]
    fn a_blocked_program_is_remembered_per_path_and_hash() {
        let dir = temp("grant");
        let prog = program(&dir, "tool", "#!/bin/sh\necho one\n");
        let prefs = Prefs::memory();
        let id = identity_of(&prog).unwrap();
        let policy = load_policy(&prefs, &dir);
        let verdict = assess(&policy, &prog, record(), &id);
        assert!(matches!(verdict, Verdict::Block(Reason::NotIdentified)), "{verdict:?}");
        write_blocked(&dir, &prog, "x");
        assert!(open_anyway(&dir, &prefs));
        assert!(read_blocked(&dir).is_none(), "the block is cleared");
        let policy = load_policy(&prefs, &dir);
        assert_eq!(assess(&policy, &prog, record(), &id), Verdict::Allow);
        // other code at the same path is asked about again
        std::fs::write(&prog, "#!/bin/sh\necho two\n").unwrap();
        let replaced = identity_of(&prog).unwrap();
        assert_ne!(replaced.code_directory_hash, id.code_directory_hash);
        assert!(matches!(assess(&policy, &prog, record(), &replaced), Verdict::Block(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_whose_sandbox_cannot_be_built_is_not_started() {
        let dir = temp("refused");
        let marker = dir.join("ran");
        let prog = program(&dir, "tool", &format!("#!/bin/sh\necho x > {}\n", marker.display()));
        let refused = start(&prog, |_| Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no sandbox")));
        assert_eq!(refused.unwrap_err().kind(), std::io::ErrorKind::Unsupported);
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!marker.exists(), "nothing ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_started_program_runs_detached_through_the_command_that_was_built() {
        let dir = temp("started");
        let marker = dir.join("ran");
        let prog = program(&dir, "tool", &format!("#!/bin/sh\necho x > {}\n", marker.display()));
        start(&prog, |p| Ok(std::process::Command::new(p))).unwrap();
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(marker.exists(), "the built command ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_sandbox_in_this_build_refuses_instead_of_running_as_root() {
        #[cfg(not(feature = "desktop-wayland"))]
        {
            let err = sandboxed_command(Path::new("/x")).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
            if running_as_root() {
                assert!(program_command(Path::new("/x")).is_err(), "a root shell does not fall back to a plain spawn");
            }
        }
    }

    #[test]
    fn the_setting_is_read_from_the_prefs() {
        let dir = temp("setting");
        let mut prefs = Prefs::memory();
        assert_eq!(load_policy(&prefs, &dir).setting(), Setting::StoreAndIdentified);
        prefs.set(SETTING_PREF, Setting::Anywhere.label()).unwrap();
        assert_eq!(load_policy(&prefs, &dir).setting(), Setting::Anywhere);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
