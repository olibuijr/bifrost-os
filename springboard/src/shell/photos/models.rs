//! Photo library models the Photos app reads its albums and its order through. Favourites, Recents and Screenshots are
//! the smart album rules of `bifrost_ui::photo_smart_albums`; the sort by date is the fetch model of
//! `bifrost_ui::photo_fetch`. This file only hands the models ids, names and times, and maps their answers back to the
//! photos. The library itself stays in `library.rs`.

use super::library::Photo;
use bifrost_ui::photo_fetch::{self, FetchOptions, Item, Sort};
use bifrost_ui::photo_media::Kind;
use bifrost_ui::photo_smart_albums::{self, Asset, SmartAlbum};

/// The model id of the photo at `index`. Zero-padded, so items with the same time sort by library order.
fn id(index: usize) -> String {
    format!("{index:08}")
}

/// The photo index a model id stands for.
fn index_of(id: &str) -> Option<usize> {
    id.parse().ok()
}

/// A photo's creation time for the models: Unix seconds, never negative.
fn created(p: &Photo) -> u64 {
    p.mtime.max(0) as u64
}

/// The fetch sort for the grid: newest first, or oldest first (the library's own order).
pub fn sort_for(newest_first: bool) -> Sort {
    if newest_first {
        Sort::NewestFirst
    } else {
        Sort::OldestFirst
    }
}

/// `photos` ordered by capture time through the fetch model. Items with the same time keep the order they had.
pub fn by_date(photos: &[Photo], newest_first: bool) -> Vec<Photo> {
    // The ordering reads only the times; the favourite and trash marks do not change it.
    let items: Vec<Item> = photos.iter().enumerate().map(|(i, p)| Item { id: id(i), kind: Kind::Image, favourite: false, trashed: false, created: created(p) }).collect();
    let opts = FetchOptions { sort: sort_for(newest_first), include_trashed: true, ..FetchOptions::default() };
    photo_fetch::fetch(&items, &opts).into_iter().filter_map(|it| index_of(&it.id)).map(|i| photos[i].clone()).collect()
}

/// The photos that belong to a smart album at `now`, in library order. `is_favourite` gives each photo's favourite mark.
/// Every photo in the list is in the library, so none is trashed here (the trash has its own list).
pub fn smart_album(album: SmartAlbum, photos: &[Photo], is_favourite: impl Fn(&Photo) -> bool, now: i64) -> Vec<Photo> {
    let assets: Vec<Asset> = photos
        .iter()
        .enumerate()
        .map(|(i, p)| Asset { id: id(i), name: p.name(), favourite: is_favourite(p), trashed: false, created: created(p), deleted_at: None })
        .collect();
    photo_smart_albums::members(album, &assets, now.max(0) as u64).into_iter().filter_map(|a| index_of(&a.id)).map(|i| photos[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const DAY: i64 = 86_400;
    // "Now" is day 100 after the epoch.
    const NOW: i64 = 100 * DAY;

    fn photo(path: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(path), mtime, size: 0 }
    }

    fn library() -> Vec<Photo> {
        vec![
            photo("/p/Screenshot_1.png", 99 * DAY),
            photo("/p/IMG_2.jpg", 60 * DAY),
            photo("/p/IMG_3.jpg", 100 * DAY + 5),
            photo("/p/IMG_4.jpg", 70 * DAY),
            photo("/p/screenshot_old.png", 10 * DAY),
        ]
    }

    fn names(list: &[Photo]) -> Vec<String> {
        list.iter().map(|p| p.name()).collect()
    }

    #[test]
    fn oldest_first_is_the_library_order_by_time() {
        // times in days: 99, 60, 100 (+5 s), 70, 10 -> oldest first: 10, 60, 70, 99, 100
        let got = by_date(&library(), false);
        assert_eq!(names(&got), vec!["screenshot_old.png", "IMG_2.jpg", "IMG_4.jpg", "Screenshot_1.png", "IMG_3.jpg"]);
    }

    #[test]
    fn newest_first_reverses_the_times() {
        let got = by_date(&library(), true);
        assert_eq!(names(&got), vec!["IMG_3.jpg", "Screenshot_1.png", "IMG_4.jpg", "IMG_2.jpg", "screenshot_old.png"]);
    }

    #[test]
    fn items_with_the_same_time_keep_their_library_order_in_both_directions() {
        let tied = vec![photo("/p/y.jpg", 5), photo("/p/x.jpg", 5)];
        assert_eq!(names(&by_date(&tied, false)), vec!["y.jpg", "x.jpg"]);
        assert_eq!(names(&by_date(&tied, true)), vec!["y.jpg", "x.jpg"]);
        assert!(by_date(&[], true).is_empty());
    }

    #[test]
    fn screenshots_match_the_name_rule() {
        let got = smart_album(SmartAlbum::Screenshots, &library(), |_| false, NOW);
        assert_eq!(names(&got), vec!["Screenshot_1.png", "screenshot_old.png"]);
    }

    #[test]
    fn recents_are_the_last_thirty_days_up_to_a_day_ahead() {
        // day 99 and the day 100 + 5 s picture are inside; day 70 is exactly 30 days old, so out; day 60 and 10 are out.
        let got = smart_album(SmartAlbum::RecentlyAdded, &library(), |_| false, NOW);
        assert_eq!(names(&got), vec!["Screenshot_1.png", "IMG_3.jpg"]);
    }

    #[test]
    fn favourites_follow_the_mark_the_shell_passes_in() {
        let got = smart_album(SmartAlbum::Favourites, &library(), |p| p.name() == "IMG_2.jpg", NOW);
        assert_eq!(names(&got), vec!["IMG_2.jpg"]);
        assert!(smart_album(SmartAlbum::Favourites, &[], |_| true, NOW).is_empty());
    }

    #[test]
    fn a_negative_time_counts_as_the_epoch() {
        assert_eq!(created(&photo("/p/a.jpg", -4)), 0);
    }
}
