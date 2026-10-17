//! The `backup-exclusions` service: the items the system backup leaves out. The model is
//! `bifrost_core::backup_exclusion` (one flag per absolute path, not inherited by the items below it); the backup
//! writer (`shell::backup::create_with`) reads the same set, so an excluded file or folder is never archived.
//!
//! Methods (JSON parameters, the caller's app id is not needed: the set is the user's, not the app's):
//! - `set {path, excluded}` sets or clears the flag on one absolute path. Answers `{excluded}`, the flag now held.
//! - `is_excluded {path}` answers `{excluded}` for that exact item.
//! - `list {}` answers `{paths}`, every excluded path in path order.
//!
//! A path that is not absolute is a service error. The set is stored in the data folder
//! (`backup::EXCLUSIONS_FILE`), so it survives a restart and travels with the backups.
use super::{Shell, SystemService};
use crate::shell::backup;
use bifrost_core::json::Json;
use std::path::Path;

/// The service name.
pub const SERVICE: &str = "backup-exclusions";

/// The `backup-exclusions` service.
pub struct BackupExclusions;

impl SystemService for BackupExclusions {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(&shell.data_dir, method, params)
    }
}

/// Runs one method against the set stored in `dir`. Split out so the tests can use a scratch folder.
pub fn answer(dir: &Path, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "set" => {
            let path = text_field(params, "path")?;
            let excluded = flag_field(params, "excluded")?;
            let mut set = backup::exclusions(dir);
            set.set_excluded(&path, excluded).map_err(|e| format!("{e:?}"))?;
            backup::save_exclusions(dir, &set).map_err(|e| e.to_string())?;
            let now = set.is_excluded(&path).map_err(|e| format!("{e:?}"))?;
            Ok(Json::Obj(vec![("excluded".into(), Json::Bool(now))]))
        }
        "is_excluded" => {
            let path = text_field(params, "path")?;
            let set = backup::exclusions(dir);
            let excluded = set.is_excluded(&path).map_err(|e| format!("{e:?}"))?;
            Ok(Json::Obj(vec![("excluded".into(), Json::Bool(excluded))]))
        }
        "list" => {
            let set = backup::exclusions(dir);
            let paths = set.excluded_paths().into_iter().map(|p| Json::Str(p.to_string())).collect();
            Ok(Json::Obj(vec![("paths".into(), Json::Arr(paths))]))
        }
        _ => Err(format!("backup-exclusions has no method {method}")),
    }
}

fn text_field(params: &Json, key: &str) -> Result<String, String> {
    params.str_field(key).ok_or_else(|| format!("{key} must be a string"))
}

fn flag_field(params: &Json, key: &str) -> Result<bool, String> {
    match params.get(key) {
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err(format!("{key} must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sb-excl-svc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn set_then_read_back_and_clear() {
        let dir = scratch("set");
        let on = obj(vec![("path", Json::Str("/home/u/Notes/".into())), ("excluded", Json::Bool(true))]);
        assert_eq!(answer(&dir, "set", &on).unwrap().get("excluded"), Some(&Json::Bool(true)));
        let check = obj(vec![("path", Json::Str("/home/u/Notes".into()))]);
        assert_eq!(answer(&dir, "is_excluded", &check).unwrap().get("excluded"), Some(&Json::Bool(true)));
        let listed = answer(&dir, "list", &Json::Null).unwrap();
        assert_eq!(listed.get("paths").and_then(Json::as_array).map(|a| a.len()), Some(1));
        let off = obj(vec![("path", Json::Str("/home/u/Notes".into())), ("excluded", Json::Bool(false))]);
        assert_eq!(answer(&dir, "set", &off).unwrap().get("excluded"), Some(&Json::Bool(false)));
        assert_eq!(answer(&dir, "list", &Json::Null).unwrap().get("paths").and_then(Json::as_array).map(|a| a.len()), Some(0));
    }

    #[test]
    fn relative_paths_and_bad_flags_are_refused() {
        let dir = scratch("bad");
        let rel = obj(vec![("path", Json::Str("Notes".into())), ("excluded", Json::Bool(true))]);
        assert!(answer(&dir, "set", &rel).is_err());
        let flag = obj(vec![("path", Json::Str("/x".into())), ("excluded", Json::Str("yes".into()))]);
        assert!(answer(&dir, "set", &flag).is_err());
        assert!(answer(&dir, "nope", &Json::Null).is_err());
    }
}
