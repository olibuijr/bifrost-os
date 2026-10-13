//! `mdutil`: shows and changes whether a volume is indexed for search, and erases its index. The index settings are kept
//! per volume under the user's state folder (`bifrost/index/<volume>/`): an `enabled` file holding `on` or `off`
//! (a volume is indexed until it says otherwise), and a `store` folder that holds the index data.
//! Gap: the desktop's quick search does not read these settings yet, so `-i off` does not stop its index today, and
//! `-E` only removes the command line's own store.
//! Supported: `-s` (status), `-i on|off` and `-E` (erase), each with one or more volume paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: mdutil [-s] [-i on | off] [-E] volume ...";

/// What the command line asks for. Each volume path is one operand.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Status(Vec<String>),
    Set { on: bool, volumes: Vec<String> },
    Erase(Vec<String>),
}

/// Reads the arguments. Exactly one action is allowed, and it needs at least one volume.
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut status = false;
    let mut erase = false;
    let mut set: Option<bool> = None;
    let mut volumes: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "-help" | "--help" => return Ok(Action::Help),
            "-s" => status = true,
            "-E" => erase = true,
            "-i" => {
                let value = it.next().ok_or("-i needs on or off")?;
                set = Some(match value.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("-i takes on or off, not {other}")),
                });
            }
            other if other.starts_with('-') && other.len() > 1 => return Err(format!("option {other} is not supported yet")),
            _ => volumes.push(arg.clone()),
        }
    }
    let chosen = [status, erase, set.is_some()].iter().filter(|x| **x).count();
    if chosen > 1 {
        return Err("choose one of -s, -i and -E".into());
    }
    if volumes.is_empty() {
        return Err(USAGE.to_string());
    }
    Ok(match (set, erase) {
        (Some(on), _) => Action::Set { on, volumes },
        (None, true) => Action::Erase(volumes),
        _ => Action::Status(volumes),
    })
}

/// The folder that holds the per-volume index settings: `$XDG_STATE_HOME/bifrost/index`, else `~/.local/state/...`.
pub fn state_root(xdg_state_home: Option<&str>, home: Option<&str>) -> PathBuf {
    match (xdg_state_home.filter(|s| !s.is_empty()), home.filter(|s| !s.is_empty())) {
        (Some(x), _) => PathBuf::from(x).join("bifrost/index"),
        (None, Some(h)) => PathBuf::from(h).join(".local/state/bifrost/index"),
        (None, None) => PathBuf::from(".bifrost/index"),
    }
}

/// The folder name for a volume: `/` becomes `root`, and other separators become `_`.
pub fn volume_key(volume: &str) -> String {
    let trimmed = volume.trim_matches('/');
    if trimmed.is_empty() {
        return "root".into();
    }
    trimmed.replace('/', "_")
}

/// The folder that holds one volume's settings and index.
pub fn volume_dir(root: &Path, volume: &str) -> PathBuf {
    root.join(volume_key(volume))
}

/// Whether a volume is indexed. A volume with no setting yet is indexed.
pub fn is_enabled(root: &Path, volume: &str) -> bool {
    match fs::read_to_string(volume_dir(root, volume).join("enabled")) {
        Ok(text) => text.trim() != "off",
        Err(_) => true,
    }
}

/// Saves whether a volume is indexed.
pub fn set_enabled(root: &Path, volume: &str, on: bool) -> io::Result<()> {
    let dir = volume_dir(root, volume);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("enabled"), if on { "on\n" } else { "off\n" })
}

/// Removes a volume's index data. The setting is kept. A volume that was never indexed is not an error.
pub fn erase(root: &Path, volume: &str) -> io::Result<()> {
    match fs::remove_dir_all(volume_dir(root, volume).join("store")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The status lines for one volume, in the same shape `mdutil -s` prints.
pub fn status_text(root: &Path, volume: &str) -> String {
    let state = if is_enabled(root, volume) { "enabled" } else { "disabled" };
    format!("{volume}:\nIndexing {state}.\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-mdutil-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_action_is_read_with_its_volumes() {
        assert_eq!(parse(&args(&["-s", "/"])), Ok(Action::Status(vec!["/".into()])));
        assert_eq!(parse(&args(&["-i", "off", "/home"])), Ok(Action::Set { on: false, volumes: vec!["/home".into()] }));
        assert_eq!(parse(&args(&["-E", "/"])), Ok(Action::Erase(vec!["/".into()])));
        assert_eq!(parse(&args(&["-h"])), Ok(Action::Help));
    }

    #[test]
    fn bad_arguments_are_refused_with_a_reason() {
        assert!(parse(&args(&["-i", "maybe", "/"])).unwrap_err().contains("on or off"));
        assert!(parse(&args(&["-s", "-E", "/"])).unwrap_err().contains("choose one"));
        assert_eq!(parse(&args(&["-s"])), Err(USAGE.to_string()));
        assert!(parse(&args(&["-a", "/"])).unwrap_err().contains("-a"));
    }

    #[test]
    fn volumes_map_to_folder_names() {
        assert_eq!(volume_key("/"), "root");
        assert_eq!(volume_key("/home/u/disk"), "home_u_disk");
        assert_eq!(state_root(Some("/s"), Some("/h")), PathBuf::from("/s/bifrost/index"));
        assert_eq!(state_root(None, Some("/h")), PathBuf::from("/h/.local/state/bifrost/index"));
    }

    #[test]
    fn a_volume_is_indexed_until_it_is_turned_off() {
        let root = scratch("toggle");
        assert!(is_enabled(&root, "/"));
        set_enabled(&root, "/", false).unwrap();
        assert!(!is_enabled(&root, "/"));
        assert_eq!(status_text(&root, "/"), "/:\nIndexing disabled.\n");
        set_enabled(&root, "/", true).unwrap();
        assert!(is_enabled(&root, "/"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn erase_removes_the_store_and_keeps_the_setting() {
        let root = scratch("erase");
        set_enabled(&root, "/data", false).unwrap();
        fs::create_dir_all(volume_dir(&root, "/data").join("store")).unwrap();
        erase(&root, "/data").unwrap();
        assert!(!volume_dir(&root, "/data").join("store").exists());
        assert!(!is_enabled(&root, "/data"));
        erase(&root, "/never-indexed").unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
