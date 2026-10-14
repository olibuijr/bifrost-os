//! The read state on disk: the ids of the cards the user has opened, in `seen.txt` in the app's data directory.
//! A missing or unreadable file means nothing has been read yet, so every card starts as New.

use crate::model::Seen;
use std::io;
use std::path::Path;

const FILE: &str = "seen.txt";
const TEMP: &str = "seen.txt.tmp";

/// The stored read state, or an empty one when the file cannot be read.
pub fn load(dir: &Path) -> Seen {
    std::fs::read_to_string(dir.join(FILE)).map(|text| Seen::parse(&text)).unwrap_or_default()
}

/// Writes the read state. The new file is written beside the old one and renamed over it, so an interrupted save
/// keeps the previous state.
pub fn save(dir: &Path, seen: &Seen) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(TEMP);
    std::fs::write(&temp, seen.to_text())?;
    std::fs::rename(&temp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_loads_as_nothing_read() {
        // A relative directory that does not exist: the read fails and the default is used.
        let seen = load(Path::new("tips-no-such-directory-for-tests"));
        assert_eq!(seen, Seen::default());
    }
}
