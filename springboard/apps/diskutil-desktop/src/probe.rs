//! The reads that need a process or the mount table: `/proc/mounts`, `df` for usage, and `smartctl -H` for SMART
//! health. Each runs a fixed program with fixed arguments; device and mount point names are passed as separate
//! arguments, never through a shell.

use crate::model::{parse_df, parse_lsblk_pairs, parse_smart, Usage};
use std::process::Command;

/// The mount table as text. Empty when it cannot be read.
pub fn read_mounts() -> String {
    std::fs::read_to_string("/proc/mounts").unwrap_or_default()
}

/// Usage for the given mount points, from `df`. Paths that do not start with `/` are dropped, so an option can never
/// be passed where a path is expected.
pub fn read_df(points: &[String]) -> Vec<(String, Usage)> {
    let paths: Vec<&String> = points.iter().filter(|p| p.starts_with('/')).collect();
    if paths.is_empty() {
        return Vec::new();
    }
    match Command::new("df").args(["-B1", "--output=target,size,used,avail", "--"]).args(&paths).output() {
        Ok(out) => parse_df(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// Every field `lsblk` knows about one device, as `KEY="value"` pairs (empty when the tool is missing or the name is
/// not a plain device name). Get Info shows the ones the scan of `/sys` does not read.
pub fn read_lsblk(name: &str) -> Vec<(String, String)> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Vec::new();
    }
    match Command::new("lsblk").args(["-d", "-P", "-O"]).arg(format!("/dev/{name}")).output() {
        Ok(out) => parse_lsblk_pairs(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// The SMART overall health of a disk, or `Not available` when the tool is missing or the drive reports nothing.
pub fn read_smart(name: &str) -> String {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return "Not available".to_string();
    }
    match Command::new("smartctl").arg("-H").arg(format!("/dev/{name}")).output() {
        Ok(out) => parse_smart(&String::from_utf8_lossy(&out.stdout)).unwrap_or_else(|| "Not available".to_string()),
        Err(_) => "Not available".to_string(),
    }
}
