//! Droplets: a shortcut whose automation is "When a File Is Dropped" runs when files are dropped on its icon, and the
//! dropped files are its input. The input is the paths, one per line, so the first step sees them as `{Output}` and a
//! "Repeat with Each" step visits one file at a time with the path as `{Item}`.
//!
//! This module turns what the shell was handed into that input and runs the shortcut with it. It reads no file: a path
//! is only text here, and a step that opens it does so through the host.

use super::flow::{self, Host, Step};
use super::{Automation, Shortcut, MAX_REPEAT};

/// The most files one drop passes in. A Repeat with Each pass visits at most `MAX_REPEAT` lines, so more would be
/// ignored anyway.
pub const MAX_FILES: usize = MAX_REPEAT as usize;

/// The input for a drop: the absolute paths in `paths`, in order, without repeats, one per line. A path that is not
/// absolute, is empty or holds a control character (a line break would split it in two) is left out, and so is
/// everything past `MAX_FILES`.
pub fn input_for_files(paths: &[&str]) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for path in paths {
        let path = path.trim();
        if path.starts_with('/') && !path.chars().any(char::is_control) && !kept.contains(&path) {
            kept.push(path);
            if kept.len() == MAX_FILES {
                break;
            }
        }
    }
    kept.join("\n")
}

/// Whether a drop on this shortcut's icon runs it.
pub fn accepts(shortcut: &Shortcut) -> bool {
    shortcut.automation == Some(Automation::FileDropped)
}

/// Runs `shortcut` for the dropped `paths`. `None` when the shortcut is not a droplet or none of the paths is usable;
/// otherwise the steps that ran, as `flow::run` reports them.
pub fn run(shortcut: &Shortcut, paths: &[&str], host: &mut dyn Host) -> Option<Vec<Step>> {
    if !accepts(shortcut) {
        return None;
    }
    let input = input_for_files(paths);
    if input.is_empty() {
        return None;
    }
    Some(flow::run_with_input(&shortcut.actions, host, &input))
}

#[cfg(test)]
mod tests {
    use super::super::flow::Done;
    use super::super::Action;
    use super::*;

    /// A host that remembers the text of every step it performs and the input it was given.
    #[derive(Default)]
    struct Seen(Vec<String>);

    impl Host for Seen {
        fn perform(&mut self, action: &Action, input: &str) -> Result<Done, String> {
            self.0.push(format!("{}|{}", action.text().unwrap_or(""), input));
            Ok(Done::said("done"))
        }
    }

    fn droplet(actions: Vec<Action>) -> Shortcut {
        let mut s = Shortcut::new("Convert", 0, actions);
        s.automation = Some(Automation::FileDropped);
        s
    }

    #[test]
    fn input_keeps_absolute_distinct_paths_in_order() {
        let input = input_for_files(&["/home/a/x.txt", "relative.txt", "", "/home/a/x.txt", " /home/b/y.png ", "/tmp/bad\nname"]);
        assert_eq!(input, "/home/a/x.txt\n/home/b/y.png");
    }

    #[test]
    fn input_stops_at_the_file_limit() {
        let many: Vec<String> = (0..MAX_FILES + 5).map(|i| format!("/f/{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(input_for_files(&refs).lines().count(), MAX_FILES);
    }

    #[test]
    fn the_first_step_sees_the_dropped_paths() {
        let s = droplet(vec![Action::ShowResult("{Output}".into())]);
        let mut host = Seen::default();
        let steps = run(&s, &["/home/a/x.txt", "/home/a/y.txt"], &mut host).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(host.0, vec!["/home/a/x.txt\n/home/a/y.txt|/home/a/x.txt\n/home/a/y.txt".to_string()]);
    }

    #[test]
    fn repeat_with_each_visits_one_file_at_a_time() {
        let s = droplet(vec![Action::RepeatEach, Action::ShowResult("{Item}".into()), Action::EndRepeat]);
        let mut host = Seen::default();
        run(&s, &["/home/a/x.txt", "/home/a/y.png"], &mut host).unwrap();
        let shown: Vec<&str> = host.0.iter().map(|l| l.split('|').next().unwrap()).collect();
        assert_eq!(shown, vec!["/home/a/x.txt", "/home/a/y.png"]);
    }

    #[test]
    fn a_shortcut_that_is_not_a_droplet_or_a_drop_without_usable_paths_does_not_run() {
        let plain = Shortcut::new("Plain", 0, vec![Action::ShowResult("x".into())]);
        let mut host = Seen::default();
        assert!(!accepts(&plain));
        assert!(run(&plain, &["/a"], &mut host).is_none());
        let s = droplet(vec![Action::ShowResult("x".into())]);
        assert!(accepts(&s));
        assert!(run(&s, &["relative", ""], &mut host).is_none());
        assert!(host.0.is_empty());
    }

    #[test]
    fn plain_run_still_starts_with_no_input() {
        let mut host = Seen::default();
        flow::run(&[Action::ShowResult("x".into())], &mut host);
        assert_eq!(host.0, vec!["x|".to_string()]);
    }
}
