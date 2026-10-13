//! The per-conversation choices (pinned, alerts hidden, marked as unread) and the recently used emoji saved in the app's
//! data directory, one file per list with one entry per line.

use crate::model::RoomPrefs;
use std::path::Path;

const PINNED: &str = "pinned.txt";
const MUTED: &str = "muted.txt";
const UNREAD: &str = "unread.txt";
const EMOJI: &str = "emoji.txt";

/// Reads the saved choices. A missing or unreadable file is an empty list.
pub fn load(dir: &Path) -> RoomPrefs {
    let read = |name: &str| RoomPrefs::decode(&std::fs::read_to_string(dir.join(name)).unwrap_or_default());
    RoomPrefs { pinned: read(PINNED), muted: read(MUTED), unread_marks: read(UNREAD), recent_emoji: read(EMOJI) }
}

/// Saves the choices. The first failure is returned; the other lists are still tried.
pub fn save(dir: &Path, prefs: &RoomPrefs) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut first = Ok(());
    for (name, list) in [(PINNED, &prefs.pinned), (MUTED, &prefs.muted), (UNREAD, &prefs.unread_marks), (EMOJI, &prefs.recent_emoji)] {
        let result = std::fs::write(dir.join(name), RoomPrefs::encode(list)).map_err(|e| e.to_string());
        if first.is_ok() {
            first = result;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("messages-store-{tag}-{}", std::process::id()))
    }

    #[test]
    fn choices_survive_a_save_and_a_load() {
        let dir = temp("round");
        let prefs = RoomPrefs {
            pinned: vec!["!a:x".into(), "!b:x".into()],
            muted: vec!["!c:x".into()],
            unread_marks: Vec::new(),
            recent_emoji: vec!["\u{1F525}".into()],
        };
        save(&dir, &prefs).unwrap();
        assert_eq!(load(&dir), prefs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_saved_loads_as_empty() {
        assert_eq!(load(&temp("none")), RoomPrefs::default());
    }
}
