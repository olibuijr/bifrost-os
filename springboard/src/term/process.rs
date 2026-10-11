//! Linux process snapshots; RSS and measured CPU, without an invented energy estimate.
use termcore::procs::Proc;
use std::{fs, io};
use crate::bare::sys::{kill, sysconf};
#[cfg(target_os = "android")]
const CLK: i32 = 0x0006;
#[cfg(not(target_os = "android"))]
const CLK: i32 = 2;
#[cfg(target_os = "android")]
const PAGE: i32 = 0x0027;
#[cfg(not(target_os = "android"))]
const PAGE: i32 = 30;
#[derive(Clone)]
pub struct Process {
    pub info: Proc,
    pub start: u64,
}
fn parse(pid: u32, stat: &str, ticks: u64, page: u64) -> Option<Process> {
    let begin = stat.find('(')?;
    let end = stat.rfind(')')?;
    let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();
    let n = |i: usize| fields.get(i)?.parse::<u64>().ok();
    Some(Process {
        info: Proc {
            pid,
            name: stat[begin + 1..end].to_string(),
            footprint: n(21)?.saturating_mul(page),
            cpu_ns: (n(11)? + n(12)?).saturating_mul(1_000_000_000) / ticks.max(1),
            wakeups: 0,
        },
        start: n(19)?,
    })
}
pub fn snapshot() -> Vec<Process> {
    // SAFETY: sysconf accepts these platform-specific names and has no pointer arguments.
    let ticks = unsafe { sysconf(CLK) }.max(1) as u64;
    let page = unsafe { sysconf(PAGE) }.max(1) as u64;
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            if let Ok(stat) = fs::read_to_string(entry.path().join("stat")) {
                if let Some(p) = parse(pid, &stat, ticks, page) {
                    out.push(p);
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.info
            .footprint
            .cmp(&a.info.footprint)
            .then(a.info.pid.cmp(&b.info.pid))
    });
    out
}
pub fn signal(process: &Process, force: bool) -> io::Result<()> {
    if !termcore::procs::killable(process.info.pid, std::process::id()) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let stat = fs::read_to_string(format!("/proc/{}/stat", process.info.pid))?;
    if parse(process.info.pid, &stat, 1, 1).map(|p| p.start) != Some(process.start) {
        return Err(io::Error::other("Process has exited or its PID was reused"));
    }
    // SAFETY: signal is delivered to the validated PID, excluding init and this shell.
    let result = unsafe { kill(process.info.pid as i32, if force { 9 } else { 15 }) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stat_names_with_parentheses_and_spaces_keep_field_offsets() {
        let mut f = vec!["0"; 22];
        f[0] = "S";
        f[11] = "200";
        f[12] = "100";
        f[19] = "77";
        f[21] = "20";
        let p = parse(
            42,
            &format!("42 (a (worker) name) {}", f.join(" ")),
            100,
            4096,
        )
        .unwrap();
        assert_eq!(p.info.name, "a (worker) name");
        assert_eq!(p.info.cpu_ns, 3_000_000_000);
        assert_eq!(p.info.footprint, 81920);
        assert_eq!(p.start, 77);
    }
    #[test]
    fn process_list_reads_this_process_and_refuses_self_signal() {
        let all = snapshot();
        let p = all
            .iter()
            .find(|p| p.info.pid == std::process::id())
            .unwrap();
        assert!(p.info.footprint > 0);
        assert!(signal(p, false).is_err());
    }
}
