//! Parity sdk-tests-12: headless tests of the app package layout rules. The rules look only at entry names, so each test
//! passes names as text and never reads a file, opens an archive or touches a device.

use bifrost_app::package_layout::{check_package, classify, LayoutError, Placement};

#[test]
fn headless_the_manifest_icon_and_executable_sit_at_the_top_level() {
    assert_eq!(classify("manifest.toml", "calc"), Ok(Placement::Manifest));
    assert_eq!(classify("icon.png", "calc"), Ok(Placement::Icon));
    assert_eq!(classify("calc", "calc"), Ok(Placement::Executable));
    assert!(matches!(classify("calc/inside", "calc"), Err(LayoutError::Unplaced(_))), "the binary is a file, not a folder");
}

#[test]
fn headless_resources_go_under_assets_at_any_depth() {
    assert_eq!(classify("assets/OFL.txt", "calc"), Ok(Placement::Resource));
    assert_eq!(classify("assets/words/en/common.txt", "calc"), Ok(Placement::Resource));
    assert!(matches!(classify("assets", "calc"), Err(LayoutError::Unplaced(_))), "the folder itself has no place");
}

#[test]
fn headless_shared_libraries_and_plug_ins_are_flat_so_files() {
    assert_eq!(classify("libs/libgfx.so", "calc"), Ok(Placement::Library));
    assert_eq!(classify("plugins/export.so", "calc"), Ok(Placement::Plugin));
    assert!(matches!(classify("libs/x.dylib", "calc"), Err(LayoutError::BadModule { .. })), "only .so modules load");
    assert!(matches!(classify("libs/.so", "calc"), Err(LayoutError::BadModule { .. })), "a module needs a name");
    assert!(matches!(classify("plugins/deep/export.so", "calc"), Err(LayoutError::BadModule { .. })), "no folders");
    assert!(matches!(classify("libs", "calc"), Err(LayoutError::Unplaced(_))));
}

#[test]
fn headless_an_entry_outside_the_layout_is_unplaced() {
    assert!(matches!(classify("secret.txt", "calc"), Err(LayoutError::Unplaced(_))));
    assert!(matches!(classify("bin/calc", "calc"), Err(LayoutError::Unplaced(_))));
    assert!(matches!(classify("data/store.db", "calc"), Err(LayoutError::Unplaced(_))));
}

#[test]
fn headless_names_that_could_escape_the_package_are_unsafe() {
    for bad in ["", "/etc/passwd", "assets/../../x", "./manifest.toml", "assets//a", "assets\\a", "a/./b"] {
        assert!(matches!(classify(bad, "calc"), Err(LayoutError::UnsafeName(_))), "{bad:?} must be refused");
    }
}

#[test]
fn headless_a_complete_package_places_every_entry_in_its_listed_order() {
    let names = ["manifest.toml", "icon.png", "calc", "assets/OFL.txt", "libs/libmath.so", "plugins/export.so"];
    let placed = check_package(names, "calc").unwrap();
    assert_eq!(
        placed,
        vec![
            Placement::Manifest,
            Placement::Icon,
            Placement::Executable,
            Placement::Resource,
            Placement::Library,
            Placement::Plugin,
        ]
    );
}

#[test]
fn headless_a_package_without_a_manifest_or_binary_reports_both() {
    let errs = check_package(["icon.png"], "calc").unwrap_err();
    assert!(errs.contains(&LayoutError::Missing("manifest.toml")));
    assert!(errs.contains(&LayoutError::Missing("the exec binary")));
    assert_eq!(errs.len(), 2);
}

#[test]
fn headless_a_package_listing_collects_every_problem_at_once() {
    let errs = check_package(["manifest.toml", "calc", "calc", "junk", "../up"], "calc").unwrap_err();
    assert!(errs.contains(&LayoutError::Duplicate("calc".into())));
    assert!(errs.contains(&LayoutError::Unplaced("junk".into())));
    assert!(errs.iter().any(|e| matches!(e, LayoutError::UnsafeName(_))));
    assert_eq!(errs.len(), 3, "the duplicate, the unplaced name and the unsafe name");
}

#[test]
fn headless_the_exec_name_must_be_a_bare_file_name() {
    let errs = check_package(["manifest.toml", "bin/calc"], "bin/calc").unwrap_err();
    assert!(errs.iter().any(|e| matches!(e, LayoutError::BadExec(_))));
    let dots = check_package(["manifest.toml"], "..").unwrap_err();
    assert!(dots.contains(&LayoutError::BadExec("..".into())));
}

#[test]
fn headless_a_layout_error_says_what_is_wrong_in_plain_words() {
    assert_eq!(LayoutError::Missing("manifest.toml").to_string(), "package is missing manifest.toml");
    assert_eq!(
        LayoutError::Unplaced("junk".into()).to_string(),
        "package entry has no place in the layout: junk"
    );
    assert_eq!(LayoutError::Duplicate("calc".into()).to_string(), "package entry appears twice: calc");
}
