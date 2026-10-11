//! Crash and flood guards of the engine: one bad command must not kill the process, and no queue may grow without bound.
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Longest command line the engine will parse (bytes). Longer lines are cut on a char boundary.
pub const MAX_LINE: usize = 256 * 1024;
/// Notes queued between two loop turns. Past `NOTES_SOFT` console notes are dropped, past `NOTES_HARD` every noisy
/// note (frame, title, url, history) is too; load, crash and eval notes are always kept.
pub const NOTES_SOFT: usize = 2048;
pub const NOTES_HARD: usize = 8192;
/// Eval callbacks the engine will have in flight; further `eval` verbs are answered with an error at once.
pub const MAX_EVALS: usize = 64;

/// Cuts `s` to at most `MAX_LINE` bytes without splitting a character.
pub fn cap_line(s: &str) -> &str {
    if s.len() <= MAX_LINE {
        return s;
    }
    let mut end = MAX_LINE;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Runs one command; a panic is returned as its message and the engine keeps going.
pub fn guarded<F: FnOnce() -> bool>(f: F) -> Result<bool, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_is_contained() {
        assert_eq!(guarded(|| true), Ok(true));
        assert_eq!(guarded(|| panic!("boom")), Err("boom".to_string()));
    }

    #[test]
    fn long_line_cut_on_boundary() {
        let s = "é".repeat(MAX_LINE);
        let c = cap_line(&s);
        assert!(c.len() <= MAX_LINE && s.starts_with(c));
        assert_eq!(cap_line("abc"), "abc");
    }
}
