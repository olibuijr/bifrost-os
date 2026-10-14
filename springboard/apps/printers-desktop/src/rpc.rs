//! The control methods of Print Center (`bifrost pc control call is.olibuijr.printers.desktop <method>`), which are also
//! the automation actions Shortcuts calls: list the printers and the jobs of the selected one, pause and resume a
//! printer, cancel a job. They read the window's model, so they answer as fast as the last listing; the actions go
//! through the same worker as the buttons.

use crate::model::{self, Action};
use crate::window::PrintCenter;
use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::Json;

/// The names this app serves besides `app.*`.
pub fn methods() -> Vec<&'static str> {
    vec!["printers.list", "printers.jobs", "printers.pause", "printers.resume", "printers.cancel_job"]
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}

fn bad(message: &str) -> RpcError {
    RpcError::new(INVALID_PARAMS, message)
}

/// Answers one request against the window.
pub fn call(p: &mut PrintCenter, method: &str, params: &Json) -> Result<Json, RpcError> {
    match method {
        "printers.list" => Ok(list(p)),
        "printers.jobs" => match params.str_field("printer") {
            Some(name) if p.model.selected.as_deref() != Some(name.as_str()) => Err(bad("only the selected printer's queue is loaded; select it first")),
            _ => Ok(jobs(p)),
        },
        "printers.pause" | "printers.resume" => {
            let name = printer_param(p, params)?;
            let action = if method == "printers.pause" { Action::Pause } else { Action::Resume };
            p.request(name.clone(), action);
            Ok(obj(vec![("printer", Json::Str(name)), ("requested", text(method))]))
        }
        "printers.cancel_job" => {
            let name = printer_param(p, params)?;
            let id = job_number(params, "id")?;
            p.request(name.clone(), Action::CancelJob(id));
            Ok(obj(vec![("printer", Json::Str(name)), ("job", Json::Num(f64::from(id))), ("requested", text(method))]))
        }
        _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
    }
}

/// The job number in the parameter `key`: a whole number above zero.
pub fn job_number(params: &Json, key: &str) -> Result<i32, RpcError> {
    params
        .get(key)
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n > 0.0 && *n < f64::from(i32::MAX))
        .map(|n| n as i32)
        .ok_or_else(|| bad(&format!("{key} must be a job number")))
}

/// The named printer, which must be listed, else the selected one.
pub fn printer_param(p: &PrintCenter, params: &Json) -> Result<String, RpcError> {
    let name = params.str_field("printer").or_else(|| p.model.selected.clone()).ok_or_else(|| bad("no printer is selected"))?;
    if p.model.printers.iter().any(|x| x.name == name) {
        Ok(name)
    } else {
        Err(bad(&format!("no printer named {name}")))
    }
}

pub fn list(p: &PrintCenter) -> Json {
    Json::Arr(
        p.model
            .printers
            .iter()
            .map(|x| {
                let offline = p.model.is_offline(&x.name);
                obj(vec![
                    ("name", text(&x.name)),
                    ("state", text(if offline { "Offline" } else { model::printer_state_name(x.state) })),
                    ("default", Json::Bool(x.is_default)),
                    ("pool", Json::Bool(x.is_class)),
                    ("shared", Json::Bool(x.shared)),
                    ("jobs", Json::Num(f64::from(x.queued_jobs.max(0)))),
                    ("problem", Json::Str(model::problem_line(x).map(|(_, t)| t).unwrap_or_default())),
                ])
            })
            .collect(),
    )
}

fn jobs(p: &PrintCenter) -> Json {
    Json::Arr(
        p.model
            .jobs
            .iter()
            .map(|j| {
                obj(vec![
                    ("id", Json::Num(f64::from(j.id))),
                    ("name", text(&j.name)),
                    ("owner", text(&j.user)),
                    ("status", Json::Str(model::job_status_text(j))),
                    ("pages", Json::Num(f64::from(j.pages.max(0)))),
                ])
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::cups::{Job, JobState, Printer, PrinterState};

    fn printer(name: &str) -> Printer {
        Printer {
            name: name.into(),
            uri: String::new(),
            state: PrinterState::Idle,
            state_message: String::new(),
            accepting_jobs: true,
            is_default: true,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: 1,
            state_reasons: vec!["media-jam-error".into()],
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    fn window() -> PrintCenter {
        let mut w = PrintCenter::detached();
        w.model.printers = vec![printer("Office")];
        w.model.selected = Some("Office".into());
        w.model.jobs = vec![Job {
            id: 4,
            uri: String::new(),
            state: JobState::Pending,
            name: "a.pdf".into(),
            user: "u".into(),
            size_kb: 1,
            reasons: Vec::new(),
            created: 0,
            pages: 3,
            pages_done: 0,
            progress: None,
        }];
        w
    }

    #[test]
    fn the_list_and_the_queue_come_from_the_model() {
        let mut w = window();
        let list = call(&mut w, "printers.list", &Json::Null).unwrap();
        let first = &list.as_array().unwrap()[0];
        assert_eq!(first.str_field("name").as_deref(), Some("Office"));
        assert_eq!(first.str_field("problem").as_deref(), Some("Paper jam"));
        let jobs = call(&mut w, "printers.jobs", &Json::Null).unwrap();
        assert_eq!(jobs.as_array().unwrap()[0].str_field("status").as_deref(), Some("Waiting"));
        assert!(call(&mut w, "printers.jobs", &obj(vec![("printer", text("Other"))])).is_err());
    }

    #[test]
    fn actions_name_a_listed_printer_and_a_job_number() {
        let mut w = window();
        assert!(call(&mut w, "printers.pause", &obj(vec![("printer", text("Nope"))])).is_err());
        assert!(call(&mut w, "printers.cancel_job", &obj(vec![("id", Json::Num(-1.0))])).is_err());
        assert_eq!(w.requests_sent(), 0);
        assert!(call(&mut w, "printers.pause", &Json::Null).is_ok());
        assert!(call(&mut w, "printers.cancel_job", &obj(vec![("id", Json::Num(4.0))])).is_ok());
        assert_eq!(w.requests_sent(), 2);
        assert!(call(&mut w, "printers.nothing", &Json::Null).is_err());
    }
}
