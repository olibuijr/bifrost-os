use super::*;

#[test]
fn stock_app_data_moves_once_into_the_package_dir() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Notes")).unwrap();
    std::fs::write(root.join("Notes/a.txt"), "hello").unwrap();
    std::fs::create_dir_all(root.join("Contacts")).unwrap();
    std::fs::write(root.join("Contacts/contacts.tsv"), "x").unwrap();
    std::fs::write(root.join("reminders.tsv"), "r").unwrap();
    std::fs::create_dir_all(root.join("VoiceMemos")).unwrap();
    std::fs::write(root.join("VoiceMemos/Voice Memo 1.wav"), "w").unwrap();
    for id in ["is.olibuijr.notes", "is.olibuijr.contacts", "is.olibuijr.reminders", "is.olibuijr.voicememos"] {
        migrate_legacy(&root, id, &root.join("appdata").join(id));
    }
    assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.notes/a.txt")).unwrap(), "hello");
    assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.contacts/Contacts/contacts.tsv")).unwrap(), "x");
    assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.reminders/reminders.tsv")).unwrap(), "r");
    assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.voicememos/Voice Memo 1.wav")).unwrap(), "w");
    // a second run with fresh old data does not overwrite what the app already owns
    std::fs::write(root.join("Notes/b.txt"), "later").unwrap();
    migrate_legacy(&root, "is.olibuijr.notes", &root.join("appdata/is.olibuijr.notes"));
    assert!(!root.join("appdata/is.olibuijr.notes/b.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}
