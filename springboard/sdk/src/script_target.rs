//! Attaching to a running scriptable app: the target is named by bundle identifier, process id or bundle URL, and the
//! scripting handle is made for that app. The lookup is pure. The caller passes the apps the shell reports as running and gets
//! back the one app the locator names. [`socket_path`] gives where that app's control socket lives, and the caller
//! connects with [`crate::control`].
//!
//! Differences from the native API: a locator that matches no running app is an error rather than a nil object, and
//! the shell's running list carries only app identifiers today. Process ids and bundle URLs are optional in
//! [`RunningApp`], so a locator on them matches only when the caller knows them.

use std::fmt;
use std::path::{Path, PathBuf};

/// How the caller names the app to attach to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Locator {
    /// The bundle identifier, for example `is.example.notes`.
    BundleId(String),
    /// The process id of the running app. Zero is not a process id.
    Pid(u32),
    /// The bundle URL of the installed app, a `file://` URL. A trailing slash does not matter.
    Url(String),
}

/// An app the shell reports as running. `pid` and `url` are `None` when the shell did not report them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningApp {
    pub id: String,
    pub pid: Option<u32>,
    pub url: Option<String>,
}

/// The running app a locator resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    pub pid: Option<u32>,
    pub url: Option<String>,
}

/// Why a locator did not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachError {
    /// The locator is empty or a zero process id, so it can name no app.
    Invalid(Locator),
    /// No running app matches the locator.
    NotRunning(Locator),
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachError::Invalid(locator) => write!(f, "invalid target: {}", describe(locator)),
            AttachError::NotRunning(locator) => write!(f, "no running app matches {}", describe(locator)),
        }
    }
}

fn describe(locator: &Locator) -> String {
    match locator {
        Locator::BundleId(id) => format!("bundle id \"{id}\""),
        Locator::Pid(pid) => format!("process id {pid}"),
        Locator::Url(url) => format!("URL \"{url}\""),
    }
}

/// Removes trailing slashes so that `file:///apps/notes/` and `file:///apps/notes` name the same bundle.
fn normal_url(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Resolves `locator` against the running apps. The first app in `running` that matches wins, so the shell's list
/// order decides between two instances that share a bundle id.
pub fn attach(locator: &Locator, running: &[RunningApp]) -> Result<Target, AttachError> {
    let invalid = match locator {
        Locator::BundleId(id) => id.is_empty(),
        Locator::Pid(pid) => *pid == 0,
        Locator::Url(url) => normal_url(url).is_empty(),
    };
    if invalid {
        return Err(AttachError::Invalid(locator.clone()));
    }
    running
        .iter()
        .find(|app| matches(locator, app))
        .map(|app| Target { id: app.id.clone(), pid: app.pid, url: app.url.clone() })
        .ok_or_else(|| AttachError::NotRunning(locator.clone()))
}

fn matches(locator: &Locator, app: &RunningApp) -> bool {
    match locator {
        Locator::BundleId(id) => app.id == *id,
        Locator::Pid(pid) => app.pid == Some(*pid),
        Locator::Url(url) => app.url.as_deref().is_some_and(|u| normal_url(u) == normal_url(url)),
    }
}

/// The control socket of app `app_id`. The `explicit` path wins, else `runtime_dir/bifrost/apps/<id>.sock`, else
/// `control.sock` in the app's data directory. The shell and the SDK runtime both use this rule.
pub fn socket_path(app_id: &str, explicit: Option<&Path>, runtime_dir: Option<&Path>, data_dir: &Path) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    match runtime_dir {
        Some(dir) => dir.join("bifrost").join("apps").join(format!("{app_id}.sock")),
        None => data_dir.join("control.sock"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, pid: Option<u32>, url: Option<&str>) -> RunningApp {
        RunningApp { id: id.into(), pid, url: url.map(String::from) }
    }

    fn sample() -> Vec<RunningApp> {
        vec![
            app("is.example.notes", Some(412), Some("file:///apps/notes/")),
            app("is.example.files", Some(977), Some("file:///apps/files")),
            app("is.example.notes", Some(1200), Some("file:///apps/notes-2")),
        ]
    }

    #[test]
    fn a_bundle_id_resolves_to_the_first_running_instance() {
        let t = attach(&Locator::BundleId("is.example.notes".into()), &sample()).unwrap();
        assert_eq!(t.id, "is.example.notes");
        assert_eq!(t.pid, Some(412));
    }

    #[test]
    fn a_process_id_resolves_to_that_process_only() {
        let t = attach(&Locator::Pid(1200), &sample()).unwrap();
        assert_eq!(t.url.as_deref(), Some("file:///apps/notes-2"));
        assert_eq!(attach(&Locator::Pid(5), &sample()), Err(AttachError::NotRunning(Locator::Pid(5))));
    }

    #[test]
    fn a_bundle_url_matches_with_or_without_its_trailing_slash() {
        let with_slash = attach(&Locator::Url("file:///apps/notes".into()), &sample()).unwrap();
        assert_eq!(with_slash.pid, Some(412));
        let without = attach(&Locator::Url("file:///apps/files/".into()), &sample()).unwrap();
        assert_eq!(without.id, "is.example.files");
    }

    #[test]
    fn an_app_that_did_not_report_a_pid_or_url_is_matched_by_id_only() {
        let running = vec![app("is.example.clock", None, None)];
        assert_eq!(attach(&Locator::Pid(9), &running), Err(AttachError::NotRunning(Locator::Pid(9))));
        assert_eq!(attach(&Locator::Url("file:///x".into()), &running), Err(AttachError::NotRunning(Locator::Url("file:///x".into()))));
        assert_eq!(attach(&Locator::BundleId("is.example.clock".into()), &running).unwrap().pid, None);
    }

    #[test]
    fn an_unknown_bundle_id_is_not_running() {
        let err = attach(&Locator::BundleId("is.example.absent".into()), &sample()).unwrap_err();
        assert_eq!(err, AttachError::NotRunning(Locator::BundleId("is.example.absent".into())));
        assert_eq!(err.to_string(), "no running app matches bundle id \"is.example.absent\"");
    }

    #[test]
    fn an_empty_bundle_id_or_url_and_a_zero_pid_are_invalid() {
        assert_eq!(attach(&Locator::BundleId(String::new()), &sample()), Err(AttachError::Invalid(Locator::BundleId(String::new()))));
        assert_eq!(attach(&Locator::Url("/".into()), &sample()), Err(AttachError::Invalid(Locator::Url("/".into()))));
        assert_eq!(attach(&Locator::Pid(0), &sample()), Err(AttachError::Invalid(Locator::Pid(0))));
        assert_eq!(AttachError::Invalid(Locator::Pid(0)).to_string(), "invalid target: process id 0");
    }

    #[test]
    fn an_empty_running_list_resolves_nothing() {
        assert!(matches!(attach(&Locator::Pid(412), &[]), Err(AttachError::NotRunning(_))));
    }

    #[test]
    fn an_explicit_socket_path_wins_over_the_runtime_directory() {
        let explicit = Path::new("/run/custom/notes.sock");
        let p = socket_path("is.example.notes", Some(explicit), Some(Path::new("/run/user/1000")), Path::new("/data"));
        assert_eq!(p, PathBuf::from("/run/custom/notes.sock"));
    }

    #[test]
    fn the_socket_lives_under_the_runtime_directory_when_there_is_one() {
        let p = socket_path("is.example.notes", None, Some(Path::new("/run/user/1000")), Path::new("/data"));
        assert_eq!(p, PathBuf::from("/run/user/1000/bifrost/apps/is.example.notes.sock"));
    }

    #[test]
    fn without_a_runtime_directory_the_socket_is_in_the_data_directory() {
        let p = socket_path("is.example.notes", None, None, Path::new("/home/u/.data/notes"));
        assert_eq!(p, PathBuf::from("/home/u/.data/notes/control.sock"));
    }
}
