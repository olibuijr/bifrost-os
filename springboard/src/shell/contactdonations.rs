//! The people apps saw and donated, kept beside the contacts file as a sidecar (`contact-donations.tsv`) so the
//! contacts file keeps its format. The model is `bifrost_ui::contact_donation`: its file form is the sidecar text. A
//! missing or unreadable sidecar is an empty store. The file stays on the machine; nothing is sent anywhere.
use bifrost_ui::contact_donation::DonationStore;
use std::io;
use std::path::{Path, PathBuf};

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contact-donations.tsv")
}

/// The store at `path`; empty when it is missing or unreadable.
pub fn load(path: &Path) -> DonationStore {
    std::fs::read_to_string(path).map(|t| DonationStore::from_text(&t)).unwrap_or_default()
}

/// Writes the store atomically.
pub fn save(path: &Path, store: &DonationStore) -> io::Result<()> {
    super::fsutil::write_atomic(path, store.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::contact_donation::{Feedback, Handle};

    #[test]
    fn the_sidecar_sits_beside_the_contacts_file_and_keeps_donations_and_answers() {
        let dir = std::env::temp_dir().join(format!("bifrost-donations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = path_for(&dir.join("contacts.tsv"));
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("contact-donations.tsv"));
        assert!(load(&path).is_empty(), "a missing sidecar is an empty store");
        let mut store = DonationStore::new();
        store.donate("mail", "Ada Lovelace", Handle::Email, "ada@example.com", 10).unwrap();
        store.donate("mail", "Bob", Handle::Email, "bob@example.com", 11).unwrap();
        store.feedback("bob@example.com", Feedback::Dismissed);
        save(&path, &store).unwrap();
        let back = load(&path);
        assert_eq!(back.suggestions(&[], 10).len(), 1);
        assert!(back.is_blocked("bob@example.com"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
