//! Parity sdk-rest-13: headless tests for the udev database facade. Records are parsed from text, and records are read
//! from a temporary directory standing in for the udev data directory. No test touches the real database or a device.

use bifrost_app::udev::{parse_record, read_in, DbId, NodeKind, UdevError, DATA_ROOT};

const KEYBOARD: &str = "I:1790000000123456\nE:ID_INPUT=1\nE:ID_INPUT_KEYBOARD=1\nE:DEVNAME=/dev/input/event3\nS:input/by-id/usb-keyboard\nG:seat\nQ:seat\nV:1\nL:0\n";

#[test]
fn a_record_keeps_its_properties_symlinks_and_tags() {
    let rec = parse_record(KEYBOARD).unwrap();
    assert_eq!(rec.initialized_usec, Some(1_790_000_000_123_456));
    assert_eq!(rec.property("ID_INPUT_KEYBOARD"), Some("1"));
    assert_eq!(rec.property("DEVNAME"), Some("/dev/input/event3"));
    assert_eq!(rec.property("ID_FS_TYPE"), None);
    assert_eq!(rec.symlinks, vec!["input/by-id/usb-keyboard".to_string()]);
    assert!(rec.has_tag("seat"));
    assert!(!rec.has_tag("power-switch"));
}

#[test]
fn a_property_value_keeps_its_own_equals_signs() {
    let rec = parse_record("E:PROGRAM=a=b\n").unwrap();
    assert_eq!(rec.property("PROGRAM"), Some("a=b"));
}

#[test]
fn unknown_record_types_and_blank_lines_are_skipped() {
    let rec = parse_record("\nZ:a record type from a newer udev\nE:A=1\n\n").unwrap();
    assert_eq!(rec.properties.len(), 1);
    assert_eq!(rec.property("A"), Some("1"));
    assert!(rec.symlinks.is_empty() && rec.tags.is_empty());
}

#[test]
fn a_device_node_id_names_its_kind_and_numbers() {
    let block = DbId::parse("b8:0").unwrap();
    assert_eq!(block, DbId::Node { kind: NodeKind::Block, major: 8, minor: 0 });
    assert_eq!(block.file_name(), "b8:0");
    assert_eq!(DbId::parse("c226:128").unwrap().file_name(), "c226:128");
}

#[test]
fn a_subsystem_id_names_a_device_without_a_node() {
    let id = DbId::parse("+input:event3").unwrap();
    assert_eq!(id.file_name(), "+input:event3");
}

#[test]
fn ids_that_are_not_database_names_are_refused() {
    for bad in ["x8:0", "b8", "b8:-1", "b+8:0", "c:0", "+input", "+input:../etc", "+:event3", "+in put:event3", ""] {
        assert!(matches!(DbId::parse(bad), Err(UdevError::BadId(_))), "{bad:?} should be refused");
    }
}

#[test]
fn a_property_line_without_a_value_names_its_line() {
    assert_eq!(parse_record("I:1\nE:NOEQUALS\n"), Err(UdevError::BadLine { line: 2, reason: "property has no value" }));
}

#[test]
fn a_lower_case_property_key_is_refused() {
    assert_eq!(
        parse_record("E:id_fs=1\n"),
        Err(UdevError::BadLine { line: 1, reason: "property key must be upper case letters, digits or underscores" })
    );
    assert!(parse_record("E:ID_FS_TYPE=ext4\n").is_ok());
}

#[test]
fn a_symlink_must_stay_inside_the_device_directory() {
    let reason = "symlink must be a relative path inside the device directory";
    assert_eq!(parse_record("S:/dev/null\n"), Err(UdevError::BadLine { line: 1, reason }));
    assert_eq!(parse_record("E:A=1\nS:../../etc/passwd\n"), Err(UdevError::BadLine { line: 2, reason }));
    assert_eq!(parse_record("S:\n"), Err(UdevError::BadLine { line: 1, reason }));
}

#[test]
fn a_line_with_no_record_type_or_a_bad_number_is_refused() {
    assert_eq!(parse_record("garbage\n"), Err(UdevError::BadLine { line: 1, reason: "expected a record type and a colon" }));
    assert_eq!(parse_record("I:soon\n"), Err(UdevError::BadLine { line: 1, reason: "initialisation time is not a number" }));
    assert_eq!(parse_record("G:\n"), Err(UdevError::BadLine { line: 1, reason: "tag is empty" }));
}

#[test]
fn read_in_reads_the_record_named_by_the_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("c226:128"), "E:ID_PATH=pci-0000:00:02.0\nG:seat\n").unwrap();
    let rec = read_in(dir.path(), &DbId::parse("c226:128").unwrap()).unwrap();
    assert_eq!(rec.property("ID_PATH"), Some("pci-0000:00:02.0"));
    assert!(rec.has_tag("seat"));
}

#[test]
fn a_device_without_a_record_or_without_a_database_reads_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let id = DbId::parse("b8:0").unwrap();
    assert_eq!(read_in(dir.path(), &id), Err(UdevError::Missing));
    assert_eq!(read_in(&dir.path().join("no-udev-here"), &id), Err(UdevError::Missing));
}

#[test]
fn a_corrupt_record_file_is_reported_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("b8:16"), "E:OK=1\nE:bad key=2\n").unwrap();
    let id = DbId::parse("b8:16").unwrap();
    assert_eq!(
        read_in(dir.path(), &id),
        Err(UdevError::BadLine { line: 2, reason: "property key must be upper case letters, digits or underscores" })
    );
}

#[test]
fn the_facade_reads_the_udev_data_directory_the_sandbox_binds() {
    assert_eq!(DATA_ROOT, "/run/udev/data");
}

#[test]
fn errors_read_as_plain_sentences() {
    assert_eq!(UdevError::Missing.to_string(), "no udev record for this device");
    assert_eq!(UdevError::BadLine { line: 4, reason: "tag is empty" }.to_string(), "udev record line 4: tag is empty");
    assert_eq!(UdevError::BadId("x".into()).to_string(), "not a udev database id: x");
}
