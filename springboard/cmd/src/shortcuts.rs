//! `shortcuts`: lists, shows and runs the shortcuts the Shortcuts app saves. The list is the app's own file
//! (`springboard::shortcuts`), read from the app's data directory, so the command and the window always agree.
//!
//! `list` prints each shortcut's id, name and step count. `view` prints a shortcut's reminder and its steps in order.
//! `run` performs the steps the command line can reach: copying text (`wl-copy`), showing a folder in Files (the
//! shell's `open-folder` request) and running a shell script. Other steps need the shell's own services (Wi-Fi, Low
//! Power, Do Not Disturb, brightness, apps, screenshots, timers, notifications) and are not run. Like the app, `run`
//! stops at the first step that does not run, reports the rest as not run, and exits with 1.

use crate::{open, pasteboard};
use springboard::shortcuts::{Action, Shortcut};
use std::path::PathBuf;
use std::process::Command as Process;

pub const USAGE: &str = "usage: shortcuts [-h] | list | view NAME-OR-ID | run NAME-OR-ID";

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    List,
    View(String),
    Run(String),
}

/// One step of a run: its plan line and what happened to it.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub line: String,
    pub outcome: Result<String, String>,
}

/// Reads the command line. A name may have several words (`run Save Battery`).
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["-h" | "-help" | "--help" | "help"] => Ok(Cmd::Help),
        ["list"] => Ok(Cmd::List),
        ["view", rest @ ..] if !rest.is_empty() => Ok(Cmd::View(rest.join(" "))),
        ["run", rest @ ..] if !rest.is_empty() => Ok(Cmd::Run(rest.join(" "))),
        ["view" | "run"] => Err("view and run need a shortcut name or id".into()),
        [other, ..] => Err(format!("unknown command {other}")),
    }
}

/// The index of the shortcut `what` names: its id (its position in the list) or its name. A name matches exactly
/// first, ignoring case, and otherwise as part of a name. A name that several shortcuts match asks for the id.
pub fn resolve(list: &[Shortcut], what: &str) -> Result<usize, String> {
    if let Ok(i) = what.parse::<usize>() {
        return if i < list.len() { Ok(i) } else { Err(format!("no shortcut has id {i}")) };
    }
    let lower = what.to_lowercase();
    let exact: Vec<usize> =
        list.iter().enumerate().filter(|(_, s)| s.name.to_lowercase() == lower).map(|(i, _)| i).collect();
    let found = if exact.is_empty() {
        list.iter().enumerate().filter(|(_, s)| s.name.to_lowercase().contains(&lower)).map(|(i, _)| i).collect()
    } else {
        exact
    };
    match found.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!("no shortcut named {what}")),
        many => {
            let options: Vec<String> = many.iter().map(|i| format!("{} (id {i})", list[*i].name)).collect();
            Err(format!("more than one shortcut matches {what}; pass the id of one: {}", options.join(", ")))
        }
    }
}

/// One line per shortcut: its id, its name and its step count, tab-separated.
pub fn list_text(list: &[Shortcut]) -> String {
    if list.is_empty() {
        return "No shortcuts.\n".into();
    }
    list.iter()
        .enumerate()
        .map(|(i, s)| format!("{i}\t{}\t{}\n", s.name, actions_word(s.actions.len())))
        .collect()
}

/// The shortcut's name, its reminder when it has one, and its steps numbered in order.
pub fn view_text(s: &Shortcut) -> String {
    let mut out = format!("{}\n", s.name);
    if let Some(at) = s.schedule {
        out += &format!("Daily reminder: {}\n", at.label());
    }
    if s.actions.is_empty() {
        out += "No actions.\n";
        return out;
    }
    for (k, a) in s.actions.iter().enumerate() {
        out += &format!("{}. {}\n", k + 1, a.line());
    }
    out
}

fn actions_word(n: usize) -> String {
    if n == 1 {
        "1 action".into()
    } else {
        format!("{n} actions")
    }
}

/// Performs `s`'s steps in order with `perform`. The first step that does not run stops the shortcut: the steps after
/// it are reported as not run, as the app does.
pub fn run_steps(s: &Shortcut, mut perform: impl FnMut(&Action) -> Result<String, String>) -> Vec<Step> {
    let mut out = Vec::new();
    let mut stopped = false;
    for a in &s.actions {
        let line = a.line();
        if stopped {
            out.push(Step { line, outcome: Err("not run, an earlier step stopped the shortcut".into()) });
            continue;
        }
        let outcome = perform(a);
        stopped = outcome.is_err();
        out.push(Step { line, outcome });
    }
    out
}

/// Performs one step from the command line. `Ok` says what was done; `Err` says why it did not run.
pub fn perform(a: &Action) -> Result<String, String> {
    match a {
        Action::CopyText(text) => {
            pasteboard::copy(text.as_bytes())?;
            Ok(format!("copied \"{text}\""))
        }
        Action::OpenFolder(name) => {
            let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
            let folder = PathBuf::from(home).join(name);
            let open::Parsed::Run(options) = open::parse(&[folder.display().to_string()])? else {
                return Err("nothing to open".into());
            };
            for request in open::plan(&options)? {
                open::send(&request, open::port())?;
            }
            Ok(format!("showed {name}"))
        }
        Action::RunScript(script) => {
            let out = Process::new("sh").arg("-c").arg(script).output().map_err(|e| format!("sh: {e}"))?;
            if out.status.success() {
                Ok(format!("ran {script}"))
            } else {
                Err(format!("{script} exited with {}", out.status))
            }
        }
        Action::Text(_)
        | Action::AskInput(_)
        | Action::GetUrl(_)
        | Action::SaveFile(_)
        | Action::ShowResult
        | Action::SetVariable(_)
        | Action::If(..)
        | Action::Otherwise
        | Action::Repeat(_)
        | Action::Menu(_)
        | Action::MenuItem(_)
        | Action::End => Err(format!("{} runs inside the Shortcuts app, not from the command line", a.title())),
        other => Err(format!("{} needs the shell's services and cannot run from the command line yet", other.title())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shortcuts::{defaults, Daily};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn named(name: &str, actions: Vec<Action>) -> Shortcut {
        Shortcut { name: name.into(), color: 0, actions, schedule: None, ..Shortcut::default() }
    }

    fn two() -> Vec<Shortcut> {
        vec![named("Save Battery", vec![Action::LowPower(true)]), named("Save Power", Vec::new())]
    }

    #[test]
    fn the_command_line_reads_its_words() {
        assert_eq!(parse(&[]), Ok(Cmd::Help));
        assert_eq!(parse(&words("-h")), Ok(Cmd::Help));
        assert_eq!(parse(&words("list")), Ok(Cmd::List));
        assert_eq!(parse(&words("view Save Battery")), Ok(Cmd::View("Save Battery".into())));
        assert_eq!(parse(&words("run 2")), Ok(Cmd::Run("2".into())));
        assert!(parse(&words("view")).is_err());
        assert!(parse(&words("run")).is_err());
        assert_eq!(parse(&words("frobnicate")), Err("unknown command frobnicate".into()));
    }

    #[test]
    fn a_shortcut_is_found_by_id_or_by_name() {
        let list = defaults();
        assert_eq!(resolve(&list, "0"), Ok(0));
        assert_eq!(resolve(&list, "Bedtime"), Ok(0));
        assert_eq!(resolve(&list, "bedtime"), Ok(0));
        assert_eq!(resolve(&list, "bed"), Ok(0), "a part of one name is enough");
        assert_eq!(resolve(&list, "9"), Err("no shortcut has id 9".into()));
        assert_eq!(resolve(&list, "Nope"), Err("no shortcut named Nope".into()));
    }

    #[test]
    fn an_exact_name_wins_and_a_shared_part_asks_for_the_id() {
        let list = two();
        assert_eq!(resolve(&list, "save power"), Ok(1));
        let err = resolve(&list, "Save").unwrap_err();
        assert!(err.contains("Save Battery (id 0)") && err.contains("Save Power (id 1)"), "{err}");
    }

    #[test]
    fn the_list_has_one_row_per_shortcut() {
        assert_eq!(list_text(&[]), "No shortcuts.\n");
        let text = list_text(&two());
        assert_eq!(text, "0\tSave Battery\t1 action\n1\tSave Power\t0 actions\n");
    }

    #[test]
    fn the_view_numbers_the_steps_in_order_with_the_reminder() {
        let mut s = defaults()[0].clone();
        s.schedule = Some(Daily { hour: 22, minute: 30 });
        assert_eq!(
            view_text(&s),
            "Bedtime\nDaily reminder: 22:30\n1. Set Do Not Disturb: On\n2. Set Brightness: 10%\n3. Set Wi-Fi: Off\n"
        );
        assert_eq!(view_text(&two()[1]), "Save Power\nNo actions.\n");
    }

    #[test]
    fn a_run_stops_at_the_first_step_that_does_not_run() {
        let s = named("Mix", vec![Action::CopyText("Thanks".into()), Action::Wifi(true), Action::Timer(5)]);
        let mut seen = Vec::new();
        let steps = run_steps(&s, |a| {
            seen.push(a.title());
            if matches!(a, Action::Wifi(_)) { Err("cannot run".into()) } else { Ok("done".into()) }
        });
        assert_eq!(seen, vec!["Copy Text", "Set Wi-Fi"], "the timer after the failure is not performed");
        assert_eq!(steps[0].outcome, Ok("done".into()));
        assert_eq!(steps[1].outcome, Err("cannot run".into()));
        assert_eq!(steps[2].line, "Start Timer: 5 min");
        assert!(steps[2].outcome.is_err());
    }

    #[test]
    fn a_step_the_command_line_cannot_reach_says_so_without_running_anything() {
        assert_eq!(
            perform(&Action::Notify("Dinner is ready".into())),
            Err("Show Notification needs the shell's services and cannot run from the command line yet".into())
        );
        assert!(perform(&Action::Dnd(true)).unwrap_err().contains("Set Do Not Disturb"));
    }
}
