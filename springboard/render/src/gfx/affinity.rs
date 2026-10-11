//! Pins the render workers to a set of cpus, only when `SPRINGBOARD_BIG_CPUS` is set (e.g. "6-7").
//! The list parsing and mask building are pure; only `pin_big` calls the kernel.

/// The cpus `plan` falls back to when given no list.
pub const DEFAULT_BIG: &str = "6-7";

/// "6-7", "0,2-3" -> the cpu numbers; None for anything malformed or an empty list.
pub fn parse_cpu_list(s: &str) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in s.trim().split(',') {
        let part = part.trim();
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (usize, usize) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
                if a > b || b >= 1024 {
                    return None;
                }
                out.extend(a..=b);
            }
            None => {
                let c: usize = part.parse().ok()?;
                if c >= 1024 {
                    return None;
                }
                out.push(c);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    (!out.is_empty()).then_some(out)
}

/// The cpu_set_t words (64 cpus each) for `cpus`, keeping only those below `present`
/// (the count of cpus the kernel has); None when none is left.
pub fn mask(cpus: &[usize], present: usize) -> Option<[u64; 16]> {
    let mut m = [0u64; 16];
    let mut any = false;
    for &c in cpus.iter().filter(|&&c| c < present && c < 1024) {
        m[c / 64] |= 1 << (c % 64);
        any = true;
    }
    any.then_some(m)
}

/// The mask for the big cluster given the env value, "SPRINGBOARD_PIN" ("0" = off) and the
/// kernel's present list ("0-7"); None = do not pin.
pub fn plan(big: Option<&str>, pin: Option<&str>, present: &str) -> Option<[u64; 16]> {
    if pin.map(str::trim) == Some("0") {
        return None;
    }
    let present = parse_cpu_list(present)?.into_iter().max()? + 1;
    mask(&parse_cpu_list(big.unwrap_or(DEFAULT_BIG))?, present)
}

/// The mask of the cpus the render pool runs on, decided once per process.
fn big_plan() -> &'static Option<[u64; 16]> {
    static PLAN: std::sync::OnceLock<Option<[u64; 16]>> = std::sync::OnceLock::new();
    PLAN.get_or_init(|| {
        let big = std::env::var("SPRINGBOARD_BIG_CPUS").ok();
        if big.is_none() {
            return None;
        }
        let present = std::fs::read_to_string("/sys/devices/system/cpu/present").unwrap_or_default();
        plan(big.as_deref(), std::env::var("SPRINGBOARD_PIN").ok().as_deref(), &present)
    })
}

/// How many cpus the render pool and the copy-out threads may use at once: the pinned big
/// cluster when there is one, else every cpu. More threads than this only queue on the same
/// cores.
pub fn pool_cpus() -> usize {
    // decided once: available_parallelism reads the affinity mask and cgroup files on every call (~15 us),
    // which would cost more than a small damage copy
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| match big_plan() {
        Some(m) => m.iter().map(|w| w.count_ones() as usize).sum::<usize>().max(1),
        None => std::thread::available_parallelism().map_or(1, |n| n.get()),
    })
}

/// Pins the calling thread to the big cluster, once decided per process; false when not done
/// (disabled, fewer cpus, not Linux, or the kernel refused).
pub fn pin_big() -> bool {
    match big_plan() {
        Some(m) => apply(m),
        None => false,
    }
}

#[cfg(target_os = "linux")]
fn apply(m: &[u64; 16]) -> bool {
    extern "C" {
        fn sched_setaffinity(pid: i32, size: usize, mask: *const u64) -> i32;
    }
    // SAFETY: pid 0 is the calling thread; `m` is a 128-byte cpu_set_t that lives through the call.
    unsafe { sched_setaffinity(0, std::mem::size_of_val(m), m.as_ptr()) == 0 }
}

#[cfg(not(target_os = "linux"))]
fn apply(_: &[u64; 16]) -> bool {
    false
}

/// "SPRINGBOARD_SCHED=0" turns every priority change below off.
pub fn sched_enabled(env: Option<&str>) -> bool {
    env.map(str::trim) != Some("0")
}

fn sched_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| sched_enabled(std::env::var("SPRINGBOARD_SCHED").ok().as_deref()))
}

/// Nice value of the calling thread (Linux: setpriority on tid 0 is per thread); false when
/// disabled or refused (not root).
pub fn set_nice(nice: i32) -> bool {
    sched_on() && apply_nice(nice)
}

/// The UI thread: nice -10, pinned only when `SPRINGBOARD_UI_CPUS` is set (e.g. "6-7").
pub fn ui_thread() {
    if !sched_on() {
        return;
    }
    set_nice(-10);
    let Ok(cpus) = std::env::var("SPRINGBOARD_UI_CPUS") else {
        return;
    };
    let present = std::fs::read_to_string("/sys/devices/system/cpu/present").unwrap_or_default();
    if let Some(m) = plan(Some(&cpus), None, &present) {
        apply(&m);
    }
}

/// The audio writer: SCHED_FIFO priority 2, else nice -16. `SPRINGBOARD_AUDIO_CPUS` (e.g. "7", an
/// isolated core, see the installer's AUDIO_CORE) pins it there.
pub fn audio_thread() {
    if !sched_on() {
        return;
    }
    if !apply_fifo(2) {
        set_nice(-16);
    }
    if let Ok(cpus) = std::env::var("SPRINGBOARD_AUDIO_CPUS") {
        let present = std::fs::read_to_string("/sys/devices/system/cpu/present").unwrap_or_default();
        if let Some(m) = plan(Some(&cpus), None, &present) {
            apply(&m);
        }
    }
}

/// Storage scans, photo loading, network fetches: nice +10 so they never steal the UI's time.
pub fn background_thread() {
    set_nice(10);
}

#[cfg(target_os = "linux")]
fn apply_nice(nice: i32) -> bool {
    extern "C" {
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }
    // SAFETY: PRIO_PROCESS (0) with who 0 is the calling thread.
    unsafe { setpriority(0, 0, nice) == 0 }
}

#[cfg(target_os = "linux")]
fn apply_fifo(prio: i32) -> bool {
    extern "C" {
        fn sched_setscheduler(pid: i32, policy: i32, param: *const i32) -> i32;
    }
    // SAFETY: SCHED_FIFO is 1; sched_param is one int; pid 0 is the calling thread.
    unsafe { sched_setscheduler(0, 1, &prio) == 0 }
}

#[cfg(not(target_os = "linux"))]
fn apply_nice(_: i32) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
fn apply_fifo(_: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sched_env_switch() {
        assert!(sched_enabled(None));
        assert!(sched_enabled(Some("1")));
        assert!(!sched_enabled(Some("0")));
        assert!(!sched_enabled(Some(" 0\n")));
    }

    #[test]
    fn lists_parse() {
        assert_eq!(parse_cpu_list("6-7\n"), Some(vec![6, 7]));
        assert_eq!(parse_cpu_list("0,2-3, 3"), Some(vec![0, 2, 3]));
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        for bad in ["", "a", "7-6", "1-", "2000", "1,,2"] {
            assert_eq!(parse_cpu_list(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn mask_keeps_present_cpus_only() {
        assert_eq!(mask(&[6, 7], 8).map(|m| m[0]), Some(0xC0));
        assert_eq!(mask(&[6, 7], 7).map(|m| m[0]), Some(0x40));
        assert_eq!(mask(&[6, 7], 4), None);
        assert_eq!(mask(&[65], 128).map(|m| m[1]), Some(2));
    }

    #[test]
    fn plan_follows_env_and_hardware() {
        assert_eq!(plan(None, None, "0-7").map(|m| m[0]), Some(0xC0));
        assert_eq!(plan(Some("4-5"), None, "0-7").map(|m| m[0]), Some(0x30));
        assert_eq!(plan(None, Some("0"), "0-7"), None);
        assert_eq!(plan(None, None, "0-3"), None, "a 4-core host has no big cluster");
        assert_eq!(plan(None, None, ""), None);
    }
}
