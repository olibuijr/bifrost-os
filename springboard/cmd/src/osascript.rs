//! `osascript`: runs a script in the BifrOSt scripting language (`bifrost_core::script`). Input is `-e` statements,
//! a file (a source text or an `osacompile` output), or standard input. Commands in a `tell app "Name"` block are
//! looked up in that app's manifest dictionary (`[[script.command]]`, see `bifrost_core::script::dictionary`), and each
//! one is sent to the app's control socket as an AppEvent with `bifrost_app::app_events::send_message`. Positional
//! arguments after the script (or after the `-e` statements) are its `argv`, passed to `on run argv`.
//!
//! Limits: the app must already be running (the tool does not launch it); `display dialog` and `display notification`
//! are printed on standard error, because the shell has no request path for a command-line tool yet.

use bifrost_app::app_events::{self, AppEvent};
use bifrost_core::json::Json;
use bifrost_core::script::{self, dictionary::Dictionary, interp::{self, Host, ScriptError, Value}};
use springboard::shell::{apphost, intents, js_automation};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const USAGE: &str = "usage: osascript [-s s|o] [-l AppleScript|JavaScript] [-e statement]... [file [argument ...]]\n       osascript [-s s|o] -e statement ... [argument ...]";

/// `errAEEventNotHandled`-style numbers are in `interp`; this one is `errAENoSuchObject`-like: the app is not running.
const ERR_NOT_RUNNING: i64 = -600;

/// How a result is printed: `-s o` plain text (the default), `-s s` in source form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Source,
}

/// Where the script text comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Lines(Vec<String>),
    File(PathBuf),
    Stdin,
}

/// The language of the script: the AppleScript-style language (`bifrost_core::script`) or JavaScript
/// (`springboard::shell::js_automation`, run with every grant because the person started it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    AppleScript,
    JavaScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub input: Input,
    pub style: Style,
    pub language: Language,
    pub argv: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the command line. `-e` may repeat; the first positional argument is the file when there is no `-e`, and every
/// positional argument after that (or every one, when there are `-e` statements) is an `argv` entry.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut lines: Vec<String> = Vec::new();
    let mut style = Style::Plain;
    let mut language = Language::AppleScript;
    let mut file: Option<PathBuf> = None;
    let mut argv: Vec<String> = Vec::new();
    let mut positional = false;
    let mut options_done = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if positional || options_done {
            if !positional && file.is_none() && lines.is_empty() {
                file = Some(PathBuf::from(a));
            } else {
                argv.push(a.to_string());
            }
            positional = true;
            i += 1;
            continue;
        }
        if a == "--" {
            options_done = true;
        } else if a.starts_with('-') && a.len() > 1 {
            let next: Option<String> = args.get(i + 1).cloned();
            match a {
                "-h" | "--help" => return Ok(Parsed::Help),
                "-e" => {
                    lines.push(next.ok_or("-e needs a statement")?);
                    i += 1;
                }
                "-s" => {
                    style = match next.as_deref() {
                        Some("s") => Style::Source,
                        Some("o") => Style::Plain,
                        Some(other) => return Err(format!("unknown output style {other}; use s or o")),
                        None => return Err("-s needs a style: s or o".into()),
                    };
                    i += 1;
                }
                "-l" => {
                    match next.as_deref() {
                        Some("AppleScript") => language = Language::AppleScript,
                        Some("JavaScript") => language = Language::JavaScript,
                        Some(other) => return Err(format!("language {other} is not supported; use AppleScript or JavaScript")),
                        None => return Err("-l needs a language".into()),
                    }
                    i += 1;
                }
                other => return Err(format!("unknown option {other}")),
            }
        } else {
            // the first positional argument: the file, or an argv entry when there are -e statements
            positional = true;
            if lines.is_empty() {
                file = Some(PathBuf::from(a));
            } else {
                argv.push(a.to_string());
            }
        }
        i += 1;
    }
    let input = if !lines.is_empty() {
        Input::Lines(lines)
    } else if let Some(f) = file {
        Input::File(f)
    } else {
        Input::Stdin
    };
    Ok(Parsed::Run(Options { input, style, language, argv }))
}

/// The script text. A file holding an `osacompile` output is read back to its canonical source.
pub fn read_source(input: &Input) -> Result<String, String> {
    match input {
        Input::Lines(lines) => Ok(lines.join("\n")),
        Input::File(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            Ok(script::compiled_source(&text).unwrap_or(text))
        }
        Input::Stdin => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).map_err(|e| format!("cannot read standard input: {e}"))?;
            Ok(text)
        }
    }
}

/// The line a result prints as, or nothing for a missing value.
pub fn render(value: &Value, style: Style) -> Option<String> {
    match (value, style) {
        (Value::Missing, _) => None,
        (v, Style::Source) => Some(v.source()),
        (v, Style::Plain) => Some(v.text()),
    }
}

/// The installed app a script names: by id, or by its manifest name (case does not matter).
pub fn find_app<'a>(apps: &'a [apphost::Installed], name: &str) -> Option<&'a apphost::Installed> {
    apps.iter().find(|a| a.manifest.id == name).or_else(|| apps.iter().find(|a| a.manifest.name.eq_ignore_ascii_case(name)))
}

/// The host of the tool: apps are the installed manifests, events go to their control sockets.
pub struct SystemHost {
    root: PathBuf,
}

impl SystemHost {
    pub fn new() -> SystemHost {
        SystemHost { root: apphost::root() }
    }

    fn apps(&self) -> Vec<apphost::Installed> {
        apphost::scan(&self.root)
    }
}

impl Default for SystemHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for SystemHost {
    fn send_event(&mut self, app: &str, class: &str, id: &str, params: &Json, timeout_secs: u64) -> Result<Json, ScriptError> {
        let apps = self.apps();
        let found = find_app(&apps, app).ok_or_else(|| ScriptError::new(ERR_NOT_RUNNING, format!("Application \"{app}\" isn't running.")))?;
        let socket = intents::socket_path(&found.manifest.id);
        let reply = app_events::send_message(&socket, &AppEvent::new(class, id, params.clone()), Duration::from_secs(timeout_secs));
        if reply.ok() {
            Ok(reply.result)
        } else {
            Err(ScriptError::new(reply.error_number, reply.error_string.unwrap_or_else(|| "The event failed.".to_string())))
        }
    }

    fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError> {
        let apps = self.apps();
        let found = find_app(&apps, app).ok_or_else(|| ScriptError::new(ERR_NOT_RUNNING, format!("Application \"{app}\" isn't running.")))?;
        if found.manifest.scripting.is_empty() {
            return Err(ScriptError::new(interp::ERR_NOT_HANDLED, format!("{app} has no scripting dictionary.")));
        }
        Ok(found.manifest.scripting.clone())
    }

    fn display_dialog(&mut self, text: &str, title: Option<&str>) -> Result<String, ScriptError> {
        eprintln!("[dialog] {}{text}", title.map(|t| format!("{t}: ")).unwrap_or_default());
        Ok("OK".to_string())
    }

    fn display_notification(&mut self, text: &str, title: Option<&str>) -> Result<(), ScriptError> {
        eprintln!("[notification] {}{text}", title.map(|t| format!("{t}: ")).unwrap_or_default());
        Ok(())
    }

    fn shell(&mut self, command: &str) -> Result<String, ScriptError> {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| ScriptError::new(-1, format!("cannot run the shell: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let code = out.status.code().unwrap_or(1) as i64;
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(ScriptError::new(code, if err.is_empty() { format!("exit status {code}") } else { err }))
        }
    }

    fn sleep(&mut self, secs: f64) {
        std::thread::sleep(Duration::from_secs_f64(secs.max(0.0)));
    }

    fn app_id(&mut self, app: &str) -> Option<String> {
        find_app(&self.apps(), app).map(|a| a.manifest.id.clone())
    }
}

/// Runs the script with the system host and returns the result to print, if any.
pub fn run(opts: &Options) -> Result<Option<String>, String> {
    let source = read_source(&opts.input)?;
    if opts.language == Language::JavaScript {
        let run = js_automation::run(&source, &opts.argv, Box::new(SystemHost::new()), js_automation::Policy::allow_all());
        for line in &run.output {
            eprintln!("{line}");
        }
        let value = run.result.map_err(|e| format!("execution error: {e}"))?;
        return Ok(render(&value, opts.style));
    }
    let mut host = SystemHost::new();
    let value = interp::run_source(&source, &mut host, &opts.argv).map_err(|e| format!("execution error: {e}"))?;
    Ok(render(&value, opts.style))
}

/// The path a default output takes for `input` (not used by `osascript`; the compiler's default).
pub fn default_output(input: &Path) -> PathBuf {
    input.with_extension("scpt")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn run_of(list: &[&str]) -> Options {
        match parse(&args(list)).unwrap() {
            Parsed::Run(o) => o,
            Parsed::Help => panic!("expected a run"),
        }
    }

    #[test]
    fn statements_from_e_and_the_rest_are_argv() {
        let o = run_of(&["-e", "on run argv", "-e", "return argv", "-e", "end run", "first", "second"]);
        assert_eq!(o.input, Input::Lines(args(&["on run argv", "return argv", "end run"])));
        assert_eq!(o.argv, args(&["first", "second"]));
        assert_eq!(o.style, Style::Plain);
    }

    #[test]
    fn a_file_comes_first_and_its_arguments_follow() {
        let o = run_of(&["script.bfs", "a", "-b"]);
        assert_eq!(o.input, Input::File(PathBuf::from("script.bfs")));
        assert_eq!(o.argv, args(&["a", "-b"]), "after the file, dashes are arguments too");
    }

    #[test]
    fn no_input_reads_standard_input() {
        assert_eq!(run_of(&[]).input, Input::Stdin);
    }

    #[test]
    fn style_and_language_options() {
        assert_eq!(run_of(&["-s", "s", "-e", "return 1"]).style, Style::Source);
        assert_eq!(run_of(&["-s", "o", "-l", "AppleScript", "-e", "return 1"]).style, Style::Plain);
        assert!(parse(&args(&["-s", "q", "-e", "x"])).is_err());
        assert!(parse(&args(&["-l", "Perl", "-e", "x"])).is_err());
        assert_eq!(run_of(&["-l", "JavaScript", "-e", "1"]).language, Language::JavaScript);
        assert_eq!(run_of(&["-e", "1"]).language, Language::AppleScript);
        assert!(parse(&args(&["-s"])).is_err());
    }

    #[test]
    fn help_and_bad_options() {
        assert_eq!(parse(&args(&["-h"])).unwrap(), Parsed::Help);
        assert!(parse(&args(&["-z"])).is_err());
        assert!(parse(&args(&["-e"])).is_err(), "-e needs a statement");
        assert!(matches!(parse(&args(&["--", "-e"])).unwrap(), Parsed::Run(o) if o.input == Input::File(PathBuf::from("-e"))));
    }

    #[test]
    fn results_print_as_text_or_source() {
        let s = Value::Str("a \"b\"".into());
        assert_eq!(render(&s, Style::Plain).as_deref(), Some("a \"b\""));
        assert_eq!(render(&s, Style::Source).as_deref(), Some("\"a \\\"b\\\"\""));
        assert_eq!(render(&Value::Missing, Style::Plain), None);
        assert_eq!(render(&Value::Num(3.0), Style::Plain).as_deref(), Some("3"));
    }

    #[test]
    fn a_compiled_file_is_read_back_as_its_source() {
        let dir = std::env::temp_dir().join(format!("osascript-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.scpt");
        std::fs::write(&path, script::compile("return 42\n").unwrap()).unwrap();
        assert_eq!(read_source(&Input::File(path.clone())).unwrap(), "return 42\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apps_are_found_by_id_or_by_name() {
        let text = "id = \"is.example.notes\"\nname = \"Notes\"\nexec = \"notes\"\n[[script.command]]\nname = \"make new note\"\nevent = \"notes.make\"\n";
        let apps = vec![apphost::Installed { manifest: apphost::Manifest::parse(text).unwrap(), dir: PathBuf::new() }];
        assert!(find_app(&apps, "is.example.notes").is_some());
        assert!(find_app(&apps, "notes").is_some(), "the name matches without case");
        assert!(find_app(&apps, "Mail").is_none());
        let found = find_app(&apps, "NOTES").unwrap();
        assert!(found.manifest.scripting.command("make new note").is_some());
    }

    #[test]
    fn the_default_output_name_is_scpt() {
        assert_eq!(default_output(Path::new("/x/notes.bfs")), PathBuf::from("/x/notes.scpt"));
    }
}
