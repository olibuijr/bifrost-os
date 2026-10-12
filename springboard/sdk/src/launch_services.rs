//! Open With and default apps, over the shell's `launch` service (`springboard/src/shell/services/launch_services.rs`).
//!
//! ```ignore
//! let id = launch_services::open_with(ctx, "notes.txt");
//! // in App::service_reply: launch_services::candidates(&result) gives the apps that can open it, the default first
//! // launch_services::default_app(&result) gives the single app of default_for_file / default_for_scheme
//! ```
//! Read-only: the user chooses the defaults in Settings > General > Default Apps; an app cannot change them.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "launch";
/// Every app that can open a file name, default first.
pub const METHOD_OPEN_WITH: &str = "open_with";
/// The default app for a file name.
pub const METHOD_DEFAULT_FOR_FILE: &str = "default_for_file";
/// The app that opens a URL scheme (`mailto`, `bifrost-app`).
pub const METHOD_DEFAULT_FOR_SCHEME: &str = "default_for_scheme";

/// An app that can open a file, as the Open With menu lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// the app's id (its manifest id for installed apps)
    pub app: String,
    /// the display name the shell shows
    pub name: String,
    /// true for the app the user's choice or the best declaration makes the default
    pub is_default: bool,
}

fn request(ctx: &Ctx, method: &str, key: &str, value: &str) -> u32 {
    service::call(ctx, SERVICE, method, &Json::Obj(vec![(key.to_string(), Json::Str(value.to_string()))]))
}

/// Asks which apps can open the file called `file_name`. Returns the request id the reply carries.
pub fn open_with(ctx: &Ctx, file_name: &str) -> u32 {
    request(ctx, METHOD_OPEN_WITH, "name", file_name)
}

/// Asks which app is the default for the file called `file_name`. Returns the request id.
pub fn default_for_file(ctx: &Ctx, file_name: &str) -> u32 {
    request(ctx, METHOD_DEFAULT_FOR_FILE, "name", file_name)
}

/// Asks which app opens the URL scheme `scheme` (without the colon). Returns the request id.
pub fn default_for_scheme(ctx: &Ctx, scheme: &str) -> u32 {
    request(ctx, METHOD_DEFAULT_FOR_SCHEME, "scheme", scheme)
}

/// Decodes an `open_with` reply. Errors are the shell's message.
pub fn candidates(result: &Result<Json, String>) -> Result<Vec<Candidate>, String> {
    match result {
        Ok(body) => from_candidates(body),
        Err(e) => Err(e.clone()),
    }
}

/// Decodes an `open_with` answer, `{apps: [{app, name, default}]}`. A missing name is shown as the app id.
pub fn from_candidates(v: &Json) -> Result<Vec<Candidate>, String> {
    let items = v.get("apps").and_then(Json::as_array).ok_or("launch: the answer has no apps list")?;
    items
        .iter()
        .map(|j| -> Result<Candidate, String> {
            let app = j.str_field("app").ok_or("launch: a candidate has no app")?;
            let name = j.str_field("name").unwrap_or_else(|| app.clone());
            let is_default = matches!(j.get("default"), Some(Json::Bool(true)));
            Ok(Candidate { app, name, is_default })
        })
        .collect()
}

/// Decodes a `default_for_file` or `default_for_scheme` reply: the app id, or `None` when no app opens it.
pub fn default_app(result: &Result<Json, String>) -> Result<Option<String>, String> {
    match result {
        Ok(v) => match v.get("app") {
            Some(Json::Str(app)) => Ok(Some(app.clone())),
            Some(Json::Null) | None => Ok(None),
            Some(_) => Err("launch: the answer has no app".into()),
        },
        Err(e) => Err(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn candidates_keep_the_order_and_the_default_flag() {
        let v = parsed(r#"{"apps":[{"app":"bifrost.photos","name":"Photos","default":true},{"app":"x.y","name":"X","default":false}]}"#);
        let c = from_candidates(&v).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0], Candidate { app: "bifrost.photos".into(), name: "Photos".into(), is_default: true });
        assert!(!c[1].is_default);
    }

    #[test]
    fn a_candidate_without_a_name_is_shown_as_its_id_and_is_not_the_default() {
        let c = from_candidates(&parsed(r#"{"apps":[{"app":"a.b"}]}"#)).unwrap();
        assert_eq!(c, [Candidate { app: "a.b".into(), name: "a.b".into(), is_default: false }]);
        assert!(from_candidates(&parsed(r#"{"apps":[{"name":"no id"}]}"#)).is_err());
        assert!(from_candidates(&parsed(r#"{}"#)).is_err());
        assert_eq!(from_candidates(&parsed(r#"{"apps":[]}"#)).unwrap(), []);
    }

    #[test]
    fn a_reply_error_is_passed_on() {
        assert_eq!(candidates(&Err("no service launch".into())).err().as_deref(), Some("no service launch"));
        assert_eq!(default_app(&Err("no app".into())).err().as_deref(), Some("no app"));
    }

    #[test]
    fn a_single_app_reply_is_none_when_nothing_opens_it() {
        assert_eq!(default_app(&Ok(parsed(r#"{"app":null}"#))), Ok(None));
        assert_eq!(default_app(&Ok(parsed(r#"{}"#))), Ok(None));
        assert_eq!(default_app(&Ok(parsed(r#"{"app":"bifrost.files"}"#))), Ok(Some("bifrost.files".into())));
        assert!(default_app(&Ok(parsed(r#"{"app":4}"#))).is_err());
    }
}
