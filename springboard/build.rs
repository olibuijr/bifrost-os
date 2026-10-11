//! Stamps the build with its git commit and date for Settings › General ›
//! Software Update (`SPRINGBOARD_GIT`, `SPRINGBOARD_BUILD_DATE`).

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// `YYYY-MM-DD` for a day count since 1970-01-01 (Howard Hinnant's civil-from-days).
fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    let git = Command::new("git").args(["rev-parse", "--short=12", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty());
    let dirty = Command::new("git").args(["status", "--porcelain", "--untracked-files=no", "."]).output().ok().is_some_and(|o| o.status.success() && !o.stdout.is_empty());
    let git = match git {
        Some(g) if dirty => format!("{g}+"),
        Some(g) => g,
        None => "untracked".to_string(),
    };
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86_400).unwrap_or(0);
    println!("cargo:rustc-env=SPRINGBOARD_GIT={git}");
    println!("cargo:rustc-env=SPRINGBOARD_BUILD_DATE={}", civil(days));
    println!("cargo:rerun-if-changed=build.rs");
    // HEAD contains only the symbolic branch name; commits update its ref.
    // Resolve paths through git so linked worktrees are tracked correctly too.
    for name in [Some("HEAD".to_string()), Command::new("git").args(["symbolic-ref", "-q", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())].into_iter().flatten() {
        if let Ok(output) = Command::new("git").args(["rev-parse", "--path-format=absolute", "--git-path", &name]).output() {
            if output.status.success() {
                println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&output.stdout).trim());
            }
        }
    }
}
