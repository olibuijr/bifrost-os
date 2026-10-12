//! The sort by date: the grid follows the fetch model's order, the picker's sort row flips it, and the choice is kept
//! in the prefs file.

use super::tests::{frame, photos, tap};
use super::*;

fn paths(p: &Photos) -> Vec<PathBuf> {
    p.photos().iter().map(|x| x.path.clone()).collect()
}

#[test]
fn the_sort_row_flips_the_grid_between_oldest_and_newest_first() {
    let (mut p, dir, ui) = photos("sort-by-date", 3);
    let oldest_first = paths(&p);
    assert_eq!(oldest_first.len(), 3);
    assert!(!p.newest_first, "the library opens oldest first, as before");
    frame(&mut p, &ui);

    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 100.0);
    assert!(p.album_menu);
    frame(&mut p, &ui);
    let (x, y, w, h) = p.sort_row(&ui);
    tap(&mut p, &ui, x + w / 2.0, y + h / 2.0, 200.0);
    assert!(!p.album_menu, "the picker closes after the sort changes");
    assert!(p.newest_first);
    let mut reversed = oldest_first.clone();
    reversed.reverse();
    assert_eq!(paths(&p), reversed);
    assert_eq!(p.prefs.get("sort"), Some("newest"));

    // a second tap on the row goes back to oldest first
    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 300.0);
    let (x, y, w, h) = p.sort_row(&ui);
    tap(&mut p, &ui, x + w / 2.0, y + h / 2.0, 400.0);
    assert!(!p.newest_first);
    assert_eq!(paths(&p), oldest_first);
    assert_eq!(p.prefs.get("sort"), Some("oldest"));
    let _ = std::fs::remove_dir_all(dir);
}
