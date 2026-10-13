//! Service Management items shipped inside an app bundle: the embedded login-item helpers (`Contents/Library/LoginItems`),
//! and the launch agent and launch daemon property lists (`Contents/Library/LaunchAgents`, `Contents/Library/LaunchDaemons`).
//!
//! This is the pure lookup behind `SMAppService.loginItem(identifier:)`, `agent(plistName:)` and `daemon(plistName:)`. The
//! caller lists the files of the app bundle and reads each helper's bundle identifier; nothing here touches the disk. The
//! handles in `service_management` do not use it yet: they still answer `NotFound` and `JobPlistNotFound`, because no
//! bundle is scanned at launch. Wiring it in needs the shell to report the enabled helpers, which it does not do yet.

use crate::service_management::{valid_plist_name, ServiceError, Status};

/// The directory of embedded login-item helpers, relative to the app bundle root.
pub const LOGIN_ITEMS_DIR: &str = "Contents/Library/LoginItems";
/// The directory of launch agent property lists, relative to the app bundle root.
pub const LAUNCH_AGENTS_DIR: &str = "Contents/Library/LaunchAgents";
/// The directory of launch daemon property lists, relative to the app bundle root.
pub const LAUNCH_DAEMONS_DIR: &str = "Contents/Library/LaunchDaemons";

/// The kind of job a property list describes. Login items are helper applications; agents and daemons are plists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Agent,
    Daemon,
}

impl JobKind {
    /// The bundle directory that holds this kind's property lists.
    pub fn dir(self) -> &'static str {
        match self {
            JobKind::Agent => LAUNCH_AGENTS_DIR,
            JobKind::Daemon => LAUNCH_DAEMONS_DIR,
        }
    }
}

/// An embedded login-item helper: its path in the app bundle and the bundle identifier it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperItem {
    /// Path relative to the app bundle root, such as `Contents/Library/LoginItems/Updater.app`.
    pub path: String,
    /// The `CFBundleIdentifier` of the helper, the identifier `loginItem(identifier:)` names.
    pub bundle_id: String,
}

/// Builds the helper entry for one file of the app bundle. `None` unless the path is a `.app` directly inside
/// [`LOGIN_ITEMS_DIR`], has no parent step, and the helper declares a non-empty identifier.
pub fn helper_from_entry(path: &str, bundle_id: Option<&str>) -> Option<HelperItem> {
    let bundle_id = bundle_id?.trim();
    if bundle_id.is_empty() || !helper_path_ok(path) {
        return None;
    }
    Some(HelperItem { path: path.to_string(), bundle_id: bundle_id.to_string() })
}

/// True when `path` names an application bundle directly inside the login-item directory.
pub fn helper_path_ok(path: &str) -> bool {
    let Some(name) = path.strip_prefix(LOGIN_ITEMS_DIR).and_then(|rest| rest.strip_prefix('/')) else {
        return false;
    };
    name.len() > ".app".len() && name.ends_with(".app") && !name.contains('/') && !name.contains("..")
}

/// `SMAppService.loginItem(identifier:)` lookup: the helper whose bundle identifier is `identifier`. An empty identifier,
/// or one that names no embedded helper, is `JobNotFound`, the error the reference gives for an unknown item.
pub fn find_login_item<'a>(identifier: &str, items: &'a [HelperItem]) -> Result<&'a HelperItem, ServiceError> {
    let id = identifier.trim();
    if id.is_empty() || id.contains('/') {
        return Err(ServiceError::JobNotFound);
    }
    items.iter().find(|item| item.bundle_id == id).ok_or(ServiceError::JobNotFound)
}

/// The status of an embedded login item: `NotFound` when no helper has that identifier, otherwise `Enabled` when the
/// caller reports it enabled (`enabled` lists the identifiers the user has switched on) and `NotRegistered` otherwise.
pub fn helper_status(identifier: &str, items: &[HelperItem], enabled: &[&str]) -> Status {
    match find_login_item(identifier, items) {
        Err(_) => Status::NotFound,
        Ok(item) if enabled.contains(&item.bundle_id.as_str()) => Status::Enabled,
        Ok(_) => Status::NotRegistered,
    }
}

/// `SMAppService.agent(plistName:)` and `daemon(plistName:)` lookup. The name must be a `.plist` name (as
/// `service_management` checks it); the result is the bundle path of the property list when the bundle's file list
/// (`bundle_files`, paths relative to the app bundle root) holds it. A name that is not a plist name is `InvalidPlist`,
/// and a plist the bundle does not ship is `JobPlistNotFound`.
pub fn locate_job_plist(kind: JobKind, plist_name: &str, bundle_files: &[&str]) -> Result<String, ServiceError> {
    if !valid_plist_name(plist_name) {
        return Err(ServiceError::InvalidPlist);
    }
    let path = format!("{}/{}", kind.dir(), plist_name);
    if bundle_files.iter().any(|f| *f == path) {
        Ok(path)
    } else {
        Err(ServiceError::JobPlistNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(id: &str) -> HelperItem {
        HelperItem { path: format!("{LOGIN_ITEMS_DIR}/{id}.app"), bundle_id: id.to_string() }
    }

    #[test]
    fn a_helper_must_be_an_app_directly_in_the_login_item_directory() {
        assert!(helper_path_ok("Contents/Library/LoginItems/Updater.app"));
        assert!(!helper_path_ok("Contents/Library/LoginItems/Updater"), "no app suffix");
        assert!(!helper_path_ok("Contents/Library/LoginItems/.app"), "no name before the suffix");
        assert!(!helper_path_ok("Contents/Library/LoginItems/nested/Updater.app"), "nested directory");
        assert!(!helper_path_ok("Contents/Library/LoginItems/../Updater.app"), "parent step");
        assert!(!helper_path_ok("Contents/Library/LaunchAgents/Updater.app"), "other directory");
        assert!(!helper_path_ok("/Contents/Library/LoginItems/Updater.app"), "not relative to the bundle");
    }

    #[test]
    fn an_entry_needs_a_path_and_a_non_empty_identifier() {
        let item = helper_from_entry("Contents/Library/LoginItems/Updater.app", Some(" org.example.updater ")).unwrap();
        assert_eq!(item.bundle_id, "org.example.updater");
        assert_eq!(item.path, "Contents/Library/LoginItems/Updater.app");
        assert_eq!(helper_from_entry("Contents/Library/LoginItems/Updater.app", None), None);
        assert_eq!(helper_from_entry("Contents/Library/LoginItems/Updater.app", Some("  ")), None);
        assert_eq!(helper_from_entry("Contents/Resources/Updater.app", Some("org.example.updater")), None);
    }

    #[test]
    fn the_identifier_finds_its_helper() {
        let items = vec![helper("org.example.updater"), helper("org.example.sync")];
        assert_eq!(find_login_item("org.example.sync", &items).unwrap().path, "Contents/Library/LoginItems/org.example.sync.app");
        assert_eq!(find_login_item("org.example.missing", &items), Err(ServiceError::JobNotFound));
        assert_eq!(find_login_item("", &items), Err(ServiceError::JobNotFound));
        assert_eq!(find_login_item("org.example/updater", &items), Err(ServiceError::JobNotFound));
        assert_eq!(find_login_item("org.example.updater", &[]), Err(ServiceError::JobNotFound));
    }

    #[test]
    fn helper_status_is_not_found_unless_the_bundle_ships_it() {
        let items = vec![helper("org.example.updater")];
        assert_eq!(helper_status("org.example.gone", &items, &["org.example.updater"]), Status::NotFound);
        assert_eq!(helper_status("org.example.updater", &items, &["org.example.updater"]), Status::Enabled);
        assert_eq!(helper_status("org.example.updater", &items, &[]), Status::NotRegistered);
        assert_eq!(helper_status("org.example.updater", &items, &["org.example.other"]), Status::NotRegistered);
    }

    #[test]
    fn a_job_plist_is_found_only_in_its_own_directory() {
        let files = [
            "Contents/Library/LaunchAgents/com.example.agent.plist",
            "Contents/Library/LaunchDaemons/com.example.daemon.plist",
        ];
        assert_eq!(
            locate_job_plist(JobKind::Agent, "com.example.agent.plist", &files),
            Ok("Contents/Library/LaunchAgents/com.example.agent.plist".to_string())
        );
        assert_eq!(
            locate_job_plist(JobKind::Daemon, "com.example.daemon.plist", &files),
            Ok("Contents/Library/LaunchDaemons/com.example.daemon.plist".to_string())
        );
        assert_eq!(locate_job_plist(JobKind::Daemon, "com.example.agent.plist", &files), Err(ServiceError::JobPlistNotFound));
        assert_eq!(locate_job_plist(JobKind::Agent, "com.example.gone.plist", &files), Err(ServiceError::JobPlistNotFound));
    }

    #[test]
    fn a_job_plist_name_is_checked_before_the_bundle_is_searched() {
        let files = ["Contents/Library/LaunchAgents/com.example.agent"];
        assert_eq!(locate_job_plist(JobKind::Agent, "com.example.agent", &files), Err(ServiceError::InvalidPlist));
        assert_eq!(locate_job_plist(JobKind::Agent, "dir/job.plist", &files), Err(ServiceError::InvalidPlist));
        assert_eq!(locate_job_plist(JobKind::Daemon, ".plist", &files), Err(ServiceError::InvalidPlist));
    }

    #[test]
    fn job_kinds_point_at_their_bundle_directories() {
        assert_eq!(JobKind::Agent.dir(), "Contents/Library/LaunchAgents");
        assert_eq!(JobKind::Daemon.dir(), "Contents/Library/LaunchDaemons");
    }
}
