//! BifrOSt scripting: the scripting dictionaries apps declare in their manifest (`dictionary`), and a small
//! AppleScript-style language that sends dictionary commands to apps as typed events.
//!
//! - `lexer` and `parser` turn source text into an `ast::Program`.
//! - `interp` runs a program against a [`interp::Host`], which owns every side effect (events, dialogs, shell, sleep).
//! - `compile` is what `osacompile` writes: a JSON document with the canonical source and the syntax tree.
//!
//! The front ends are `osascript` and `osacompile` in `springboard/cmd/src/bin`.

pub mod ast;
pub mod dictionary;
pub mod interp;
pub mod lexer;
pub mod parser;

use crate::json::Json;

/// The format name and version `compile` writes.
pub const COMPILED_FORMAT: &str = "bifrost-osa";
pub const COMPILED_VERSION: f64 = 1.0;

/// The canonical text of a script: CRLF line ends become LF, trailing blanks on each line are dropped, and the text
/// ends with one newline. Two sources that differ only in that way compile to the same canonical text.
pub fn canonical_source(source: &str) -> String {
    let unix = source.replace("\r\n", "\n");
    let mut out: Vec<&str> = unix.lines().map(|l| l.trim_end()).collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Checks a script and returns its compiled form as JSON text:
/// `{"format": "bifrost-osa", "version": 1, "source": <canonical source>, "ast": <syntax tree>}`.
/// The `ast` is `ast::program_json`: each node is an object whose `node` names its kind.
pub fn compile(source: &str) -> Result<String, lexer::ParseError> {
    let program = parser::parse(source)?;
    let doc = Json::Obj(vec![
        ("format".into(), Json::Str(COMPILED_FORMAT.into())),
        ("version".into(), Json::Num(COMPILED_VERSION)),
        ("source".into(), Json::Str(canonical_source(source))),
        ("ast".into(), ast::program_json(&program)),
    ]);
    Ok(doc.encode())
}

/// The `language` a compiled JavaScript script carries. A compiled AppleScript-style script has no `language` field (the
/// first version of the format had none), and `compiled_language` reads that as `LANG_APPLESCRIPT`.
pub const LANG_JAVASCRIPT: &str = "JavaScript";
pub const LANG_APPLESCRIPT: &str = "AppleScript";

/// The canonical text of a JavaScript script: line ends become LF and the text ends with one newline. Nothing else is
/// touched, because trailing blanks and blank lines can be part of a template literal.
pub fn canonical_js_source(source: &str) -> String {
    let mut text = source.replace("\r\n", "\n").replace('\r', "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The compiled form of a JavaScript script, as JSON text:
/// `{"format": "bifrost-osa", "version": 1, "language": "JavaScript", "source": <canonical source>}`. There is no syntax
/// tree: the JavaScript engine owns the grammar, so the caller checks the syntax before it writes this
/// (`js_automation::check_syntax`) and the engine parses the stored source again when the script runs.
pub fn compile_js(source: &str) -> String {
    let doc = Json::Obj(vec![
        ("format".into(), Json::Str(COMPILED_FORMAT.into())),
        ("version".into(), Json::Num(COMPILED_VERSION)),
        ("language".into(), Json::Str(LANG_JAVASCRIPT.into())),
        ("source".into(), Json::Str(canonical_js_source(source))),
    ]);
    doc.encode()
}

/// The language of a compiled script: `LANG_JAVASCRIPT` or `LANG_APPLESCRIPT`. `None` when the text is not a compiled
/// script or names a language this version does not know.
pub fn compiled_language(text: &str) -> Option<&'static str> {
    let doc = crate::json::parse(text).ok()?;
    if doc.str_field("format")? != COMPILED_FORMAT {
        return None;
    }
    match doc.str_field("language").as_deref() {
        None | Some(LANG_APPLESCRIPT) => Some(LANG_APPLESCRIPT),
        Some(LANG_JAVASCRIPT) => Some(LANG_JAVASCRIPT),
        Some(_) => None,
    }
}

/// The compiled form's canonical source, read back from `compile` output. None when the text is not a compiled script.
pub fn compiled_source(text: &str) -> Option<String> {
    let doc = crate::json::parse(text).ok()?;
    if doc.str_field("format")? != COMPILED_FORMAT {
        return None;
    }
    doc.str_field("source")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_source_normalises_line_ends_and_trailing_blanks() {
        assert_eq!(canonical_source("set a to 1  \r\nreturn a\r\n\r\n"), "set a to 1\nreturn a\n");
    }

    #[test]
    fn compile_round_trips_the_canonical_source() {
        let text = compile("on run argv\nreturn argv\nend run\n").unwrap();
        assert_eq!(compiled_source(&text).as_deref(), Some("on run argv\nreturn argv\nend run\n"));
        assert!(text.contains(r#""node":"return""#), "the AST is in the document");
    }

    #[test]
    fn a_compiled_javascript_script_keeps_its_source_and_names_its_language() {
        let text = compile_js("const s = `a  \r\nb`;\r\nrun = () => s");
        assert_eq!(compiled_language(&text), Some(LANG_JAVASCRIPT));
        assert_eq!(compiled_source(&text).as_deref(), Some("const s = `a  \nb`;\nrun = () => s\n"), "trailing blanks inside the source survive");
        assert!(!text.contains("\"ast\""));
    }

    #[test]
    fn a_compiled_applescript_script_and_older_files_read_as_applescript() {
        let text = compile("return 1\n").unwrap();
        assert_eq!(compiled_language(&text), Some(LANG_APPLESCRIPT));
        assert_eq!(compiled_language(r#"{"format":"bifrost-osa","version":1,"source":"x"}"#), Some(LANG_APPLESCRIPT));
        assert_eq!(compiled_language(r#"{"format":"bifrost-osa","language":"Perl","source":"x"}"#), None);
        assert_eq!(compiled_language("return 1"), None);
        assert_eq!(compiled_language(r#"{"format":"other"}"#), None);
    }

    #[test]
    fn canonical_javascript_source_changes_only_line_ends_and_the_final_newline() {
        assert_eq!(canonical_js_source("a  \r\n\r\nb"), "a  \n\nb\n");
        assert_eq!(canonical_js_source("x\n"), "x\n");
        assert_eq!(canonical_js_source(""), "\n");
    }

    #[test]
    fn compile_refuses_a_syntax_error_and_plain_text_is_not_compiled() {
        assert!(compile("set a to )").is_err());
        assert_eq!(compiled_source("set a to 1"), None);
        assert_eq!(compiled_source(r#"{"format":"other","source":"x"}"#), None);
    }
}
