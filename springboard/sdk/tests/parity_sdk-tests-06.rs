//! Parity sdk-tests-06: headless tests of file backup exclusion, the file manager's resource values, the ColorSync
//! face of the colour model, and the failure monitor with the crash records. The tests use temporary directories
//! and in-memory values only; none of them starts a process, opens a display or touches a device.

use std::fs;
use std::path::Path;

use bifrost_app::colorsync::{gamma_ramps, ColorError, IccError, ProfileConverter, Space};
use bifrost_app::exception_handler::{
    crash_records, default_handler, handling, hanging, Decision, ExceptionHandler, ExceptionHandlerDelegate, Failure,
    FailureKind,
};
use bifrost_app::file_manager::{NativeFileManager, ResourceKey};
use bifrost_app::file_metadata::{
    backup_items, copy_xattrs, is_excluded_from_backup, list_xattrs, remove_xattr, set_excluded_from_backup, set_xattr,
    xattr, BACKUP_EXCLUDED_XATTR,
};

// --- backup exclusion (file_metadata) ---------------------------------------------------------------------------------

#[test]
fn marking_an_item_excluded_from_backup_sets_its_attribute() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cache.bin");
    fs::write(&file, b"cached").unwrap();
    assert!(!is_excluded_from_backup(&file));
    set_excluded_from_backup(&file, true).unwrap();
    assert!(is_excluded_from_backup(&file));
    assert_eq!(xattr(&file, BACKUP_EXCLUDED_XATTR).unwrap(), Some(b"1".to_vec()));
}

#[test]
fn unmarking_an_item_clears_the_exclusion_and_unmarking_twice_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cache.bin");
    fs::write(&file, b"cached").unwrap();
    set_excluded_from_backup(&file, true).unwrap();
    set_excluded_from_backup(&file, false).unwrap();
    assert!(!is_excluded_from_backup(&file));
    assert_eq!(xattr(&file, BACKUP_EXCLUDED_XATTR).unwrap(), None);
    set_excluded_from_backup(&file, false).unwrap();
}

#[test]
fn a_missing_item_counts_as_included_in_backup() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!is_excluded_from_backup(&dir.path().join("missing")));
}

#[test]
fn an_extended_attribute_round_trips_a_value_and_reports_absence() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    fs::write(&file, b"n").unwrap();
    set_xattr(&file, "user.bifrost.test", b"abc").unwrap();
    assert_eq!(xattr(&file, "user.bifrost.test").unwrap(), Some(b"abc".to_vec()));
    remove_xattr(&file, "user.bifrost.test").unwrap();
    assert_eq!(xattr(&file, "user.bifrost.test").unwrap(), None);
    remove_xattr(&file, "user.bifrost.test").unwrap();
}

#[test]
fn listed_user_attributes_come_back_sorted_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    fs::write(&file, b"n").unwrap();
    set_xattr(&file, "user.bifrost.zeta", b"1").unwrap();
    set_xattr(&file, "user.bifrost.alpha", b"2").unwrap();
    let user: Vec<String> = list_xattrs(&file).unwrap().into_iter().filter(|n| n.starts_with("user.bifrost.")).collect();
    assert_eq!(user, vec!["user.bifrost.alpha".to_string(), "user.bifrost.zeta".to_string()]);
}

#[test]
fn copying_attributes_gives_the_copy_the_same_values() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("from.txt");
    let to = dir.path().join("to.txt");
    fs::write(&from, b"a").unwrap();
    fs::write(&to, b"b").unwrap();
    set_xattr(&from, "user.bifrost.note", b"kept").unwrap();
    copy_xattrs(&from, &to).unwrap();
    assert_eq!(xattr(&to, "user.bifrost.note").unwrap(), Some(b"kept".to_vec()));
}

#[test]
fn a_backup_walk_lists_items_depth_first_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("d")).unwrap();
    fs::write(root.join("a/x.txt"), b"x").unwrap();
    fs::write(root.join("b.txt"), b"b").unwrap();
    fs::create_dir_all(root.join("c")).unwrap();
    fs::write(root.join("c/y.txt"), b"y").unwrap();
    fs::write(root.join("d/z.txt"), b"z").unwrap();
    let items = backup_items(root).unwrap();
    assert_eq!(
        items,
        vec![root.join("a"), root.join("a/x.txt"), root.join("b.txt"), root.join("c"), root.join("c/y.txt"), root.join("d"), root.join("d/z.txt")]
    );
}

#[test]
fn an_excluded_directory_is_skipped_together_with_its_contents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("c/deep")).unwrap();
    fs::write(root.join("a/x.txt"), b"x").unwrap();
    fs::write(root.join("c/y.txt"), b"y").unwrap();
    fs::write(root.join("c/deep/w.txt"), b"w").unwrap();
    set_excluded_from_backup(&root.join("c"), true).unwrap();
    let items = backup_items(root).unwrap();
    assert_eq!(items, vec![root.join("a"), root.join("a/x.txt")]);
}

#[test]
fn an_excluded_file_is_left_out_and_its_siblings_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("keep.txt"), b"k").unwrap();
    fs::write(root.join("skip.tmp"), b"s").unwrap();
    set_excluded_from_backup(&root.join("skip.tmp"), true).unwrap();
    assert_eq!(backup_items(root).unwrap(), vec![root.join("keep.txt")]);
}

#[test]
fn an_excluded_root_gives_an_empty_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.txt"), b"a").unwrap();
    set_excluded_from_backup(root, true).unwrap();
    assert!(backup_items(root).unwrap().is_empty());
}

#[test]
fn a_symbolic_link_is_listed_but_its_target_is_not_entered() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("home");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), b"s").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    assert_eq!(backup_items(&root).unwrap(), vec![root.join("link")]);
}

#[test]
fn the_file_manager_reports_the_backup_exclusion_as_a_resource_value() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cache.bin");
    fs::write(&file, b"cached").unwrap();
    let fm = NativeFileManager;
    let before = fm.resource_values(&file, &[ResourceKey::IsExcludedFromBackup]).unwrap();
    assert_eq!(before.is_excluded_from_backup, Some(false));
    assert_eq!(before.file_size, None, "a key that was not asked for stays unset");
    set_excluded_from_backup(&file, true).unwrap();
    let after = fm.resource_values(&file, &[ResourceKey::IsExcludedFromBackup]).unwrap();
    assert_eq!(after.is_excluded_from_backup, Some(true));
}

// --- ColorSync face of the colour model (colorsync) -------------------------------------------------------------------

/// Builds a profile with the given device class, colour space and tags, each padded to four bytes.
fn build_profile(class: &str, color_space: &str, tags: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let table_end = 132 + 12 * tags.len();
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    for (_, bytes) in tags {
        offsets.push(table_end + body.len());
        body.extend_from_slice(bytes);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    let mut data = vec![0u8; table_end];
    data[0..4].copy_from_slice(&((table_end + body.len()) as u32).to_be_bytes());
    data[8] = 2;
    data[9] = 0x10;
    data[12..16].copy_from_slice(class.as_bytes());
    data[16..20].copy_from_slice(color_space.as_bytes());
    data[20..24].copy_from_slice(b"XYZ ");
    data[36..40].copy_from_slice(b"acsp");
    data[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
    for (i, (sig, bytes)) in tags.iter().enumerate() {
        let at = 132 + 12 * i;
        data[at..at + 4].copy_from_slice(sig.as_bytes());
        data[at + 4..at + 8].copy_from_slice(&(offsets[i] as u32).to_be_bytes());
        data[at + 8..at + 12].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
    }
    data.extend_from_slice(&body);
    data
}

fn xyz_tag(xyz: [f64; 3]) -> Vec<u8> {
    let mut v = b"XYZ ".to_vec();
    v.extend_from_slice(&[0; 4]);
    for c in xyz {
        v.extend_from_slice(&((c * 65536.0).round() as i32).to_be_bytes());
    }
    v
}

fn curv_identity() -> Vec<u8> {
    let mut v = b"curv".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

/// An RGB matrix-shaper profile with the sRGB primaries adapted to D50 and linear tone curves.
fn rgb_profile() -> Vec<u8> {
    build_profile(
        "mntr",
        "RGB ",
        &[
            ("rXYZ", xyz_tag([0.4361, 0.2225, 0.0139])),
            ("gXYZ", xyz_tag([0.3851, 0.7169, 0.0971])),
            ("bXYZ", xyz_tag([0.1431, 0.0606, 0.7141])),
            ("rTRC", curv_identity()),
            ("gTRC", curv_identity()),
            ("bTRC", curv_identity()),
        ],
    )
}

/// A `vcgt` table with two entries per channel: red ramps 0 to full, green 0 to half, blue 0 to a quarter.
fn vcgt_table() -> Vec<u8> {
    let mut v = b"vcgt".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&2u16.to_be_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&[0, 255, 0, 128, 0, 64]);
    v
}

#[test]
fn a_data_block_without_the_acsp_magic_is_refused_by_the_converter() {
    let mut data = vec![0u8; 200];
    data[0..4].copy_from_slice(&200u32.to_be_bytes());
    assert_eq!(ProfileConverter::open(&data, Space::Srgb).err(), Some(ColorError::Profile(IccError::NotAProfile)));
}

#[test]
fn a_profile_outside_the_rgb_space_is_refused_as_not_a_matrix_shaper() {
    let data = build_profile("mntr", "GRAY", &[("kTRC", curv_identity())]);
    assert_eq!(ProfileConverter::open(&data, Space::Srgb).err(), Some(ColorError::NotMatrixShaper));
}

#[test]
fn a_converter_writes_its_destination_space_and_maps_white_to_white() {
    let data = rgb_profile();
    let converter = ProfileConverter::open(&data, Space::DisplayP3).unwrap();
    assert_eq!(converter.destination(), Space::DisplayP3);
    let white = converter.apply([1.0, 1.0, 1.0]);
    assert!(white.iter().all(|c| (c - 1.0).abs() < 0.01), "got {white:?}");
}

#[test]
fn a_converter_clamps_eight_bit_output_to_the_range() {
    let data = rgb_profile();
    let converter = ProfileConverter::open(&data, Space::Srgb).unwrap();
    let out = converter.apply_u8([255, 255, 255]);
    assert!(out.iter().all(|&c| c >= 253), "got {out:?}");
}

#[test]
fn gamma_ramps_are_read_from_a_display_profile_table() {
    let data = build_profile("mntr", "RGB ", &[("vcgt", vcgt_table())]);
    let ramps = gamma_ramps(&data, 2).unwrap();
    assert_eq!(ramps[0], vec![0, 65535]);
    assert_eq!(ramps[1], vec![0, 32896]);
    assert_eq!(ramps[2], vec![0, 16448]);
}

#[test]
fn a_profile_without_a_gamma_table_gives_no_ramps() {
    let data = rgb_profile();
    assert_eq!(gamma_ramps(&data, 4).err(), Some(ColorError::NoGammaTable));
}

// --- failure monitor and crash records (exception_handler) ------------------------------------------------------------

fn failure(kind: FailureKind) -> Failure {
    Failure::new(kind, "boom", Some("src/a.rs:3:9"))
}

#[test]
fn each_kind_of_failure_owns_its_log_handle_and_hang_bits() {
    assert_eq!(FailureKind::Uncaught.index(), 0);
    assert_eq!(FailureKind::Other.index(), 4);
    assert_eq!(FailureKind::Other.log_bit(), handling::LOG_OTHER);
    assert_eq!(FailureKind::TopLevel.handling_bits(), handling::LOG_TOP_LEVEL | handling::HANDLE_TOP_LEVEL);
    assert_eq!(FailureKind::UncaughtRuntime.hang_bit(), hanging::UNCAUGHT_RUNTIME);
    assert_eq!(handling::ALL.count_ones(), 10);
    assert_eq!(hanging::ALL.count_ones(), 5);
}

#[test]
fn an_empty_mask_neither_logs_nor_handles_any_kind() {
    let h = ExceptionHandler::new();
    for kind in [FailureKind::Uncaught, FailureKind::UncaughtSystem, FailureKind::UncaughtRuntime, FailureKind::TopLevel, FailureKind::Other] {
        assert_eq!(h.decide(&failure(kind)), Decision { handle: false, log: false, halt: false });
    }
}

#[test]
fn a_monitored_kind_is_logged_and_handled_only_through_its_own_bits() {
    let mut h = ExceptionHandler::new();
    h.set_handling_mask(handling::LOG_TOP_LEVEL | handling::HANDLE_OTHER);
    assert_eq!(h.decide(&failure(FailureKind::TopLevel)), Decision { handle: false, log: true, halt: false });
    assert_eq!(h.decide(&failure(FailureKind::Other)), Decision { handle: true, log: false, halt: false });
    assert_eq!(h.decide(&failure(FailureKind::Uncaught)), Decision { handle: false, log: false, halt: false });
}

#[test]
fn halting_follows_the_hanging_mask_even_when_nothing_is_handled() {
    let mut h = ExceptionHandler::new();
    h.set_hanging_mask(hanging::UNCAUGHT_SYSTEM);
    assert_eq!(h.handling_mask(), 0);
    assert_eq!(h.decide(&failure(FailureKind::UncaughtSystem)), Decision { handle: false, log: false, halt: true });
    assert!(!h.decide(&failure(FailureKind::Uncaught)).halt);
}

#[test]
fn a_delegate_can_refuse_the_logging_of_a_monitored_kind() {
    struct RefuseLogs;
    impl ExceptionHandlerDelegate for RefuseLogs {
        fn should_log(&self, _failure: &Failure, _mask: u32) -> bool {
            false
        }
    }
    let mut h = ExceptionHandler::new();
    h.set_handling_mask(handling::LOG_UNCAUGHT | handling::HANDLE_UNCAUGHT);
    h.set_delegate(Some(Box::new(RefuseLogs)));
    assert_eq!(h.decide(&failure(FailureKind::Uncaught)), Decision { handle: true, log: false, halt: false });
    h.set_delegate(None);
    assert!(h.delegate().is_none());
    assert_eq!(h.decide(&failure(FailureKind::Uncaught)), Decision { handle: true, log: true, halt: false });
}

#[test]
fn the_process_wide_handler_starts_with_nothing_monitored() {
    assert_eq!(default_handler().lock().unwrap().handling_mask(), 0);
}

#[test]
fn crash_records_list_the_newest_crash_first_across_both_logs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("crash.log.old"),
        "[1700000000 00:00:00] thread 'main': panic: old at src/old.rs:1:1\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("crash.log"),
        "[1700000500 00:08:20] thread 'main': panic: boom at src/a.rs:3:9\nnot a record\n[1700000100 00:01:40] thread 'w': exit: code 3\n",
    )
    .unwrap();
    let records = crash_records(dir.path());
    let times: Vec<i64> = records.iter().map(|r| r.unix).collect();
    assert_eq!(times, vec![1700000500, 1700000100, 1700000000]);
    assert_eq!(records[0].signature, "src/a.rs:3 boom");
    assert_eq!(records[1].signature, "exit: code 3", "a message that is not a panic is shown as it is");
    assert_eq!(records[2].file, dir.path().join("crash.log.old"));
    assert_eq!(records[0].file, dir.path().join("crash.log"));
}

#[test]
fn a_long_crash_message_is_cut_to_sixty_characters() {
    let dir = tempfile::tempdir().unwrap();
    let message = "x".repeat(80);
    fs::write(dir.path().join("crash.log"), format!("[1700000000 00:00:00] thread 'main': {message}\n")).unwrap();
    let records = crash_records(dir.path());
    assert_eq!(records.len(), 1);
    let sig = &records[0].signature;
    assert_eq!(sig.chars().count(), 60);
    assert!(sig.ends_with('…'));
}

#[test]
fn a_directory_without_crash_logs_has_no_records() {
    let dir = tempfile::tempdir().unwrap();
    assert!(crash_records(dir.path()).is_empty());
    assert!(crash_records(Path::new("/nonexistent-bifrost-crash-dir")).is_empty());
}
