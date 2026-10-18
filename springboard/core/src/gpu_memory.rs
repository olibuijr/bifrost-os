//! GPU memory per process (`IOAccelMemoryInfo` parity: the collector that reports how much video memory each process
//! holds).
//!
//! Pure: the caller reads `/proc/<pid>/fdinfo/<fd>` for each descriptor that is a DRM client and passes the text in.
//! The kernel's drm-usage-stats format gives, per client, the memory it has in each region as `drm-memory-<region>`:
//! `vram` (dedicated video memory), `gtt` (GPU-mapped system memory) and `cpu` (plain system memory). Several
//! descriptors of one process can be the same client; `collect` counts each client once, taking the largest value
//! seen for each region.
//!
//! Units: `KiB`, `MiB` and `GiB` as the kernel writes them; a bare number is bytes.

use std::collections::BTreeMap;

/// One DRM client's memory, in bytes per region.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientMemory {
    /// `pdev/client-id`, the same key the activity monitor uses for a client.
    pub client: String,
    pub vram: u64,
    pub gtt: u64,
    pub system: u64,
}

impl ClientMemory {
    /// All regions together, saturating.
    pub fn total(&self) -> u64 {
        self.vram.saturating_add(self.gtt).saturating_add(self.system)
    }
}

/// A size as the kernel writes it: `"8192 KiB"`, `"1 MiB"`, `"2 GiB"`, or a bare byte count.
pub fn parse_size(value: &str) -> Option<u64> {
    let value = value.trim();
    let (number, factor) = if let Some(n) = value.strip_suffix("GiB") {
        (n, 1u64 << 30)
    } else if let Some(n) = value.strip_suffix("MiB") {
        (n, 1u64 << 20)
    } else if let Some(n) = value.strip_suffix("KiB") {
        (n, 1u64 << 10)
    } else {
        (value, 1)
    };
    number.trim().parse::<u64>().ok()?.checked_mul(factor)
}

/// The DRM client described by one `fdinfo` text, or `None` when the text is not a DRM client.
pub fn parse_fdinfo(text: &str) -> Option<ClientMemory> {
    let (mut driver, mut client, mut pdev) = (false, String::new(), String::new());
    let mut mem = ClientMemory::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key {
            "drm-driver" => driver = true,
            "drm-client-id" => client = value.to_string(),
            "drm-pdev" => pdev = value.to_string(),
            _ => {
                let Some(region) = key.strip_prefix("drm-memory-") else { continue };
                let Some(bytes) = parse_size(value) else { continue };
                match region {
                    "vram" => mem.vram = bytes,
                    "gtt" => mem.gtt = bytes,
                    "cpu" => mem.system = bytes,
                    _ => {}
                }
            }
        }
    }
    driver.then(|| ClientMemory { client: format!("{pdev}/{client}"), ..mem })
}

/// Group descriptors by process id. Each process lists its clients once, with the largest value seen per region.
pub fn collect(descriptors: &[(u32, &str)]) -> BTreeMap<u32, Vec<ClientMemory>> {
    let mut by_pid: BTreeMap<u32, Vec<ClientMemory>> = BTreeMap::new();
    for (pid, text) in descriptors {
        let Some(found) = parse_fdinfo(text) else { continue };
        let clients = by_pid.entry(*pid).or_default();
        match clients.iter_mut().find(|c| c.client == found.client) {
            Some(known) => {
                known.vram = known.vram.max(found.vram);
                known.gtt = known.gtt.max(found.gtt);
                known.system = known.system.max(found.system);
            }
            None => clients.push(found),
        }
    }
    by_pid
}

/// The background collector's pass: read every `<proc_root>/<pid>/fdinfo/<fd>` and group the DRM clients by process.
/// Descriptors that cannot be read (the process exited, or another user's process) are skipped, and entries whose
/// name is not a process id are ignored. `proc_root` is `/proc` in the shell and a temporary directory in tests.
pub fn scan_proc(proc_root: &std::path::Path) -> BTreeMap<u32, Vec<ClientMemory>> {
    let mut texts: Vec<(u32, String)> = Vec::new();
    let Ok(procs) = std::fs::read_dir(proc_root) else { return BTreeMap::new() };
    for proc_entry in procs.flatten() {
        let Some(pid) = proc_entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = std::fs::read_dir(proc_entry.path().join("fdinfo")) else { continue };
        for fd in fds.flatten() {
            if let Ok(text) = std::fs::read_to_string(fd.path()) {
                texts.push((pid, text));
            }
        }
    }
    let refs: Vec<(u32, &str)> = texts.iter().map(|(pid, text)| (*pid, text.as_str())).collect();
    collect(&refs)
}

/// The processes holding video or system memory through the GPU, as `(pid, bytes)`, largest first. Ties sort by
/// pid. Processes with no GPU memory are left out.
pub fn ranked(by_pid: &BTreeMap<u32, Vec<ClientMemory>>) -> Vec<(u32, u64)> {
    let mut rows: Vec<(u32, u64)> = by_pid
        .iter()
        .map(|(pid, clients)| (*pid, clients.iter().fold(0u64, |sum, c| sum.saturating_add(c.total()))))
        .filter(|(_, bytes)| *bytes > 0)
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &str = "pos:\t0\ndrm-driver:\tamdgpu\ndrm-client-id:\t12\ndrm-pdev:\t0000:03:00.0\ndrm-memory-vram:\t8 KiB\ndrm-memory-gtt:\t1 MiB\ndrm-memory-cpu:\t2048\ndrm-resident-vram:\t4 KiB\n";

    #[test]
    fn sizes_take_the_kernel_units() {
        assert_eq!(parse_size("8 KiB"), Some(8192));
        assert_eq!(parse_size("1 MiB"), Some(1 << 20));
        assert_eq!(parse_size("2GiB"), Some(2 << 30));
        assert_eq!(parse_size("2048"), Some(2048));
        assert_eq!(parse_size("many"), None);
        assert_eq!(parse_size("18446744073709551615 GiB"), None);
    }

    #[test]
    fn a_drm_client_reports_each_region_and_ignores_resident_memory() {
        let c = parse_fdinfo(CLIENT).expect("a DRM client");
        assert_eq!(c.client, "0000:03:00.0/12");
        assert_eq!((c.vram, c.gtt, c.system), (8192, 1 << 20, 2048));
        assert_eq!(c.total(), 8192 + (1 << 20) + 2048);
        assert_eq!(parse_fdinfo("pos:\t0\nflags:\t02\n"), None);
    }

    #[test]
    fn one_client_seen_through_two_descriptors_counts_once() {
        let map = collect(&[(40, CLIENT), (40, CLIENT)]);
        assert_eq!(map[&40].len(), 1);
        assert_eq!(map[&40][0].vram, 8192);
    }

    #[test]
    fn two_clients_of_one_process_add_up_in_the_ranking() {
        let second = "drm-driver:\tamdgpu\ndrm-client-id:\t13\ndrm-pdev:\t0000:03:00.0\ndrm-memory-vram:\t1 MiB\n";
        let map = collect(&[(40, CLIENT), (40, second), (7, "drm-driver:\ti915\ndrm-client-id:\t1\n")]);
        assert_eq!(map[&40].len(), 2);
        let rows = ranked(&map);
        assert_eq!(rows[0].0, 40);
        assert_eq!(rows[0].1, 8192 + (1 << 20) + 2048 + (1 << 20));
        assert_eq!(rows.len(), 1, "pid 7 holds no memory and is left out");
    }

    #[test]
    fn the_scan_reads_each_process_fdinfo_and_skips_what_is_not_a_process() {
        let root = tempfile::tempdir().unwrap();
        let write = |pid: &str, fd: &str, text: &str| {
            let dir = root.path().join(pid).join("fdinfo");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(fd), text).unwrap();
        };
        write("40", "5", CLIENT);
        write("40", "6", CLIENT);
        write("41", "3", "pos:\t0\nflags:\t02\n");
        write("self", "3", CLIENT);
        let map = scan_proc(root.path());
        assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec![40], "one client, counted once, for pid 40 only");
        assert_eq!(map[&40][0].vram, 8192);
        assert!(scan_proc(&root.path().join("missing")).is_empty());
    }

    #[test]
    fn a_process_without_gpu_memory_is_left_out_of_the_ranking() {
        let idle = "drm-driver:\tamdgpu\ndrm-client-id:\t3\ndrm-pdev:\t0000:03:00.0\n";
        let map = collect(&[(5, idle), (6, "not a drm descriptor")]);
        assert_eq!(map[&5][0].total(), 0);
        assert!(!map.contains_key(&6));
        assert!(ranked(&map).is_empty());
    }
}
