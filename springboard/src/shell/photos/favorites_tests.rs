use super::library::tests::picture;
use super::tests::{choose_album, frame, photos, pump_until, tap};
use super::*;

fn favorite(p: &mut Photos, ui: &Ui, index: usize) -> Photo {
    p.open_viewer(index);
    let photo = p.viewing().cloned().unwrap();
    p.activate(ui, Target::Favourite, 0.0);
    assert!(p.is_favourite(&photo));
    photo
}

fn page(p: &mut Photos, ui: &Ui, forward: bool, t: f64) {
    let (from, to) = if forward {
        (1000.0, 100.0)
    } else {
        (100.0, 1000.0)
    };
    p.touch(ui, Event::Down(from, 900.0), t);
    p.touch(ui, Event::Move(to, 900.0), t + 100.0);
    p.touch(ui, Event::Up(to, 900.0), t + 200.0);
    p.tick(ui, t + 1500.0);
}

#[test]
fn favorites_grid_and_paging_use_filtered_indices_after_unfavoriting() {
    let (mut p, dir, ui) = photos("favorites-navigation", 6);
    let expected: Vec<_> = [0, 2, 4]
        .into_iter()
        .map(|i| favorite(&mut p, &ui, i))
        .collect();
    p.viewer = None;
    choose_album(&mut p, &ui, Album::Favorites, 100.0);
    assert_eq!(p.photos(), expected);
    let (x, y, w, h) = p.grid.cell_rect(1);
    tap(
        &mut p,
        &ui,
        x + w / 2.0,
        content_top(&ui) + y + h / 2.0,
        1000.0,
    );
    assert_eq!(p.viewing(), Some(&expected[1]));
    p.activate(&ui, Target::Favourite, 0.0);
    assert_eq!(
        p.viewing(),
        Some(&expected[2]),
        "the next favorite replaces the removed one"
    );
    let v = p.viewer.as_ref().unwrap();
    assert_eq!((v.index, v.count, v.offset()), (1, 2, 1200.0));
    page(&mut p, &ui, false, 2000.0);
    assert_eq!(
        p.viewing(),
        Some(&expected[0]),
        "paging cannot land on a nonfavorite"
    );
    page(&mut p, &ui, true, 4000.0);
    assert_eq!(p.viewing(), Some(&expected[2]));
    p.activate(&ui, Target::Favourite, 0.0);
    assert_eq!(
        p.viewing(),
        Some(&expected[0]),
        "last removal falls back to the previous favorite"
    );
    p.activate(&ui, Target::Favourite, 0.0);
    assert!(p.photos().is_empty() && p.viewer.is_none());
    assert_eq!(p.all.len(), 6, "unfavoriting never removes a photo file");
    assert!(p.all.iter().all(|photo| photo.path.exists()));
    frame(&mut p, &ui);
    let path = p.all[3].path.clone();
    assert!(p.show(&ui, &path), "external open falls back to All Photos");
    assert_eq!(p.album(), Album::All);
    assert_eq!(p.viewing().map(|photo| &photo.path), Some(&path));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn favorites_load_existing_markers_and_survive_reopening_photos() {
    let (mut p, dir, ui) = photos("favorites-persistence", 4);
    let selected = favorite(&mut p, &ui, 1);
    let legacy = p.all[3].clone();
    p.prefs
        .set(&format!("fav:{}", legacy.path.display()), "1")
        .unwrap();
    p.prefs.set("unrelated", "keep").unwrap();
    drop(p);
    let mut again =
        Photos::new(&ui, 1200, 1920, dir.clone(), vec![dir.join("Pictures")], 0).unwrap();
    pump_until(&mut again, &ui, |p| !p.scanning);
    choose_album(&mut again, &ui, Album::Favorites, 100.0);
    assert_eq!(again.photos(), [selected, legacy]);
    assert_eq!(again.prefs.get("unrelated"), Some("keep"));
    again.open_viewer(0);
    again.activate(&ui, Target::Favourite, 0.0);
    let path = again.photos()[0].path.clone();
    assert!(Prefs::load(dir.join(PREFS_FILE))
        .get(&format!("fav:{}", path.display()))
        .is_some_and(|v| v == "1"));
    assert_eq!(again.photos().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn scans_and_insertions_preserve_the_viewed_photo_and_pager_count() {
    let (mut p, dir, ui) = photos("favorites-remap", 4);
    favorite(&mut p, &ui, 1);
    let current = favorite(&mut p, &ui, 3);
    p.select_album(&ui, Album::Favorites);
    p.open_viewer(1);
    let older = dir.join("Pictures/older.png");
    picture(&older, 1_789_000_000);
    p.prefs
        .set(&format!("fav:{}", older.display()), "1")
        .unwrap();
    p.rescan();
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.viewing(), Some(&current));
    let viewer = p.viewer.as_ref().unwrap();
    assert_eq!(
        (viewer.index, viewer.count, viewer.offset()),
        (2, 3, 2400.0)
    );
    page(&mut p, &ui, false, 1000.0);
    assert_eq!(p.viewing().unwrap().name(), "IMG_0001.png");
    let nonfavorite = dir.join("Pictures/earlier.png");
    picture(&nonfavorite, 1_788_000_000);
    let photo = Photo::from_path(nonfavorite).unwrap();
    p.add_photo(
        &ui,
        photo,
        Image {
            w: 4,
            h: 4,
            px: vec![WHITE; 16],
        },
    );
    assert_eq!(
        p.viewing().unwrap().name(),
        "IMG_0001.png",
        "nonfavorite insertions do not shift the filtered viewer"
    );
    p.select_album(&ui, Album::All);
    assert!(p.show(&ui, &current.path));
    let extra = dir.join("Pictures/earliest.png");
    picture(&extra, 1_787_000_000);
    p.add_photo(
        &ui,
        Photo::from_path(extra).unwrap(),
        Image {
            w: 4,
            h: 4,
            px: vec![WHITE; 16],
        },
    );
    assert_eq!(
        p.viewing(),
        Some(&current),
        "All Photos also tracks identity across insertions"
    );
    assert_eq!(p.viewer.as_ref().unwrap().count, p.all.len());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn trash_and_recovery_keep_favorites_even_when_names_collide() {
    let (mut p, dir, ui) = photos("favorites-recover", 3);
    let original = favorite(&mut p, &ui, 2);
    // A different deleted picture already has the same filename.
    picture(&dir.join("Pictures/.trash/IMG_0002.png"), 1_788_000_000);
    p.select_album(&ui, Album::Favorites);
    p.open_viewer(0);
    p.delete(&ui, &original);
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert!(p.photos().is_empty() && p.viewer.is_none());
    assert!(!p.is_favourite(&original));
    let trashed = p
        .trash
        .iter()
        .find(|photo| photo.name() == "IMG_0002 2.png")
        .cloned()
        .unwrap();
    assert!(p.is_favourite(&trashed));
    p.select_album(&ui, Album::Trash);
    p.open_viewer(
        p.photos()
            .iter()
            .position(|photo| photo.path == trashed.path)
            .unwrap(),
    );
    assert!(!p
        .bar_buttons()
        .iter()
        .any(|(target, _)| *target == Target::Favourite));
    // Recovery also needs a new filename rather than replacing this picture.
    let occupied = dir.join("Pictures/IMG_0002 2.png");
    picture(&occupied, 1_789_000_000);
    let occupied_bytes = std::fs::read(&occupied).unwrap();
    p.activate(&ui, Target::Recover, 0.0);
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(std::fs::read(&occupied).unwrap(), occupied_bytes);
    assert!(!p.is_favourite(&trashed));
    p.select_album(&ui, Album::Favorites);
    assert_eq!(p.photos().len(), 1);
    let recovered = p.photos()[0].clone();
    assert_eq!(recovered.name(), "IMG_0002 2 2.png");
    assert!(p.is_favourite(&recovered));
    drop(p);
    let mut again =
        Photos::new(&ui, 1200, 1920, dir.clone(), vec![dir.join("Pictures")], 0).unwrap();
    pump_until(&mut again, &ui, |p| !p.scanning);
    again.select_album(&ui, Album::Favorites);
    assert_eq!(again.photos(), [recovered.clone()]);
    again.open_viewer(0);
    again.delete(&ui, &recovered);
    pump_until(&mut again, &ui, |p| !p.scanning);
    again.select_album(&ui, Album::Trash);
    let index = again
        .photos()
        .iter()
        .position(|photo| again.is_favourite(photo))
        .unwrap();
    again.open_viewer(index);
    let purge = again.viewing().cloned().unwrap();
    again.delete(&ui, &purge);
    pump_until(&mut again, &ui, |p| !p.scanning);
    assert!(!purge.path.exists() && !again.is_favourite(&purge));
    again.select_album(&ui, Album::Favorites);
    assert!(again.photos().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_move_recover_and_permanent_delete_preserve_favorite_markers() {
    let (mut p, dir, ui) = photos("favorites-file-failures", 1);
    let original = favorite(&mut p, &ui, 0);
    let blocked_trash = dir.join("Pictures/.trash");
    std::fs::write(&blocked_trash, b"not a directory").unwrap();
    p.delete(&ui, &original);
    assert!(original.path.exists() && p.is_favourite(&original));
    assert_eq!((p.all.len(), p.trash.len()), (1, 0));
    assert_eq!(
        Prefs::load(dir.join(PREFS_FILE)).get(&format!("fav:{}", original.path.display())),
        Some("1")
    );
    std::fs::remove_file(blocked_trash).unwrap();
    p.delete(&ui, &original);
    pump_until(&mut p, &ui, |p| !p.scanning);
    p.select_album(&ui, Album::Trash);
    p.open_viewer(0);
    let trashed = p.viewing().cloned().unwrap();
    let pictures = p.pictures.clone();
    let blocked_recovery = dir.join("blocked-recovery");
    std::fs::write(&blocked_recovery, b"not a directory").unwrap();
    p.pictures = blocked_recovery;
    p.activate(&ui, Target::Recover, 0.0);
    p.pictures = pictures;
    assert!(trashed.path.exists() && p.is_favourite(&trashed));
    assert_eq!(p.viewing(), Some(&trashed));
    // Simulate a file being replaced by a directory before confirmation.
    std::fs::rename(&trashed.path, dir.join("held.png")).unwrap();
    std::fs::create_dir_all(&trashed.path).unwrap();
    p.delete(&ui, &trashed);
    assert!(p.is_favourite(&trashed) && p.trash.contains(&trashed));
    assert_eq!(
        Prefs::load(dir.join(PREFS_FILE)).get(&format!("fav:{}", trashed.path.display())),
        Some("1")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_preferences_writes_keep_the_current_filter_and_source_marker() {
    let (mut p, dir, ui) = photos("favorites-prefs-failure", 2);
    let selected = favorite(&mut p, &ui, 0);
    p.select_album(&ui, Album::Favorites);
    p.open_viewer(0);
    // a read-only directory makes every preferences write fail (no temporary file can be created)
    let writable = std::fs::metadata(&dir).unwrap().permissions();
    let mut read_only = writable.clone();
    std::os::unix::fs::PermissionsExt::set_mode(&mut read_only, 0o555);
    std::fs::set_permissions(&dir, read_only).unwrap();
    p.activate(&ui, Target::Favourite, 0.0);
    assert_eq!(p.photos(), [selected.clone()]);
    assert_eq!(p.viewing(), Some(&selected));
    let destination = p.all[1].clone();
    p.move_favourite(&selected, &destination);
    assert!(p.is_favourite(&selected) && !p.is_favourite(&destination));
    std::fs::set_permissions(&dir, writable).unwrap();
    let saved = Prefs::load(dir.join(PREFS_FILE));
    assert_eq!(
        saved.get(&format!("fav:{}", selected.path.display())),
        Some("1")
    );
    assert_eq!(
        saved.get(&format!("fav:{}", destination.path.display())),
        None
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn empty_album_picker_and_outside_dismissal_remain_usable() {
    let (mut p, dir, ui) = photos("favorites-empty", 0);
    choose_album(&mut p, &ui, Album::Favorites, 100.0);
    assert!(p.photos().is_empty());
    frame(&mut p, &ui);
    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 300.0);
    assert!(p.album_menu);
    frame(&mut p, &ui);
    tap(&mut p, &ui, 1000.0, 900.0, 400.0);
    assert!(!p.album_menu && p.viewer.is_none());
    assert_eq!(p.album(), Album::Favorites);
    choose_album(&mut p, &ui, Album::Trash, 600.0);
    choose_album(&mut p, &ui, Album::All, 900.0);
    p.open_viewer(0);
    assert!(
        p.viewer.is_none(),
        "empty albums cannot open an invalid viewer"
    );
    let _ = std::fs::remove_dir_all(dir);
}
