//! Resource-limit exception diagnostics (docs/parity-matrix, theme-20): the memory and CPU diagnostics a process
//! gets when it goes over a resource limit.
//!
//! A pure classifier over numbers the caller already has: peak memory, or CPU time over a sampled wall-clock window.
//! It takes no samples itself. The per-app limits in `springboard/src/bare/app_manager.rs` are the intended source.
//! Diagnostics carry no call-stack tree: there is no stack capture behind them.

/// A diagnostic for one resource limit that was exceeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    /// The process peaked above its memory limit.
    Memory { limit_bytes: u64, peak_bytes: u64 },
    /// The process used more CPU time over the window than the limit allows.
    Cpu { limit_percent: u32, cpu_ms: u64, wall_ms: u64 },
}

impl Diagnostic {
    /// One line for a crash-log style record.
    pub fn log_line(&self) -> String {
        match self {
            Diagnostic::Memory { limit_bytes, peak_bytes } => {
                format!("resource memory peak={peak_bytes} limit={limit_bytes} bytes")
            }
            Diagnostic::Cpu { limit_percent, cpu_ms, wall_ms } => {
                format!("resource cpu used={cpu_ms}ms of wall={wall_ms}ms limit={limit_percent}%")
            }
        }
    }
}

/// A memory exception: the peak is above the limit. A limit of zero means no limit.
pub fn classify_memory(limit_bytes: u64, peak_bytes: u64) -> Option<Diagnostic> {
    (limit_bytes > 0 && peak_bytes > limit_bytes).then(|| Diagnostic::Memory { limit_bytes, peak_bytes })
}

/// A CPU exception: the CPU time over the window is above `limit_percent` of the wall-clock time. One core is 100
/// percent, so four busy cores are 400. A limit of zero, or an empty window, gives no diagnostic.
pub fn classify_cpu(limit_percent: u32, cpu_ms: u64, wall_ms: u64) -> Option<Diagnostic> {
    if limit_percent == 0 || wall_ms == 0 {
        return None;
    }
    // Compared as cpu_ms * 100 > limit * wall_ms, widened so the product cannot overflow.
    let over = (cpu_ms as u128) * 100 > (limit_percent as u128) * (wall_ms as u128);
    over.then(|| Diagnostic::Cpu { limit_percent, cpu_ms, wall_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_above_the_limit_is_an_exception() {
        assert_eq!(
            classify_memory(1000, 1001),
            Some(Diagnostic::Memory { limit_bytes: 1000, peak_bytes: 1001 })
        );
    }

    #[test]
    fn memory_at_the_limit_is_not_an_exception() {
        assert_eq!(classify_memory(1000, 1000), None);
        assert_eq!(classify_memory(1000, 999), None);
    }

    #[test]
    fn zero_memory_limit_means_no_limit() {
        assert_eq!(classify_memory(0, 5000), None);
    }

    #[test]
    fn cpu_above_the_percentage_is_an_exception() {
        // 900 ms of CPU in a 1000 ms window is 90 percent, above 80.
        assert_eq!(
            classify_cpu(80, 900, 1000),
            Some(Diagnostic::Cpu { limit_percent: 80, cpu_ms: 900, wall_ms: 1000 })
        );
    }

    #[test]
    fn cpu_exactly_at_the_percentage_is_not_an_exception() {
        // 800 ms in 1000 ms is exactly 80 percent.
        assert_eq!(classify_cpu(80, 800, 1000), None);
    }

    #[test]
    fn several_cores_count_above_one_hundred_percent() {
        // 3000 ms in 1000 ms is 300 percent: under a 400 limit, over a 200 limit.
        assert_eq!(classify_cpu(400, 3000, 1000), None);
        assert!(classify_cpu(200, 3000, 1000).is_some());
    }

    #[test]
    fn zero_limit_or_empty_window_gives_no_cpu_diagnostic() {
        assert_eq!(classify_cpu(0, 500, 1000), None);
        assert_eq!(classify_cpu(80, 500, 0), None);
    }

    #[test]
    fn log_lines_have_a_fixed_shape() {
        let mem = Diagnostic::Memory { limit_bytes: 1024, peak_bytes: 2048 };
        assert_eq!(mem.log_line(), "resource memory peak=2048 limit=1024 bytes");
        let cpu = Diagnostic::Cpu { limit_percent: 80, cpu_ms: 900, wall_ms: 1000 };
        assert_eq!(cpu.log_line(), "resource cpu used=900ms of wall=1000ms limit=80%");
    }
}
