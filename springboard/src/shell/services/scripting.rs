//! The `scripting` service: which installed apps are scriptable, and their dictionaries (the `[[script.command]]` and
//! `[[script.class]]` tables of manifest.toml, read by `bifrost_core::script::dictionary`). Methods:
//! - `list {}` returns `{apps: [{id, name, commands, classes}]}` for the installed apps that declare a dictionary.
//! - `dictionary {app}` returns that app's `{commands, classes}`; an app without a dictionary, or not installed, is an error.
//!
//! Sending a command is not a service call: `osascript` resolves the phrase against this dictionary and sends the
//! AppEvent straight to the app's control socket (`bifrost_app::app_events`).
use super::{Shell, SystemService};
use crate::shell::apphost::installed;
use bifrost_core::json::Json;

/// The `scripting` service.
pub struct Scripting;

impl SystemService for Scripting {
    fn name(&self) -> &'static str {
        "scripting"
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let apps = installed();
        match method {
            "list" => {
                let rows = apps
                    .iter()
                    .filter(|a| !a.manifest.scripting.is_empty())
                    .map(|a| {
                        let d = &a.manifest.scripting;
                        Json::Obj(vec![
                            ("id".into(), Json::Str(a.manifest.id.clone())),
                            ("name".into(), Json::Str(a.manifest.name.clone())),
                            ("commands".into(), Json::Num(d.commands.len() as f64)),
                            ("classes".into(), Json::Num(d.classes.len() as f64)),
                        ])
                    })
                    .collect();
                Ok(Json::Obj(vec![("apps".into(), Json::Arr(rows))]))
            }
            "dictionary" => {
                let app = params.str_field("app").ok_or("scripting: dictionary needs an app")?;
                let found = apps.iter().find(|a| a.manifest.id == app).ok_or_else(|| format!("scripting: no installed app {app}"))?;
                if found.manifest.scripting.is_empty() {
                    return Err(format!("scripting: {app} declares no dictionary"));
                }
                Ok(found.manifest.scripting.to_json())
            }
            _ => Err(format!("scripting has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_is_registered_under_scripting() {
        assert!(super::super::names(&super::super::all()).contains(&"scripting"));
    }

    #[test]
    fn a_manifest_dictionary_is_read_with_the_manifest() {
        let text = "id = \"is.example.a\"\nname = \"A\"\nexec = \"a\"\n[[script.command]]\nname = \"x\"\nevent = \"a.x\"\n";
        let m = crate::shell::apphost::Manifest::parse(text).expect("a manifest with a dictionary parses");
        assert_eq!(m.name, "A", "the script tables do not overwrite the manifest name");
        assert_eq!(m.scripting.command("x").map(|c| c.method()), Some("ae.a.x".to_string()));
    }
}
