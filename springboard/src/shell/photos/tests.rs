use super::library::tests::{picture, scratch};
use super::*;
use crate::shell::tests::ui;

/// The embedded 8x8 fixture with an EXIF block of the given orientation spliced in.
fn oriented_jpeg(orientation: u16) -> Vec<u8> {
    use super::exif::tests::{short, tiff};
    let dot = include_bytes!("../../deildu/fixtures/dot.jpg");
    let t = tiff(true, vec![short(true, 0x0112, orientation)], vec![], vec![]);
    let mut out = dot[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((t.len() + 8) as u16).to_be_bytes());
    out.extend(b"Exif\0\0");
    out.extend(&t);
    out.extend(&dot[2..]);
    out
}

/// The Dock's stack opens a picture from Downloads in Photos: one in the library opens in place, one outside it is
/// added for the session and shown, and a file that is no picture is refused.
#[test]
fn a_picture_outside_the_library_opens_in_the_viewer() {
    let (mut p, dir, ui) = photos("open-file", 2);
    let inside = dir.join("Pictures/IMG_0001.png");
    assert!(p.open_file(&ui, &inside));
    assert_eq!(p.viewing().map(|v| v.path.clone()), Some(inside));
    let outside = dir.join("Downloads/shot.png");
    picture(&outside, 1_790_500_000);
    assert!(p.open_file(&ui, &outside));
    assert_eq!(p.viewing().map(|v| v.path.clone()), Some(outside.clone()));
    let text = dir.join("Downloads/notes.png");
    std::fs::write(&text, b"not a picture").unwrap();
    assert!(!p.open_file(&ui, &text));
    assert!(!p.open_file(&ui, &dir.join("Downloads/missing.png")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn decoding_applies_the_exif_orientation() {
    let plain = crate::cg::imageio::decode(include_bytes!("../../deildu/fixtures/dot.jpg")).unwrap();
    assert_eq!(decode_upright(&oriented_jpeg(1)).unwrap(), plain);
    assert_eq!(decode_upright(&oriented_jpeg(3)).unwrap(), edit::rotate(&plain, 2));
    assert_eq!(decode_upright(&oriented_jpeg(6)).unwrap(), edit::rotate(&plain, 1));
}

/// A Photos app over a scratch library of `n` pictures a day apart.
pub(super) fn photos(name: &str, n: usize) -> (Photos, PathBuf, Ui) {
    let ui = ui();
    let dir = scratch(name);
    let pics = dir.join("Pictures");
    for i in 0..n {
        picture(&pics.join(format!("IMG_{i:04}.png")), 1_790_000_000 + i as i64 * 86_400);
    }
    let mut p = Photos::new(&ui, 1200, 1920, dir.clone(), vec![pics, dir.join("missing")], 0).unwrap();
    pump_until(&mut p, &ui, |p| !p.scanning);
    (p, dir, ui)
}

pub(super) fn pump_until(p: &mut Photos, ui: &Ui, done: impl Fn(&Photos) -> bool) {
    let start = std::time::Instant::now();
    while !done(p) && start.elapsed().as_secs() < 30 {
        crate::bare::sys::wait(&p.fds(), 50);
        p.pump(ui);
    }
    assert!(done(p), "timed out: {}", p.describe());
}

pub(super) fn tap(p: &mut Photos, ui: &Ui, x: f32, y: f32, t: f64) {
    p.touch(ui, Event::Down(x, y), t);
    p.touch(ui, Event::Up(x, y), t + 30.0);
}

pub(super) fn choose_album(p: &mut Photos, ui: &Ui, album: Album, t: f64) {
    tap(p, ui, ui.pt(40.0), nav_mid(ui), t);
    assert!(p.album_menu);
    let (x, y, w, h) = p.album_row(ui, album);
    tap(p, ui, x + w / 2.0, y + h / 2.0, t + 100.0);
    assert_eq!(p.album(), album);
    assert!(!p.album_menu);
}

pub(super) fn frame(p: &mut Photos, ui: &Ui) -> Vec<u32> {
    let mut buf = vec![0u32; 1200 * 1920];
    p.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, 0.0);
    buf
}

/// E2E7 finding 3: in a narrow desktop window the empty library's hint wraps to the pane (right of the source
/// list) instead of running under it, and the Open Camera button follows the taller block; a wide pane keeps the
/// reference's two lines.
#[test]
fn the_empty_library_hint_wraps_to_a_narrow_pane() {
    let (mut p, dir, ui) = photos("empty-wrap", 0);
    p.set_desktop(&ui, true);
    p.set_size(&ui, ui.pt(1400.0) as i32, ui.pt(900.0) as i32);
    assert!(p.blank_library());
    assert_eq!(p.empty_hint_lines(&ui), [EMPTY_HINT_FIRST, "copy photos and videos into Pictures using Files."], "a wide pane: the reference's two lines");
    let wide = p.camera_button(&ui);
    assert_eq!(wide.1, p.h as f32 / 2.0 + ui.pt(34.0));

    p.set_size(&ui, ui.pt(500.0) as i32, ui.pt(300.0) as i32);
    let lines = p.empty_hint_lines(&ui);
    let max = p.w as f32 - 2.0 * ui.pt(MARGIN_PT);
    assert!(lines.len() > 2, "a narrow pane wraps the hint: {lines:?}");
    assert!(lines.iter().all(|l| ui.measure(l, EMPTY_HINT_PT, Weight::Regular) <= max), "every line fits the pane: {lines:?}");
    assert_eq!(lines.join(" "), EMPTY_HINT, "no words dropped");
    let (bx, by, bw, bh) = p.camera_button(&ui);
    assert!(bx >= 0.0 && bx + bw <= p.w as f32, "the button is centred in the pane");
    assert_eq!(by, p.h as f32 / 2.0 + p.empty_shift(&ui, lines.len()) + ui.pt(34.0), "the button moves down with the extra lines");
    // a click on the moved button (window coordinates: right of the source list) still opens the camera
    let sx = p.sidebar_px() as f32;
    tap(&mut p, &ui, sx + bx + bw / 2.0, by + bh / 2.0, 100.0);
    assert!(p.take_camera_request());
    p.set_desktop(&ui, false);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn album_rows_delete_sort_and_set_the_slide_length() {
    let (mut p, dir, ui) = photos("album-rows", 2);
    assert_eq!((p.new_album(), p.new_album(), p.new_album()), (Some(0), Some(1), Some(2)));
    p.pref_set("ua.0.name", "Trip");
    p.pref_set("ua.1.name", "aspen");
    p.pref_set("ua.2.name", "Beach");
    p.rebuild(&ui);
    let order = |p: &Photos| p.menu_albums().into_iter().filter_map(|a| if let Album::User(i) = a { Some(i) } else { None }).collect::<Vec<_>>();
    assert_eq!(order(&p), [0, 1, 2]);

    // Sort Albums flips between creation order and by name
    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 100.0);
    let (x, y, w, h) = p.extra_row_of(&ui, Target::AlbumSort);
    tap(&mut p, &ui, x + w / 2.0, y + h / 2.0, 200.0);
    assert_eq!(p.prefs.get("album_sort"), Some("name"));
    assert_eq!(order(&p), [1, 2, 0]);

    // the Slideshow row steps the seconds and the show uses them
    assert_eq!(p.show_seconds(), 4);
    p.touch(&ui, Event::Down(x + w / 2.0, y + h / 2.0 + h), 300.0);
    p.touch(&ui, Event::Up(x + w / 2.0, y + h / 2.0 + h), 330.0);
    assert!(p.album_menu, "the picker stays open after the first tap");
    assert_eq!(p.show_seconds(), 6);
    p.start_slideshow(p.photos().to_vec(), 0, 400.0);
    assert!(p.slideshow.is_some());
    p.slideshow = None;
    p.album_menu = false;

    // Delete Album removes the open album; the later one moves down
    p.select_album(&ui, Album::User(1));
    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 500.0);
    let (x, y, w, h) = p.extra_row_of(&ui, Target::DeleteAlbum);
    tap(&mut p, &ui, x + w / 2.0, y + h / 2.0, 600.0);
    assert_eq!(p.album(), Album::All);
    assert_eq!(p.user_albums.len(), 2);
    assert_eq!(p.album_title(Album::User(0)), "Trip");
    assert_eq!(p.album_title(Album::User(1)), "Beach");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scans_into_month_sections_and_starts_at_the_bottom() {
    let (mut p, dir, ui) = photos("sections", 12);
    assert_eq!(p.photos().len(), 12);
    assert_eq!(p.sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["September 2026", "October 2026"]);
    assert_eq!(p.cols(), 5);
    assert_eq!(p.scroll.offset(), 0.0, "twelve photos fit on one screen");
    assert!(p.describe().contains("photos_screen=grid"));
    // thumbnails are decoded for the visible cells and cached on disk
    p.tick(&ui, 0.0);
    pump_until(&mut p, &ui, |p| p.thumbs.len() == 12);
    assert_eq!(p.disk.len(), 12);
    assert!(p.thumb_pending.is_empty());
    let buf = frame(&mut p, &ui);
    let (x, y, cw, ch) = p.grid.cell_rect(0);
    let px = buf[(content_top(&ui) + y + ch / 2.0) as usize * 1200 + (x + cw / 2.0) as usize];
    assert_eq!(px, pack(10, 20, 30, 255), "the thumbnail's colour fills its cell");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thumbnail_requests_cover_the_visible_cells_and_drop_the_rest() {
    let (mut p, dir, ui) = photos("wanted", 12);
    p.tick(&ui, 0.0);
    let visible: HashSet<String> = p.grid.visible(0.0, p.content_bottom(&ui) - content_top(&ui)).into_iter().map(|i| p.photos()[i].key()).collect();
    assert_eq!(*p.thumb_wanted.lock().unwrap_or_else(|p| p.into_inner()), visible);
    assert_eq!(visible.len(), 12);
    // the caching manager holds one preparation per visible thumbnail, at the grid's thumbnail size
    assert_eq!(p.preheat.len(), 12);
    assert!(p.preheat.is_started(&PreKey::new(&p.photos()[0].key(), thumbs::SIZE as u32, thumbs::SIZE as u32)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn revert_to_original_drops_the_saved_edit_and_never_touches_the_file() {
    let (mut p, dir, ui) = photos("revert", 1);
    let original = dir.join("Pictures/IMG_0000.png");
    p.open_viewer(0);
    frame(&mut p, &ui);
    let photo = p.photos()[0].clone();
    p.set_recipe(&photo, Edit { turns: 1, ..Edit::default() });
    assert!(p.recipe(&photo).is_some());
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
    assert_eq!(p.sheet, Some(Sheet::Info));
    assert!(p.info_revert, "an edited photo offers Revert to Original");
    // the Revert row sits under the detail rows, the rating, Hide Photo, Edit Details and Look Up rows
    let (sx, sy, sw, _) = p.sheet_rect(&ui, Sheet::Info);
    let row = ui.pt(INFO_ROW_PT);
    let y = sy + row * (p.info.len() + 1 + 3 + 1) as f32 + row / 2.0;
    tap(&mut p, &ui, sx + sw / 2.0, y, 2000.0);
    assert!(p.recipe(&photo).is_none(), "the saved edit is dropped");
    assert!(original.exists(), "the original is never changed");
    assert_eq!(p.photos().len(), 1, "nothing leaves the library");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_info_sheet_look_up_row_hands_the_open_photo_to_the_shell_once() {
    let (mut p, dir, ui) = photos("lookup", 1);
    p.open_viewer(0);
    frame(&mut p, &ui);
    let path = p.photos()[0].path.clone();
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
    assert_eq!(p.sheet, Some(Sheet::Info));
    assert!(!p.info_revert, "an unedited photo has no Revert row, so Look Up is the last row");
    assert_eq!(p.take_visual_search(), None, "nothing is sent before the row is tapped");
    // the Look Up row sits under the detail rows, the rating, Hide Photo and Edit Details rows
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Info);
    let row = ui.pt(INFO_ROW_PT);
    let y = sy + row * (p.info.len() + 1 + 3) as f32 + row / 2.0;
    assert!(y < sy + sh, "the row is inside the sheet");
    tap(&mut p, &ui, sx + sw / 2.0, y, 2000.0);
    assert_eq!(p.sheet, None, "the sheet closes");
    assert_eq!(p.take_visual_search(), Some(path), "the shell is handed the open photo");
    assert_eq!(p.take_visual_search(), None, "once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_library_opens_scrolled_to_the_newest_row() {
    let (mut p, dir, ui) = photos("long", 60);
    assert!(p.grid.height > 1920.0);
    assert_eq!(p.scroll.offset(), p.grid.height - (p.content_bottom(&ui) - content_top(&ui)));
    // zoom in: three columns, position kept proportionally
    let k = p.scroll.offset() / p.grid.height;
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 100.0);
    assert_eq!(p.cols(), 3);
    assert!((p.scroll.offset() / p.grid.height - k).abs() < 0.05);
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 200.0);
    assert_eq!(p.cols(), 2);
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 250.0);
    assert_eq!(p.cols(), 2, "no further zoom level");
    assert_eq!(p.prefs.get("cols"), Some("2"), "the level is remembered");
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 300.0);
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 350.0);
    assert_eq!(p.cols(), 5);
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 400.0);
    tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 450.0);
    assert_eq!(p.cols(), 8, "more, smaller thumbnails");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tapping_a_cell_opens_the_viewer_and_back_returns() {
    let (mut p, dir, ui) = photos("viewer", 3);
    let (x, y, cw, ch) = p.grid.cell_rect(1);
    tap(&mut p, &ui, x + cw / 2.0, content_top(&ui) + y + ch / 2.0, 1000.0);
    assert_eq!(p.viewing().map(Photo::name).as_deref(), Some("IMG_0001.png"));
    assert!(p.describe().contains("photos_screen=viewer"));
    p.tick(&ui, 1100.0);
    pump_until(&mut p, &ui, |p| p.full.len() == 3);
    let buf = frame(&mut p, &ui);
    assert_eq!(buf[960 * 1200 + 600], pack(10, 20, 30, 255), "the picture is centred");
    assert!(!p.fullscreen());
    // a single tap hides the chrome once the double-tap window passes
    tap(&mut p, &ui, 600.0, 900.0, 2000.0);
    p.tick(&ui, 2400.0);
    assert!(p.fullscreen());
    assert_eq!(p.next_wake(), None);
    tap(&mut p, &ui, 600.0, 900.0, 3000.0);
    p.tick(&ui, 3400.0);
    assert!(!p.fullscreen());
    tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 4000.0);
    assert!(p.viewer.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn favourite_persists_in_the_prefs_file() {
    let (mut p, dir, ui) = photos("fav", 2);
    p.open_viewer(0);
    let photo = p.viewing().cloned().unwrap();
    assert!(!p.is_favourite(&photo));
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 600.0, cy, 1000.0);
    assert!(p.is_favourite(&photo));
    let again = Prefs::load(dir.join(PREFS_FILE));
    assert_eq!(again.get(&format!("fav:{}", photo.path.display())), Some("1"));
    tap(&mut p, &ui, 600.0, cy, 2000.0);
    assert!(!p.is_favourite(&photo));
    frame(&mut p, &ui);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_moves_to_recently_deleted_where_it_can_be_recovered_or_purged() {
    let (mut p, dir, ui) = photos("delete", 3);
    p.open_viewer(2);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.88, cy, 1000.0);
    assert_eq!(p.sheet, Some(Sheet::Delete));
    assert!(p.describe().contains("photos_screen=sheet"));
    frame(&mut p, &ui);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Delete);
    tap(&mut p, &ui, sx + sw / 2.0, sy + 3.0 * sh / 4.0, 1100.0);
    assert_eq!(p.sheet, None, "cancelled");
    assert_eq!(p.photos().len(), 3);
    tap(&mut p, &ui, 1200.0 * 0.88, cy, 1200.0);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 1300.0);
    assert_eq!(p.photos().len(), 2);
    assert_eq!(p.viewing().map(Photo::name).as_deref(), Some("IMG_0001.png"), "the viewer moves to the last remaining");
    assert!(dir.join("Pictures/.trash/IMG_0002.png").exists());
    // Recently Deleted
    p.viewer = None;
    choose_album(&mut p, &ui, Album::Trash, 2000.0);
    assert_eq!(p.album(), Album::Trash);
    assert_eq!(p.photos().len(), 1);
    frame(&mut p, &ui);
    p.open_viewer(0);
    frame(&mut p, &ui);
    tap(&mut p, &ui, 1200.0 * 0.15, cy, 3000.0);
    assert!(dir.join("Pictures/IMG_0002.png").exists(), "recovered");
    assert_eq!((p.all.len(), p.trash.len()), (3, 0));
    assert!(p.viewer.is_none(), "nothing left to view in the trash");
    frame(&mut p, &ui);
    // delete for good
    choose_album(&mut p, &ui, Album::All, 4000.0);
    p.open_viewer(0);
    tap(&mut p, &ui, 1200.0 * 0.88, cy, 5000.0);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 5100.0);
    assert_eq!(p.all.len(), 2, "trashed again");
    p.viewer = None;
    choose_album(&mut p, &ui, Album::Trash, 6000.0);
    assert_eq!(p.album(), Album::Trash);
    p.open_viewer(0);
    tap(&mut p, &ui, 1200.0 * 0.85, cy, 7000.0);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Delete);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 7100.0);
    assert!(p.trash.is_empty() && !dir.join("Pictures/.trash/IMG_0000.png").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_as_wallpaper_packs_the_photo_and_emits_the_effect() {
    let (mut p, dir, ui) = photos("wallpaper", 1);
    p.open_viewer(0);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
    assert_eq!(p.sheet, Some(Sheet::Share));
    frame(&mut p, &ui);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 6.5 / 8.0, 1100.0);
    pump_until(&mut p, &ui, |p| !p.effects.is_empty());
    assert_eq!(p.take_effects(), vec![Effect::Wallpaper("custom-1".into())]);
    let raw = std::fs::read(dir.join("wallpapers/custom-1.raw")).unwrap();
    let img = Image::from_raw(&raw).unwrap();
    assert_eq!((img.w, img.h), (1200, 1920));
    assert_eq!(img.px[0], pack(10, 20, 30, 255));
    assert!(Image::from_raw(&std::fs::read(dir.join("wallpapers/custom-1.thumb.raw")).unwrap()).is_some());
    assert_eq!(Prefs::load(dir.join("wallpapers/index.ini")).get("custom-1"), Some("Photo 1"));
    assert_eq!(p.next_wallpaper_id(), "custom-2");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_copies_the_photo_with_its_edit_and_rating() {
    let (mut p, dir, ui) = photos("duplicate", 1);
    p.open_viewer(0);
    let photo = p.photos()[0].clone();
    p.set_recipe(&photo, Edit { turns: 1, ..Edit::default() });
    p.set_rating(&photo, 4);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
    frame(&mut p, &ui);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 4.5 / 8.0, 1100.0);
    assert_eq!(p.sheet, None);
    pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 2);
    let copy = dir.join("Pictures/IMG_0000 2.png");
    assert!(copy.exists() && dir.join("Pictures/IMG_0000.png").exists());
    let dup = p.all.iter().find(|q| q.path == copy).cloned().expect("the copy is in the library");
    assert_eq!(p.rating(&dup), 4);
    assert_eq!(p.recipe(&dup).map(|e| e.turns), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_info_sheet_rates_with_stars() {
    let (mut p, dir, ui) = photos("rating", 1);
    p.open_viewer(0);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
    frame(&mut p, &ui);
    let photo = p.photos()[0].clone();
    assert_eq!(p.rating(&photo), 0);
    let ((rx, ry, _, rh), (x0, x1)) = p.info_rating_row(&ui);
    let star = |n: f32| x0 + (x1 - x0) * (n - 0.5) / 5.0;
    tap(&mut p, &ui, star(4.0), ry + rh / 2.0, 1100.0);
    assert_eq!(p.sheet, Some(Sheet::Info), "the sheet stays open");
    assert_eq!(p.rating(&photo), 4);
    frame(&mut p, &ui);
    tap(&mut p, &ui, star(4.0), ry + rh / 2.0, 1200.0);
    assert_eq!(p.rating(&photo), 0, "the same star clears it");
    tap(&mut p, &ui, star(1.0), ry + rh / 2.0, 1300.0);
    assert_eq!(p.rating(&photo), 1);
    assert!(rx < x0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hiding_a_photo_moves_it_from_all_photos_to_the_hidden_album() {
    let (mut p, dir, ui) = photos("hidden", 3);
    p.open_viewer(0);
    let photo = p.photos()[0].clone();
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
    frame(&mut p, &ui);
    let (hx, hy, hw, hh) = p.info_hide_row(&ui);
    tap(&mut p, &ui, hx + hw / 2.0, hy + hh / 2.0, 1100.0);
    assert!(p.is_hidden(&photo));
    assert_eq!((p.visible.len(), p.hidden.len(), p.all.len()), (2, 1, 3));
    assert!(!p.album_list(Album::All).iter().any(|q| q.path == photo.path));
    assert!(p.menu_albums().contains(&Album::Hidden));
    assert!(!p.favorites.iter().chain(&p.recents).any(|q| q.path == photo.path));
    p.viewer = None;
    p.select_album(&ui, Album::Hidden);
    assert_eq!(p.photos().len(), 1);
    p.open_viewer(0);
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1200.0);
    frame(&mut p, &ui);
    let (hx, hy, hw, hh) = p.info_hide_row(&ui);
    tap(&mut p, &ui, hx + hw / 2.0, hy + hh / 2.0, 1300.0);
    assert_eq!((p.visible.len(), p.hidden.len()), (3, 0));
    assert_eq!(p.album(), Album::All, "an empty Hidden album closes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_nearby_hands_the_photo_path_to_the_shell() {
    let (mut p, dir, ui) = photos("nearby", 1);
    p.open_viewer(0);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
    frame(&mut p, &ui);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 3.5 / 8.0, 1100.0);
    assert_eq!(p.sheet, None);
    assert_eq!(p.take_share(), p.photos().first().map(|q| q.path.clone()));
    assert_eq!(p.take_share(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn share_sheet_slideshow_plays_and_pauses() {
    let (mut p, dir, ui) = photos("slideshow", 2);
    p.open_viewer(0);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
    frame(&mut p, &ui);
    let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
    tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 16.0, 1100.0);
    assert!(p.slideshow.is_some());
    p.activate(&ui, Target::ShowPlay, 1200.0);
    assert!(p.slideshow.as_ref().unwrap().paused);
    p.activate(&ui, Target::ShowDone, 1300.0);
    assert!(p.slideshow.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_screenshot_shows_at_once_and_opens_by_path() {
    let (mut p, dir, ui) = photos("shot", 2);
    let path = dir.join("Pictures/Screenshot-20261004-002315.png");
    let shot = Photo { path: path.clone(), mtime: 1_800_000_000, size: 9 };
    let img = Image { w: 1200, h: 1920, px: vec![pack(1, 2, 3, 255); 1200 * 1920] };
    p.add_photo(&ui, shot, img);
    assert_eq!(p.photos().len(), 3);
    assert!(p.thumbs.contains(&p.photos()[2].key()) && p.full.len() == 1);
    p.rescan();
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.photos().len(), 3, "a scan that cannot see the file yet keeps it");
    assert!(p.show(&ui, &path));
    assert_eq!(p.viewing().map(|q| q.path.clone()), Some(path));
    let buf = frame(&mut p, &ui);
    assert_eq!(buf[960 * 1200 + 600], pack(1, 2, 3, 255));
    assert!(!p.show(&ui, Path::new("/nowhere.png")));
    let empty = Photos::new(&ui, 1200, 1920, dir.join("empty"), vec![dir.join("empty/Pictures")], 0).unwrap();
    let mut empty = empty;
    pump_until(&mut empty, &ui, |p| !p.scanning);
    frame(&mut empty, &ui);
    let _ = std::fs::remove_dir_all(&dir);
}

fn select_button(ui: &Ui) -> f32 {
    1200.0 - ui.pt(MARGIN_PT + 96.0 + 38.0)
}

fn cell_centre(p: &Photos, ui: &Ui, i: usize) -> (f32, f32) {
    let (x, y, cw, ch) = p.grid.cell_rect(i);
    (x + cw / 2.0, content_top(ui) + y - p.scroll.offset() + ch / 2.0)
}

#[test]
fn screenshots_and_recents_are_albums_of_their_own() {
    let (mut p, dir, ui) = photos("albums", 3);
    picture(&dir.join("Pictures/Screenshot-20261004-002315.png"), 1_791_000_000);
    p.rescan();
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.album_list(Album::All).len(), 4);
    choose_album(&mut p, &ui, Album::Screenshots, 1000.0);
    assert_eq!(p.photos().iter().map(Photo::name).collect::<Vec<_>>(), ["Screenshot-20261004-002315.png"]);
    frame(&mut p, &ui);
    choose_album(&mut p, &ui, Album::Recents, 2000.0);
    assert!(p.photos().len() <= 4, "only what is within 30 days of now");
    choose_album(&mut p, &ui, Album::All, 3000.0);
    assert_eq!(p.photos().len(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_title_cycles_through_months_and_dates_can_be_typed() {
    let (mut p, dir, ui) = photos("filter", 12);
    assert_eq!(p.photos().len(), 12);
    tap(&mut p, &ui, 600.0, nav_mid(&ui), 1000.0);
    assert_eq!(p.filter.as_ref().map(|f| f.0.as_str()), Some("October 2026"));
    assert_eq!(p.photos().len(), 2);
    assert!(p.describe().contains("photos_filter=October 2026"));
    frame(&mut p, &ui);
    tap(&mut p, &ui, 600.0, nav_mid(&ui), 2000.0);
    assert_eq!(p.photos().len(), 10, "September");
    tap(&mut p, &ui, 600.0, nav_mid(&ui), 3000.0);
    assert!(p.filter.is_none());
    assert_eq!(p.photos().len(), 12);
    assert!(p.set_date_query(&ui, "2026-09-25"));
    assert_eq!(p.photos().len(), 1);
    assert!(!p.set_date_query(&ui, "not a date"));
    assert_eq!(p.photos().len(), 1, "a bad query leaves the filter");
    p.clear_filter(&ui);
    assert_eq!(p.photos().len(), 12);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_select_deletes_to_recently_deleted_with_a_purge_clock() {
    let (mut p, dir, ui) = photos("select", 4);
    tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1000.0);
    assert!(p.selection.active);
    let (x, y) = cell_centre(&p, &ui, 0);
    tap(&mut p, &ui, x, y, 1100.0);
    assert!(p.viewer.is_none(), "taps select while selecting");
    let (x, y) = cell_centre(&p, &ui, 2);
    tap(&mut p, &ui, x, y, 1200.0);
    assert_eq!(p.selection.len(), 2);
    assert!(p.describe().contains("photos_selected=2"));
    frame(&mut p, &ui);
    let bar_y = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.88, bar_y, 1300.0);
    pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 2);
    assert_eq!(p.trash.len(), 2);
    assert!(!p.selection.active);
    assert!(dir.join("Pictures/.trash/IMG_0000.png").exists() && dir.join("Pictures/.trash/IMG_0002.png").exists());
    let t = p.trash[0].clone();
    assert!(p.deleted_at(&t).is_some_and(|at| (unix_now() - at).abs() < 60));
    assert_eq!(p.trash_note(&t), "30 days left");
    // an old deletion is purged at the next scan; a recent one stays
    let old = p.trash[0].clone();
    p.pref_set(&format!("del:{}", old.path.display()), &(unix_now() - 31 * 86_400).to_string());
    p.rescan();
    pump_until(&mut p, &ui, |p| !p.scanning);
    assert_eq!(p.trash.len(), 1);
    assert!(!old.path.exists());
    // Recently Deleted: select and recover
    choose_album(&mut p, &ui, Album::Trash, 5000.0);
    tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 5100.0);
    let (x, y) = cell_centre(&p, &ui, 0);
    tap(&mut p, &ui, x, y, 5200.0);
    frame(&mut p, &ui);
    tap(&mut p, &ui, 1200.0 * 0.15, bar_y, 5300.0);
    pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 3);
    assert!(p.trash.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tapping_the_selection_title_selects_and_deselects_everything() {
    let (mut p, dir, ui) = photos("selectall", 4);
    tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1000.0);
    tap(&mut p, &ui, 600.0, nav_mid(&ui), 1100.0);
    assert_eq!(p.selection.len(), 4);
    tap(&mut p, &ui, 600.0, nav_mid(&ui), 1200.0);
    assert!(p.selection.is_empty() && p.selection.active, "Deselect All keeps selecting");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn permanent_multi_delete_asks_twice() {
    let (mut p, dir, ui) = photos("purge2", 2);
    p.delete(&ui, &p.photos()[0].clone());
    pump_until(&mut p, &ui, |p| !p.scanning && p.trash.len() == 1);
    choose_album(&mut p, &ui, Album::Trash, 1000.0);
    tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1100.0);
    let (x, y) = cell_centre(&p, &ui, 0);
    tap(&mut p, &ui, x, y, 1200.0);
    let bar_y = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.8, bar_y, 1300.0);
    assert!(p.confirm_purge && p.trash.len() == 1, "the first tap only asks");
    frame(&mut p, &ui);
    tap(&mut p, &ui, 1200.0 * 0.8, bar_y, 1400.0);
    pump_until(&mut p, &ui, |p| !p.scanning && p.trash.is_empty());
    assert!(!dir.join("Pictures/.trash/IMG_0000.png").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_info_sheet_lists_the_file_details_and_closes_on_a_tap() {
    let (mut p, dir, ui) = photos("info", 1);
    p.open_viewer(0);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
    assert_eq!(p.sheet, Some(Sheet::Info));
    let labels: Vec<&str> = p.info.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Name", "Date", "Size", "File Size"], "PNG dimensions are available without EXIF");
    assert_eq!(p.info[0].1, "IMG_0000.png");
    frame(&mut p, &ui);
    tap(&mut p, &ui, 100.0, 100.0, 2000.0);
    assert_eq!(p.sheet, None);
    assert!(p.viewer.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_editor_previews_edits_and_saves_a_recipe() {
    let (mut p, dir, ui) = photos("editor", 1);
    p.open_viewer(0);
    p.tick(&ui, 1000.0);
    pump_until(&mut p, &ui, |p| p.full.len() == 1);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.69, cy, 2000.0);
    assert!(p.editor.is_some() && p.describe().contains("photos_screen=editor"));
    frame(&mut p, &ui);
    let row = ui.pt(EDIT_ROW_PT);
    let top = p.edit_panel_top(&ui);
    let col = 1200.0 / 7.0;
    // 16:9 is the sixth preset: the 4x4 picture crops to 4x2
    tap(&mut p, &ui, col * 6.5, top + row / 2.0, 2100.0);
    assert_eq!(p.editor.as_ref().map(|e| (e.preview.w, e.preview.h)), Some((4, 2)));
    tap(&mut p, &ui, col / 2.0, top + row / 2.0, 2200.0);
    assert_eq!(p.editor.as_ref().map(|e| e.edit.turns), Some(1));
    assert_eq!(p.editor.as_ref().map(|e| (e.preview.w, e.preview.h)), Some((4, 2)), "a square rotates into itself and keeps the 16:9 crop");
    // brightness slider: the right end is +1
    let (_, x1) = p.slider_track(&ui);
    tap(&mut p, &ui, x1, top + row * 3.5, 2300.0);
    assert_eq!(p.editor.as_ref().map(|e| e.edit.brightness), Some(1.0));
    // the Effects tab: its first slider is blur, and the right end is the full radius
    tap(&mut p, &ui, 1200.0 / 4.0 * 1.5, top + row * 2.5, 2350.0);
    assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Effects));
    tap(&mut p, &ui, x1, top + row * 3.5, 2360.0);
    assert_eq!(p.editor.as_ref().map(|e| (e.edit.blur, e.edit.brightness)), Some((1.0, 1.0)));
    frame(&mut p, &ui);
    // Save
    tap(&mut p, &ui, 1100.0, ui.pt(STATUS_PT) + 5.0, 2400.0);
    assert!(p.editor.is_none());
    let photo = p.photos()[0].clone();
    assert!(p.recipe(&photo).is_some_and(|e| e.turns == 1 && e.aspect == Aspect::R16x9), "the edit is saved as a recipe");
    assert!(dir.join("Pictures/IMG_0000.png").exists(), "the original stays");
    assert_eq!(p.all.len(), 1, "no copy is written");
    // Cancel leaves nothing behind
    p.open_viewer(0);
    p.tick(&ui, 3000.0);
    pump_until(&mut p, &ui, |p| p.full.len() >= 1);
    tap(&mut p, &ui, 1200.0 * 0.69, cy, 3100.0);
    tap(&mut p, &ui, 100.0, ui.pt(STATUS_PT) + 5.0, 3200.0);
    assert!(p.editor.is_none() && p.viewer.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tools_page_flips_rotates_and_enhances() {
    let (mut p, dir, ui) = photos("tools", 1);
    p.open_viewer(0);
    p.tick(&ui, 1000.0);
    pump_until(&mut p, &ui, |p| p.full.len() == 1);
    let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
    tap(&mut p, &ui, 1200.0 * 0.69, cy, 2000.0);
    let (row, top) = (ui.pt(EDIT_ROW_PT), p.edit_panel_top(&ui));
    tap(&mut p, &ui, 1200.0 / 4.0 * 3.5, top + row * 2.5, 2100.0);
    assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Tools));
    tap(&mut p, &ui, 1200.0 / 6.0, top + row * 3.5, 2200.0);
    assert_eq!(p.editor.as_ref().map(|e| e.edit.turns), Some(3), "Rotate Left is a quarter back");
    tap(&mut p, &ui, 600.0, top + row * 3.5, 2300.0);
    tap(&mut p, &ui, 1000.0, top + row * 3.5, 2400.0);
    tap(&mut p, &ui, 600.0, top + row * 4.5, 2500.0);
    let e = p.editor.as_ref().map(|e| e.edit).unwrap();
    assert!(e.flip_h && e.flip_v && e.auto);
    tap(&mut p, &ui, 600.0, top + row * 3.5, 2600.0);
    assert_eq!(p.editor.as_ref().map(|e| e.edit.flip_h), Some(false), "a second tap turns the flip off");
    assert!(p.editor.as_ref().is_some_and(|e| e.hist.iter().sum::<u32>() > 0), "the histogram follows the preview");
    frame(&mut p, &ui);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn histogram_columns_scale_to_the_tallest() {
    let mut bins = [0u32; edit::HISTOGRAM_BINS];
    bins[0] = 10;
    bins[255] = 5;
    let cols = histogram_columns(&bins, 128);
    assert_eq!((cols.len(), cols[0], cols[127]), (128, 1.0, 0.5));
    assert!(histogram_columns(&bins, 0).is_empty());
    assert_eq!(histogram_columns(&bins, 256).len(), 256);
    assert_eq!(histogram_columns(&[0; edit::HISTOGRAM_BINS], 4), vec![0.0; 4]);
}

#[test]
fn at_most_three_full_size_pictures_stay_decoded() {
    let (mut p, dir, ui) = photos("budget", 7);
    p.open_viewer(2);
    p.tick(&ui, 1000.0);
    pump_until(&mut p, &ui, |p| p.full.len() == 3);
    p.open_viewer(5);
    p.tick(&ui, 2000.0);
    let last = p.photos()[6].key();
    pump_until(&mut p, &ui, |p| p.full.contains(&last));
    assert_eq!(p.full.len(), 3, "the oldest were evicted");
    let _ = std::fs::remove_dir_all(&dir);
}
