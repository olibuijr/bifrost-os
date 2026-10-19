//! Complete data removal for storage types missing from Servo 0.7's public SiteDataManager.
use std::{fs, io, path::Path};

pub fn needs_reset(profile: &Path) -> bool {
    ["clientstorage", "cachestorage", "auth_cache.json"].iter().any(|name| profile.join(name).exists())
}

/// Call only after the engine exits: its databases must not remain open while their files are removed.
/// Content rules are browser configuration and survive clearing website data.
pub fn reset(profile: &Path) -> io::Result<()> {
    let rules = profile.join("content-rules");
    let saved = profile.with_extension(format!("saved-rules-{}", std::process::id()));
    let had_rules = rules.exists();
    if had_rules { fs::rename(&rules, &saved)?; }
    let result = fs::remove_dir_all(profile);
    // Restore the configuration on failure as well, so an unsuccessful clear cannot discard it.
    fs::create_dir_all(profile)?;
    if had_rules { fs::rename(saved, rules)?; }
    match result {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_clear_preserves_rules_and_removes_unmanaged_site_storage() {
        let profile = std::env::temp_dir().join(format!("bifrost-profile-clear-{}", std::process::id()));
        fs::create_dir_all(profile.join("content-rules")).unwrap();
        fs::write(profile.join("content-rules/rules.json"), b"rules").unwrap();
        fs::create_dir_all(profile.join("clientstorage")).unwrap();
        fs::write(profile.join("clientstorage/indexeddb.sqlite"), b"site data").unwrap();
        assert!(needs_reset(&profile));
        reset(&profile).unwrap();
        assert!(!needs_reset(&profile));
        assert_eq!(fs::read(profile.join("content-rules/rules.json")).unwrap(), b"rules");
        fs::remove_dir_all(profile).unwrap();
    }
}
