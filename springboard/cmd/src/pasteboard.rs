//! `pbcopy` and `pbpaste`: the general clipboard from the command line. Both use the Wayland selection through the `wl-copy`
//! and `wl-paste` tools, the same path the shell offers its typed items on (`shell::pasteboard`).
//! Named boards (ruler, find, font) are kept as one file each under `board_dir()`, with the MIME type beside it. The
//! shell's own pasteboard service is in-process and does not read these files yet, so an app's Find or Ruler board is
//! not the same data as `pbpaste -pboard find` until that bridge exists.
//! Gap: the shell's service keeps item grouping and several representations per copy; this tool reads and writes one
//! flat item.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The general board, the Wayland selection. Named boards are the other entries of `BOARDS`.
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
    /// The named board, `None` for the general board.
    pub board: Option<String>,
}

/// Reads the options. `paste` enables `-Prefer`, which only `pbpaste` has. An unknown `-pboard` name is an error.
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
                out.board = (board != GENERAL).then(|| board.clone());
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

/// The folder named boards are kept in: `$XDG_DATA_HOME/bifrost/pasteboards`, else under `$HOME/.local/share`.
pub fn board_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    Ok(base.join("bifrost").join("pasteboards"))
}

/// Puts `input` on the named `board` (one of `BOARDS` other than general). Empty input clears it. The board's bytes go
/// in a file named for the board, and its MIME type in `<board>.type`.
pub fn copy_to_board(board: &str, input: &[u8]) -> Result<(), String> {
    let dir = board_dir()?;
    let data = dir.join(board);
    if input.is_empty() {
        return match std::fs::remove_file(&data) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", data.display())),
        };
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&data, input).map_err(|e| format!("{}: {e}", data.display()))?;
    let kind = dir.join(format!("{board}.type"));
    std::fs::write(&kind, mime_for_input(input)).map_err(|e| format!("{}: {e}", kind.display()))
}

/// The bytes on the named `board`, empty when it holds nothing. The type is not applied: a named board holds one flat item.
pub fn paste_from_board(board: &str) -> Result<Vec<u8>, String> {
    let data = board_dir()?.join(board);
    match std::fs::read(&data) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", data.display())),
    }
}

/// Copies to the general board or the named one, as `-pboard` chose.
pub fn copy_for(board: Option<&str>, input: &[u8]) -> Result<(), String> {
    match board {
        Some(b) => copy_to_board(b, input),
        None => copy(input),
    }
}

/// Reads the general board or the named one, as `-pboard` chose.
pub fn paste_for(board: Option<&str>, prefer: Option<Prefer>) -> Result<Vec<u8>, String> {
    match board {
        Some(b) => paste_from_board(b),
        None => paste(prefer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn copy_accepts_every_board_name_and_refuses_unknown_ones() {
        assert_eq!(parse(&args(&["-pboard", "general"]), false), Ok(Args::default()));
        assert_eq!(parse(&args(&["-help"]), false), Ok(Args { help: true, ..Args::default() }));
        assert_eq!(parse(&args(&["-pboard", "ruler"]), false).unwrap().board.as_deref(), Some("ruler"));
        assert_eq!(parse(&args(&["-pboard", "font"]), true).unwrap().board.as_deref(), Some("font"));
        assert!(parse(&args(&["-pboard", "nope"]), false).unwrap_err().contains("unknown pasteboard"));
        assert!(parse(&args(&["-pboard"]), false).is_err());
        assert!(parse(&args(&["-Prefer", "rtf"]), false).is_err(), "pbcopy has no -Prefer");
    }

    #[test]
    fn named_boards_round_trip_through_their_own_files() {
        let dir = std::env::temp_dir().join(format!("bifrost-pb-test-{}", std::process::id()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        assert_eq!(board_dir().unwrap(), dir.join("bifrost").join("pasteboards"));
        assert_eq!(paste_from_board("find").unwrap(), Vec::<u8>::new());
        copy_to_board("find", b"needle").unwrap();
        assert_eq!(paste_from_board("find").unwrap(), b"needle".to_vec());
        assert_eq!(paste_for(Some("find"), None).unwrap(), b"needle".to_vec());
        assert_eq!(std::fs::read_to_string(board_dir().unwrap().join("find.type")).unwrap(), TEXT_MIME);
        copy_to_board("find", b"").unwrap();
        assert_eq!(paste_from_board("find").unwrap(), Vec::<u8>::new());
        copy_to_board("find", b"").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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
