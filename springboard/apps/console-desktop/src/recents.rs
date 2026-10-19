//! Open Recent across launches and the file named on the command line. The recent logs are document grants the shell
//! keeps (a grant reopens the file, `Ctx::reopen_document`); this module keeps the list of them in the app's
//! preferences, in the text the SDK's Recents store uses (`document::format_recents`). No window code here: lists and
//! paths in, lists and paths out.

use bifrost_app::document::{RecentDocument, MAX_RECENTS};
use bifrost_app::proto::GrantId;
use std::path::{Path, PathBuf};

/// The preference holding the recent logs.
pub const PREF: &str = "recentLogs";

/// The list with `grant` newest, and the grants that fell off the end (the caller releases them unless something
/// else holds them). Noting a log already in the list moves it to the front and does not drop it.
pub fn noted(list: &[RecentDocument], grant: GrantId, name: &str) -> (Vec<RecentDocument>, Vec<GrantId>) {
    let mut out = vec![RecentDocument { grant, name: name.to_string() }];
    out.extend(list.iter().filter(|r| r.grant != grant).cloned());
    let dropped = if out.len() > MAX_RECENTS { out.split_off(MAX_RECENTS).into_iter().map(|r| r.grant).collect() } else { Vec::new() };
    (out, dropped)
}

/// The list without `grant`.
pub fn without(list: &[RecentDocument], grant: GrantId) -> Vec<RecentDocument> {
    list.iter().filter(|r| r.grant != grant).cloned().collect()
}

/// The file named on the command line: the first argument after the program's name that is not an option. A relative
/// path is taken from `cwd`.
pub fn argv_path(args: impl IntoIterator<Item = String>, cwd: &Path) -> Option<PathBuf> {
    let arg = args.into_iter().skip(1).find(|a| !a.starts_with('-') && !a.is_empty())?;
    let path = PathBuf::from(arg);
    Some(if path.is_relative() { cwd.join(path) } else { path })
}

/// Whether `path` is a file or folder inside `home`. The shell grants paths only there (`Ctx::grant_path`), so a file
/// elsewhere is read directly and has no grant to reveal or trash.
pub fn under_home(path: &Path, home: &Path) -> bool {
    !home.as_os_str().is_empty() && path.strip_prefix(home).map_or(false, |rest| !rest.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(n: u8, name: &str) -> RecentDocument {
        RecentDocument { grant: [n; 16], name: name.to_string() }
    }

    #[test]
    fn a_noted_log_goes_first_and_a_repeat_moves_it_up() {
        let list = vec![recent(1, "a.log"), recent(2, "b.log")];
        let (now, dropped) = noted(&list, [3; 16], "c.log");
        assert_eq!(now.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["c.log", "a.log", "b.log"]);
        assert!(dropped.is_empty());
        let (again, dropped) = noted(&now, [2; 16], "b.log");
        assert_eq!(again.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["b.log", "c.log", "a.log"]);
        assert!(dropped.is_empty(), "a repeat never drops anything");
    }

    #[test]
    fn the_oldest_logs_fall_off_a_full_list_and_are_handed_back() {
        let full: Vec<RecentDocument> = (0..MAX_RECENTS as u8).map(|n| recent(n, "x.log")).collect();
        let (now, dropped) = noted(&full, [99; 16], "new.log");
        assert_eq!(now.len(), MAX_RECENTS);
        assert_eq!(now[0].grant, [99; 16]);
        assert_eq!(dropped, vec![[MAX_RECENTS as u8 - 1; 16]], "the oldest one");
    }

    #[test]
    fn a_forgotten_log_leaves_the_list() {
        let list = vec![recent(1, "a.log"), recent(2, "b.log")];
        assert_eq!(without(&list, [1; 16]), vec![recent(2, "b.log")]);
        assert_eq!(without(&list, [9; 16]), list);
    }

    #[test]
    fn the_list_survives_the_preference_text() {
        let list = vec![recent(1, "auth.log"), recent(2, "a tab\there.log")];
        let text = bifrost_app::document::format_recents(&list);
        let back = bifrost_app::document::parse_recents(&text);
        assert_eq!(back[0], recent(1, "auth.log"));
        assert_eq!(back[1].name, "a tab here.log", "a tab in a name would split the line, so it reads back as a space");
        assert!(bifrost_app::document::parse_recents("not a recent line\nzz\tbad grant\n").is_empty());
    }

    #[test]
    fn the_command_line_file_is_the_first_argument_that_is_not_an_option() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cwd = Path::new("/home/me");
        assert_eq!(argv_path(args(&["bifrost-console"]), cwd), None);
        assert_eq!(argv_path(args(&["bifrost-console", "--verbose", "/var/log/pacman.log"]), cwd), Some(PathBuf::from("/var/log/pacman.log")));
        assert_eq!(argv_path(args(&["bifrost-console", "logs/auth.log"]), cwd), Some(PathBuf::from("/home/me/logs/auth.log")), "relative to the working folder");
        assert_eq!(argv_path(args(&["bifrost-console", ""]), cwd), None);
    }

    #[test]
    fn only_paths_inside_the_home_folder_can_be_granted() {
        let home = Path::new("/home/me");
        assert!(under_home(Path::new("/home/me/logs/a.log"), home));
        assert!(!under_home(Path::new("/home/me"), home), "the folder itself is not a file");
        assert!(!under_home(Path::new("/var/log/a.log"), home));
        assert!(!under_home(Path::new("/home/mexico/a.log"), home), "a sibling folder that shares a prefix");
        assert!(!under_home(Path::new("/home/me/a.log"), Path::new("")), "no home folder known");
    }
}
