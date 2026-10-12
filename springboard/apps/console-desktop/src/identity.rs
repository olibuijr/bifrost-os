//! Code requirements for the process a message came from, in the detail pane. The executable is read from
//! `/proc/<pid>/exe`, hashed, and checked against the requirements the core model defines (`bifrost_core::codereq`).
//! BifrOSt does not sign code yet, so the identity has no signature: the unsigned requirement holds and the platform
//! requirement does not. Nothing is started or changed; a process that has gone shows no lines.

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

/// The detail text for the process `pid`, or None when there is no such process or its executable cannot be read.
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
