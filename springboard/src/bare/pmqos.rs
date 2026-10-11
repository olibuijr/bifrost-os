//! CPU latency request through /dev/cpu_dma_latency (PM QoS). While the file is open with a
//! value of 0 the kernel keeps cpuidle from entering deep C-states, so a wake-up for the next
//! audio buffer is not paid for with exit latency. The request lives as long as the fd: dropping
//! the guard closes it and the kernel restores the previous target. Best effort: a missing node
//! or a refused open just means no request.

use std::fs::{File, OpenOptions};
use std::io::Write;

const NODE: &str = "/dev/cpu_dma_latency";

/// Holds a 0 us latency target for as long as it lives.
#[derive(Debug)]
pub struct CpuDmaLatency {
    _file: File,
}

impl CpuDmaLatency {
    /// The request, or None when the node is absent or not writable.
    pub fn hold() -> Option<Self> {
        Self::hold_at(NODE)
    }

    fn hold_at(path: &str) -> Option<Self> {
        let mut file = OpenOptions::new().write(true).open(path).ok()?;
        // the node takes a native-endian s32 in microseconds
        file.write_all(&0i32.to_ne_bytes()).ok()?;
        Some(CpuDmaLatency { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_node_is_no_request() {
        assert!(CpuDmaLatency::hold_at("/nonexistent/cpu_dma_latency").is_none());
    }

    #[test]
    fn writes_zero_while_held() {
        let dir = std::env::temp_dir().join(format!("pmqos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("node");
        std::fs::write(&p, 5i32.to_ne_bytes()).unwrap();
        let g = CpuDmaLatency::hold_at(p.to_str().unwrap()).expect("file is writable");
        drop(g);
        assert_eq!(std::fs::read(&p).unwrap(), 0i32.to_ne_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
