//! `say`: speaks text through the system's speech engine, or writes the speech to a file with `-o`. The desktop has no
//! speech engine of its own, so the tool runs `espeak-ng` (else `espeak`), the system package the speech stack is built
//! on. With no engine installed it says so and exits.
//!
//! Supported: `-v voice` (an engine voice name), `-r rate` (words per minute), `-o file` (writes a WAV file, not AIFF),
//! `-f file` (the text is read from the file), and text operands joined with spaces. With no text, the engine reads
//! standard input.
//! Refused with a message: the listing of voices (`-v ?`), the file and data format options, the audio device (`-a`), the
//! name service (`-n`) and an interactive mode (`-i`).
//! Gap: `-o` writes WAV (the engine's format), where the reference tool writes AIFF by default; the rate is passed through as words
//! per minute without checking the engine's range.

use bifrost_core::speech_engine::{engine_args as shared_engine_args, ENGINES, NO_ENGINE};
use std::io::Write;
use std::process::{Command, Stdio};

pub const USAGE: &str = "usage: say [-v voice] [-r rate] [-o file] [-f file | text ...]";

/// The parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub voice: Option<String>,
    pub rate: Option<u32>,
    pub out: Option<String>,
    pub file: Option<String>,
    /// The text operands, joined with spaces.
    pub text: Vec<String>,
}

/// What the command line asks for: the help text, or a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments. A value option takes the next argument. Text operands end option parsing only after `--`.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    let mut only_text = false;
    while let Some(arg) = it.next() {
        if only_text || !arg.starts_with('-') || arg == "-" {
            o.text.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => only_text = true,
            "-h" | "-help" | "--help" => return Ok(Parsed::Help),
            "-v" => {
                let v = it.next().ok_or("-v needs a voice name")?;
                if v == "?" {
                    return Err("the voice list is not available yet; pick a voice name the engine knows".into());
                }
                o.voice = Some(v.clone());
            }
            "-r" => {
                let v = it.next().ok_or("-r needs a rate in words per minute")?;
                o.rate = Some(v.parse().map_err(|_| format!("-r needs a rate in words per minute, not {v}"))?);
            }
            "-o" => o.out = Some(it.next().ok_or("-o needs a file name")?.clone()),
            "-f" => o.file = Some(it.next().ok_or("-f needs a file name")?.clone()),
            "-a" | "-n" | "-i" | "-e" | "--file-format" | "--data-format" | "--channels" | "--bit-rate" | "--quality" => {
                return Err(format!("{arg} is not supported yet"));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if o.file.is_some() && !o.text.is_empty() {
        return Err("give text or -f, not both".into());
    }
    Ok(Parsed::Run(o))
}

/// The engine's arguments for `o`. The text itself goes on standard input, so text that starts with a dash is safe.
pub fn engine_args(o: &Options) -> Vec<String> {
    shared_engine_args(o.voice.as_deref(), o.rate, o.out.as_deref())
}

/// The text to speak: the `-f` file's contents, else the operands joined with spaces, else nothing (the engine then reads
/// standard input).
pub fn text_of(o: &Options) -> Result<Option<Vec<u8>>, String> {
    if let Some(path) = &o.file {
        return std::fs::read(path).map(Some).map_err(|e| format!("{path}: {e}"));
    }
    if o.text.is_empty() {
        return Ok(None);
    }
    Ok(Some(o.text.join(" ").into_bytes()))
}

/// Runs the first engine that is installed with `args`, feeding it `text` (or inheriting standard input when there is none).
pub fn speak(args: &[String], text: Option<&[u8]>) -> Result<(), String> {
    for engine in ENGINES {
        let mut cmd = Command::new(engine);
        cmd.args(args).stdout(Stdio::null()).stdin(if text.is_some() { Stdio::piped() } else { Stdio::inherit() });
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{engine}: {e}")),
        };
        if let (Some(bytes), Some(mut stdin)) = (text, child.stdin.take()) {
            stdin.write_all(bytes).map_err(|e| format!("{engine}: {e}"))?;
        }
        let status = child.wait().map_err(|e| format!("{engine}: {e}"))?;
        return if status.success() { Ok(()) } else { Err(format!("{engine} failed ({status})")) };
    }
    Err(NO_ENGINE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_take_their_values_and_text_is_joined() {
        let Parsed::Run(o) = parse(&args(&["-v", "en-us", "-r", "180", "-o", "hi.wav", "hello", "world"])).unwrap() else { panic!("help") };
        assert_eq!(o.voice.as_deref(), Some("en-us"));
        assert_eq!(o.rate, Some(180));
        assert_eq!(o.out.as_deref(), Some("hi.wav"));
        assert_eq!(text_of(&o).unwrap(), Some(b"hello world".to_vec()));
    }

    #[test]
    fn dash_text_after_double_dash_is_spoken_not_parsed() {
        let Parsed::Run(o) = parse(&args(&["--", "-v", "is"])).unwrap() else { panic!("help") };
        assert_eq!(o.text, vec!["-v".to_string(), "is".to_string()]);
        assert_eq!(o.voice, None);
    }

    #[test]
    fn refusals_name_the_option() {
        assert!(parse(&args(&["-v", "?"])).unwrap_err().contains("voice list"));
        assert!(parse(&args(&["-a", "dev"])).unwrap_err().contains("-a is not supported"));
        assert!(parse(&args(&["-r", "fast"])).unwrap_err().contains("words per minute"));
        assert!(parse(&args(&["-q"])).unwrap_err().contains("unknown option"));
        assert!(parse(&args(&["-f", "a.txt", "text"])).unwrap_err().contains("not both"));
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
    }

    #[test]
    fn the_engine_gets_voice_rate_and_output() {
        let o = Options { voice: Some("is".into()), rate: Some(150), out: Some("x.wav".into()), ..Options::default() };
        assert_eq!(engine_args(&o), args(&["-v", "is", "-s", "150", "-w", "x.wav"]));
        assert!(engine_args(&Options::default()).is_empty());
    }

    #[test]
    fn with_no_text_the_engine_reads_standard_input() {
        assert_eq!(text_of(&Options::default()).unwrap(), None);
    }
}
