//! Stability gate: the Browser must start and stay alive on damaged state. No GPU and no web engine is needed: the app
//! runs headlessly under `AppHost` (as in compliance.rs) and the engine is never reached because the saved tabs are
//! about:blank or unreadable.
use bifrost_app::proto::Configure;
use springboard::shell::apphost::AppHost;
use std::path::Path;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_bifrost-browser");
const APP_ID: &str = "is.olibuijr.browser.desktop";

/// Starts the app with `session` as the saved session file (None: no file) and returns whether it survived 3 s.
fn survives(tag: &str, session: Option<&[u8]>) -> Result<(), String> {
    let home = Path::new("/tmp").join(format!("bfs-home-{tag}"));
    let data = Path::new("/tmp").join(format!("bfs-{tag}"));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&data);
    let dir = home.join(".local/share/bifrost-browser");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(bytes) = session {
        std::fs::write(dir.join("session.txt"), bytes).map_err(|e| e.to_string())?;
    }
    std::env::set_var("HOME", &home);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let cfg = Configure { w: 1200, h: 780, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: false, text_scale: 1.0, reduce_motion: false };
    let mut h = AppHost::spawn(APP_ID, Path::new(BIN), &data, &assets, cfg).map_err(|e| format!("spawn: {e:?}"))?;
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        let p = h.pump();
        if p.died || !h.running() {
            return Err(format!("{tag}: the app exited: {:?}", h.status()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

// One #[test] only: they share HOME, which is process-wide.
#[test]
fn browser_survives_damaged_session_files() {
    let cases: [(&str, Option<&[u8]>); 5] = [
        ("none", None),
        ("binary", Some(&[0, 159, 146, 150, 0, 255, 254])),
        ("future", Some(b"v 99\ntab about:blank\t\n")),
        ("blank", Some(b"v 1\nsel 7\ntab about:blank\t\nzoom x nan\n")),
        ("garbage", Some(b"tab\ntab \t\nzoom\nsel -9\n\x00\n")),
    ];
    let fails: Vec<String> = cases.iter().filter_map(|(tag, s)| survives(tag, *s).err()).collect();
    assert!(fails.is_empty(), "{fails:?}");
}
