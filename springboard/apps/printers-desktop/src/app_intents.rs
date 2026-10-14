//! The App Intents of Print Center: typed actions that Shortcuts and Search call as `intent.<id>` over the control socket
//! (see `bifrost_app::intents`). They act on the running window's model, as the control methods in `rpc.rs` do, and the
//! changes go through the same worker as the buttons. The same intents are declared as `[[intent]]` tables in
//! `manifest.toml` so the shell can list them without starting the app; a test keeps the two in step.

use crate::model::{self, Action};
use crate::rpc;
use crate::window::PrintCenter;
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult, Param, ParamType, Snippet};
use bifrost_core::json::Json;

/// The entity type of a printer or pool.
pub const PRINTER: &str = "Printer";

fn printer_param() -> Param {
    Param::new("printer", "Printer", ParamType::Entity(PRINTER.into()))
}

/// The intents this app serves. A printer left out means the one selected in the window.
pub fn declared() -> Vec<Intent> {
    vec![
        Intent::new("list-printers", "List Printers", "Lists the printers with their state and the jobs waiting.").shortcut(),
        Intent::new("pause-printer", "Pause Printer", "Stops a printer from printing its queue.").param(printer_param().optional()),
        Intent::new("resume-printer", "Resume Printer", "Lets a paused printer print its queue again.").param(printer_param().optional()),
        Intent::new("set-default-printer", "Set Default Printer", "Makes a printer the one documents print on by default.").param(printer_param()),
        Intent::new("cancel-job", "Cancel Print Job", "Cancels a job in the queue of a printer.")
            .param(printer_param().optional())
            .param(Param::new("job", "Job number", ParamType::Int)),
        Intent::new("print-file", "Print File", "Sends a document to a printer.")
            .param(Param::new("path", "File path", ParamType::String))
            .param(printer_param().optional()),
    ]
}

/// Runs the declared intent `id`; `params` is validated already.
pub fn perform(p: &mut PrintCenter, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
    match id {
        "list-printers" => Ok(list(p)),
        "pause-printer" | "resume-printer" | "set-default-printer" => {
            let name = rpc::printer_param(p, params)?;
            let (action, dialog) = match id {
                "pause-printer" => (Action::Pause, format!("Pausing {name}")),
                "resume-printer" => (Action::Resume, format!("Resuming {name}")),
                _ => (Action::SetDefault, format!("{name} is now the default printer")),
            };
            p.request(name, action);
            Ok(IntentResult::dialog(&dialog))
        }
        "cancel-job" => {
            let name = rpc::printer_param(p, params)?;
            let job = rpc::job_number(params, "job")?;
            p.request(name.clone(), Action::CancelJob(job));
            Ok(IntentResult::dialog(&format!("Canceling job {job} on {name}")))
        }
        "print-file" => {
            let path = intents::str_param(params, "path")?;
            let name = rpc::printer_param(p, params)?;
            p.print_path_on(&name, path).map_err(intents::failed)?;
            let file = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
            Ok(IntentResult::dialog(&format!("Printing {file} on {name}")))
        }
        _ => Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
    }
}

/// The printers as data, with a sentence and a table of their states for the result view.
fn list(p: &PrintCenter) -> IntentResult {
    let mut snippet = Snippet::new("Printers");
    for printer in &p.model.printers {
        snippet = snippet.row(&printer.name, &model::row_summary(printer, p.model.is_offline(&printer.name)));
    }
    IntentResult::value(rpc::list(p)).with_dialog(&model::list_dialog(&p.model.printers)).with_snippet(snippet)
}

/// The printers an intent parameter can name: all of them, those with these names, or those whose name has this text.
pub fn entities(p: &PrintCenter, ty: &str, query: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
    if ty != PRINTER {
        return Err(RpcError::new(INVALID_PARAMS, format!("unknown entity type {ty}")));
    }
    let all = p.model.printers.iter().map(|x| Entity::new(PRINTER, &x.name, &x.name).subtitle(&model::row_summary(x, p.model.is_offline(&x.name))));
    Ok(match query {
        EntityQuery::Suggested => all.collect(),
        EntityQuery::Ids(ids) => all.filter(|e| ids.contains(&e.id)).collect(),
        EntityQuery::String(text) => {
            let needle = text.to_lowercase();
            all.filter(|e| e.title.to_lowercase().contains(&needle)).collect()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::cups::{Printer, PrinterState};

    fn printer(name: &str, queued: i32) -> Printer {
        Printer {
            name: name.into(),
            uri: String::new(),
            state: PrinterState::Idle,
            state_message: String::new(),
            accepting_jobs: true,
            is_default: false,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: queued,
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    fn window() -> PrintCenter {
        let mut w = PrintCenter::detached();
        w.model.printers = vec![printer("Office", 2), printer("Lab", 0)];
        w.model.selected = Some("Office".into());
        w
    }

    fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn text(s: &str) -> Json {
        Json::Str(s.to_string())
    }

    #[test]
    fn the_declared_intents_have_unique_ids_and_are_in_the_manifest() {
        let manifest = include_str!("../manifest.toml");
        let intents = declared();
        let mut ids: Vec<&str> = intents.iter().map(|i| i.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), intents.len());
        for intent in &intents {
            assert!(manifest.contains(&intent.manifest_toml()), "manifest.toml lacks the table for {}", intent.id);
        }
        assert_eq!(manifest.matches("[[intent]]").count(), intents.len(), "manifest.toml declares an intent the app does not serve");
    }

    #[test]
    fn only_list_printers_is_a_shortcut_because_the_others_need_a_choice() {
        let shortcuts: Vec<String> = declared().into_iter().filter(|i| i.shortcut).map(|i| i.id).collect();
        assert_eq!(shortcuts, vec!["list-printers".to_string()]);
    }

    #[test]
    fn parameters_are_checked_against_the_declaration() {
        let find = |id: &str| declared().into_iter().find(|i| i.id == id).unwrap();
        assert!(find("pause-printer").validate(&Json::Null).is_ok(), "the printer is optional");
        assert!(find("set-default-printer").validate(&Json::Null).is_err(), "the printer is required");
        assert!(find("cancel-job").validate(&obj(vec![("job", Json::Num(3.0))])).is_ok());
        assert!(find("cancel-job").validate(&obj(vec![("job", text("three"))])).is_err());
        assert!(find("print-file").validate(&obj(vec![("path", text("/home/a.pdf")), ("printer", text("Lab"))])).is_ok());
    }

    #[test]
    fn listing_answers_with_the_printers_a_sentence_and_a_table() {
        let mut w = window();
        let result = perform(&mut w, "list-printers", &Json::Null).unwrap();
        assert_eq!(result.dialog.as_deref(), Some("2 printers, 2 jobs waiting"));
        assert_eq!(result.value.as_array().map(<[Json]>::len), Some(2));
        let snippet = result.snippet.unwrap();
        assert_eq!(snippet.rows.len(), 2);
        assert_eq!((snippet.rows[0].label.as_str(), snippet.rows[0].text.as_str()), ("Office", "Idle, 2 in queue"));
        assert_eq!(w.requests_sent(), 0, "listing changes nothing");
    }

    #[test]
    fn pausing_resuming_and_defaulting_name_a_listed_printer_or_use_the_selected_one() {
        let mut w = window();
        assert_eq!(perform(&mut w, "pause-printer", &Json::Null).unwrap().dialog.as_deref(), Some("Pausing Office"));
        assert_eq!(perform(&mut w, "resume-printer", &obj(vec![("printer", text("Lab"))])).unwrap().dialog.as_deref(), Some("Resuming Lab"));
        assert_eq!(perform(&mut w, "set-default-printer", &obj(vec![("printer", text("Lab"))])).unwrap().dialog.as_deref(), Some("Lab is now the default printer"));
        assert_eq!(w.requests_sent(), 3);
        assert!(perform(&mut w, "pause-printer", &obj(vec![("printer", text("Nope"))])).is_err());
        assert_eq!(w.requests_sent(), 3, "an unknown printer sends nothing");
    }

    #[test]
    fn canceling_needs_a_job_number() {
        let mut w = window();
        assert!(perform(&mut w, "cancel-job", &obj(vec![("job", Json::Num(0.0))])).is_err());
        assert_eq!(w.requests_sent(), 0);
        let done = perform(&mut w, "cancel-job", &obj(vec![("job", Json::Num(12.0)), ("printer", text("Lab"))])).unwrap();
        assert_eq!(done.dialog.as_deref(), Some("Canceling job 12 on Lab"));
        assert_eq!(w.requests_sent(), 1);
    }

    #[test]
    fn printing_a_file_that_cannot_be_read_is_a_failure_and_sends_nothing() {
        let mut w = window();
        let err = perform(&mut w, "print-file", &obj(vec![("path", text("/no/such/file.pdf"))])).unwrap_err();
        assert!(err.message.contains("Could not open"), "{}", err.message);
        let unknown = perform(&mut w, "print-file", &obj(vec![("path", text("/no/such/file.pdf")), ("printer", text("Nope"))]));
        assert!(unknown.is_err());
        assert_eq!(w.requests_sent(), 0);
        assert!(perform(&mut w, "nothing", &Json::Null).is_err());
    }

    #[test]
    fn printers_are_entities_by_suggestion_name_and_text() {
        let w = window();
        let names = |q: EntityQuery| entities(&w, PRINTER, &q).unwrap().into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(names(EntityQuery::Suggested), vec!["Office", "Lab"]);
        assert_eq!(names(EntityQuery::Ids(vec!["Lab".into(), "Gone".into()])), vec!["Lab"]);
        assert_eq!(names(EntityQuery::String("OFF".into())), vec!["Office"]);
        let first = entities(&w, PRINTER, &EntityQuery::Suggested).unwrap().remove(0);
        assert_eq!(first.subtitle.as_deref(), Some("Idle, 2 in queue"));
        assert!(entities(&w, "Note", &EntityQuery::Suggested).is_err());
    }
}
