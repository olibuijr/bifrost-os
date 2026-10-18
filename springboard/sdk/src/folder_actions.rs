//! Folder Actions setup: attach an app's intent to a folder so it runs for each file that lands there. A client of the
//! `folder-actions` shell service (`springboard/src/shell/services/folder_actions_service.rs`), over `service::call`.
//! The shell runs the actions (`shell::folder_actions`); this module only edits the list, and the service answers only
//! the Folder Actions Setup app.
//!
//! Each call returns a request id. The answer arrives in `App::service_reply`: [`decode_setup`] for [`list`],
//! [`decode_flag`] for [`attach`] and [`detach`], [`decode_count`] for [`detach_folder`].
//!
//! ```ignore
//! let id = folder_actions::list(ctx);
//! // later, in App::service_reply(ctx, request, result):
//! //   folder_actions::decode_setup(result)  ->  Some(Setup { actions, offers })
//! ```

use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;

/// The shell service this client calls.
pub const SERVICE: &str = "folder-actions";

/// One attachment: `intent` of `app` runs for each new file in `folder`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub folder: String,
    pub app: String,
    pub intent: String,
}

/// An intent an installed app offers for a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub app: String,
    /// The app's display name.
    pub name: String,
    pub intent: String,
    pub title: String,
}

/// The answer to [`list`]: what is attached and what could be.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Setup {
    pub actions: Vec<Action>,
    pub offers: Vec<Offer>,
}

fn text(v: &Json, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

/// The request body naming one action.
fn triple(folder: &str, app: &str, intent: &str) -> Json {
    Json::Obj(vec![
        ("folder".to_string(), Json::Str(folder.to_string())),
        ("app".to_string(), Json::Str(app.to_string())),
        ("intent".to_string(), Json::Str(intent.to_string())),
    ])
}

/// Asks for the attachments and the intents that can be attached. Answer: [`decode_setup`].
pub fn list(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "list", &Json::Null)
}

/// Attaches `intent` of `app` to the folder at the absolute path `folder`. Answer: [`decode_flag`] with `"attached"`.
pub fn attach(ctx: &Ctx, folder: &str, app: &str, intent: &str) -> u32 {
    service::call(ctx, SERVICE, "attach", &triple(folder, app, intent))
}

/// Takes one action off the folder. Answer: [`decode_flag`] with `"detached"`.
pub fn detach(ctx: &Ctx, folder: &str, app: &str, intent: &str) -> u32 {
    service::call(ctx, SERVICE, "detach", &triple(folder, app, intent))
}

/// Takes every action off the folder. Answer: [`decode_count`] with `"removed"`.
pub fn detach_folder(ctx: &Ctx, folder: &str) -> u32 {
    service::call(ctx, SERVICE, "detach_folder", &Json::Obj(vec![("folder".to_string(), Json::Str(folder.to_string()))]))
}

/// Decodes the answer to [`list`]; `None` when it is an error or not a setup. Malformed rows are skipped.
pub fn decode_setup(result: &Result<Json, String>) -> Option<Setup> {
    let body = result.as_ref().ok()?;
    let actions = body
        .get("actions")?
        .as_array()?
        .iter()
        .filter_map(|a| Some(Action { folder: text(a, "folder")?, app: text(a, "app")?, intent: text(a, "intent")? }))
        .collect();
    let offers = body
        .get("available")?
        .as_array()?
        .iter()
        .filter_map(|o| Some(Offer { app: text(o, "app")?, name: text(o, "name")?, intent: text(o, "intent")?, title: text(o, "title")? }))
        .collect();
    Some(Setup { actions, offers })
}

/// Decodes a yes-or-no answer (`key` is `"attached"` or `"detached"`). An error is the shell's message.
pub fn decode_flag(result: &Result<Json, String>, key: &str) -> Result<bool, String> {
    match result.as_ref().map_err(|e| e.clone())?.get(key) {
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err(format!("the answer has no {key}")),
    }
}

/// Decodes a count answer (`key` is `"removed"`). An error is the shell's message.
pub fn decode_count(result: &Result<Json, String>, key: &str) -> Result<usize, String> {
    match result.as_ref().map_err(|e| e.clone())?.get(key).and_then(Json::number) {
        Some(n) if n >= 0.0 => Ok(n as usize),
        _ => Err(format!("the answer has no {key}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Json {
        Json::Str(v.to_string())
    }

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn a_request_names_folder_app_and_intent() {
        let j = triple("/srv/in", "a.notes", "import");
        assert_eq!(j.get("folder"), Some(&s("/srv/in")));
        assert_eq!(j.get("app"), Some(&s("a.notes")));
        assert_eq!(j.get("intent"), Some(&s("import")));
    }

    #[test]
    fn the_setup_answer_lists_actions_and_offers_and_skips_malformed_rows() {
        let answer = Ok(obj(vec![
            ("actions", Json::Arr(vec![obj(vec![("folder", s("/srv/in")), ("app", s("a.notes")), ("intent", s("import"))]), obj(vec![("folder", s("/x"))])])),
            ("available", Json::Arr(vec![obj(vec![("app", s("a.notes")), ("name", s("Notes")), ("intent", s("import")), ("title", s("Import File"))])])),
        ]));
        let setup = decode_setup(&answer).unwrap();
        assert_eq!(setup.actions, vec![Action { folder: "/srv/in".into(), app: "a.notes".into(), intent: "import".into() }]);
        assert_eq!(setup.offers, vec![Offer { app: "a.notes".into(), name: "Notes".into(), intent: "import".into(), title: "Import File".into() }]);
        assert_eq!(decode_setup(&Err("no".into())), None);
        assert_eq!(decode_setup(&Ok(Json::Null)), None);
    }

    #[test]
    fn flags_and_counts_decode_and_errors_carry_the_shell_message() {
        assert_eq!(decode_flag(&Ok(obj(vec![("attached", Json::Bool(true))])), "attached"), Ok(true));
        assert_eq!(decode_flag(&Ok(obj(vec![("detached", Json::Bool(false))])), "detached"), Ok(false));
        assert!(decode_flag(&Ok(Json::Null), "attached").is_err());
        assert_eq!(decode_flag(&Err("not a folder".into()), "attached"), Err("not a folder".to_string()));
        assert_eq!(decode_count(&Ok(obj(vec![("removed", Json::Num(2.0))])), "removed"), Ok(2));
        assert!(decode_count(&Ok(obj(vec![("removed", Json::Num(-1.0))])), "removed").is_err());
    }
}
