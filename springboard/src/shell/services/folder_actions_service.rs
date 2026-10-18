//! The `folder-actions` service: the setup side of Folder Actions (`shell::folder_actions`). Only the Folder Actions
//! Setup app ([`SETUP_APP`]) may call it, because an attached action runs an app's intent on every file that lands in a
//! folder, so any other app could use it to start work in a third app.
//!
//! Methods (JSON parameters):
//! - `list {}` answers `{actions, available}`. `actions` is every attachment as `{folder, app, intent}`; `available` is
//!   every intent an installed app offers for a folder (see [`available_actions`]) as `{app, name, intent, title}`.
//! - `attach {folder, app, intent}` attaches the intent to an existing folder; answers `{attached}` (false when it was
//!   already attached). The folder must be an absolute path of a directory and the pair must be in `available`.
//! - `detach {folder, app, intent}` answers `{detached}`.
//! - `detach_folder {folder}` removes every action on the folder; answers `{removed}`.
//!
//! Every change is saved at once (`Watcher::save`), so the next start finds it.
use super::{Shell, SystemService};
use crate::shell::apphost::{self, Installed};
use crate::shell::folder_actions::{Attachment, Watcher};
use bifrost_core::json::Json;
use std::path::Path;

/// The service name.
pub const SERVICE: &str = "folder-actions";
/// The one app that may call the service.
pub const SETUP_APP: &str = "is.olibuijr.folderactions.desktop";

/// The `folder-actions` service.
pub struct FolderActions;

impl SystemService for FolderActions {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        if app_id != SETUP_APP {
            return Err("folder-actions is only for the Folder Actions Setup app".to_string());
        }
        let available = available_actions(&apphost::installed());
        answer(&mut shell.folder_actions, &apphost::root(), method, params, &available)
    }
}

/// One intent an installed app offers for a folder action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub app_id: String,
    pub app_name: String,
    pub intent_id: String,
    pub title: String,
}

/// The name of an intent parameter written `name:type` (with a trailing `?` when optional).
fn param_name(p: &str) -> &str {
    p.split(':').next().unwrap_or(p)
}

/// The intents that can be a folder action: ones that take the new file's name (a parameter called `file`) or take no
/// parameter at all, with every other parameter optional, because the shell passes only `file`. An intent whose id the
/// attachment file cannot hold is left out.
pub fn available_actions(apps: &[Installed]) -> Vec<Available> {
    let mut out = Vec::new();
    for app in apps {
        for i in &app.manifest.intents {
            let takes_file = i.params.iter().any(|p| param_name(p) == "file");
            let rest_optional = i.params.iter().filter(|p| param_name(p) != "file").all(|p| p.ends_with('?'));
            let offered = (takes_file || i.params.is_empty()) && rest_optional && !i.title.is_empty();
            if offered && Attachment::new(Path::new("/"), &app.manifest.id, &i.id).is_some() {
                out.push(Available { app_id: app.manifest.id.clone(), app_name: app.manifest.name.clone(), intent_id: i.id.clone(), title: i.title.clone() });
            }
        }
    }
    out
}

/// Runs one method against `watcher`, saving to `root` after a change. Split out so the tests need no shell.
pub fn answer(watcher: &mut Watcher, root: &Path, method: &str, params: &Json, available: &[Available]) -> Result<Json, String> {
    match method {
        "list" => {
            let actions = watcher
                .registry()
                .folders()
                .into_iter()
                .flat_map(|folder| {
                    watcher
                        .registry()
                        .for_folder(&folder)
                        .map(|a| {
                            Json::Obj(vec![
                                ("folder".into(), Json::Str(a.folder.to_string_lossy().into_owned())),
                                ("app".into(), Json::Str(a.app_id.clone())),
                                ("intent".into(), Json::Str(a.intent_id.clone())),
                            ])
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            let offered = available
                .iter()
                .map(|a| {
                    Json::Obj(vec![
                        ("app".into(), Json::Str(a.app_id.clone())),
                        ("name".into(), Json::Str(a.app_name.clone())),
                        ("intent".into(), Json::Str(a.intent_id.clone())),
                        ("title".into(), Json::Str(a.title.clone())),
                    ])
                })
                .collect();
            Ok(Json::Obj(vec![("actions".into(), Json::Arr(actions)), ("available".into(), Json::Arr(offered))]))
        }
        "attach" => {
            let (folder, app, intent) = triple(params)?;
            if !Path::new(&folder).is_dir() {
                return Err(format!("{folder} is not a folder"));
            }
            if !available.iter().any(|a| a.app_id == app && a.intent_id == intent) {
                return Err(format!("{app} offers no folder action {intent}"));
            }
            let attachment = Attachment::new(Path::new(&folder), &app, &intent).ok_or("the folder must be an absolute path without tabs or line breaks")?;
            let attached = watcher.attach(attachment);
            watcher.save(root).map_err(|e| e.to_string())?;
            Ok(Json::Obj(vec![("attached".into(), Json::Bool(attached))]))
        }
        "detach" => {
            let (folder, app, intent) = triple(params)?;
            let attachment = Attachment::new(Path::new(&folder), &app, &intent).ok_or("not a valid folder action")?;
            let detached = watcher.detach(&attachment);
            watcher.save(root).map_err(|e| e.to_string())?;
            Ok(Json::Obj(vec![("detached".into(), Json::Bool(detached))]))
        }
        "detach_folder" => {
            let folder = text(params, "folder")?;
            let removed = watcher.detach_folder(Path::new(&folder));
            watcher.save(root).map_err(|e| e.to_string())?;
            Ok(Json::Obj(vec![("removed".into(), Json::Num(removed as f64))]))
        }
        _ => Err(format!("folder-actions has no method {method}")),
    }
}

fn text(params: &Json, key: &str) -> Result<String, String> {
    params.str_field(key).ok_or_else(|| format!("{key} must be a string"))
}

fn triple(params: &Json) -> Result<(String, String, String), String> {
    Ok((text(params, "folder")?, text(params, "app")?, text(params, "intent")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::folder_actions::Registry;

    fn obj(pairs: Vec<(&str, &str)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), Json::Str(v.to_string()))).collect())
    }

    fn avail() -> Vec<Available> {
        vec![Available { app_id: "a.notes".into(), app_name: "Notes".into(), intent_id: "import".into(), title: "Import File".into() }]
    }

    fn count(j: &Json, key: &str) -> usize {
        j.get(key).and_then(Json::as_array).map_or(0, |a| a.len())
    }

    #[test]
    fn attach_list_detach_and_detach_folder_are_saved_each_time() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let f = folder.path().to_string_lossy().into_owned();
        let mut w = Watcher::new(Registry::new());
        let attach = obj(vec![("folder", f.as_str()), ("app", "a.notes"), ("intent", "import")]);
        assert_eq!(answer(&mut w, root.path(), "attach", &attach, &avail()).unwrap().get("attached"), Some(&Json::Bool(true)));
        assert_eq!(answer(&mut w, root.path(), "attach", &attach, &avail()).unwrap().get("attached"), Some(&Json::Bool(false)), "already attached");
        let saved = std::fs::read_to_string(crate::shell::folder_actions::file_path(root.path())).unwrap();
        assert_eq!(saved, format!("{f}\ta.notes\timport\n"));
        let listed = answer(&mut w, root.path(), "list", &Json::Null, &avail()).unwrap();
        assert_eq!((count(&listed, "actions"), count(&listed, "available")), (1, 1));
        assert_eq!(answer(&mut w, root.path(), "detach", &attach, &avail()).unwrap().get("detached"), Some(&Json::Bool(true)));
        answer(&mut w, root.path(), "attach", &attach, &avail()).unwrap();
        let by_folder = obj(vec![("folder", f.as_str())]);
        assert_eq!(answer(&mut w, root.path(), "detach_folder", &by_folder, &avail()).unwrap().get("removed"), Some(&Json::Num(1.0)));
        assert_eq!(std::fs::read_to_string(crate::shell::folder_actions::file_path(root.path())).unwrap(), "");
    }

    #[test]
    fn a_missing_folder_an_unlisted_action_and_unknown_methods_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let mut w = Watcher::new(Registry::new());
        let gone = obj(vec![("folder", "/no/such/folder"), ("app", "a.notes"), ("intent", "import")]);
        assert!(answer(&mut w, root.path(), "attach", &gone, &avail()).unwrap_err().contains("not a folder"));
        let f = folder.path().to_string_lossy().into_owned();
        let other = obj(vec![("folder", f.as_str()), ("app", "a.notes"), ("intent", "shout")]);
        assert!(answer(&mut w, root.path(), "attach", &other, &avail()).unwrap_err().contains("offers no folder action"));
        assert!(answer(&mut w, root.path(), "attach", &obj(vec![("folder", f.as_str())]), &avail()).is_err());
        assert!(answer(&mut w, root.path(), "fly", &Json::Null, &avail()).is_err());
        assert_eq!(w.registry().folders().len(), 0, "nothing was attached");
    }

    #[test]
    fn only_intents_that_take_a_file_or_nothing_are_offered() {
        let manifest = "id = \"a.notes\"\nname = \"Notes\"\nexec = \"x\"\n\
[[intent]]\nid = \"import\"\ntitle = \"Import File\"\nparams = [\"file:string\"]\n\
[[intent]]\nid = \"touch\"\ntitle = \"Touch\"\nparams = []\n\
[[intent]]\nid = \"find\"\ntitle = \"Find\"\nparams = [\"text:string\"]\n\
[[intent]]\nid = \"tag\"\ntitle = \"Tag\"\nparams = [\"file:string\", \"label:string?\"]\n\
[[intent]]\nid = \"bad id\"\ntitle = \"Bad\"\nparams = []\n";
        let apps = [Installed { manifest: crate::shell::apphost::Manifest::parse(manifest).unwrap(), dir: std::path::PathBuf::new() }];
        let ids: Vec<String> = available_actions(&apps).into_iter().map(|a| a.intent_id).collect();
        assert_eq!(ids, ["import", "touch", "tag"]);
    }

    #[test]
    fn the_service_has_the_name_apps_call() {
        assert_eq!(FolderActions.name(), "folder-actions");
        assert_eq!(SETUP_APP, "is.olibuijr.folderactions.desktop");
    }
}
