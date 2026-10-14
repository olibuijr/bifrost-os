//! The password typed for a PDF that needs one. The window asks for it in the status line, the way Go to Page and Find
//! take their text, and shows the typing as stars. This is the pure model: the text typed so far, its edits, and the
//! message. The password is held in memory for the session and is never written to a file or shown.

use std::path::{Path, PathBuf};

/// A document waiting for its user password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub path: PathBuf,
    typed: String,
    /// True when a password was already tried and refused, so the message says so.
    retry: bool,
}

impl Ask {
    /// A request for the password of the document at `path`. `retry` is true after a refused password.
    pub fn new(path: &Path, retry: bool) -> Ask {
        Ask { path: path.to_path_buf(), typed: String::new(), retry }
    }

    /// Adds the typed text. Control characters are dropped.
    pub fn type_text(&mut self, text: &str) {
        self.typed.extend(text.chars().filter(|c| !c.is_control()));
    }

    /// Removes the last character typed.
    pub fn delete(&mut self) {
        self.typed.pop();
    }

    /// The password as the document is opened with it.
    pub fn password(&self) -> Vec<u8> {
        self.typed.as_bytes().to_vec()
    }

    /// The status line: what is asked for, with the typing shown as one star per character.
    pub fn message(&self, name: &str) -> String {
        let stars = "*".repeat(self.typed.chars().count());
        let lead = if self.retry { "That password is not right. " } else { "" };
        format!("{lead}Password for {name}: {stars}|  Return opens it, Escape cancels")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_builds_the_password_and_backspace_takes_a_character_back() {
        let mut ask = Ask::new(Path::new("/a/secret.pdf"), false);
        ask.type_text("pa");
        ask.type_text("ss\u{7}");
        assert_eq!(ask.password(), b"pass".to_vec(), "a control character is dropped");
        ask.delete();
        assert_eq!(ask.password(), b"pas".to_vec());
        ask.delete();
        ask.delete();
        ask.delete();
        ask.delete();
        assert!(ask.password().is_empty(), "deleting past the start does nothing");
    }

    #[test]
    fn the_message_names_the_file_and_hides_the_typing() {
        let mut ask = Ask::new(Path::new("/a/secret.pdf"), false);
        ask.type_text("hunter2");
        let msg = ask.message("secret.pdf");
        assert!(msg.starts_with("Password for secret.pdf: *******|"), "{msg}");
        assert!(!msg.contains("hunter"), "the typed text is never shown");
    }

    #[test]
    fn a_refused_password_is_said_so() {
        let ask = Ask::new(Path::new("/a/secret.pdf"), true);
        assert!(ask.message("secret.pdf").starts_with("That password is not right. Password for secret.pdf"));
        assert!(!Ask::new(Path::new("/a/x.pdf"), false).message("x.pdf").contains("not right"));
    }

    #[test]
    fn stars_count_characters_not_bytes() {
        let mut ask = Ask::new(Path::new("/a/x.pdf"), false);
        ask.type_text("\u{f6}\u{f0}");
        assert!(ask.message("x.pdf").contains(": **|"));
        assert_eq!(ask.password().len(), 4, "the password keeps its UTF-8 bytes");
    }
}
