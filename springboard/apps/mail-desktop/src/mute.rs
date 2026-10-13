//! Muted conversations (Message > Mute). A conversation is named by its first message: the first id of the
//! `References` header, else the `In-Reply-To` id, else the message's own id, else the subject without its `Re:`
//! prefixes. New mail in a muted conversation raises no notification. The set is kept in a text file, one key a line.

use crate::message::Summary;
use crate::threads::normalize_subject;
use std::collections::HashSet;
use std::path::Path;

/// File name of the muted set in the app's data directory.
pub const FILE: &str = "muted.txt";

/// The key of the conversation a message belongs to.
pub fn key(summary: &Summary) -> String {
    if let Some(id) = summary.references.first().or_else(|| summary.in_reply_to.first()).or(summary.message_id.as_ref()) {
        return id.clone();
    }
    format!("subject:{}", normalize_subject(&summary.subject))
}

/// Reads the muted keys. A missing or unreadable file is an empty set.
pub fn load(path: &Path) -> HashSet<String> {
    std::fs::read_to_string(path).map(|text| text.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

/// Writes the muted keys, sorted so the file stays the same when the set does.
pub fn save(path: &Path, muted: &HashSet<String>) -> Result<(), String> {
    let mut keys: Vec<&str> = muted.iter().map(String::as_str).filter(|k| !k.contains(['\r', '\n'])).collect();
    keys.sort_unstable();
    let mut text = keys.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(raw: &str) -> Summary {
        Summary::from_headers(raw)
    }

    #[test]
    fn a_reply_and_its_first_message_share_a_key() {
        let first = summary("Message-ID: <a@x>\r\nSubject: Fundur\r\n\r\n");
        let reply = summary("Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nReferences: <a@x>\r\nSubject: Re: Fundur\r\n\r\n");
        let third = summary("Message-ID: <c@x>\r\nIn-Reply-To: <b@x>\r\nReferences: <a@x> <b@x>\r\nSubject: Re: Re: Fundur\r\n\r\n");
        assert_eq!(key(&first), "<a@x>");
        assert_eq!(key(&reply), key(&first));
        assert_eq!(key(&third), key(&first));
    }

    #[test]
    fn a_message_without_ids_falls_back_to_its_subject() {
        assert_eq!(key(&summary("Subject: Re: Fwd: Plan\r\n\r\n")), key(&summary("Subject: plan\r\n\r\n")));
        assert_ne!(key(&summary("Subject: Plan\r\n\r\n")), key(&summary("Subject: Other\r\n\r\n")));
    }

    #[test]
    fn the_set_survives_a_round_trip_through_its_file() {
        let path = std::env::temp_dir().join(format!("bifrost-mail-mute-{}", std::process::id())).join(FILE);
        assert!(load(&path).is_empty(), "a missing file is an empty set");
        let muted: HashSet<String> = ["<b@x>".to_string(), "<a@x>".to_string()].into_iter().collect();
        save(&path, &muted).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<a@x>\n<b@x>\n");
        assert_eq!(load(&path), muted);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
