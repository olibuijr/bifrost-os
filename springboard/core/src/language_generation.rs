//! Language switch generation: the shell records each change of language in the shared state directory, and each
//! running app watches the number to reload its strings. Pure file I/O; the shell writes it and the SDK re-exports
//! it for apps (`bifrost_sdk::locale`).

use std::path::Path;

/// The file in the shared state directory that counts language switches. One number serves every process: the
/// switcher writes it, and each running app reads it through a [`LanguageWatch`].
const LANGUAGE_GENERATION_FILE: &str = "language.generation";

/// Records a language switch in `dir` and returns the new generation. The write is atomic (temp file, then rename),
/// so a reader never sees a half-written number.
pub fn announce_language_change(dir: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(dir)?;
    let next = language_generation(dir).wrapping_add(1);
    let tmp = dir.join(format!("{LANGUAGE_GENERATION_FILE}.tmp"));
    std::fs::write(&tmp, next.to_string())?;
    std::fs::rename(&tmp, dir.join(LANGUAGE_GENERATION_FILE))?;
    Ok(next)
}

/// The current language generation in `dir`: 0 before any switch, or when the file is missing or unreadable.
pub fn language_generation(dir: &Path) -> u64 {
    std::fs::read_to_string(dir.join(LANGUAGE_GENERATION_FILE)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// How a running app notices that the person switched language: `changed` is true once for each new generation,
/// and the app then reloads its strings (`set_main_bundle` with the tables for the new language).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageWatch {
    seen: u64,
}

impl LanguageWatch {
    /// Starts watching from the generation current now, so the first `changed` call is false.
    pub fn new(dir: &Path) -> Self {
        Self { seen: language_generation(dir) }
    }

    /// True when the generation moved since the last call, and marks it seen.
    pub fn changed(&mut self, dir: &Path) -> bool {
        let now = language_generation(dir);
        if now == self.seen {
            return false;
        }
        self.seen = now;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_switch_is_seen_once_by_each_watching_process() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(language_generation(d.path()), 0);
        let mut app = LanguageWatch::new(d.path());
        assert!(!app.changed(d.path()), "nothing switched yet");
        assert_eq!(announce_language_change(d.path()).unwrap(), 1);
        assert_eq!(announce_language_change(d.path()).unwrap(), 2);
        assert!(app.changed(d.path()), "the switch is seen");
        assert!(!app.changed(d.path()), "and only once per generation");
        let mut late = LanguageWatch::new(d.path());
        assert!(!late.changed(d.path()), "a process that starts after the switch does not reload for it");
    }

    #[test]
    fn a_garbage_generation_file_reads_as_zero() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(LANGUAGE_GENERATION_FILE), "not a number").unwrap();
        assert_eq!(language_generation(d.path()), 0);
    }
}
