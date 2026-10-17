//! `shortcuts`: lists, shows and runs the shortcuts the Shortcuts app saves. The list is the app's own file
//! (`springboard::shortcuts`), read from the app's data directory, so the command and the window always agree.
//!
//! `list` prints each shortcut's id, name and step count. `view` prints a shortcut's reminder and its steps in order.
//! `run` performs the steps the command line can reach: copying text (`wl-copy`), showing a folder in Files (the
//! shell's `open-folder` request), running a shell script (its output goes to the next step), saving the previous
//! output to a file in Documents and showing a result, plus the steps that only work on text (Text, Set Variable, If and
//! Repeat, see `springboard::shortcuts::flow`). Other steps need the shell's own services (Wi-Fi, Low Power, Do Not
//! Disturb, brightness, apps, screenshots, timers, notifications) or an answer from the user and are not run. Like the
//! app, `run` stops at the first step that does not run, reports the rest as not run, and exits with 1.
//!
//! `drop NAME-OR-ID FILE...` runs a shortcut whose automation is "When a File Is Dropped" with the files as its input
//! (`springboard::shortcuts::droplet`). `launcher NAME-OR-ID` writes a desktop launcher for such a shortcut into the
//! account's applications folder; dropping files on that launcher runs `drop` with them, so the shortcut has a droplet
//! icon wherever the launcher is listed.

use crate::{open, pasteboard};
use springboard::shortcuts::flow::{self, Done, Host};
use springboard::shortcuts::{clean_file, droplet, Action, Shortcut};
use std::path::{Path, PathBuf};
use std::process::Command as Process;

pub const USAGE: &str = "usage: shortcuts [-h] | list | view NAME-OR-ID | run NAME-OR-ID | drop NAME-OR-ID FILE... | launcher NAME-OR-ID";

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    List,
    View(String),
    Run(String),
    /// A shortcut name or id, then the absolute paths of the dropped files.
    Drop(String, Vec<String>),
    Launcher(String),
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
        ["launcher", rest @ ..] if !rest.is_empty() => Ok(Cmd::Launcher(rest.join(" "))),
        ["drop", rest @ ..] if !rest.is_empty() => {
            // the name is every word before the first absolute path; the paths are the files
            let at = rest.iter().position(|w| w.starts_with('/')).ok_or("drop needs the files after the shortcut")?;
            if at == 0 {
                return Err("drop needs a shortcut name or id before the files".into());
            }
            Ok(Cmd::Drop(rest[..at].join(" "), rest[at..].iter().map(|w| w.to_string()).collect()))
        }
        ["view" | "run" | "drop" | "launcher"] => Err("view, run, drop and launcher need a shortcut name or id".into()),
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

/// Performs `s`'s steps in order with `perform`, which only reports what it did. The first step that does not run stops
/// the shortcut: the steps after it are reported as not run, as the app does. Blocks (If, Repeat) and variables work as
/// in `flow::run`; this form has no step output, so use `run_shortcut` to pass output from step to step.
pub fn run_steps(s: &Shortcut, perform: impl FnMut(&Action) -> Result<String, String>) -> Vec<Step> {
    struct Plain<F>(F);
    impl<F: FnMut(&Action) -> Result<String, String>> Host for Plain<F> {
        fn perform(&mut self, action: &Action, _input: &str) -> Result<Done, String> {
            (self.0)(action).map(|message| Done { message, output: None })
        }
    }
    steps_of(flow::run(&s.actions, &mut Plain(perform)))
}

/// Runs `s` from the command line: the steps it can reach (see `Cli`), with output passed from step to step.
pub fn run_shortcut(s: &Shortcut) -> Vec<Step> {
    steps_of(flow::run(&s.actions, &mut Cli))
}

/// Runs the droplet `s` with the dropped `paths` as its input. `Err` says why nothing ran: the shortcut is not a
/// droplet, or none of the paths is usable (only absolute paths count).
pub fn drop_shortcut(s: &Shortcut, paths: &[String]) -> Result<Vec<Step>, String> {
    if !droplet::accepts(s) {
        return Err(format!("{} does not run when files are dropped; set its automation to \"When a File Is Dropped\"", s.name));
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    droplet::run(s, &refs, &mut Cli).map(steps_of).ok_or_else(|| "none of the files has a usable absolute path".to_string())
}

/// The desktop launcher for droplet number `id`: dropping files on it runs `shortcuts drop <id> <files>`. The id, not
/// the name, goes in the command so the name never needs quoting.
pub fn launcher_entry(id: usize, s: &Shortcut) -> String {
    let name: String = s.name.chars().filter(|c| !c.is_control()).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Runs the shortcut {name} when files are dropped on it\nExec=shortcuts drop {id} %F\nIcon=application-x-executable\nTerminal=false\nCategories=Utility;\n"
    )
}

/// Where the launcher of `s` is written: `<home>/.local/share/applications/bifrost-droplet-<name>.desktop`.
pub fn launcher_path(home: &Path, s: &Shortcut) -> PathBuf {
    let stem: String = clean_file(&s.name).chars().map(|c| if c.is_whitespace() { '-' } else { c }).collect();
    home.join(".local/share/applications").join(format!("bifrost-droplet-{stem}.desktop"))
}

/// Writes the launcher of droplet `s` (number `id` in the list) and returns where. A shortcut that is not a droplet
/// has none.
pub fn write_launcher(home: &Path, id: usize, s: &Shortcut) -> Result<PathBuf, String> {
    if !droplet::accepts(s) {
        return Err(format!("{} is not a droplet; set its automation to \"When a File Is Dropped\" first", s.name));
    }
    let path = launcher_path(home, s);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, launcher_entry(id, s)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn steps_of(steps: Vec<flow::Step>) -> Vec<Step> {
    steps.into_iter().map(|st| Step { line: st.line, outcome: st.outcome }).collect()
}

/// The host of a command-line run. Steps the command line cannot reach say so, and stop the shortcut.
struct Cli;

impl Host for Cli {
    fn perform(&mut self, action: &Action, input: &str) -> Result<Done, String> {
        match action {
            Action::RunScript(script) => {
                let out = run_script(script)?;
                Ok(Done::with_output(&format!("ran {script}"), out.trim_end()))
            }
            Action::ShowResult(text) => Ok(Done::said(text)),
            Action::SaveFile(name) => {
                let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
                save_file(Path::new(&home), name, input).map(|message| Done::said(&message))
            }
            other => perform(other).map(|message| Done::said(&message)),
        }
    }
}

/// Runs a shell script and returns what it printed. `Err` says how it failed.
fn run_script(script: &str) -> Result<String, String> {
    let out = Process::new("sh").arg("-c").arg(script).output().map_err(|e| format!("sh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{script} exited with {}", out.status))
    }
}

/// Writes `text` to the file `name` in `home`'s Documents folder (made if missing). The name is cleaned first, so it can
/// never leave that folder.
fn save_file(home: &Path, name: &str, text: &str) -> Result<String, String> {
    let file = clean_file(name);
    if file.is_empty() {
        return Err("the file needs a name".into());
    }
    let dir = home.join("Documents");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(&file), text).map_err(|e| format!("{file}: {e}"))?;
    Ok(format!("saved {file} in Documents"))
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
            run_script(script)?;
            Ok(format!("ran {script}"))
        }
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
        Shortcut::new(name, 0, actions)
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

    fn droplet_named(name: &str) -> Shortcut {
        let mut s = named(name, vec![Action::ShowResult("got {Output}".into())]);
        s.automation = Some(springboard::shortcuts::Automation::FileDropped);
        s
    }

    #[test]
    fn drop_reads_the_name_up_to_the_first_path() {
        let cmd = parse(&words("drop Convert Images /home/a.png /home/b.png"));
        assert_eq!(cmd, Ok(Cmd::Drop("Convert Images".into(), vec!["/home/a.png".into(), "/home/b.png".into()])));
        assert_eq!(parse(&words("drop 3 /x")), Ok(Cmd::Drop("3".into(), vec!["/x".into()])));
        assert!(parse(&words("drop Convert")).is_err(), "no files");
        assert!(parse(&words("drop /x")).is_err(), "no shortcut");
        assert_eq!(parse(&words("launcher Convert Images")), Ok(Cmd::Launcher("Convert Images".into())));
        assert!(parse(&words("launcher")).is_err());
    }

    #[test]
    fn dropped_files_are_the_input_of_a_droplet_only() {
        let s = droplet_named("Convert");
        let steps = drop_shortcut(&s, &["/home/a.png".into(), "relative.png".into(), "/home/b.png".into()]).unwrap();
        assert_eq!(steps.last().unwrap().outcome, Ok("got /home/a.png\n/home/b.png".into()));
        assert!(drop_shortcut(&s, &["relative.png".into()]).unwrap_err().contains("usable absolute path"));
        let plain = named("Plain", vec![Action::ShowResult("x".into())]);
        assert!(drop_shortcut(&plain, &["/x".into()]).unwrap_err().contains("does not run when files are dropped"));
    }

    #[test]
    fn a_droplet_gets_a_launcher_that_runs_drop_with_its_files() {
        let s = droplet_named("Convert Images");
        let entry = launcher_entry(4, &s);
        assert!(entry.contains("Name=Convert Images\n") && entry.contains("Exec=shortcuts drop 4 %F\n"), "{entry}");
        let home = std::env::temp_dir().join(format!("shortcuts-launcher-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let path = write_launcher(&home, 4, &s).unwrap();
        assert_eq!(path, home.join(".local/share/applications/bifrost-droplet-Convert-Images.desktop"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), entry);
        assert!(write_launcher(&home, 0, &named("Plain", vec![])).unwrap_err().contains("not a droplet"));
        let _ = std::fs::remove_dir_all(&home);
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

    #[test]
    fn a_run_passes_text_from_step_to_step_and_shows_the_result() {
        let s = named(
            "Greet",
            vec![
                Action::Text("Ada".into()),
                Action::SetVariable("Name".into()),
                Action::ShowResult("Hello, {Name}".into()),
            ],
        );
        let steps = run_shortcut(&s);
        assert_eq!(steps.last().unwrap().outcome, Ok("Hello, Ada".into()));
    }

    #[test]
    fn a_step_that_needs_the_user_stops_the_run() {
        let s = named("Ask", vec![Action::AskInput("Name?".into()), Action::ShowResult("{Output}".into())]);
        let steps = run_shortcut(&s);
        assert!(steps[0].outcome.as_ref().unwrap_err().contains("Ask for Input"));
        assert!(steps[1].outcome.is_err(), "the result is not shown after a step that did not run");
    }

    #[test]
    fn a_script_prints_into_the_next_step() {
        let s = named("Say", vec![Action::RunScript("echo hi there".into()), Action::ShowResult("got {Output}".into())]);
        let steps = run_shortcut(&s);
        assert_eq!(steps[1].outcome, Ok("got hi there".into()));
    }

    #[test]
    fn a_saved_file_stays_in_the_documents_folder() {
        let home = std::env::temp_dir().join(format!("shortcuts-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(save_file(&home, "notes.txt", "hello"), Ok("saved notes.txt in Documents".into()));
        assert_eq!(std::fs::read_to_string(home.join("Documents/notes.txt")).unwrap(), "hello");
        assert_eq!(save_file(&home, "../escape.txt", "x"), Ok("saved escape.txt in Documents".into()));
        assert!(!home.join("escape.txt").exists());
        assert!(save_file(&home, "///", "x").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
