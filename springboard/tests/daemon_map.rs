//! Guards the launchd daemon rows in `docs/parity-matrix/os/daemons/*.json`: a row that claims a BifrOSt counterpart
//! (status `partial` or `parity`) must cite a path that exists in the repo and say in its note what fills the role.
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("springboard has a parent").to_path_buf()
}

/// Every `"key": "value"` string pair of one JSON object line group is enough here: rows are flat objects, so a small
/// scanner over the shard text avoids a JSON dependency in the test.
fn rows(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cur: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t == "{" && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if let Some((k, v)) = t.strip_prefix('"').and_then(|r| r.split_once("\": \"")) {
            let v = v.trim_end_matches(',').trim_end_matches('"');
            if !k.contains('"') {
                cur.push((k.to_string(), v.to_string()));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn field<'a>(row: &'a [(String, String)], key: &str) -> &'a str {
    row.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn shards() -> Vec<(PathBuf, String)> {
    let dir = repo_root().join("docs/parity-matrix/os/daemons");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .expect("daemon shards directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let t = std::fs::read_to_string(&p).expect("read shard");
            (p, t)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn claimed_rows_point_at_existing_paths_with_a_note() {
    let root = repo_root();
    let mut claimed = 0;
    for (path, text) in shards() {
        for row in rows(&text) {
            let status = field(&row, "status");
            if status != "partial" && status != "parity" {
                continue;
            }
            claimed += 1;
            let name = field(&row, "name");
            let bifrost = field(&row, "bifrost");
            assert!(!bifrost.is_empty(), "{name} in {path:?} is {status} without a bifrost path");
            for file in bifrost.split(',').map(str::trim) {
                let file = file.split(':').next().unwrap_or("");
                assert!(root.join(file).exists(), "{name}: cited path {file} does not exist");
            }
            assert!(!field(&row, "note").trim().is_empty(), "{name}: {status} row has an empty note");
        }
    }
    assert!(claimed > 0, "no partial daemon rows found; the scanner or the data moved");
}

#[test]
fn shell_roles_are_mapped_and_platform_services_are_classified() {
    let all: Vec<_> = shards().into_iter().flat_map(|(_, t)| rows(&t)).collect();
    let status_of = |n: &str| {
        all.iter().find(|r| field(r, "name") == n).map(|r| field(r, "status").to_string()).unwrap_or_default()
    };
    for n in [
        "com.apple.Dock",
        "com.apple.Finder",
        "com.apple.controlcenter",
        "com.apple.notificationcenterui",
        "com.apple.usernotificationsd",
        "com.apple.WindowManager",
        "com.apple.WindowServer",
        "com.apple.Spotlight",
        "com.apple.corespotlightd",
        "com.apple.mdworker.shared",
        "com.apple.screencaptureui",
        "com.apple.wallpaper",
        "com.apple.loginwindow",
        "com.apple.TextInputSwitcher",
        "com.apple.TextInputMenuAgent",
        "com.apple.menuextra.battery.helper",
        "com.apple.xpc.loginitemregisterd",
        "com.apple.coreservices.sharedfilelistd",
        "com.apple.screensharing",
    ] {
        assert_eq!(status_of(n), "partial", "{n} should be mapped to the shell service that fills its role");
    }
    for n in ["com.apple.Siri.agent", "com.apple.FamilyControlsAgent"] {
        assert_eq!(status_of(n), "gap", "{n} has no real counterpart and must stay a gap");
    }
    for n in ["com.apple.iCloudNotificationAgent", "com.apple.icloud.searchpartyd"] {
        assert_eq!(status_of(n), "n/a", "{n} is a vendor cloud service with no BifrOSt counterpart");
    }
    assert_eq!(status_of("com.apple.ScreenTimeAgent"), "partial", "Screen Time usage and limits are mapped in the shell");
    assert_eq!(status_of("com.apple.MemorySlotNotification"), "n/a", "Apple-only memory slot hardware");
}
