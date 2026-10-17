//! `osacompile`: checks a script in the BifrOSt scripting language and writes its compiled form. The compiled file is a
//! JSON document (format `bifrost-osa`, version 1, see `bifrost_core::script::compile`): `source` is the canonical
//! source text (CRLF to LF, trailing blanks dropped) and `ast` is the parsed syntax tree, each node an object whose
//! `node` names its kind. `osascript` runs a compiled file by reading its `source` back, so the two tools agree.
//!
//! A syntax error is reported with its line and nothing is written.

use bifrost_core::script;
use std::path::PathBuf;

pub const USAGE: &str = "usage: osacompile [-o output] (-e statement ... | file)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Lines(Vec<String>),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub input: Input,
    /// `-o`: where the compiled file goes. Without it: the input file with `.scpt`, or `script.scpt` for `-e` lines.
    pub output: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the command line: `-o output`, repeated `-e statement`, or one file.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut lines = Vec::new();
    let mut output: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next: Option<String> = args.get(i + 1).cloned();
        match a {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-o" => {
                output = Some(PathBuf::from(next.ok_or("-o needs an output path")?));
                i += 1;
            }
            "-e" => {
                lines.push(next.ok_or("-e needs a statement")?);
                i += 1;
            }
            _ if a.starts_with('-') && a.len() > 1 => return Err(format!("unknown option {a}")),
            _ => {
                if file.is_some() {
                    return Err("only one script file can be compiled".into());
                }
                file = Some(PathBuf::from(a));
            }
        }
        i += 1;
    }
    let input = match (lines.is_empty(), file) {
        (false, None) => Input::Lines(lines),
        (true, Some(f)) => Input::File(f),
        (false, Some(_)) => return Err("give either -e statements or a file, not both".into()),
        (true, None) => return Err("nothing to compile: give -e statements or a file".into()),
    };
    Ok(Parsed::Run(Options { input, output }))
}

/// The script text of the input.
pub fn source_of(input: &Input) -> Result<String, String> {
    match input {
        Input::Lines(lines) => Ok(lines.join("\n")),
        Input::File(path) => std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display())),
    }
}

/// Where the compiled file goes when `-o` is not given.
pub fn output_of(opts: &Options) -> PathBuf {
    match (&opts.output, &opts.input) {
        (Some(out), _) => out.clone(),
        (None, Input::File(path)) => path.with_extension("scpt"),
        (None, Input::Lines(_)) => PathBuf::from("script.scpt"),
    }
}

/// Compiles the input and writes the compiled file; returns the path it wrote.
pub fn compile_to_file(opts: &Options) -> Result<PathBuf, String> {
    let source = source_of(&opts.input)?;
    let compiled = script::compile(&source).map_err(|e| e.to_string())?;
    let out = output_of(opts);
    std::fs::write(&out, compiled).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn e_statements_or_one_file() {
        match parse(&args(&["-e", "return 1", "-e", "return 2"])).unwrap() {
            Parsed::Run(o) => assert_eq!(o.input, Input::Lines(args(&["return 1", "return 2"]))),
            Parsed::Help => panic!(),
        }
        match parse(&args(&["-o", "out.scpt", "notes.bfs"])).unwrap() {
            Parsed::Run(o) => {
                assert_eq!(o.input, Input::File(PathBuf::from("notes.bfs")));
                assert_eq!(o.output, Some(PathBuf::from("out.scpt")));
            }
            Parsed::Help => panic!(),
        }
    }

    #[test]
    fn bad_command_lines_are_refused() {
        assert!(parse(&args(&[])).is_err(), "nothing to compile");
        assert!(parse(&args(&["-e", "return 1", "x.bfs"])).is_err(), "statements and a file together");
        assert!(parse(&args(&["a.bfs", "b.bfs"])).is_err());
        assert!(parse(&args(&["-o"])).is_err());
        assert!(parse(&args(&["-x", "a.bfs"])).is_err());
        assert_eq!(parse(&args(&["--help"])).unwrap(), Parsed::Help);
    }

    #[test]
    fn the_default_output_is_beside_the_source() {
        let file = Options { input: Input::File(PathBuf::from("/tmp/notes.bfs")), output: None };
        assert_eq!(output_of(&file), PathBuf::from("/tmp/notes.scpt"));
        let lines = Options { input: Input::Lines(vec!["return 1".into()]), output: None };
        assert_eq!(output_of(&lines), PathBuf::from("script.scpt"));
    }

    #[test]
    fn a_syntax_error_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("osacompile-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("bad.scpt");
        let opts = Options { input: Input::Lines(vec!["set a to )".into()]), output: Some(out.clone()) };
        assert!(compile_to_file(&opts).is_err());
        assert!(!out.exists());
        let good = Options { input: Input::Lines(vec!["return 1".into()]), output: Some(out.clone()) };
        assert_eq!(compile_to_file(&good).unwrap(), out);
        assert_eq!(script::compiled_source(&std::fs::read_to_string(&out).unwrap()).as_deref(), Some("return 1\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
