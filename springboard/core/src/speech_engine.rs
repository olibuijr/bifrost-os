//! The text-to-speech engine that the `speech` shell service and the `say` tool both drive: the engine programs they try,
//! the arguments that select a voice, rate and output file, and the parser for the engine's voice listing. The engine
//! is `espeak-ng` (else `espeak`), a system package; this module runs nothing itself.

/// The engines to try in order: the packaged one first, then the older name.
pub const ENGINES: &[&str] = &["espeak-ng", "espeak"];
/// The slowest and fastest rates the engine takes, in words per minute.
pub const MIN_RATE: u32 = 80;
pub const MAX_RATE: u32 = 450;
/// The message when no engine program is on the path.
pub const NO_ENGINE: &str = "no speech engine is installed (install espeak-ng)";

/// One voice the engine knows: its name (what `-v` takes) and its language code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub name: String,
    pub language: String,
}

/// The engine's arguments for a voice, a rate and an output file. Absent options add nothing. The text itself goes on
/// standard input, so text that starts with a dash is safe.
pub fn engine_args(voice: Option<&str>, rate: Option<u32>, out: Option<&str>) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(v) = voice {
        a.extend(["-v".to_string(), v.to_string()]);
    }
    if let Some(r) = rate {
        a.extend(["-s".to_string(), r.to_string()]);
    }
    if let Some(out) = out {
        a.extend(["-w".to_string(), out.to_string()]);
    }
    a
}

/// Reads the output of `espeak-ng --voices`. Each row is priority, language, age and gender, voice name, file, and the
/// other languages; the header row (its first field is not a number) and short or malformed rows are skipped.
pub fn parse_voices(text: &str) -> Vec<Voice> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 || f[0].parse::<u32>().is_err() {
                return None;
            }
            Some(Voice { name: f[3].to_string(), language: f[1].to_string() })
        })
        .collect()
}

/// Whether `voice` is a name the engine can take: non-empty, at most 64 bytes, no spaces or control characters, and
/// not starting with a dash (it would read as an option).
pub fn voice_name_ok(voice: &str) -> bool {
    !voice.is_empty() && voice.len() <= 64 && !voice.starts_with('-') && !voice.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_args_carry_only_the_options_given() {
        assert_eq!(engine_args(Some("is"), Some(150), Some("x.wav")), vec!["-v", "is", "-s", "150", "-w", "x.wav"]);
        assert_eq!(engine_args(None, Some(90), None), vec!["-s", "90"]);
        assert!(engine_args(None, None, None).is_empty());
    }

    #[test]
    fn the_voice_listing_gives_name_and_language() {
        let listing = "Pty Language       Age/Gender VoiceName          File          Other Languages\n\
             \x20 5  af             M  Afrikaans          gmw/af\n\
             \x20 5  en-us          M  english-us         en-us              (en 2)\n\
             \x20 2  is             F  Icelandic          zle/is\n\
             junk line\n";
        let voices = parse_voices(listing);
        assert_eq!(voices.len(), 3);
        assert_eq!(voices[1], Voice { name: "english-us".into(), language: "en-us".into() });
        assert_eq!(voices[2].language, "is");
    }

    #[test]
    fn an_empty_listing_has_no_voices() {
        assert!(parse_voices("").is_empty());
        assert!(parse_voices("Pty Language Age/Gender VoiceName File\n").is_empty());
    }

    #[test]
    fn voice_names_that_would_read_as_options_are_refused() {
        assert!(voice_name_ok("en-us"));
        assert!(!voice_name_ok(""));
        assert!(!voice_name_ok("-v"));
        assert!(!voice_name_ok("en us"));
        assert!(!voice_name_ok(&"a".repeat(65)));
    }
}
