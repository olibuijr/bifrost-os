//! Frame-time budget for the Files listing with 5,000 entries: `bench_frame_files_5k`, run by `tools/bench.py` with the
//! other `bench_frame_*` scenes (see `shell/frame_budget.rs`). The folder is drawn in a 1200x1920 window body.
use super::fs::{Entry, Glyph, Kind, Location, Sort};
use super::pages::{DirPage, Page};
use super::Files;
use crate::gfx::Surface;
use crate::shell::frame_budget::{frame_ms, report};

const W: usize = 1200;
const H: usize = 1920;
const ITEMS: usize = 5000;

fn entries(home: &std::path::Path) -> Vec<Entry> {
    (0..ITEMS)
        .map(|i| {
            let name = format!("Item {i:04}.txt");
            Entry {
                path: home.join(&name),
                name,
                is_dir: false,
                size: 1024 * i as u64,
                mtime: 1_700_000_000 + 60 * i as i64,
                kind: Kind::Text,
                items: None,
            }
        })
        .collect()
}

#[test]
#[ignore]
fn bench_frame_files_5k() {
    let ui = crate::shell::tests::ui();
    let dir = std::env::temp_dir().join(format!("bifrost-frame-files-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = Location { name: "On My PC", path: home.clone(), glyph: Glyph::Computer };
    let mono: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice());
    let mut f = Files::with_locations(&ui, W as i32, H as i32, mono, vec![root.clone()]).unwrap();
    // the listing is installed directly (no disk walk): the folder page is the root of the stack
    f.nav.reset(Page::Dir(DirPage::new(&ui, &root, &home)));
    if let Page::Dir(d) = f.nav.top_mut() {
        d.set_entries(&ui, entries(&home), Sort::Name, None);
    }
    let mut buf = vec![0u32; W * H];
    let ms = frame_ms(3, 60, |k| {
        f.draw(&mut Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 }, &ui, 1000.0 + k as f64 * 16.0);
    });
    let _ = std::fs::remove_dir_all(&dir);
    report("files_5k", ms);
}
