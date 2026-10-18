//! An app's own Screen Time usage: how many foreground seconds the app itself had on a day. The shell's usage tracker
//! writes one `key=value` file per day under `screentime/` in the shell root. An app reads only its own total from
//! that file. It never sees other apps' seconds, the hourly split, pickups or notifications, so an app cannot learn
//! how the person uses the rest of the device.

use bifrost_ui::os_services::usage_record::UsageDay;
use std::io;
use std::path::{Path, PathBuf};

/// The directory of the per-day usage files under the shell's default root.
pub fn system_usage_dir() -> PathBuf {
    Path::new(crate::screen_time::SYSTEM_ROOT).join("screentime")
}

/// The seconds `app_key` was in the foreground on `day` (days since the Unix epoch, local). `app_key` is the app's
/// lowercase name, as the tracker writes it. A day with no file, or an app with no entry, has zero seconds.
pub fn own_seconds(dir: &Path, day: i64, app_key: &str) -> io::Result<u32> {
    match std::fs::read_to_string(dir.join(format!("{day}.txt"))) {
        Ok(text) => Ok(UsageDay::parse(&text).secs.get(app_key).copied().unwrap_or(0)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-screen-usage")).join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_app_reads_only_its_own_seconds() {
        let dir = scratch("own");
        std::fs::write(dir.join("20000.txt"), "pickups=3\napp.terminal=90\napp.photos=4000\nhour.9=50\n").unwrap();
        assert_eq!(own_seconds(&dir, 20000, "terminal").unwrap(), 90);
        assert_eq!(own_seconds(&dir, 20000, "files").unwrap(), 0, "an app with no entry has no time");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_day_with_no_file_has_no_time() {
        let dir = scratch("missing");
        assert_eq!(own_seconds(&dir, 19999, "terminal").unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_system_directory_sits_under_the_shell_root() {
        assert_eq!(system_usage_dir(), Path::new("/var/lib/springboard/screentime"));
    }
}
