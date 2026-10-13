//! `what`: finds the `@(#)` version markers that build tools embed in binaries and prints the text after each one. A marker
//! runs until the first `"`, `>`, backslash, newline or NUL byte, the same stop set the BSD `what` tool uses. Output per
//! file is the file name with a colon, then each marker text on its own line, indented by a tab. With `-s`, only the
//! first marker of each file is printed.
//!
//! Gap: the tool reads whole files into memory; it has no special handling for very large inputs.

pub const USAGE: &str = "usage: what [-s] file ...";
const MARKER: &[u8] = b"@(#)";

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    /// `-s`: stop after the first marker of each file.
    pub first_only: bool,
    /// The files to read, in the order given.
    pub files: Vec<String>,
}

/// What the command line asks for: the usage text, or a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments. `-s` may appear anywhere before `--`, `-h` and `--help` ask for the usage text, and every other
/// argument is a file name. A lone `-` is a file name.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut first_only = false;
    let mut files = Vec::new();
    let mut options_done = false;
    for arg in args {
        if !options_done && arg == "--" {
            options_done = true;
        } else if !options_done && (arg == "-h" || arg == "-help" || arg == "--help") {
            return Ok(Parsed::Help);
        } else if !options_done && arg == "-s" {
            first_only = true;
        } else if !options_done && arg.starts_with('-') && arg != "-" {
            return Err(format!("unknown option {arg}"));
        } else {
            files.push(arg.clone());
        }
    }
    if files.is_empty() {
        return Err("no input files".to_string());
    }
    Ok(Parsed::Run(Options { first_only, files }))
}

/// The marker texts found in the bytes of one file, in file order. Each text is decoded as UTF-8 with invalid bytes
/// replaced. With `first_only`, at most one text is returned.
pub fn markers(data: &[u8], first_only: bool) -> Vec<String> {
    let mut found = Vec::new();
    let mut i = 0;
    while let Some(pos) = find(&data[i..], MARKER) {
        let start = i + pos + MARKER.len();
        let end = data[start..]
            .iter()
            .position(|b| matches!(b, b'"' | b'>' | b'\\' | b'\n' | 0))
            .map_or(data.len(), |n| start + n);
        found.push(String::from_utf8_lossy(&data[start..end]).into_owned());
        if first_only {
            break;
        }
        i = end;
    }
    found
}

/// The position of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The report for one file: its name with a colon, then each marker on an indented line.
pub fn render(name: &str, markers: &[String]) -> String {
    let mut out = format!("{name}:\n");
    for m in markers {
        out.push('\t');
        out.push_str(m);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_marker_runs_to_the_stop_byte() {
        assert_eq!(markers(b"xx@(#)Version 1.2\"tail", false), vec!["Version 1.2"]);
        assert_eq!(markers(b"@(#)a>b", false), vec!["a"]);
        assert_eq!(markers(b"@(#)a\\b", false), vec!["a"]);
        assert_eq!(markers(b"@(#)a\nb", false), vec!["a"]);
        assert_eq!(markers(b"@(#)a\0b", false), vec!["a"]);
    }

    #[test]
    fn a_marker_at_the_end_of_the_data_runs_to_the_end() {
        assert_eq!(markers(b"@(#)last", false), vec!["last"]);
    }

    #[test]
    fn every_marker_is_listed_in_order_unless_only_the_first_is_asked_for() {
        let data = b"@(#)one\0@(#)two\0@(#)three";
        assert_eq!(markers(data, false), vec!["one", "two", "three"]);
        assert_eq!(markers(data, true), vec!["one"]);
    }

    #[test]
    fn a_file_without_markers_has_none() {
        assert!(markers(b"plain bytes @ (#) and (#)", false).is_empty());
        assert!(markers(b"", false).is_empty());
    }

    #[test]
    fn a_marker_with_no_text_is_an_empty_string() {
        assert_eq!(markers(b"@(#)\0", false), vec![""]);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_dropped() {
        assert_eq!(markers(b"@(#)a\xffb\0", false), vec!["a\u{fffd}b"]);
    }

    #[test]
    fn the_report_names_the_file_and_indents_each_marker() {
        assert_eq!(render("tool", &["1.0".to_string(), "x".to_string()]), "tool:\n\t1.0\n\tx\n");
        assert_eq!(render("empty", &[]), "empty:\n");
    }

    #[test]
    fn options_and_files_are_read() {
        assert_eq!(
            parse(&args(&["-s", "a", "b"])),
            Ok(Parsed::Run(Options { first_only: true, files: args(&["a", "b"]) }))
        );
        assert_eq!(parse(&args(&["a"])), Ok(Parsed::Run(Options { first_only: false, files: args(&["a"]) })));
    }

    #[test]
    fn a_dash_after_double_dash_is_a_file_name() {
        assert_eq!(
            parse(&args(&["--", "-s"])),
            Ok(Parsed::Run(Options { first_only: false, files: args(&["-s"]) }))
        );
        assert_eq!(parse(&args(&["-"])), Ok(Parsed::Run(Options { first_only: false, files: args(&["-"]) })));
    }

    #[test]
    fn help_and_bad_input_are_reported() {
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
        assert_eq!(parse(&args(&["--help"])), Ok(Parsed::Help));
        assert!(parse(&args(&["-x", "a"])).is_err());
        assert_eq!(parse(&args(&["-s"])), Err("no input files".to_string()));
    }
}
