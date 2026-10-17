//! `vis` and `unvis`: encode bytes as printable escape sequences and decode them back, following the vis(3) and unvis(3)
//! man page descriptions. Clean room: written from the documented behaviour only. See `bifrost_cmd::vis`.
//!
//! Encoding styles, one per flag:
//! - default: printable ASCII passes; a backslash becomes `\\`; control bytes become `\^X` (`\^@` to `\^_`, `\^?` for
//!   DEL); bytes with the high bit set become `\M-x`, or `\M^X` when their low seven bits are a control byte.
//! - `-o`: every escaped byte is a three digit octal escape `\ooo`.
//! - `-c`: C escapes `\a \b \f \n \r \t \v` for those bytes (tab and newline are escaped too), octal for the rest.
//! - `-h`: HTTP, `%XX` for every byte outside `A-Za-z0-9-_.~`.
//! - `-e` or `-m`: MIME quoted-printable, `=XX` for every byte outside printable ASCII (and newline).
//! - `-n`: no encoding.
//!
//! Modifiers: `-w` encodes whitespace (space, tab, newline), `-t` encodes tab, `-l` writes a newline as `\$` followed by
//! the newline, `-s` encodes everything except letters, digits and `-_.+`, `-b` leaves a backslash unescaped, and
//! `-F n` (or `-f` for 80) folds the output so each line is at most `n` columns, ending with a backslash-newline. Escape
//! units are never split across a fold.
//!
//! Decoding accepts every backslash escape regardless of style, and the `%XX` and `=XX` forms when `-h` or `-e`/`-m` is
//! given. An invalid escape is an error; with `-b` a backslash that starts no valid escape is kept as a literal.
//!
//! Gap: `-b` is not round-trip safe for backslashes (the decoder can only guess), and the MIME style does not wrap long
//! lines with soft breaks. Both read whole files into memory.

use std::io::Read;

pub const VIS_USAGE: &str = "usage: vis [-bcefhlmnostw] [-F foldwidth] [--] [file ...]";
pub const UNVIS_USAGE: &str = "usage: unvis [-bcefhlmnostw] [-F foldwidth] [--] [file ...]";
/// The fold width `-f` uses when `-F` does not give one.
pub const DEFAULT_FOLD: usize = 80;

/// The encoding style chosen by the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Default,
    Octal,
    CStyle,
    Http,
    Mime,
    NoEncoding,
}

/// Every option the two tools read. The decoder uses the style, `no_backslash` and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Opts {
    pub style: Style,
    /// `-w`: also encode space, tab and newline.
    pub white: bool,
    /// `-t`: also encode tab.
    pub tab: bool,
    /// `-l`: mark each newline with `\$`.
    pub line_mark: bool,
    /// `-s`: encode everything outside letters, digits and `-_.+`.
    pub safe: bool,
    /// `-b`: do not escape a backslash.
    pub no_backslash: bool,
    /// `-F` or `-f`: fold lines at this many columns.
    pub fold: Option<usize>,
}

/// The parsed command line: the options, the input names (empty means standard input) and whether help was asked.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub opts: Opts,
    pub files: Vec<String>,
    pub help: bool,
}

fn is_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+')
}

fn is_http_plain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

fn is_mime_plain(b: u8) -> bool {
    ((0x20..=0x7e).contains(&b) && b != b'=' && b != b'\\') || b == b'\n'
}

fn is_escaped_style(style: Style) -> bool {
    matches!(style, Style::Default | Style::Octal | Style::CStyle)
}

/// Whether a byte goes through unchanged in the backslash styles. A backslash never does (`unit` handles it), and a
/// newline under `-l` is handled before this is asked.
fn passes(b: u8, o: &Opts) -> bool {
    if o.safe {
        return is_safe(b);
    }
    let cstyle = o.style == Style::CStyle;
    match b {
        b' ' => !o.white,
        b'\t' => !(o.white || o.tab || cstyle),
        b'\n' => !(o.white || cstyle),
        0x21..=0x7e => b != b'\\',
        _ => false,
    }
}

fn octal(b: u8) -> Vec<u8> {
    format!("\\{b:03o}").into_bytes()
}

fn c_escape(b: u8) -> Option<u8> {
    Some(match b {
        0x07 => b'a',
        0x08 => b'b',
        0x09 => b't',
        0x0a => b'n',
        0x0b => b'v',
        0x0c => b'f',
        0x0d => b'r',
        _ => return None,
    })
}

/// The character that follows `^` for a control byte: `@` for NUL up to `_` for 0x1f, `?` for DEL.
fn ctrl_char(b: u8) -> u8 {
    if b == 0x7f {
        b'?'
    } else {
        b + 0x40
    }
}

fn meta(b: u8) -> Vec<u8> {
    let inner = b & 0x7f;
    if inner < 0x20 || inner == 0x7f {
        vec![b'\\', b'M', b'^', ctrl_char(inner)]
    } else {
        vec![b'\\', b'M', b'-', inner]
    }
}

/// The output for one input byte, with no fold or line-mark handling.
fn unit(b: u8, o: &Opts) -> Vec<u8> {
    match o.style {
        Style::NoEncoding => vec![b],
        Style::Http => {
            if is_http_plain(b) {
                vec![b]
            } else {
                format!("%{b:02X}").into_bytes()
            }
        }
        Style::Mime => {
            if is_mime_plain(b) {
                vec![b]
            } else {
                format!("={b:02X}").into_bytes()
            }
        }
        _ => {
            if passes(b, o) {
                return vec![b];
            }
            if b == b'\\' {
                return if o.no_backslash { vec![b] } else { b"\\\\".to_vec() };
            }
            match o.style {
                Style::Octal => octal(b),
                Style::CStyle => c_escape(b).map(|c| vec![b'\\', c]).unwrap_or_else(|| octal(b)),
                _ => {
                    if b >= 0x80 {
                        meta(b)
                    } else if b < 0x20 || b == 0x7f {
                        vec![b'\\', b'^', ctrl_char(b)]
                    } else {
                        octal(b)
                    }
                }
            }
        }
    }
}

/// Encodes `input` with the options.
pub fn encode(input: &[u8], o: &Opts) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let fold = if o.style == Style::NoEncoding { None } else { o.fold };
    let escaping = is_escaped_style(o.style);
    let mut col = 0usize;
    for &b in input {
        let piece = if b == b'\n' && o.line_mark && escaping {
            b"\\$\n".to_vec()
        } else {
            unit(b, o)
        };
        if let Some(width) = fold {
            if col > 0 && col + piece.len() >= width {
                out.extend_from_slice(b"\\\n");
                col = 0;
            }
        }
        out.extend_from_slice(&piece);
        col = if piece.last() == Some(&b'\n') { 0 } else { col + piece.len() };
    }
    out
}

/// The value of a control character after `^` (`@` to `_`, or `?`), or None.
fn ctrl(c: u8) -> Option<u8> {
    match c {
        b'?' => Some(0x7f),
        b'@'..=b'_' => Some(c - 0x40),
        _ => None,
    }
}

/// Two hex digits at the start of `s`.
fn hex_pair(s: &[u8]) -> Option<u8> {
    let hi = (*s.first()? as char).to_digit(16)?;
    let lo = (*s.get(1)? as char).to_digit(16)?;
    Some((hi * 16 + lo) as u8)
}

/// Decodes the backslash escape at the start of `s`. `Some((Some(byte), used))` for a byte, `Some((None, used))` for an
/// escape that produces nothing (`\$` and a folded newline), and `None` when it is not a valid escape.
fn backslash(s: &[u8]) -> Option<(Option<u8>, usize)> {
    let c = s.get(1).copied()?;
    let byte = match c {
        b'\\' => b'\\',
        b'$' | b'\n' => return Some((None, 2)),
        b'a' => 0x07,
        b'b' => 0x08,
        b'f' => 0x0c,
        b'n' => 0x0a,
        b'r' => 0x0d,
        b't' => 0x09,
        b'v' => 0x0b,
        b'0'..=b'7' => {
            let digits: Vec<u8> = s[1..]
                .iter()
                .copied()
                .take(3)
                .take_while(|d| (b'0'..=b'7').contains(d))
                .collect();
            let value = digits.iter().fold(0u32, |acc, &d| acc * 8 + u32::from(d - b'0'));
            if value > 0xff {
                return None;
            }
            return Some((Some(value as u8), 1 + digits.len()));
        }
        b'M' => {
            return match (s.get(2).copied(), s.get(3).copied()) {
                (Some(b'-'), Some(x)) if (0x20..=0x7e).contains(&x) => Some((Some(x | 0x80), 4)),
                (Some(b'^'), Some(x)) => Some((Some(ctrl(x)? | 0x80), 4)),
                _ => None,
            };
        }
        b'^' => return s.get(2).copied().and_then(ctrl).map(|v| (Some(v), 3)),
        _ => return None,
    };
    Some((Some(byte), 2))
}

/// Decodes `input` with the options. Errors with `invalid escape`.
pub fn decode(input: &[u8], o: &Opts) -> Result<Vec<u8>, String> {
    if o.style == Style::NoEncoding {
        return Ok(input.to_vec());
    }
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        let step: Option<(Option<u8>, usize)> = match b {
            b'\\' => match backslash(&input[i..]) {
                Some(step) => Some(step),
                None if o.no_backslash => Some((Some(b'\\'), 1)),
                None => None,
            },
            b'%' if o.style == Style::Http => hex_pair(&input[i + 1..]).map(|v| (Some(v), 3)),
            b'=' if o.style == Style::Mime => match &input[i + 1..] {
                [b'\n', ..] => Some((None, 2)),
                [b'\r', b'\n', ..] => Some((None, 3)),
                rest => hex_pair(rest).map(|v| (Some(v), 3)),
            },
            _ => Some((Some(b), 1)),
        };
        match step {
            Some((Some(v), used)) => {
                out.push(v);
                i += used;
            }
            Some((None, used)) => i += used,
            None => return Err("invalid escape".to_string()),
        }
    }
    Ok(out)
}

/// Reads the command line. `-F n` takes the width from the rest of the argument or the next one, `-f` folds at
/// `DEFAULT_FOLD`, `--help` asks for the usage text, and a lone `-` or any other non-option is an input name.
pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut i = 0;
    let mut only_files = false;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if only_files || arg == "-" || !arg.starts_with('-') {
            cli.files.push(arg.clone());
            continue;
        }
        if arg == "--" {
            only_files = true;
            continue;
        }
        if arg == "--help" {
            cli.help = true;
            continue;
        }
        let letters: Vec<char> = arg[1..].chars().collect();
        let mut j = 0;
        while j < letters.len() {
            match letters[j] {
                'b' => cli.opts.no_backslash = true,
                'c' => cli.opts.style = Style::CStyle,
                'e' | 'm' => cli.opts.style = Style::Mime,
                'h' => cli.opts.style = Style::Http,
                'n' => cli.opts.style = Style::NoEncoding,
                'o' => cli.opts.style = Style::Octal,
                'l' => cli.opts.line_mark = true,
                's' => cli.opts.safe = true,
                't' => cli.opts.tab = true,
                'w' => cli.opts.white = true,
                'f' => cli.opts.fold = Some(cli.opts.fold.unwrap_or(DEFAULT_FOLD)),
                'F' => {
                    let rest: String = letters[j + 1..].iter().collect();
                    let value = if rest.is_empty() {
                        let next = args.get(i).cloned().ok_or("-F needs a fold width")?;
                        i += 1;
                        next
                    } else {
                        rest
                    };
                    let width: usize = value.parse().map_err(|_| format!("bad fold width {value}"))?;
                    if width < 2 {
                        return Err(format!("fold width {width} is too small"));
                    }
                    cli.opts.fold = Some(width);
                    break;
                }
                c => return Err(format!("illegal option -- {c}")),
            }
            j += 1;
        }
    }
    Ok(cli)
}

/// Reads one input: the named file, or standard input for `-`.
pub fn read_input(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    } else {
        std::fs::read(path).map_err(|e| format!("{path}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(style: Style) -> Opts {
        Opts { style, ..Opts::default() }
    }

    fn round_trip(o: &Opts, input: &[u8]) {
        let enc = encode(input, o);
        let dec = decode(&enc, o).unwrap_or_else(|e| panic!("{o:?}: {e}"));
        assert_eq!(dec, input, "{o:?}");
    }

    #[test]
    fn default_style_escapes() {
        let o = opts(Style::Default);
        assert_eq!(encode(b"\x00", &o), b"\\^@");
        assert_eq!(encode(b"\x1b", &o), b"\\^[");
        assert_eq!(encode(b"\x7f", &o), b"\\^?");
        assert_eq!(encode(b"\x80", &o), b"\\M^@");
        assert_eq!(encode(b"\xa0", &o), b"\\M- ");
        assert_eq!(encode(b"\xff", &o), b"\\M^?");
        assert_eq!(encode(b"\\", &o), b"\\\\");
        assert_eq!(encode(b"a b\tc\n", &o), b"a b\tc\n");
    }

    #[test]
    fn octal_and_flag_escapes() {
        assert_eq!(encode(b"\x01", &opts(Style::Octal)), b"\\001");
        assert_eq!(encode(b"\x01", &opts(Style::Default)), b"\\^A");
        let white = Opts { white: true, ..opts(Style::Default) };
        assert_eq!(encode(b" \t\n", &white), b"\\040\\^I\\^J");
        let tab = Opts { tab: true, ..opts(Style::Default) };
        assert_eq!(encode(b"\t x", &tab), b"\\^I x");
        let mark = Opts { line_mark: true, ..opts(Style::Default) };
        assert_eq!(encode(b"a\nb", &mark), b"a\\$\nb");
        let safe = Opts { safe: true, ..opts(Style::Default) };
        assert_eq!(encode(b"a-b.c_d+e", &safe), b"a-b.c_d+e");
        assert_eq!(encode(b"a b/", &safe), b"a\\040b\\057");
    }

    #[test]
    fn c_style_escapes() {
        let o = opts(Style::CStyle);
        assert_eq!(encode(b"\t\n\r\x07\x08\x0b\x0c", &o), b"\\t\\n\\r\\a\\b\\v\\f");
        assert_eq!(encode(b"\x01\\", &o), b"\\001\\\\");
        assert_eq!(encode(b"\x80", &o), b"\\200");
    }

    #[test]
    fn http_and_mime_escapes() {
        assert_eq!(encode(b"a b/%=\\", &opts(Style::Http)), b"a%20b%2F%25%3D%5C");
        assert_eq!(encode(b"a=b\xe9", &opts(Style::Mime)), b"a=3Db=E9");
        assert_eq!(encode(b"x\ny", &opts(Style::Mime)), b"x\ny");
    }

    #[test]
    fn fold_keeps_lines_within_width() {
        let o = Opts { fold: Some(5), ..opts(Style::Default) };
        assert_eq!(encode(b"abcdefghij", &o), b"abcd\\\nefgh\\\nij");
    }

    #[test]
    fn decodes_escapes() {
        let d = opts(Style::Default);
        assert_eq!(decode(b"\\101", &d).unwrap(), b"A");
        assert_eq!(decode(b"\\^@\\^?", &d).unwrap(), b"\x00\x7f");
        assert_eq!(decode(b"\\M-^?\\M^@\\M- ", &d).unwrap(), b"\xff\x80\xa0");
        assert_eq!(decode(b"\\M--", &d).unwrap(), b"\xad");
        assert_eq!(decode(b"\\n\\t\\\\", &d).unwrap(), b"\n\t\\");
        assert_eq!(decode(b"a\\\nb\\$c", &d).unwrap(), b"abc");
        assert_eq!(decode(b"%41", &d).unwrap(), b"%41");
        assert_eq!(decode(b"a%20b%41", &opts(Style::Http)).unwrap(), b"a bA");
        assert_eq!(decode(b"=41=\n", &opts(Style::Mime)).unwrap(), b"A");
        let raw = Opts { no_backslash: true, ..opts(Style::Default) };
        assert_eq!(decode(b"\\q", &raw).unwrap(), b"\\q");
    }

    #[test]
    fn invalid_sequences_are_errors() {
        let d = opts(Style::Default);
        let bad: [&[u8]; 8] = [b"\\q", b"\\", b"\\400", b"\\M", b"\\M-\x01", b"\\^1", b"\\M^1", b"\\M"];
        for input in bad {
            assert_eq!(decode(input, &d).unwrap_err(), "invalid escape", "{input:?}");
        }
        assert!(decode(b"%41", &opts(Style::Http)).is_ok());
        assert!(decode(b"%4", &opts(Style::Http)).is_err());
        assert!(decode(b"%zz", &opts(Style::Http)).is_err());
        assert!(decode(b"=G1", &opts(Style::Mime)).is_err());
    }

    #[test]
    fn every_byte_round_trips_in_every_style() {
        let all: Vec<u8> = (0..=255u8).collect();
        let no_slash: Vec<u8> = all.iter().copied().filter(|&b| b != b'\\').collect();
        for style in [Style::Default, Style::Octal, Style::CStyle, Style::Http, Style::Mime] {
            for mask in 0..32u8 {
                let o = Opts {
                    style,
                    white: mask & 1 != 0,
                    tab: mask & 2 != 0,
                    line_mark: mask & 4 != 0,
                    safe: mask & 8 != 0,
                    fold: (mask & 16 != 0).then_some(8),
                    no_backslash: false,
                };
                round_trip(&o, &all);
                round_trip(&Opts { no_backslash: true, ..o.clone() }, &no_slash);
            }
        }
        round_trip(&opts(Style::NoEncoding), &all);
    }

    #[test]
    fn parses_flags_and_files() {
        let args: Vec<String> = ["-cwF", "40", "a.txt", "-"].iter().map(|s| s.to_string()).collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.opts.style, Style::CStyle);
        assert!(cli.opts.white);
        assert_eq!(cli.opts.fold, Some(40));
        assert_eq!(cli.files, ["a.txt", "-"]);
        assert!(parse_args(&["-x".to_string()]).is_err());
        assert!(parse_args(&["-F1".to_string()]).is_err());
    }
}
