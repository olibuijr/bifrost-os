//! `plutil`: checks, prints, converts and edits property lists. Logic lives here; the binary reads the files and writes
//! the results (see `bifrost_core::plist` for the value model and the text forms).
//!
//! Supported: `-lint` (the default), `-p`, `-convert xml1|json`, `-insert`, `-replace`, `-remove`, `-extract` (to `xml1`,
//! `json` or `raw`), `-type`, `-create xml1|json`, with `-s`, `-r`, `-n`, `-o path`, `-e ext`, `-expect type`, `-append`,
//! `--`, and `-` for standard input. Edits and conversions write XML text; `binary1` input and output, and the `swift`
//! and `objc` forms, are refused with a message rather than written wrong.

use bifrost_core::plist::{self, Plist};
use std::fmt::Write as _;

pub const USAGE: &str = "usage: plutil [command_option] [other_options] file ...\n\
  commands: -help | -lint | -p | -convert fmt | -insert keypath -type [value] [-append] | -replace keypath -type value\n\
            -remove keypath | -extract keypath fmt [-expect type] | -type keypath [-expect type] | -create fmt\n\
  options:  -s (silent on success) -r (readable JSON) -n (raw without newline) -o path -e extension -- (end of options)\n\
  formats:  xml1 binary1 json raw swift objc      types: -bool -integer -float -string -date -data -xml -json -array -dictionary";

/// The output formats the tool names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xml1,
    Binary1,
    Json,
    Raw,
    Swift,
    Objc,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format, String> {
        Ok(match s {
            "xml1" => Format::Xml1,
            "binary1" => Format::Binary1,
            "json" => Format::Json,
            "raw" => Format::Raw,
            "swift" => Format::Swift,
            "objc" => Format::Objc,
            other => return Err(format!("unknown format {other}")),
        })
    }
}

/// The value types an insert or replace takes, as the flag names them without the dash.
pub const VALUE_TYPES: &[&str] = &["bool", "integer", "float", "string", "date", "data", "xml", "json", "array", "dictionary"];

/// One operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Help,
    Lint,
    Print,
    Convert(Format),
    Insert { path: String, kind: String, value: Option<String>, append: bool },
    Replace { path: String, kind: String, value: String },
    Remove { path: String },
    Extract { path: String, format: Format, expect: Option<String> },
    Type { path: String, expect: Option<String> },
    Create(Format),
}

/// The parsed command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub command: Command,
    pub silent: bool,
    pub pretty: bool,
    pub no_newline: bool,
    pub output: Option<String>,
    pub extension: Option<String>,
    pub files: Vec<String>,
}

/// What an operation produced for one file.
#[derive(Debug, PartialEq)]
pub enum Produced {
    /// The file parsed (`-lint`).
    Lint,
    /// Text for standard output.
    Text(String),
    /// Bytes for the output file: the converted or edited file, or an extracted value.
    Bytes(Vec<u8>),
}

struct Cursor<'a> {
    args: &'a [String],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self) -> Option<&'a String> {
        let v = self.args.get(self.pos);
        if v.is_some() {
            self.pos += 1;
        }
        v
    }

    fn take(&mut self, what: &str) -> Result<String, String> {
        self.next().cloned().ok_or_else(|| format!("{what} needs a value"))
    }
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut c = Cursor { args, pos: 0 };
    let mut cmd: Option<Command> = None;
    let mut expect: Option<String> = None;
    let mut append = false;
    let mut silent = false;
    let mut pretty = false;
    let mut no_newline = false;
    let mut output = None;
    let mut extension = None;
    let mut files = Vec::new();
    let mut only_files = false;

    fn set(cmd: &mut Option<Command>, new: Command) -> Result<(), String> {
        if cmd.is_some() {
            return Err("only one operation can be given".into());
        }
        *cmd = Some(new);
        Ok(())
    }

    while let Some(arg) = c.next() {
        if only_files || arg == "-" || !arg.starts_with('-') {
            files.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => only_files = true,
            "-help" | "-h" | "--help" => return Ok(Options { command: Command::Help, silent, pretty, no_newline, output, extension, files }),
            "-s" => silent = true,
            "-r" => pretty = true,
            "-n" => no_newline = true,
            "-append" => append = true,
            "-o" => output = Some(c.take("-o")?),
            "-e" => extension = Some(c.take("-e")?),
            "-expect" => expect = Some(type_name(&c.take("-expect")?)?),
            "-lint" => set(&mut cmd, Command::Lint)?,
            "-p" => set(&mut cmd, Command::Print)?,
            "-convert" => set(&mut cmd, Command::Convert(Format::parse(&c.take("-convert")?)?))?,
            "-create" => set(&mut cmd, Command::Create(Format::parse(&c.take("-create")?)?))?,
            "-remove" => set(&mut cmd, Command::Remove { path: c.take("-remove")? })?,
            "-extract" => {
                let path = c.take("-extract")?;
                let format = Format::parse(&c.take("-extract")?)?;
                set(&mut cmd, Command::Extract { path, format, expect: None })?;
            }
            "-type" => set(&mut cmd, Command::Type { path: c.take("-type")?, expect: None })?,
            "-insert" => {
                let path = c.take("-insert")?;
                let kind = value_type(&c.take("-insert")?)?;
                let value = if needs_value(&kind) { Some(c.take("-insert")?) } else { None };
                set(&mut cmd, Command::Insert { path, kind, value, append: false })?;
            }
            "-replace" => {
                let path = c.take("-replace")?;
                let kind = value_type(&c.take("-replace")?)?;
                if !needs_value(&kind) {
                    return Err(format!("-replace needs a value, not -{kind}"));
                }
                let value = c.take("-replace")?;
                set(&mut cmd, Command::Replace { path, kind, value })?;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }

    let mut command = cmd.unwrap_or(Command::Lint);
    match &mut command {
        Command::Extract { expect: e, .. } | Command::Type { expect: e, .. } => *e = expect,
        Command::Insert { append: a, .. } => *a = append,
        _ => {
            if expect.is_some() {
                return Err("-expect applies only to -extract and -type".into());
            }
            if append {
                return Err("-append applies only to -insert".into());
            }
        }
    }
    if files.is_empty() && command != Command::Help {
        return Err(format!("no input file\n{USAGE}"));
    }
    Ok(Options { command, silent, pretty, no_newline, output, extension, files })
}

fn type_name(s: &str) -> Result<String, String> {
    const NAMES: &[&str] = &["bool", "integer", "float", "string", "date", "data", "array", "dictionary"];
    if NAMES.contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!("unknown type {s}"))
    }
}

fn value_type(flag: &str) -> Result<String, String> {
    let name = flag.strip_prefix('-').unwrap_or(flag);
    if VALUE_TYPES.contains(&name) {
        Ok(name.to_string())
    } else {
        Err(format!("unknown value type {flag}"))
    }
}

fn needs_value(kind: &str) -> bool {
    !matches!(kind, "array" | "dictionary")
}

/// The property-list value an insert or replace writes.
pub fn value_for(kind: &str, value: Option<&str>) -> Result<Plist, String> {
    match (kind, value) {
        ("array", _) => return Ok(Plist::Array(Vec::new())),
        ("dictionary", _) => return Ok(Plist::Dict(Vec::new())),
        _ => {}
    }
    let v = value.ok_or_else(|| format!("-{kind} needs a value"))?;
    Ok(match kind {
        "bool" => Plist::Bool(v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")),
        "integer" => Plist::Int(v.trim().parse().map_err(|_| format!("{v} is not an integer"))?),
        "float" => Plist::Real(v.trim().parse().map_err(|_| format!("{v} is not a number"))?),
        "string" => Plist::Str(v.to_string()),
        "date" => Plist::Date(v.to_string()),
        "data" => Plist::Data(plist::base64_decode(v).ok_or("the data is not base64")?),
        "xml" => plist::parse_xml(v)?,
        "json" => plist::from_json(&bifrost_core::json::parse(v)?)?,
        other => return Err(format!("unknown value type {other}")),
    })
}

fn expect_type(p: &Plist, expect: Option<&str>) -> Result<(), String> {
    match expect {
        Some(e) if e != p.type_name() => Err(format!("the value has type {}, not {e}", p.type_name())),
        _ => Ok(()),
    }
}

/// The text `-extract raw` prints: the unwrapped value, as its type's rules say.
pub fn raw_text(p: &Plist) -> String {
    match p {
        Plist::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
        Plist::Int(n) => n.to_string(),
        Plist::Real(f) => f.to_string(),
        Plist::Str(s) | Plist::Date(s) => s.clone(),
        Plist::Data(d) => plist::base64_encode(d),
        Plist::Array(items) => items.len().to_string(),
        Plist::Dict(items) => {
            let mut keys: Vec<&str> = items.iter().map(|e| e.0.as_str()).collect();
            keys.sort_unstable();
            let mut out = String::new();
            for k in keys {
                let _ = writeln!(out, "{k}");
            }
            out.trim_end_matches('\n').to_string()
        }
    }
}

fn render(p: &Plist, format: Format, pretty: bool) -> Result<Vec<u8>, String> {
    match format {
        Format::Xml1 => Ok(plist::to_xml(p).into_bytes()),
        Format::Json => Ok(plist::to_json(p, pretty)?.into_bytes()),
        Format::Binary1 => Err("the binary1 format is not written yet".into()),
        Format::Raw => Err("the raw format is only for -extract".into()),
        Format::Swift | Format::Objc => Err("the swift and objc formats are not supported yet".into()),
    }
}

/// Runs the operation on one file's bytes. `-create` ignores the input.
pub fn process(opts: &Options, input: &[u8]) -> Result<Produced, String> {
    match &opts.command {
        Command::Help => Ok(Produced::Text(format!("{USAGE}\n"))),
        Command::Create(format) => Ok(Produced::Bytes(render(&Plist::Dict(Vec::new()), *format, opts.pretty)?)),
        Command::Lint => {
            plist::parse_bytes(input)?;
            Ok(Produced::Lint)
        }
        Command::Print => {
            let p = plist::parse_bytes(input)?;
            Ok(Produced::Text(format!("{}\n", plist::to_print(&p))))
        }
        Command::Convert(format) => {
            let p = plist::parse_bytes(input)?;
            Ok(Produced::Bytes(render(&p, *format, opts.pretty)?))
        }
        Command::Insert { path, kind, value, append } => {
            let mut p = plist::parse_bytes(input)?;
            let v = value_for(kind, value.as_deref())?;
            plist::insert(&mut p, path, v, *append)?;
            Ok(Produced::Bytes(render(&p, Format::Xml1, false)?))
        }
        Command::Replace { path, kind, value } => {
            let mut p = plist::parse_bytes(input)?;
            let v = value_for(kind, Some(value.as_str()))?;
            plist::replace(&mut p, path, v)?;
            Ok(Produced::Bytes(render(&p, Format::Xml1, false)?))
        }
        Command::Remove { path } => {
            let mut p = plist::parse_bytes(input)?;
            plist::remove(&mut p, path)?;
            Ok(Produced::Bytes(render(&p, Format::Xml1, false)?))
        }
        Command::Extract { path, format, expect } => {
            let p = plist::parse_bytes(input)?;
            let v = plist::get(&p, path)?;
            expect_type(v, expect.as_deref())?;
            match format {
                Format::Raw => {
                    let mut text = raw_text(v);
                    if !opts.no_newline {
                        text.push('\n');
                    }
                    Ok(Produced::Bytes(text.into_bytes()))
                }
                other => Ok(Produced::Bytes(render(v, *other, opts.pretty)?)),
            }
        }
        Command::Type { path, expect } => {
            let p = plist::parse_bytes(input)?;
            let v = plist::get(&p, path)?;
            expect_type(v, expect.as_deref())?;
            Ok(Produced::Text(format!("{}\n", v.type_name())))
        }
    }
}

/// Where a file's bytes go: `-o` when given, else the input file for edits and conversions (with `-e` swapping the
/// extension), else standard output. Standard input goes to standard output.
pub fn target_for(opts: &Options, file: &str) -> String {
    if let Some(o) = &opts.output {
        return o.clone();
    }
    match opts.command {
        Command::Convert(_) | Command::Insert { .. } | Command::Replace { .. } | Command::Remove { .. } | Command::Create(_) => {
            if file == "-" {
                return "-".into();
            }
            match &opts.extension {
                Some(ext) => std::path::Path::new(file).with_extension(ext).to_string_lossy().into_owned(),
                None => file.to_string(),
            }
        }
        _ => "-".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn xml(v: &Plist) -> Vec<u8> {
        plist::to_xml(v).into_bytes()
    }

    fn sample() -> Plist {
        Plist::Dict(vec![
            ("AppName".into(), Plist::Str("Notes".into())),
            ("Versions".into(), Plist::Array(vec![Plist::Int(1), Plist::Int(2)])),
            ("Flags".into(), Plist::Dict(vec![("on".into(), Plist::Bool(true))])),
        ])
    }

    #[test]
    fn default_operation_is_lint_and_files_follow_options() {
        let o = parse(&args(&["a.plist", "-s", "--", "-odd.plist"])).unwrap();
        assert_eq!(o.command, Command::Lint);
        assert!(o.silent);
        assert_eq!(o.files, ["a.plist", "-odd.plist"]);
        assert!(parse(&args(&["-s"])).unwrap_err().contains("no input file"));
        assert_eq!(parse(&args(&["-help"])).unwrap().command, Command::Help);
    }

    #[test]
    fn edit_commands_parse_their_operands() {
        let o = parse(&args(&["-insert", "Flags.new", "-string", "x", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Insert { path: "Flags.new".into(), kind: "string".into(), value: Some("x".into()), append: false });
        let o = parse(&args(&["-insert", "Versions", "-integer", "3", "-append", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Insert { path: "Versions".into(), kind: "integer".into(), value: Some("3".into()), append: true });
        let o = parse(&args(&["-insert", "Empty", "-dictionary", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Insert { path: "Empty".into(), kind: "dictionary".into(), value: None, append: false });
        let o = parse(&args(&["-replace", "AppName", "-bool", "YES", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Replace { path: "AppName".into(), kind: "bool".into(), value: "YES".into() });
        let o = parse(&args(&["-remove", "Flags", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Remove { path: "Flags".into() });
        assert!(parse(&args(&["-replace", "k", "-dictionary", "f"])).is_err());
        assert!(parse(&args(&["-insert", "k", "-nonsense", "v", "f"])).is_err());
        assert!(parse(&args(&["-lint", "-p", "f"])).unwrap_err().contains("only one operation"));
    }

    #[test]
    fn extract_type_and_expect_parse_and_check() {
        let o = parse(&args(&["-extract", "Versions", "raw", "-expect", "array", "-n", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Extract { path: "Versions".into(), format: Format::Raw, expect: Some("array".into()) });
        assert!(o.no_newline);
        let o = parse(&args(&["-type", "Flags", "-expect", "dictionary", "f.plist"])).unwrap();
        assert_eq!(o.command, Command::Type { path: "Flags".into(), expect: Some("dictionary".into()) });
        assert!(parse(&args(&["-lint", "-expect", "array", "f"])).is_err());
        assert!(parse(&args(&["-type", "k", "-expect", "nope", "f"])).is_err());
        assert!(parse(&args(&["-convert", "binary2", "f"])).is_err());
    }

    #[test]
    fn values_are_made_from_their_type_flags() {
        assert_eq!(value_for("bool", Some("yes")), Ok(Plist::Bool(true)));
        assert_eq!(value_for("bool", Some("NO")), Ok(Plist::Bool(false)));
        assert_eq!(value_for("integer", Some("-4")), Ok(Plist::Int(-4)));
        assert!(value_for("integer", Some("4.5")).is_err());
        assert_eq!(value_for("float", Some("0.5")), Ok(Plist::Real(0.5)));
        assert_eq!(value_for("data", Some("TWFu")), Ok(Plist::Data(b"Man".to_vec())));
        assert_eq!(value_for("xml", Some("<true/>")), Ok(Plist::Bool(true)));
        assert_eq!(value_for("json", Some("[1, \"a\"]")), Ok(Plist::Array(vec![Plist::Int(1), Plist::Str("a".into())])));
        assert_eq!(value_for("array", None), Ok(Plist::Array(Vec::new())));
        assert!(value_for("string", None).is_err());
    }

    #[test]
    fn lint_print_and_convert_produce_the_expected_text() {
        let input = xml(&sample());
        let opts = |command| Options { command, silent: false, pretty: false, no_newline: false, output: None, extension: None, files: vec!["f".into()] };
        assert_eq!(process(&opts(Command::Lint), &input), Ok(Produced::Lint));
        assert!(process(&opts(Command::Lint), b"<dict>").is_err());
        let Ok(Produced::Text(t)) = process(&opts(Command::Print), &input) else { panic!("print") };
        assert!(t.contains("\"AppName\" => Notes;"), "{t}");
        let Ok(Produced::Bytes(json)) = process(&opts(Command::Convert(Format::Json)), &input) else { panic!("json") };
        assert!(String::from_utf8(json).unwrap().starts_with("{\"AppName\":\"Notes\""));
        assert!(process(&opts(Command::Convert(Format::Binary1)), &input).is_err());
        assert!(process(&opts(Command::Convert(Format::Swift)), &input).is_err());
    }

    #[test]
    fn edits_write_back_and_extract_unwraps_values() {
        let input = xml(&sample());
        let opts = |command, no_newline| Options { command, silent: false, pretty: false, no_newline, output: None, extension: None, files: vec!["f".into()] };
        let Ok(Produced::Bytes(out)) = process(&opts(Command::Insert { path: "Versions".into(), kind: "integer".into(), value: Some("9".into()), append: true }, false), &input) else { panic!("insert") };
        let edited = plist::parse_bytes(&out).unwrap();
        assert_eq!(plist::get(&edited, "Versions.2"), Ok(&Plist::Int(9)));

        let Ok(Produced::Bytes(out)) = process(&opts(Command::Replace { path: "AppName".into(), kind: "string".into(), value: "Pages".into() }, false), &input) else { panic!("replace") };
        assert_eq!(plist::get(&plist::parse_bytes(&out).unwrap(), "AppName"), Ok(&Plist::Str("Pages".into())));

        let Ok(Produced::Bytes(out)) = process(&opts(Command::Remove { path: "Flags".into() }, false), &input) else { panic!("remove") };
        assert!(plist::get(&plist::parse_bytes(&out).unwrap(), "Flags").is_err());

        let Ok(Produced::Bytes(raw)) = process(&opts(Command::Extract { path: "AppName".into(), format: Format::Raw, expect: Some("string".into()) }, false), &input) else { panic!("raw") };
        assert_eq!(raw, b"Notes\n");
        let Ok(Produced::Bytes(raw)) = process(&opts(Command::Extract { path: "AppName".into(), format: Format::Raw, expect: None }, true), &input) else { panic!("raw -n") };
        assert_eq!(raw, b"Notes");
        let Ok(Produced::Bytes(raw)) = process(&opts(Command::Extract { path: "Flags".into(), format: Format::Raw, expect: None }, false), &input) else { panic!("keys") };
        assert_eq!(raw, b"on\n");
        assert!(process(&opts(Command::Extract { path: "Versions".into(), format: Format::Raw, expect: Some("dictionary".into()) }, false), &input).is_err());
        assert!(process(&opts(Command::Insert { path: "AppName".into(), kind: "string".into(), value: Some("x".into()), append: false }, false), &input).is_err());
    }

    #[test]
    fn type_and_create_work_without_a_file_body() {
        let input = xml(&sample());
        let opts = |command| Options { command, silent: false, pretty: false, no_newline: false, output: None, extension: None, files: vec!["f".into()] };
        assert_eq!(process(&opts(Command::Type { path: "Versions".into(), expect: None }), &input), Ok(Produced::Text("array\n".into())));
        assert!(process(&opts(Command::Type { path: "Versions".into(), expect: Some("dictionary".into()) }), &input).is_err());
        let Ok(Produced::Bytes(empty)) = process(&opts(Command::Create(Format::Xml1)), b"") else { panic!("create") };
        assert_eq!(plist::parse_bytes(&empty), Ok(Plist::Dict(Vec::new())));
    }

    #[test]
    fn targets_follow_the_operation() {
        let mut o = Options { command: Command::Lint, silent: false, pretty: false, no_newline: false, output: None, extension: None, files: vec![] };
        assert_eq!(target_for(&o, "a.plist"), "-");
        o.command = Command::Convert(Format::Json);
        assert_eq!(target_for(&o, "a.plist"), "a.plist");
        o.extension = Some("json".into());
        assert_eq!(target_for(&o, "a.plist"), "a.json");
        assert_eq!(target_for(&o, "-"), "-");
        o.output = Some("/tmp/x.out".into());
        assert_eq!(target_for(&o, "a.plist"), "/tmp/x.out");
        o.command = Command::Extract { path: "k".into(), format: Format::Xml1, expect: None };
        o.output = None;
        assert_eq!(target_for(&o, "a.plist"), "-");
    }
}
