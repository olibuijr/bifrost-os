//! Parity sdk-tests-15: headless tests of the managed app plan and install decisions (`shell::ota`, `shell::store::api`)
//! and of the local backup archive that the privacy row relies on (`shell::backup`). Nothing here reaches the network
//! or the installed-app registry: the tests use the pure functions, the parsed catalogue and temporary directories.

use springboard::shell::backup::{self, decode, encode, name_ok};
use springboard::shell::ota::{apps_key, schedule_on, set_schedule, should_notify, Schedule, INTERVAL_S, MIN_GAP_S, STARTUP_DELAY_S};
use springboard::shell::store::api::{parse_catalog, system_plan, updates, updates_for, StoreApp};
use springboard::shell::store::install::InstalledApp;
use std::path::PathBuf;

const SHA: &str = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";

fn entry(id: &str, version: &str, kind: &str, min_shell: &str, protocol: u32) -> String {
    format!(
        r#"{{"id": "{id}", "name": "{id}", "version": "{version}", "package": "{id}.pkg", "sha256": "{SHA}", "kind": "{kind}", "min_shell": "{min_shell}", "sdk_protocol": {protocol}}}"#
    )
}

fn catalog(entries: &[String]) -> Vec<StoreApp> {
    parse_catalog(&format!(r#"{{"apps": [{}]}}"#, entries.join(","))).expect("the test catalogue parses")
}

fn installed(id: &str, version: &str, kind: &str) -> InstalledApp {
    InstalledApp {
        id: id.into(),
        name: id.into(),
        version: version.into(),
        exec_path: PathBuf::new(),
        icon_path: PathBuf::new(),
        dir: PathBuf::from("/nonexistent/apps").join(id),
        kind: kind.into(),
        provides: vec![],
        previous: None,
        purchased: 0,
    }
}

fn ids<'a>(apps: &[&'a StoreApp]) -> Vec<&'a str> {
    apps.iter().map(|a| a.id.as_str()).collect()
}

// --- managed app plan: what the background check installs ---------------------------------------------------------

#[test]
fn a_store_entry_keeps_its_metadata_and_a_lower_case_hash() {
    let apps = catalog(&[entry("org.example.notes", "1.2.3", "app", "0.1.0", 0)]);
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].name, "org.example.notes");
    assert_eq!(apps[0].version, "1.2.3");
    assert_eq!(apps[0].package, "org.example.notes.pkg");
    assert_eq!(apps[0].sha256, SHA.to_ascii_lowercase());
    assert_eq!(apps[0].kind, "app");
    assert!(!apps[0].is_preinstalled(), "a user app is installed only by the customer");
}

#[test]
fn the_system_plan_installs_missing_and_older_packages_and_skips_the_rest() {
    let cat = catalog(&[
        entry("sys.new", "1.0.0", "system", "0.1.0", 0),
        entry("sys.old", "1.0.0", "system", "0.1.0", 0),
        entry("sys.current", "1.0.0", "system", "0.1.0", 0),
        entry("sys.future", "1.0.0", "system", "9.0.0", 0),
        entry("app.one", "1.0.0", "app", "0.1.0", 0),
        entry("svc.bg", "1.0.0", "service", "0.1.0", 0),
    ]);
    let have = [installed("sys.old", "0.9.0", "system"), installed("sys.current", "1.0.0", "system")];
    let plan = system_plan(&cat, &have, "1.0.0");
    assert_eq!(ids(&plan), ["sys.new", "sys.old", "svc.bg"]);
}

#[test]
fn the_system_plan_waits_for_a_shell_new_enough_for_the_package() {
    let cat = catalog(&[entry("sys.needs-newer", "1.0.0", "system", "2.0.0", 0)]);
    assert!(system_plan(&cat, &[], "1.5.0").is_empty());
    assert_eq!(ids(&system_plan(&cat, &[], "2.0.0")), ["sys.needs-newer"]);
}

#[test]
fn updates_list_newer_user_apps_and_system_packages_but_never_services() {
    let cat = catalog(&[
        entry("app.one", "1.0.0", "app", "0.1.0", 0),
        entry("app.two", "1.0.0", "app", "0.1.0", 0),
        entry("svc.bg", "1.0.0", "service", "0.1.0", 0),
        entry("sys.pkg", "1.0.0", "system", "0.1.0", 0),
    ]);
    let have = [
        installed("app.one", "0.9.0", "app"),
        installed("app.two", "1.0.0", "app"),
        installed("svc.bg", "0.1.0", "service"),
        installed("sys.pkg", "0.9.0", "system"),
    ];
    assert_eq!(ids(&updates(&cat, &have)), ["app.one", "sys.pkg"], "the badge counts system packages; services stay hidden");
}

#[test]
fn an_app_built_for_another_sdk_protocol_is_no_update_for_this_shell() {
    let cat = catalog(&[entry("app.new-protocol", "2.0.0", "app", "0.1.0", 7), entry("app.any", "2.0.0", "app", "0.1.0", 0)]);
    let have = [installed("app.new-protocol", "1.0.0", "app"), installed("app.any", "1.0.0", "app")];
    assert_eq!(ids(&updates_for(&cat, &have, 1)), ["app.any"]);
    assert_eq!(ids(&updates_for(&cat, &have, 7)), ["app.new-protocol", "app.any"]);
}

// --- managed app checks: timing and banner identity ---------------------------------------------------------------

#[test]
fn the_background_check_waits_for_the_start_up_delay_then_runs_on_its_interval() {
    assert_eq!((STARTUP_DELAY_S, INTERVAL_S, MIN_GAP_S), (30.0, 900.0, 300.0));
    let mut s = Schedule::default();
    assert!(!s.due(10.0, true, false, false), "too early after start-up");
    assert!(s.due(35.0, true, false, false), "the first check");
    assert!(!s.due(36.0, true, false, false), "a second check is not due at once");
    assert!(s.due(35.0 + INTERVAL_S, true, false, false), "the next one after the interval");
}

#[test]
fn a_wifi_reconnect_checks_again_only_after_the_minimum_gap() {
    let mut s = Schedule::default();
    assert!(s.due(40.0, true, false, false));
    assert!(!s.due(50.0, false, false, false), "Wi-Fi dropped");
    assert!(!s.due(60.0, true, false, false), "back on Wi-Fi, but the last check is only 20 s old");
    assert!(!s.due(400.0, false, false, false));
    assert!(s.due(400.0, true, false, false), "back on Wi-Fi, 360 s after the last check");
}

#[test]
fn low_power_mode_holds_the_check_until_the_device_charges() {
    let mut low = Schedule::default();
    assert!(!low.due(40.0, true, true, false), "Low Power Mode and not charging");
    assert!(low.due(41.0, true, false, false), "the held check runs once power is back");
    let mut charging = Schedule::default();
    assert!(charging.due(40.0, true, true, true), "charging runs the check in Low Power Mode");
}

#[test]
fn no_check_runs_without_wifi() {
    let mut s = Schedule::default();
    assert!(!s.due(40.0, false, false, false));
    assert!(s.due(41.0, true, false, false));
}

#[test]
fn the_schedule_switch_file_turns_background_checks_off_and_back_on() {
    let dir = tempfile::tempdir().expect("temporary data directory");
    assert!(schedule_on(dir.path()), "background checks are on by default");
    set_schedule(dir.path(), false).expect("switch off");
    assert!(!schedule_on(dir.path()));
    set_schedule(dir.path(), true).expect("switch on");
    assert!(schedule_on(dir.path()));
    set_schedule(dir.path(), true).expect("switching on twice is not an error");
}

#[test]
fn a_banner_is_wanted_only_for_a_new_non_empty_key() {
    assert!(should_notify(None, "1.2.0"));
    assert!(!should_notify(Some("1.2.0"), "1.2.0"), "already announced");
    assert!(should_notify(Some("1.2.0"), "1.3.0"));
    assert!(!should_notify(None, ""), "an empty key never announces");
}

#[test]
fn the_app_update_key_does_not_depend_on_the_order_of_the_apps() {
    let a = vec![("b".to_string(), "1.0.0".to_string()), ("a".to_string(), "2.0.0".to_string())];
    let b = vec![("a".to_string(), "2.0.0".to_string()), ("b".to_string(), "1.0.0".to_string())];
    assert_eq!(apps_key(&a), "a@2.0.0,b@1.0.0");
    assert_eq!(apps_key(&a), apps_key(&b));
    assert_eq!(apps_key(&[]), "");
}

// --- backup archive: the local copy of the shell's data -----------------------------------------------------------

#[test]
fn only_the_shell_data_sources_are_valid_archive_names() {
    assert!(name_ok("Notes/a.txt"));
    assert!(name_ok("settings.ini"));
    assert!(name_ok("Calendar"));
    assert!(!name_ok("Notesfoo"), "a source name is a whole path component");
    assert!(!name_ok("other.txt"), "not a shell data source");
    assert!(!name_ok("../Notes/a.txt"));
    assert!(!name_ok("/Notes/a.txt"));
    assert!(!name_ok("Notes//a.txt"));
    assert!(!name_ok("Notes\\a.txt"));
    assert!(!name_ok(""));
}

#[test]
fn an_archive_decodes_to_the_entries_it_was_built_from() {
    let entries = vec![("settings.ini".to_string(), b"volume=4\n".to_vec()), ("Notes/empty.txt".to_string(), Vec::new())];
    assert_eq!(decode(&encode(&entries)).expect("a sound archive"), entries);
}

#[test]
fn an_archive_with_a_damaged_byte_or_a_bad_start_is_refused() {
    let entries = vec![("settings.ini".to_string(), b"volume=4\n".to_vec())];
    let mut bytes = encode(&entries);
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    assert_eq!(decode(&bytes).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(decode(b"not a backup at all, just text").unwrap_err().to_string(), "not a backup archive");
}

#[test]
fn an_archive_that_is_cut_short_is_refused() {
    let bytes = encode(&[("settings.ini".to_string(), b"volume=4\n".to_vec())]);
    assert!(decode(&bytes[..bytes.len() - 40]).is_err());
}

#[test]
fn an_entry_outside_the_data_sources_is_refused_even_with_sound_checksums() {
    let bytes = encode(&[("../escape.txt".to_string(), b"x".to_vec())]);
    assert_eq!(decode(&bytes).unwrap_err().to_string(), "unsafe entry name");
}

#[test]
fn a_backup_of_the_data_directory_restores_the_sources_and_leaves_other_files_out() {
    let src = tempfile::tempdir().expect("source data directory");
    let dst = tempfile::tempdir().expect("restore target");
    std::fs::create_dir_all(src.path().join("Notes")).unwrap();
    std::fs::write(src.path().join("Notes/a.txt"), b"first note").unwrap();
    std::fs::write(src.path().join("settings.ini"), b"theme=dark\n").unwrap();
    std::fs::write(src.path().join("scratch.tmp"), b"not backed up").unwrap();
    let archive = src.path().join("out.sbk");
    assert_eq!(backup::create(src.path(), &archive).expect("archive written"), 2);
    std::fs::create_dir_all(dst.path().join("Notes")).unwrap();
    assert_eq!(backup::restore(dst.path(), &archive).expect("archive restored"), 2);
    assert_eq!(std::fs::read(dst.path().join("Notes/a.txt")).unwrap(), b"first note");
    assert_eq!(std::fs::read(dst.path().join("settings.ini")).unwrap(), b"theme=dark\n");
    assert!(!dst.path().join("scratch.tmp").exists());
}

#[test]
fn the_newest_backup_in_the_backups_folder_is_found_by_its_time() {
    let dir = tempfile::tempdir().expect("data directory");
    assert!(backup::latest(dir.path()).is_none(), "no backups yet");
    std::fs::create_dir_all(dir.path().join("Notes")).unwrap();
    std::fs::write(dir.path().join("Notes/a.txt"), b"x").unwrap();
    let older = backup::backup_path(dir.path(), 1_000);
    let newer = backup::backup_path(dir.path(), 2_000);
    std::fs::create_dir_all(older.parent().unwrap()).unwrap();
    backup::create(dir.path(), &older).unwrap();
    backup::create(dir.path(), &newer).unwrap();
    assert_eq!(backup::latest(dir.path()), Some((newer, 2_000)));
}

#[test]
fn a_staged_restore_is_applied_once_at_the_next_start_and_then_removed() {
    let src = tempfile::tempdir().expect("source data directory");
    let dst = tempfile::tempdir().expect("data directory of the next start");
    std::fs::write(src.path().join("reminders.tsv"), b"buy milk\n").unwrap();
    let archive = src.path().join("out.sbk");
    backup::create(src.path(), &archive).unwrap();
    assert_eq!(backup::stage(dst.path(), &archive).expect("staged"), 1);
    assert!(dst.path().join(backup::PENDING).exists());
    assert!(matches!(backup::apply_pending(dst.path()), Some(Ok(1))));
    assert_eq!(std::fs::read(dst.path().join("reminders.tsv")).unwrap(), b"buy milk\n");
    assert!(!dst.path().join(backup::PENDING).exists(), "a staged file is removed either way");
    assert!(backup::apply_pending(dst.path()).is_none());
}

#[test]
fn a_damaged_archive_is_never_staged() {
    let src = tempfile::tempdir().expect("data directory");
    let archive = src.path().join("bad.sbk");
    std::fs::write(&archive, b"SBBK1\nnot really an archive").unwrap();
    assert!(backup::stage(src.path(), &archive).is_err());
    assert!(!src.path().join(backup::PENDING).exists());
}
