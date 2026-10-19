//! Load the pinned engine archive into the per-user cache.
use super::*;

/// Unpacks the bundled engine once per version into the user's cache and returns its binary.
pub(super) fn engine() -> io::Result<PathBuf> {
    let package = std::env::current_exe()?.canonicalize()?.parent().map(Path::to_path_buf).ok_or_else(|| io::Error::other("no package dir"))?;
    let cache = home().join(".cache/bifrost-browser");
    fs::create_dir_all(&cache)?;
    let dir = cache.join(ENGINE);
    let ready = |d: &Path| fs::read_to_string(d.join(".ready")).is_ok_and(|r| r.trim() == ENGINE) && d.join("servo/bifrost-webview").is_file();
    if !ready(&dir) {
        // One extraction at a time: concurrent launches wait on the lock, then find the engine ready.
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(cache.join(".extract.lock"))?;
        lock.lock()?;
        // Staging dirs left by crashed launches that are over a day old.
        let day = std::time::Duration::from_secs(86_400);
        for e in fs::read_dir(&cache)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > day);
            if name.starts_with(&format!("{ENGINE}.")) && old {
                let _ = fs::remove_dir_all(e.path());
            }
        }
        if !ready(&dir) {
            let _ = fs::remove_dir_all(&dir); // a partial or mismatched cache
            let staging = cache.join(format!("{ENGINE}.{}", std::process::id()));
            let _ = fs::remove_dir_all(&staging);
            fs::create_dir(&staging)?;
            let ok = Command::new("tar")
                .args(["--extract", "--gzip", "--no-same-owner", "--file"])
                .arg(package.join("assets/servo.tar.gz"))
                .arg("--directory")
                .arg(&staging)
                .status()?
                .success();
            if !ok || !staging.join("servo/bifrost-webview").is_file() || fs::write(staging.join(".ready"), format!("{ENGINE}\n")).is_err() {
                let _ = fs::remove_dir_all(&staging);
                return Err(io::Error::other("Engine extraction failed"));
            }
            if let Err(e) = fs::rename(&staging, &dir) {
                let _ = fs::remove_dir_all(&staging);
                if !ready(&dir) {
                    return Err(e);
                }
            }
        }
    }
    Ok(dir.join("servo/bifrost-webview"))
}
