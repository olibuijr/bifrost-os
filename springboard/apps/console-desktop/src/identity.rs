//! Code requirements for the process a message came from, in the detail pane. The executable is read from
//! `/proc/<pid>/exe`, hashed, and checked against the requirements the core model defines (`bifrost_core::codereq`).
//! BifrOSt does not sign code yet, so the identity has no signature: the unsigned requirement holds and the platform
//! requirement does not. Nothing is started or changed; a process that has gone shows no lines, and neither does a
//! process that started after the entry was logged (the entry's process id was recycled).

use bifrost_core::codereq::{self, Identity, Requirement, Test};

/// The largest executable that is hashed for the pane. A bigger one would stall the selection, so it is not hashed.
pub const HASH_LIMIT: u64 = 64 << 20;

/// The requirements the pane checks, with the labels it shows.
pub fn requirements() -> Vec<(&'static str, Requirement)> {
    vec![
        ("Unsigned code", Requirement::Test(Test::Unsigned)),
        ("Signed by the platform", Requirement::Test(Test::Platform("bifrost".into()))),
    ]
}

/// The identity of an executable with no code signature.
pub fn unsigned_identity() -> Identity {
    Identity::default()
}

/// The first 16 hex digits of a hash, with an ellipsis, so the line fits the pane.
fn short(hash: &str) -> String {
    format!("{}…", &hash[..hash.len().min(16)])
}

/// The pane's lines for one executable: its path, its hash (None when it was too large to hash), whether its
/// signature is valid, and each requirement.
pub fn lines(exe: &str, hash: Option<&str>) -> Vec<String> {
    let identity = unsigned_identity();
    let signature = codereq::check_on_disk(&Requirement::Test(Test::Unsigned), &identity);
    let hash_line = match hash {
        Some(h) => format!("SHA-256: {}", short(h)),
        None => "SHA-256: not computed, the program is larger than 64 MB".to_string(),
    };
    let mut out = vec![
        format!("Executable: {exe}"),
        hash_line,
        if signature.valid_signature {
            "Valid signature: yes".to_string()
        } else {
            "Valid signature: no, the program has no code signature".to_string()
        },
    ];
    for (name, req) in requirements() {
        let met = codereq::check_process(&req, &identity, None);
        out.push(format!("{name}: {}", if met { "satisfied" } else { "not satisfied" }));
    }
    out
}

/// How far after an entry's time a process may start and still be taken for the entry's process (microseconds): the
/// boot time is known to the second and the start to a clock tick.
const START_SLACK_US: i64 = 2_000_000;

/// The start time of a process in `/proc/<pid>/stat` text, as clock ticks since boot (field 22). The command name is
/// in parentheses and may hold spaces and parentheses, so the fields are counted from the last `)`.
pub fn start_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// The boot time in seconds since the epoch, from the `btime` line of `/proc/stat` text.
pub fn boot_secs(proc_stat: &str) -> Option<i64> {
    proc_stat.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())
}

/// When the process `pid` started, in microseconds since the epoch. The kernel counts start times in `USER_HZ` ticks,
/// which is 100 a second on every Linux.
fn started_at_us(pid: u32) -> Option<i64> {
    const USER_HZ: i64 = 100;
    let ticks = start_ticks(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)? as i64;
    let boot = boot_secs(&std::fs::read_to_string("/proc/stat").ok()?)?;
    Some(boot.checked_mul(1_000_000)?.checked_add(ticks.checked_mul(1_000_000 / USER_HZ)?)?)
}

/// Whether a process that started at `started_us` can be the one that wrote an entry logged at `entry_us`: it was
/// running by then. An entry with no time cannot be checked, so it is not taken for any process.
pub fn could_have_written(started_us: i64, entry_us: i64) -> bool {
    entry_us > 0 && started_us <= entry_us.saturating_add(START_SLACK_US)
}

/// The detail text for the process `pid` that wrote an entry logged at `entry_us` (microseconds since the epoch), or
/// None when there is no such process, it started after the entry, or its executable cannot be read.
pub fn detail_for_entry(pid: u32, entry_us: i64) -> Option<String> {
    if !could_have_written(started_at_us(pid)?, entry_us) {
        return None;
    }
    detail_for_pid(pid)
}

/// The detail text for the process `pid`, or None when there is no such process or its executable cannot be read.
/// This does not know when the entry was logged; `detail_for_entry` does.
pub fn detail_for_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let len = std::fs::metadata(&exe).ok()?.len();
    let hash = if len > HASH_LIMIT { None } else { Some(codereq::hash_hex(&std::fs::read(&exe).ok()?)) };
    Some(lines(&exe.display().to_string(), hash.as_deref()).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_abc_is_the_standard_vector() {
        assert_eq!(codereq::hash_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn the_hash_is_shortened_to_sixteen_digits() {
        assert_eq!(short("ba7816bf8f01cfea414140de5dae2223"), "ba7816bf8f01cfea…");
        assert_eq!(short("abc"), "abc…");
    }

    #[test]
    fn an_unsigned_program_fails_the_platform_requirement_and_meets_the_unsigned_one() {
        let text = lines("/usr/bin/tool", Some("ba7816bf8f01cfea414140de5dae2223"));
        assert_eq!(text[0], "Executable: /usr/bin/tool");
        assert_eq!(text[1], "SHA-256: ba7816bf8f01cfea…");
        assert_eq!(text[2], "Valid signature: no, the program has no code signature");
        assert_eq!(text[3], "Unsigned code: satisfied");
        assert_eq!(text[4], "Signed by the platform: not satisfied");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn a_program_too_large_to_hash_says_so_in_the_hash_line() {
        let text = lines("/opt/big", None);
        assert_eq!(text[1], "SHA-256: not computed, the program is larger than 64 MB");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn the_start_time_is_read_past_a_command_name_with_spaces_and_parentheses() {
        let stat = "123 (my (odd) name) S 1 123 123 0 -1 4194304 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000 100 18446744073709551615 0";
        assert_eq!(start_ticks(stat), Some(98765));
        assert_eq!(start_ticks("123 (x) S 1"), None);
        assert_eq!(start_ticks("garbage"), None);
        assert_eq!(boot_secs("cpu  1 2 3\nbtime 1700000000\nprocesses 5\n"), Some(1_700_000_000));
        assert_eq!(boot_secs("cpu 1\n"), None);
    }

    #[test]
    fn a_process_that_started_after_the_entry_is_not_the_one_that_wrote_it() {
        let entry = 1_700_000_100_000_000;
        assert!(could_have_written(entry - 60_000_000, entry), "running for a minute");
        assert!(could_have_written(entry + 1_000_000, entry), "within the clock slack");
        assert!(!could_have_written(entry + 30_000_000, entry), "started half a minute later: a recycled pid");
        assert!(!could_have_written(entry - 1, 0), "an entry with no time cannot be checked");
    }

    #[test]
    fn this_process_is_known_for_an_entry_after_it_started_and_not_for_an_older_one() {
        let now_us = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
        let pid = std::process::id();
        assert!(detail_for_entry(pid, now_us + 5_000_000).is_some());
        assert!(detail_for_entry(pid, 1_000_000).is_none(), "an entry from 1970 is not this process's");
        assert!(detail_for_entry(pid, 0).is_none());
        assert!(detail_for_entry(u32::MAX, now_us).is_none());
    }

    #[test]
    fn no_pid_or_a_missing_process_has_no_detail() {
        assert!(detail_for_pid(0).is_none());
        assert!(detail_for_pid(u32::MAX).is_none());
    }

    #[test]
    fn this_process_has_an_executable_detail() {
        let text = detail_for_pid(std::process::id()).expect("a running process has an executable");
        assert!(text.starts_with("Executable: "), "{text}");
        assert!(text.contains("Unsigned code: satisfied"), "{text}");
    }
}
