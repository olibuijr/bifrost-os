//! The board's file in the app's data directory. Every save writes a temporary file and renames it over the old
//! one, so a crash mid-write keeps the previous board.

use crate::model::{self, Board};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "stickies.json";

pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// True when no board has been saved yet: the app has never run here (a board emptied by the person is still a file).
pub fn first_launch(data_dir: &Path) -> bool {
    !path_in(data_dir).exists() && !data_dir.join(format!("{FILE_NAME}.bad")).exists()
}

/// Moves the damaged board file aside under a name nothing else has: `stickies.json.bad`, then `.bad.1`, `.bad.2` and
/// so on, so an earlier damaged copy is never replaced.
fn set_aside(data_dir: &Path, path: &Path) {
    let first = data_dir.join(format!("{FILE_NAME}.bad"));
    let target = if !first.exists() {
        first
    } else {
        (1..1000).map(|n| data_dir.join(format!("{FILE_NAME}.bad.{n}"))).find(|p| !p.exists()).unwrap_or_else(|| {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            data_dir.join(format!("{FILE_NAME}.bad.{secs}"))
        })
    };
    let _ = fs::rename(path, target);
}

/// The saved board, or an empty one when there is no file yet. A file that cannot be read (not text, an I/O or
/// permission error) or decoded is kept aside as `stickies.json.bad` (or the next free `.bad.N`) and the board starts
/// empty, so the user's notes are never silently overwritten.
pub fn load(data_dir: &Path) -> Board {
    let path = path_in(data_dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Board::default(),
        Err(_) => {
            set_aside(data_dir, &path);
            return Board::default();
        }
    };
    match model::decode(&text) {
        Ok(board) => board,
        Err(_) => {
            set_aside(data_dir, &path);
            Board::default()
        }
    }
}

/// Writes the board atomically: temporary file, flush, rename.
pub fn save(data_dir: &Path, board: &Board) -> std::io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let path = path_in(data_dir);
    let tmp = data_dir.join(format!("{FILE_NAME}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(model::encode(board).as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stickies-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_then_load_restores_the_board() {
        let dir = scratch("roundtrip");
        let mut b = Board::default();
        b.add_text("kept", 20.0, 30.0, 1);
        save(&dir, &b).unwrap();
        assert!(!dir.join("stickies.json.tmp").exists());
        assert_eq!(load(&dir), b);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_data_folder_with_no_board_file_is_a_first_launch() {
        let dir = scratch("first");
        assert!(first_launch(&dir));
        save(&dir, &Board::default()).unwrap();
        assert!(!first_launch(&dir), "an emptied board is not a first launch");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_loads_an_empty_board() {
        let dir = scratch("missing");
        assert_eq!(load(&dir), Board::default());
    }

    #[test]
    fn a_file_that_is_not_text_is_moved_aside_and_an_earlier_copy_is_kept() {
        let dir = scratch("binary");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stickies.json.bad"), "an older damaged board").unwrap();
        fs::write(path_in(&dir), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(load(&dir).notes.is_empty());
        assert!(!path_in(&dir).exists(), "the next save cannot overwrite the notes");
        assert_eq!(fs::read_to_string(dir.join("stickies.json.bad")).unwrap(), "an older damaged board");
        assert_eq!(fs::read(dir.join("stickies.json.bad.1")).unwrap(), vec![0xff, 0xfe, 0x00, 0x80]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_moved_aside() {
        let dir = scratch("broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(path_in(&dir), "not json").unwrap();
        assert!(load(&dir).notes.is_empty());
        assert!(dir.join("stickies.json.bad").exists());
        assert!(!path_in(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
