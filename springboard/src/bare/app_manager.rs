//! Kernel-enforced resource ownership for installed desktop applications.
use std::{
    fs, io,
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::{Mutex, OnceLock},
};
unsafe extern "C" {
    fn getpid() -> i32;
    fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
}
fn stats(text: &str) -> String {
    let fields: Vec<_> = text
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let key = p.next()?;
            let value = p.next()?.parse::<u64>().ok()?;
            if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return None;
            }
            Some(format!("\"{key}\":{value}"))
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}
const ROOT: &str = "/sys/fs/cgroup/bifrost-apps";
static GROUPS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The resource file is only written while someone reads it (`want_resources`): idle means no 5 s timer.
static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
static LAST_WANT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// True while the writer thread runs; it exits when nobody wants the file, so an idle shell holds no thread for it.
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const KEEP_WRITING_SECS: u64 = 15;
fn now_secs() -> u64 {
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs() + 1
}
/// True once nobody has asked for the file for longer than the keep window.
fn writer_should_park(last_want: u64, now: u64) -> bool {
    now.saturating_sub(last_want) > KEEP_WRITING_SECS
}
/// The Cpu/Memory readouts read `/run/bifrost/app-resources.json`: calling this keeps the writer running and starts it
/// if it has stopped.
pub fn want_resources() {
    LAST_WANT.store(now_secs(), std::sync::atomic::Ordering::SeqCst);
    ensure_writer();
}
fn ensure_writer() {
    use std::sync::atomic::Ordering::SeqCst;
    if RUNNING.swap(true, SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("app-resources".into()).spawn(|| loop {
        write_resources();
        std::thread::sleep(std::time::Duration::from_secs(5));
        if writer_should_park(LAST_WANT.load(SeqCst), now_secs()) {
            RUNNING.store(false, SeqCst);
            // a want that landed after the check and saw RUNNING still set would be lost: take the writer back
            if writer_should_park(LAST_WANT.load(SeqCst), now_secs()) || RUNNING.swap(true, SeqCst) {
                return;
            }
        }
    });
    if spawned.is_err() {
        RUNNING.store(false, SeqCst);
    }
}
fn write_resources() {
    let ids=GROUPS.lock().map(|g|g.clone()).unwrap_or_default();let mut rows=Vec::new();
    for id in ids {if let Ok(path)=group(&id){
        let read=|n:&str|fs::read_to_string(path.join(n)).unwrap_or_default();
        let number=|n:&str|read(n).trim().parse::<u64>().unwrap_or(0);
        rows.push(format!("{{\"id\":\"{id}\",\"memory_bytes\":{},\"memory_high\":{},\"memory_max\":{},\"memory_events\":{},\"cpu\":{},\"processes\":{}}}",number("memory.current"),number("memory.high"),number("memory.max"),stats(&read("memory.events")),stats(&read("cpu.stat")),number("pids.current")));
    }}
    let _=fs::create_dir_all("/run/bifrost");
    let bytes=format!("[{}]",rows.join(","));
    if fs::write("/run/bifrost/app-resources.json.tmp",bytes).is_ok(){let _=fs::rename("/run/bifrost/app-resources.json.tmp","/run/bifrost/app-resources.json");}
}
#[cfg(test)]
mod writer_tests {
    use super::*;
    #[test]
    fn writer_parks_after_the_keep_window() {
        assert!(!writer_should_park(5, 5 + KEEP_WRITING_SECS));
        assert!(writer_should_park(5, 6 + KEEP_WRITING_SECS));
    }
}
fn group(id: &str) -> io::Result<PathBuf> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || id == "."
        || id == ".."
    {
        return Err(io::Error::other("Invalid app cgroup identity"));
    }
    Ok(PathBuf::from(ROOT).join(id))
}
fn ram() -> u64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        })
        .unwrap_or(2 * 1024 * 1024)
        * 1024
}
fn budgets(total: u64, focused: bool) -> (u64, u64) {
    let high = (total / if focused { 4 } else { 8 }).max(256 * 1024 * 1024);
    let max = (total * 3 / 4).max(512 * 1024 * 1024);
    (high.min(max), max)
}
/// Prepare before fork; attach before executing any app code or spawning children.
pub fn configure(cmd: &mut Command, id: &str) -> io::Result<()> {
    let path = group(id)?;
    fs::create_dir_all(ROOT)?;
    let total = ram();
    fs::write(
        PathBuf::from(ROOT).join("memory.high"),
        (total * 3 / 4).to_string(),
    )?;
    fs::write(
        PathBuf::from(ROOT).join("memory.max"),
        (total * 9 / 10).to_string(),
    )?;
    fs::write(
        PathBuf::from(ROOT).join("cgroup.subtree_control"),
        "+cpu +memory +pids",
    )?;
    fs::create_dir_all(&path)?;
    let (high, max) = budgets(ram(), true);
    for (name, value) in [
        ("memory.high", high.to_string()),
        ("memory.max", max.to_string()),
        ("memory.oom.group", "1".into()),
        ("pids.max", "4096".into()),
        ("cpu.weight", "100".into()),
    ] {
        fs::write(path.join(name), value)?;
    }
    let procs = fs::OpenOptions::new()
        .write(true)
        .open(path.join("cgroup.procs"))?;
    // SAFETY: getpid/write and stack-only decimal conversion; no allocation after fork.
    unsafe {
        cmd.pre_exec(move || {
            let mut pid = getpid() as u32;
            let mut digits = [0u8; 16];
            let mut pos = digits.len();
            loop {
                pos -= 1;
                digits[pos] = b'0' + (pid % 10) as u8;
                pid /= 10;
                if pid == 0 {
                    break;
                }
            }
            let len = digits.len() - pos;
            if write(procs.as_raw_fd(), digits[pos..].as_ptr().cast(), len) != len as isize {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    if let Ok(mut g) = GROUPS.lock() {
        if !g.iter().any(|n| n == id) {
            g.push(id.to_owned());
        }
    }
    // the writer starts when a readout asks for the file (`want_resources`), not on every launch
    Ok(())
}
pub fn focus(id: &str, on: bool) {
    if let Ok(path) = group(id) {
        let (high, _) = budgets(ram(), on);
        let _ = fs::write(path.join("memory.high"), high.to_string());
        let _ = fs::write(path.join("cpu.weight"), if on { "100" } else { "25" });
    }
}
/// Terminate the complete owned tree, including nested Browser/content processes.
pub fn terminate(id: &str) {
    if let Ok(path) = group(id) {
        let _ = fs::write(path.join("cgroup.kill"), "1");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_manager_identity() {
        assert!(group("is.olibuijr.files.desktop").is_ok());
        for id in ["", "..", "a/b", "../shell"] {
            assert!(group(id).is_err());
        }
    }
    #[test]
    fn app_manager_budgets() {
        for total in [
            512 * 1024 * 1024,
            2 * 1024 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
        ] {
            let (fg, max) = budgets(total, true);
            let (bg, _) = budgets(total, false);
            assert!(bg <= fg && fg <= max && max <= total);
        }
    }
}
