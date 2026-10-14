//! Memories: the months with enough pictures in them, newest first, offered as albums that can be played as a
//! slideshow. They are worked out from the capture dates each time the library is read; screenshots are left out.

use super::library::{sections, Photo};

/// A month needs at least this many pictures to be a memory.
pub const MIN_PHOTOS: usize = 5;
/// At most this many memories are offered.
pub const MAX_MEMORIES: usize = 6;

/// The title and pictures of the newest `max` months that have at least `min` pictures. `photos` are oldest first (the
/// library order); `offset` is the zone in seconds east of UTC that decides which month a picture belongs to.
pub fn clusters(photos: &[Photo], offset: i32, min: usize, max: usize) -> Vec<(String, Vec<Photo>)> {
    let mut found: Vec<(String, Vec<Photo>)> = sections(photos, offset).into_iter().filter(|s| s.len >= min).map(|s| (s.title, photos[s.start..s.start + s.len].to_vec())).collect();
    found.reverse();
    found.truncate(max);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// `n` pictures a day apart from `start` (Unix seconds).
    fn run(name: &str, start: i64, n: usize) -> Vec<Photo> {
        (0..n).map(|i| Photo { path: PathBuf::from(format!("/p/{name}{i}.png")), mtime: start + i as i64 * 86_400, size: 1 }).collect()
    }

    // 2026-08-01, 2026-09-01 and 2026-10-01 00:00 UTC
    const AUG: i64 = 1_785_542_400;
    const SEP: i64 = 1_788_220_800;
    const OCT: i64 = 1_790_812_800;

    #[test]
    fn busy_months_become_memories_newest_first() {
        let mut photos = run("a", AUG, 3);
        photos.extend(run("s", SEP, 8));
        photos.extend(run("o", OCT, 5));
        let got = clusters(&photos, 0, 5, 6);
        assert_eq!(got.iter().map(|(t, p)| (t.as_str(), p.len())).collect::<Vec<_>>(), [("October 2026", 5), ("September 2026", 8)], "August has too few");
        assert!(got[1].1.windows(2).all(|w| w[0].mtime < w[1].mtime), "the pictures keep their order");
    }

    #[test]
    fn only_the_newest_few_are_offered() {
        let mut photos = Vec::new();
        for m in 0..4 {
            photos.extend(run(&format!("m{m}-"), AUG + m * 31 * 86_400, 6));
        }
        let got = clusters(&photos, 0, 5, 2);
        assert_eq!(got.len(), 2);
        assert!(got[0].1[0].mtime > got[1].1[0].mtime);
        assert!(clusters(&[], 0, 5, 6).is_empty());
        assert!(clusters(&photos, 0, 50, 6).is_empty());
    }

    #[test]
    fn the_zone_decides_which_month_a_late_picture_is_in() {
        // 23:00 on 30 September UTC is already 1 October an hour east
        let late = Photo { path: PathBuf::from("/p/late.png"), mtime: OCT - 3600, size: 1 };
        let mut photos = run("s", SEP, 5);
        photos.push(late.clone());
        let utc = clusters(&photos, 0, 1, 6);
        assert_eq!(utc.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["September 2026"]);
        let east = clusters(&photos, 3600, 1, 6);
        assert_eq!(east.iter().map(|(t, p)| (t.as_str(), p.len())).collect::<Vec<_>>(), [("October 2026", 1), ("September 2026", 5)]);
    }
}
