//! Smart albums of a photo library: the albums the system keeps itself, which an app lists without building them.
//! Membership follows from each asset's own state: its name (screenshots), its creation time (recently added), its
//! favourite mark, and its trash state (recently deleted, purged after `TRASH_DAYS`). Trashed assets are in no album
//! except Recently Deleted. The order of the assets is kept. Pure: the shell keeps the library and passes it in.
//! Mirrors the smart album kinds of the official photo asset collection subtypes (screenshots, recently added,
//! favourites). The recently deleted album has no subtype in the official list and is a BifrOSt album.

/// Recently added keeps assets created within this many days before `now`.
pub const RECENT_DAYS: u64 = 30;
/// A recently deleted asset is purged this many days after it was deleted.
pub const TRASH_DAYS: u64 = 30;
const DAY: u64 = 86_400;

/// The smart albums. Each one is a fixed rule over the library, not a list the person edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartAlbum {
    /// Assets captured with the screenshot command: the name starts with `screenshot`, in any case.
    Screenshots,
    /// Assets created in the last `RECENT_DAYS` days, and not more than a day in the future.
    RecentlyAdded,
    /// Assets marked as favourites.
    Favourites,
    /// Assets in the trash, until they are purged.
    RecentlyDeleted,
}

impl SmartAlbum {
    /// Every smart album, in the order a sidebar lists them.
    pub const ALL: [SmartAlbum; 4] = [SmartAlbum::Screenshots, SmartAlbum::RecentlyAdded, SmartAlbum::Favourites, SmartAlbum::RecentlyDeleted];

    /// The name a sidebar shows for the album.
    pub fn title(self) -> &'static str {
        match self {
            SmartAlbum::Screenshots => "Screenshots",
            SmartAlbum::RecentlyAdded => "Recents",
            SmartAlbum::Favourites => "Favourites",
            SmartAlbum::RecentlyDeleted => "Recently Deleted",
        }
    }
}

/// One asset of the library, with the state the smart album rules read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub favourite: bool,
    pub trashed: bool,
    /// Creation time, seconds since the epoch.
    pub created: u64,
    /// When the asset went to the trash. Only read for trashed assets.
    pub deleted_at: Option<u64>,
}

/// True when a file name marks a screenshot: it starts with `screenshot`, in any case.
pub fn is_screenshot(name: &str) -> bool {
    name.as_bytes().get(..10).is_some_and(|head| head.eq_ignore_ascii_case(b"screenshot"))
}

/// True when `asset` belongs to `album` at time `now`.
pub fn contains(album: SmartAlbum, asset: &Asset, now: u64) -> bool {
    match album {
        SmartAlbum::Screenshots => !asset.trashed && is_screenshot(&asset.name),
        SmartAlbum::RecentlyAdded => !asset.trashed && asset.created.saturating_add(RECENT_DAYS * DAY) > now && asset.created <= now.saturating_add(DAY),
        SmartAlbum::Favourites => !asset.trashed && asset.favourite,
        SmartAlbum::RecentlyDeleted => asset.trashed,
    }
}

/// The assets of `album` at time `now`, in the order they were given.
pub fn members<'a>(album: SmartAlbum, assets: &'a [Asset], now: u64) -> Vec<&'a Asset> {
    assets.iter().filter(|a| contains(album, a, now)).collect()
}

/// True once an asset deleted at `deleted_at` is due for the purge. A deletion time after `now` counts as just deleted.
pub fn purge_due(deleted_at: u64, now: u64) -> bool {
    now.saturating_sub(deleted_at) >= TRASH_DAYS * DAY
}

/// Whole days left before the purge, at least 0. A deletion time after `now` gives the full `TRASH_DAYS`.
pub fn days_left(deleted_at: u64, now: u64) -> u64 {
    let elapsed_days = now.saturating_sub(deleted_at) / DAY;
    TRASH_DAYS.saturating_sub(elapsed_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, name: &str, favourite: bool, trashed: bool, created: u64) -> Asset {
        Asset { id: id.to_string(), name: name.to_string(), favourite, trashed, created, deleted_at: None }
    }

    fn ids(list: &[&Asset]) -> Vec<String> {
        list.iter().map(|a| a.id.clone()).collect()
    }

    // "Now" is day 100 after the epoch, so day arithmetic stays readable.
    const NOW: u64 = 100 * DAY;

    fn library() -> Vec<Asset> {
        let mut trashed = asset("c", "screenshot_old.png", true, true, 10 * DAY);
        trashed.deleted_at = Some(80 * DAY);
        vec![
            asset("a", "Screenshot 1.png", false, false, 99 * DAY),
            asset("b", "IMG_2.jpg", true, false, 60 * DAY),
            trashed,
            asset("d", "IMG_3.jpg", false, false, 100 * DAY + 5),
            // created exactly RECENT_DAYS before now is no longer recent
            asset("e", "IMG_4.jpg", false, false, 70 * DAY),
            asset("f", "IMG_5.jpg", false, false, 71 * DAY),
        ]
    }

    #[test]
    fn screenshots_match_the_name_prefix_in_any_case() {
        assert!(is_screenshot("SCREENSHOT-1.png"));
        assert!(is_screenshot("screenshot_2.png"));
        assert!(!is_screenshot("my screenshot.png"));
        assert!(!is_screenshot("screen.png"));
        assert!(!is_screenshot(""));
        let lib = library();
        // c is a screenshot name but is trashed, so only a is listed.
        assert_eq!(ids(&members(SmartAlbum::Screenshots, &lib, NOW)), vec!["a"]);
    }

    #[test]
    fn recently_added_keeps_thirty_days_and_skips_the_trash() {
        let lib = library();
        // a (day 99) and f (day 71) are inside the window. e is exactly 30 days old, so out. c is trashed.
        assert_eq!(ids(&members(SmartAlbum::RecentlyAdded, &lib, NOW)), vec!["a", "d", "f"]);
    }

    #[test]
    fn recently_added_drops_assets_more_than_a_day_in_the_future() {
        let future = vec![asset("x", "IMG_9.jpg", false, false, NOW + DAY + 1)];
        assert!(members(SmartAlbum::RecentlyAdded, &future, NOW).is_empty());
        let tomorrow = vec![asset("y", "IMG_9.jpg", false, false, NOW + DAY)];
        assert_eq!(ids(&members(SmartAlbum::RecentlyAdded, &tomorrow, NOW)), vec!["y"]);
    }

    #[test]
    fn favourites_exclude_trashed_assets() {
        let lib = library();
        // b is a favourite. c is a favourite too but is in the trash.
        assert_eq!(ids(&members(SmartAlbum::Favourites, &lib, NOW)), vec!["b"]);
    }

    #[test]
    fn recently_deleted_holds_only_trashed_assets() {
        let lib = library();
        assert_eq!(ids(&members(SmartAlbum::RecentlyDeleted, &lib, NOW)), vec!["c"]);
        assert_eq!(SmartAlbum::ALL.len(), 4);
        assert_eq!(SmartAlbum::RecentlyDeleted.title(), "Recently Deleted");
    }

    #[test]
    fn trash_is_purged_thirty_days_after_deletion() {
        // deleted 80 days in, so 20 days have passed at NOW
        assert_eq!(days_left(80 * DAY, NOW), 10);
        assert_eq!(days_left(NOW, NOW), 30);
        // a deletion time after now counts as just deleted
        assert_eq!(days_left(NOW + 5 * DAY, NOW), 30);
        assert_eq!(days_left(0, NOW), 0);
        assert!(purge_due(70 * DAY, NOW), "exactly 30 days is due");
        assert!(!purge_due(71 * DAY, NOW), "29 days is not due");
        assert!(!purge_due(NOW + DAY, NOW));
    }
}
