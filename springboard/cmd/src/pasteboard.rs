//! `pbcopy` and `pbpaste`: the general clipboard from the command line. Both use the Wayland selection through the `wl-copy`
//! and `wl-paste` tools, the same path the shell offers its typed items on (`shell::pasteboard`). Named boards (ruler,
//! find, font) live in the shell's pasteboard service, which the command line cannot reach yet, so they are refused
//! rather than written to the general board.
//! Gap: the shell's service keeps item grouping and several representations per copy; this tool reads and writes one
//! flat item.

use std::io::Write;
use std::process::{Command, Stdio};

/// The general board, the only one the command line reaches.
pub const GENERAL: &str = "general";
/// The board names `-pboard` accepts.
pub const BOARDS: &[&str] = &["general", "ruler", "find", "font"];

pub const TEXT_MIME: &str = "text/plain;charset=utf-8";
pub const RTF_MIME: &str = "text/rtf";
pub const EPS_MIME: &str = "application/postscript";

pub const COPY_USAGE: &str = "usage: pbcopy [-help] [-pboard {general | ruler | find | font}]";
pub const PASTE_USAGE: &str = "usage: pbpaste [-help] [-pboard {general | ruler | find | font}] [-Prefer {txt | rtf | ps}]";

/// The data type `pbpaste -Prefer` looks for first. `ascii` is the older name for `txt` and still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefer {
    Text,
    Rtf,
    Eps,
}

impl Prefer {
    pub fn parse(s: &str) -> Option<Prefer> {
        match s {
            "txt" | "ascii" => Some(Prefer::Text),
            "rtf" => Some(Prefer::Rtf),
            "ps" => Some(Prefer::Eps),
            _ => None,
        }
    }
}

/// The parsed options of either tool.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub help: bool,
    pub prefer: Option<Prefer>,
}

/// Reads the options. `paste` enables `-Prefer`, which only `pbpaste` has. A `-pboard` other than general is an error.
pub fn parse(args: &[String], paste: bool) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-help" | "--help" | "-h" => out.help = true,
            "-pboard" => {
                let board = it.next().ok_or("-pboard needs a pasteboard name")?;
                if !BOARDS.contains(&board.as_str()) {
                    return Err(format!("unknown pasteboard {board}"));
                }
                if board != GENERAL {
                    return Err(format!("the {board} pasteboard is not reachable from the command line yet"));
                }
            }
            "-Prefer" if paste => {
                let v = it.next().ok_or("-Prefer needs txt, rtf or ps")?;
                out.prefer = Some(Prefer::parse(v).ok_or_else(|| format!("unknown type {v}"))?);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(out)
}

/// The type of text `pbcopy` writes: RTF or EPS when the input starts with its file header, else plain text.
pub fn mime_for_input(input: &[u8]) -> &'static str {
    if input.starts_with(b"{\\rtf") {
        RTF_MIME
    } else if input.starts_with(b"%!PS-Adobe") {
        EPS_MIME
    } else {
        TEXT_MIME
    }
}

/// The order `pbpaste` looks in: the preferred type first, then the others (plain text, EPS, RTF).
pub fn paste_order(prefer: Option<Prefer>) -> Vec<Prefer> {
    let mut order = vec![Prefer::Text, Prefer::Eps, Prefer::Rtf];
    if let Some(p) = prefer {
        order.retain(|x| *x != p);
        order.insert(0, p);
    }
    order
}

/// The MIME types a type is read as, most exact first.
pub fn mimes(p: Prefer) -> &'static [&'static str] {
    match p {
        Prefer::Text => &[TEXT_MIME, "text/plain"],
        Prefer::Rtf => &[RTF_MIME],
        Prefer::Eps => &[EPS_MIME],
    }
}

/// Puts `input` on the general board. Empty input clears it.
pub fn copy(input: &[u8]) -> Result<(), String> {
    let mut cmd = Command::new("wl-copy");
    if input.is_empty() {
        cmd.arg("--clear");
    } else {
        cmd.args(["--type", mime_for_input(input)]);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| format!("wl-copy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input).map_err(|e| format!("wl-copy: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy failed ({status})"))
    }
}

/// The general board's data: the first type in `paste_order` that holds data. Empty when none does.
pub fn paste(prefer: Option<Prefer>) -> Result<Vec<u8>, String> {
    for p in paste_order(prefer) {
        for mime in mimes(p) {
            let out = Command::new("wl-paste")
                .args(["--no-newline", "--type", *mime])
                .stderr(Stdio::null())
                .output()
                .map_err(|e| format!("wl-paste: {e}"))?;
            if out.status.success() && !out.stdout.is_empty() {
                return Ok(out.stdout);
            }
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn copy_accepts_the_general_board_and_refuses_the_others() {
        assert_eq!(parse(&args(&["-pboard", "general"]), false), Ok(Args::default()));
        assert_eq!(parse(&args(&["-help"]), false), Ok(Args { help: true, prefer: None }));
        assert!(parse(&args(&["-pboard", "ruler"]), false).unwrap_err().contains("not reachable"));
        assert!(parse(&args(&["-pboard", "nope"]), false).unwrap_err().contains("unknown pasteboard"));
        assert!(parse(&args(&["-pboard"]), false).is_err());
        assert!(parse(&args(&["-Prefer", "rtf"]), false).is_err(), "pbcopy has no -Prefer");
    }

    #[test]
    fn paste_reads_prefer_and_its_older_ascii_name() {
        assert_eq!(parse(&args(&["-Prefer", "ps"]), true).unwrap().prefer, Some(Prefer::Eps));
        assert_eq!(parse(&args(&["-Prefer", "ascii"]), true).unwrap().prefer, Some(Prefer::Text));
        assert_eq!(parse(&args(&["-Prefer", "txt"]), true).unwrap().prefer, Some(Prefer::Text));
        assert!(parse(&args(&["-Prefer", "pdf"]), true).is_err());
    }

    #[test]
    fn input_header_picks_the_type() {
        assert_eq!(mime_for_input(b"{\\rtf1\\ansi hello}"), RTF_MIME);
        assert_eq!(mime_for_input(b"%!PS-Adobe-3.0 EPSF-3.0"), EPS_MIME);
        assert_eq!(mime_for_input("hello, wörld\n".as_bytes()), TEXT_MIME);
        assert_eq!(mime_for_input(b"%!PS but not adobe"), TEXT_MIME);
    }

    #[test]
    fn paste_order_puts_the_preferred_type_first() {
        assert_eq!(paste_order(None), [Prefer::Text, Prefer::Eps, Prefer::Rtf]);
        assert_eq!(paste_order(Some(Prefer::Rtf)), [Prefer::Rtf, Prefer::Text, Prefer::Eps]);
        assert_eq!(paste_order(Some(Prefer::Text)), [Prefer::Text, Prefer::Eps, Prefer::Rtf]);
    }

    #[test]
    fn text_is_read_as_utf8_then_plain() {
        assert_eq!(mimes(Prefer::Text), [TEXT_MIME, "text/plain"]);
        assert_eq!(mimes(Prefer::Rtf), [RTF_MIME]);
        assert_eq!(mimes(Prefer::Eps), [EPS_MIME]);
    }
}
