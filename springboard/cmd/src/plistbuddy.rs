//! `PlistBuddy`: runs `-c` commands on a property list and writes the result back. Logic lives here; the binary reads and
//! writes the file (see `bifrost_cmd::plistbuddy` and `bin/PlistBuddy.rs`).
//!
//! Commands: `Print [:path]`, `Set :path value`, `Add :path type [value]`, `Delete :path`, `Clear [dict | array]`,
//! `Save`, `Exit`, `Help`. A path is colon-separated (`:Dict:Key:0`); the root is `:` or an empty path. A component
//! names a dictionary key, or a whole number that indexes an array. `Set` keeps the type the entry already has. Types
//! for `Add` are `string`, `integer`, `real`, `bool`, `date`, `data`, `array` and `dict`.
//!
//! Gaps: there is no interactive mode (a `-c` command is required), and `Copy`, `Merge`, `Import` and `Revert` are not
//! supported yet. Changes are written when the run ends and `Save` only confirms them, so a failing command leaves the
//! file as it was. Keys that contain a colon cannot be addressed. The file is written as XML, so a binary input is
//! converted on the first write.

use crate::plutil;
use bifrost_core::plist::{self, Plist};

pub const USAGE: &str = "Usage: PlistBuddy [-x] [-l] [-c \"command\"]... file.plist\n\
  -c  run a command (repeatable); at least one is needed, interactive mode is not supported\n\
  -x  print values as XML\n\
  -l  print values in the ASCII form (the default)\n\
  -h  show this help";

pub const COMMANDS: &str = "Commands: Print [:path] | Set :path value | Add :path type [value] | Delete :path | \
Clear [dict | array] | Save | Exit | Help";

/// The parsed command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub commands: Vec<String>,
    pub xml: bool,
    pub help: bool,
    pub file: Option<String>,
}

/// One command from a `-c` string.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Prints the value at a path, or the whole value for an empty path.
    Print(String),
    Set { path: String, value: String },
    /// `kind` is a value type name as `plutil` spells it (`float` for `real`, `dictionary` for `dict`).
    Add { path: String, kind: String, value: Option<String> },
    Delete(String),
    /// `Some("array")` or `Some("dictionary")`; `None` clears to a dictionary.
    Clear(Option<String>),
    Save,
    Help,
    Exit,
}

/// The result of running commands on one property list.
#[derive(Debug, PartialEq)]
pub struct Run {
    /// Text for standard output, every line ending in a newline.
    pub output: String,
    /// True when a command changed the value, so the file must be written.
    pub changed: bool,
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut opts = Options { commands: Vec::new(), xml: false, help: false, file: None };
    let mut only_files = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        i += 1;
        if only_files || !arg.starts_with('-') {
            if opts.file.is_some() {
                return Err("only one file can be given".into());
            }
            opts.file = Some(arg.to_string());
            continue;
        }
        match arg {
            "--" => only_files = true,
            "-h" | "-help" | "--help" => opts.help = true,
            "-x" => opts.xml = true,
            "-l" => opts.xml = false,
            "-c" => {
                let cmd = args.get(i).ok_or_else(|| "-c needs a command".to_string())?;
                i += 1;
                opts.commands.push(cmd.clone());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if !opts.help && opts.file.is_none() {
        return Err(format!("no input file\n{USAGE}"));
    }
    Ok(opts)
}

/// Splits a command line into words. Double quotes group words, and a backslash takes the next character literally.
pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' => {
                let next = chars.next().ok_or_else(|| "a trailing backslash".to_string())?;
                cur.push(next);
                in_word = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quoted {
        return Err("a quote is not closed".into());
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// The keys and indexes of a colon-separated path. The root is `:` or an empty path.
pub fn segments(path: &str) -> Vec<&str> {
    path.split(':').filter(|s| !s.is_empty()).collect()
}

/// The value type `Add` takes, as `plutil` names it.
fn add_kind(name: &str) -> Result<&'static str, String> {
    Ok(match name {
        "string" => "string",
        "integer" => "integer",
        "real" => "float",
        "bool" => "bool",
        "date" => "date",
        "data" => "data",
        "array" => "array",
        "dict" => "dictionary",
        other => return Err(format!("unknown type {other}")),
    })
}

/// The container type `Clear` takes.
fn clear_kind(name: &str) -> Result<String, String> {
    match name {
        "dict" | "dictionary" => Ok("dictionary".into()),
        "array" => Ok("array".into()),
        other => Err(format!("Clear takes dict or array, not {other}")),
    }
}

/// Reads one command line.
pub fn parse_command(line: &str) -> Result<Command, String> {
    let w = words(line)?;
    let (verb, rest) = w.split_first().ok_or_else(|| "the command is empty".to_string())?;
    match verb.to_ascii_lowercase().as_str() {
        "print" => match rest {
            [] => Ok(Command::Print(String::new())),
            [path] => Ok(Command::Print(path.clone())),
            _ => Err("Print takes one path".into()),
        },
        "set" => match rest {
            [path, value @ ..] if !value.is_empty() => Ok(Command::Set { path: path.clone(), value: value.join(" ") }),
            _ => Err("Set needs a path and a value".into()),
        },
        "add" => match rest {
            [path, kind] => Ok(Command::Add { path: path.clone(), kind: add_kind(kind)?.to_string(), value: None }),
            [path, kind, value @ ..] => {
                let kind = add_kind(kind)?;
                if kind == "array" || kind == "dictionary" {
                    return Err(format!("{kind} takes no value"));
                }
                Ok(Command::Add { path: path.clone(), kind: kind.to_string(), value: Some(value.join(" ")) })
            }
            _ => Err("Add needs a path and a type".into()),
        },
        "delete" => match rest {
            [path] => Ok(Command::Delete(path.clone())),
            _ => Err("Delete takes one path".into()),
        },
        "clear" => match rest {
            [] => Ok(Command::Clear(None)),
            [kind] => Ok(Command::Clear(Some(clear_kind(kind)?))),
            _ => Err("Clear takes at most one type".into()),
        },
        "save" => Ok(Command::Save),
        "exit" => Ok(Command::Exit),
        "help" => Ok(Command::Help),
        _ => Err(format!("unrecognized command {}", verb)),
    }
}

fn child<'a>(v: &'a Plist, seg: &str) -> Option<&'a Plist> {
    match v {
        Plist::Dict(items) => items.iter().find(|e| e.0 == seg).map(|e| &e.1),
        Plist::Array(items) => seg.parse::<usize>().ok().and_then(|i| items.get(i)),
        _ => None,
    }
}

fn child_mut<'a>(v: &'a mut Plist, seg: &str) -> Option<&'a mut Plist> {
    match v {
        Plist::Dict(items) => items.iter_mut().find(|e| e.0 == seg).map(|e| &mut e.1),
        Plist::Array(items) => seg.parse::<usize>().ok().and_then(move |i| items.get_mut(i)),
        _ => None,
    }
}

fn missing(path: &str) -> String {
    format!("Entry, \"{path}\", Does Not Exist")
}

fn lookup<'a>(root: &'a Plist, path: &str) -> Result<&'a Plist, String> {
    let mut cur = root;
    for seg in segments(path) {
        cur = child(cur, seg).ok_or_else(|| missing(path))?;
    }
    Ok(cur)
}

fn lookup_mut<'a>(root: &'a mut Plist, segs: &[&str], path: &str) -> Result<&'a mut Plist, String> {
    let mut cur = root;
    for seg in segs {
        cur = child_mut(cur, seg).ok_or_else(|| missing(path))?;
    }
    Ok(cur)
}

/// Prints a value: a scalar as its bare text, a container in the ASCII or XML form.
fn print_value(root: &Plist, path: &str, xml: bool) -> Result<String, String> {
    let v = lookup(root, path)?;
    let text = match v {
        Plist::Dict(_) | Plist::Array(_) => {
            if xml {
                plist::to_xml(v)
            } else {
                plist::to_print(v)
            }
        }
        scalar => plutil::raw_text(scalar),
    };
    Ok(if text.ends_with('\n') { text } else { format!("{text}\n") })
}

fn set(root: &mut Plist, path: &str, value: &str) -> Result<(), String> {
    let segs = segments(path);
    let slot = lookup_mut(root, &segs, path)?;
    let kind = slot.type_name();
    if kind == "array" || kind == "dictionary" {
        return Err(format!("{path}: Set cannot replace a dictionary or an array"));
    }
    *slot = plutil::value_for(kind, Some(value))?;
    Ok(())
}

fn add(root: &mut Plist, path: &str, kind: &str, value: Option<&str>) -> Result<(), String> {
    let segs = segments(path);
    let (last, parent) = segs.split_last().ok_or_else(|| "Add needs a path that names an entry".to_string())?;
    let new = plutil::value_for(kind, value)?;
    match lookup_mut(root, parent, path)? {
        Plist::Dict(items) => {
            if items.iter().any(|e| e.0 == *last) {
                return Err(format!("{path}: Entry Already Exists"));
            }
            items.push(((*last).to_string(), new));
            Ok(())
        }
        Plist::Array(items) => {
            let i = last.parse::<usize>().map_err(|_| format!("{path}: an array index must be a number"))?;
            if i > items.len() {
                return Err(format!("{path}: index is past the end of the array"));
            }
            items.insert(i, new);
            Ok(())
        }
        _ => Err(format!("{path}: the parent is not a dictionary or an array")),
    }
}

fn delete(root: &mut Plist, path: &str) -> Result<(), String> {
    let segs = segments(path);
    let (last, parent) = segs.split_last().ok_or_else(|| "Delete needs a path that names an entry".to_string())?;
    match lookup_mut(root, parent, path)? {
        Plist::Dict(items) => {
            let at = items.iter().position(|e| e.0 == *last).ok_or_else(|| missing(path))?;
            items.remove(at);
            Ok(())
        }
        Plist::Array(items) => {
            let i = last.parse::<usize>().ok().filter(|&i| i < items.len()).ok_or_else(|| missing(path))?;
            items.remove(i);
            Ok(())
        }
        _ => Err(format!("{path}: the parent is not a dictionary or an array")),
    }
}

/// Runs the commands in order on `root`. `Exit` stops the run. An error stops it at that command.
pub fn run(root: &mut Plist, commands: &[String], xml: bool) -> Result<Run, String> {
    let mut output = String::new();
    let mut changed = false;
    for line in commands {
        match parse_command(line)? {
            Command::Print(path) => output.push_str(&print_value(root, &path, xml)?),
            Command::Set { path, value } => {
                set(root, &path, &value)?;
                changed = true;
            }
            Command::Add { path, kind, value } => {
                add(root, &path, &kind, value.as_deref())?;
                changed = true;
            }
            Command::Delete(path) => {
                delete(root, &path)?;
                changed = true;
            }
            Command::Clear(kind) => {
                *root = if kind.as_deref() == Some("array") { Plist::Array(Vec::new()) } else { Plist::Dict(Vec::new()) };
                changed = true;
            }
            Command::Save => {}
            Command::Help => {
                output.push_str(COMMANDS);
                output.push('\n');
            }
            Command::Exit => break,
        }
    }
    Ok(Run { output, changed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn sample() -> Plist {
        Plist::Dict(vec![
            ("Name".into(), Plist::Str("Ada".into())),
            ("Count".into(), Plist::Int(2)),
            ("Items".into(), Plist::Array(vec![Plist::Str("a".into()), Plist::Str("b".into())])),
            ("Nested".into(), Plist::Dict(vec![("Flag".into(), Plist::Bool(false))])),
        ])
    }

    fn cmds(v: &[&str]) -> Vec<String> {
        args(v)
    }

    #[test]
    fn options_collect_commands_and_the_file() {
        let o = parse(&args(&["-c", "Print :Name", "-x", "-c", "Save", "app.plist"])).unwrap();
        assert_eq!(o.commands, vec!["Print :Name".to_string(), "Save".to_string()]);
        assert!(o.xml);
        assert_eq!(o.file.as_deref(), Some("app.plist"));
        assert!(!o.help);
    }

    #[test]
    fn options_need_a_file_unless_help_is_asked() {
        assert!(parse(&args(&["-c", "Save"])).is_err());
        assert!(parse(&args(&["-h"])).unwrap().help);
        assert!(parse(&args(&["-z", "a.plist"])).is_err());
        assert!(parse(&args(&["a.plist", "b.plist"])).is_err());
        assert!(parse(&args(&["-c"])).is_err());
    }

    #[test]
    fn words_group_quotes_and_escapes() {
        assert_eq!(words("Set :Name \"Ada Lovelace\"").unwrap(), vec!["Set", ":Name", "Ada Lovelace"]);
        assert_eq!(words("Set  :A   x").unwrap(), vec!["Set", ":A", "x"]);
        assert_eq!(words("Set :A \"\"").unwrap(), vec!["Set", ":A", ""]);
        assert_eq!(words("Set :A a\\ b").unwrap(), vec!["Set", ":A", "a b"]);
        assert!(words("Set :A \"open").is_err());
        assert!(words("Set :A x\\").is_err());
    }

    #[test]
    fn segments_split_on_colons() {
        assert_eq!(segments(":Dict:Key:0"), vec!["Dict", "Key", "0"]);
        assert_eq!(segments("Key"), vec!["Key"]);
        assert!(segments(":").is_empty());
        assert!(segments("").is_empty());
    }

    #[test]
    fn commands_parse_their_operands() {
        assert_eq!(parse_command("Print").unwrap(), Command::Print(String::new()));
        assert_eq!(parse_command("print :Name").unwrap(), Command::Print(":Name".into()));
        assert_eq!(
            parse_command("Set :Name Grace Hopper").unwrap(),
            Command::Set { path: ":Name".into(), value: "Grace Hopper".into() }
        );
        assert_eq!(
            parse_command("Add :Count real 1.5").unwrap(),
            Command::Add { path: ":Count".into(), kind: "float".into(), value: Some("1.5".into()) }
        );
        assert_eq!(
            parse_command("Add :Box dict").unwrap(),
            Command::Add { path: ":Box".into(), kind: "dictionary".into(), value: None }
        );
        assert_eq!(parse_command("Delete :Name").unwrap(), Command::Delete(":Name".into()));
        assert_eq!(parse_command("Clear").unwrap(), Command::Clear(None));
        assert_eq!(parse_command("Clear array").unwrap(), Command::Clear(Some("array".into())));
        assert_eq!(parse_command("exit").unwrap(), Command::Exit);
    }

    #[test]
    fn bad_commands_are_refused() {
        assert!(parse_command("").is_err());
        assert!(parse_command("Frobnicate :x").is_err());
        assert!(parse_command("Set :x").is_err());
        assert!(parse_command("Add :x").is_err());
        assert!(parse_command("Add :x wrongtype 1").is_err());
        assert!(parse_command("Add :x array 1").is_err());
        assert!(parse_command("Delete").is_err());
        assert!(parse_command("Clear string").is_err());
    }

    #[test]
    fn print_shows_scalars_bare_and_containers_in_the_chosen_form() {
        let mut root = sample();
        let out = run(&mut root, &cmds(&["Print :Name", "Print :Count", "Print :Nested:Flag"]), false).unwrap();
        assert_eq!(out.output, "Ada\n2\nfalse\n");
        assert!(!out.changed);
        let xml = run(&mut root, &cmds(&["Print :Count"]), true).unwrap();
        assert_eq!(xml.output, "2\n");
        let whole = run(&mut root, &cmds(&["Print :Items"]), true).unwrap();
        assert!(whole.output.contains("<string>a</string>"));
    }

    #[test]
    fn print_of_a_missing_entry_is_an_error() {
        let mut root = sample();
        let err = run(&mut root, &cmds(&["Print :Nope"]), false).unwrap_err();
        assert!(err.contains("Does Not Exist"));
    }

    #[test]
    fn set_keeps_the_type_the_entry_has() {
        let mut root = sample();
        let out = run(&mut root, &cmds(&["Set :Count 5", "Set :Name Grace Hopper"]), false).unwrap();
        assert!(out.changed);
        assert_eq!(lookup(&root, ":Count").unwrap(), &Plist::Int(5));
        assert_eq!(lookup(&root, ":Name").unwrap(), &Plist::Str("Grace Hopper".into()));
        assert!(run(&mut root, &cmds(&["Set :Count lots"]), false).is_err());
        assert!(run(&mut root, &cmds(&["Set :Items x"]), false).is_err());
        assert!(run(&mut root, &cmds(&["Set :Missing x"]), false).is_err());
    }

    #[test]
    fn set_reaches_array_items_by_index() {
        let mut root = sample();
        run(&mut root, &cmds(&["Set :Items:1 c"]), false).unwrap();
        assert_eq!(lookup(&root, ":Items:1").unwrap(), &Plist::Str("c".into()));
    }

    #[test]
    fn add_puts_a_new_key_in_a_dictionary_and_an_index_in_an_array() {
        let mut root = sample();
        run(&mut root, &cmds(&["Add :Extra string hi", "Add :Items:0 integer 9"]), false).unwrap();
        assert_eq!(lookup(&root, ":Extra").unwrap(), &Plist::Str("hi".into()));
        assert_eq!(lookup(&root, ":Items:0").unwrap(), &Plist::Int(9));
        assert_eq!(lookup(&root, ":Items:1").unwrap(), &Plist::Str("a".into()));
    }

    #[test]
    fn add_refuses_an_existing_key_and_a_bad_index() {
        let mut root = sample();
        assert!(run(&mut root, &cmds(&["Add :Name string dup"]), false).is_err());
        assert!(run(&mut root, &cmds(&["Add :Items:9 string x"]), false).is_err());
        assert!(run(&mut root, &cmds(&["Add :Items:first string x"]), false).is_err());
    }

    #[test]
    fn add_creates_empty_containers() {
        let mut root = sample();
        run(&mut root, &cmds(&["Add :Box dict", "Add :List array", "Add :Box:Inner bool yes"]), false).unwrap();
        assert_eq!(lookup(&root, ":Box:Inner").unwrap(), &Plist::Bool(true));
        assert_eq!(lookup(&root, ":List").unwrap(), &Plist::Array(Vec::new()));
    }

    #[test]
    fn delete_removes_keys_and_array_items() {
        let mut root = sample();
        run(&mut root, &cmds(&["Delete :Items:0", "Delete :Nested"]), false).unwrap();
        assert_eq!(lookup(&root, ":Items").unwrap(), &Plist::Array(vec![Plist::Str("b".into())]));
        assert!(lookup(&root, ":Nested").is_err());
        assert!(run(&mut root, &cmds(&["Delete :Missing"]), false).is_err());
        assert!(run(&mut root, &cmds(&["Delete :Items:5"]), false).is_err());
    }

    #[test]
    fn clear_empties_the_root_as_a_dictionary_or_an_array() {
        let mut root = sample();
        run(&mut root, &cmds(&["Clear"]), false).unwrap();
        assert_eq!(root, Plist::Dict(Vec::new()));
        let mut root = sample();
        run(&mut root, &cmds(&["Clear array"]), false).unwrap();
        assert_eq!(root, Plist::Array(Vec::new()));
    }

    #[test]
    fn exit_stops_the_run_and_an_error_stops_it_at_that_command() {
        let mut root = sample();
        let out = run(&mut root, &cmds(&["Exit", "Delete :Name"]), false).unwrap();
        assert!(!out.changed);
        assert!(lookup(&root, ":Name").is_ok());
        let mut root = sample();
        assert!(run(&mut root, &cmds(&["Delete :Name", "Delete :Missing", "Delete :Count"]), false).is_err());
        assert!(lookup(&root, ":Count").is_ok());
    }

    #[test]
    fn help_and_save_produce_text_and_no_change() {
        let mut root = sample();
        let out = run(&mut root, &cmds(&["Help", "Save"]), false).unwrap();
        assert!(out.output.starts_with("Commands:"));
        assert!(!out.changed);
    }
}
