//! The `launch` service: which app opens a file or a URL scheme, for Open With and Default Apps (`shell::defaultapps`).
//! The SDK client is `bifrost_app::launch_services`. Read-only: an app cannot change the user's defaults; the user
//! chooses them in Settings > General > Default Apps.
//!
//! Methods (JSON params):
//! - `open_with` `{name}`: `{apps: [{app, name, default}]}`, every app that can open the file's type, the default first.
//! - `default_for_file` `{name}`: `{app}`, the default app for the file name (`app` is null when none).
//! - `default_for_scheme` `{scheme}`: `{app}`, the app that opens the URL scheme (null when none).
use super::SystemService;
use crate::shell::defaultapps::app_name;
use crate::shell::Shell;
use bifrost_core::json::Json;
use bifrost_ui::launch::Choice;

pub struct LaunchServices;

/// The string parameter `key`, or an error naming what the method needs.
fn text_param(params: &Json, key: &str) -> Result<String, String> {
    params.get(key).and_then(Json::as_str).map(str::to_string).ok_or_else(|| format!("needs a {key} string"))
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The `open_with` answer: the candidates in their order (the default first), with display names.
pub fn choices_json(choices: &[Choice]) -> Json {
    let apps = choices
        .iter()
        .map(|c| obj(vec![("app", Json::Str(c.app.clone())), ("name", Json::Str(app_name(&c.app))), ("default", Json::Bool(c.is_default))]))
        .collect();
    obj(vec![("apps", Json::Arr(apps))])
}

/// A single-app answer: `{app}`, null when no app is chosen.
fn app_json(app: Option<String>) -> Json {
    obj(vec![("app", app.map_or(Json::Null, Json::Str))])
}

impl SystemService for LaunchServices {
    fn name(&self) -> &'static str {
        "launch"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "open_with" => Ok(choices_json(&shell.open_with_choices(&text_param(params, "name")?))),
            "default_for_file" => Ok(app_json(shell.default_app_for_file(&text_param(params, "name")?))),
            "default_for_scheme" => Ok(app_json(shell.default_app_for_scheme(&text_param(params, "scheme")?))),
            _ => Err(format!("launch has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_with_answer_lists_apps_with_names_default_first() {
        let c = vec![Choice { app: "bifrost.photos".into(), is_default: true }, Choice { app: "ghost.app".into(), is_default: false }];
        let j = choices_json(&c);
        let apps = j.get("apps").and_then(Json::as_array).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].str_field("app").as_deref(), Some("bifrost.photos"));
        assert_eq!(apps[0].str_field("name").as_deref(), Some("Photos"), "a built-in app shows its display name");
        assert_eq!(apps[0].get("default"), Some(&Json::Bool(true)));
        assert_eq!(apps[1].str_field("name").as_deref(), Some("ghost.app"), "an unknown key is shown as it is");
        assert_eq!(apps[1].get("default"), Some(&Json::Bool(false)));
        assert_eq!(choices_json(&[]).get("apps").and_then(Json::as_array).map(<[Json]>::len), Some(0));
    }

    #[test]
    fn a_single_app_answer_is_null_when_none_is_chosen() {
        assert_eq!(app_json(None).get("app"), Some(&Json::Null));
        assert_eq!(app_json(Some("bifrost.files".into())).str_field("app").as_deref(), Some("bifrost.files"));
    }

    #[test]
    fn parameters_are_required_strings() {
        let with_name = obj(vec![("name", Json::Str("a.png".into()))]);
        assert_eq!(text_param(&with_name, "name"), Ok("a.png".to_string()));
        assert!(text_param(&obj(vec![]), "name").is_err());
        assert!(text_param(&obj(vec![("name", Json::Num(1.0))]), "name").is_err());
    }
}
