//! The breadcrumb path bar: the trail from a storage root to the current
//! folder, fitted to the bar's width (older parts collapse into a leading
//! ellipsis) and the list a long press on a crumb offers for jumping.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub label: String,
    pub path: PathBuf,
}

/// Crumbs from the root (labelled `root_name`) down to `current`. A path
/// outside the root yields just the root.
pub fn crumbs(root_name: &str, root: &Path, current: &Path) -> Vec<Crumb> {
    let mut out = vec![Crumb { label: root_name.to_string(), path: root.to_path_buf() }];
    if let Ok(rest) = current.strip_prefix(root) {
        let mut p = root.to_path_buf();
        for part in rest.components() {
            p.push(part);
            out.push(Crumb { label: part.as_os_str().to_string_lossy().into_owned(), path: p.clone() });
        }
    }
    out
}

/// Which crumbs fit: the index of the first visible one, given each label's
/// width, the separator width and the room available. The last crumb always
/// shows; when anything is hidden `ellipsis` points are reserved for the
/// leading "...".
pub fn visible_from(widths: &[f32], sep: f32, ellipsis: f32, avail: f32) -> usize {
    let Some(&last) = widths.last() else { return 0 };
    let mut used = last;
    let mut first = widths.len() - 1;
    while first > 0 {
        let w = widths[first - 1] + sep;
        let reserve = if first - 1 > 0 { ellipsis + sep } else { 0.0 };
        if used + w + reserve > avail {
            break;
        }
        used += w;
        first -= 1;
    }
    first
}

/// What a long press on crumb `index` lists: that folder and its ancestors
/// up to the root, nearest first, as bifrost does.
pub fn jump_targets(trail: &[Crumb], index: usize) -> Vec<Crumb> {
    trail.iter().take(index + 1).rev().cloned().collect()
}

/// Which crumb a tap at `x` hits, given the left edge, label widths and separator width.
pub fn hit(x: f32, left: f32, widths: &[f32], sep: f32) -> Option<usize> {
    let mut at = left;
    for (i, w) in widths.iter().enumerate() {
        if x >= at && x < at + w {
            return Some(i);
        }
        at += w + sep;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_trail() {
        let t = crumbs("On My PC", Path::new("/m"), Path::new("/m/Photos/Trips"));
        assert_eq!(t.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["On My PC", "Photos", "Trips"]);
        assert_eq!(t[2].path, PathBuf::from("/m/Photos/Trips"));
        assert_eq!(crumbs("R", Path::new("/m"), Path::new("/m")).len(), 1);
        assert_eq!(crumbs("R", Path::new("/m"), Path::new("/other/x")).len(), 1);
    }

    #[test]
    fn collapses_old_parts_when_narrow() {
        let w = [100.0, 80.0, 80.0, 80.0];
        assert_eq!(visible_from(&w, 10.0, 20.0, 1000.0), 0);
        // last (80) + 90 = 170; next needs 90 + ellipsis 30 -> 290
        assert_eq!(visible_from(&w, 10.0, 20.0, 300.0), 1);
        assert_eq!(visible_from(&w, 10.0, 20.0, 200.0), 2);
        assert_eq!(visible_from(&w, 10.0, 20.0, 10.0), 3);
        assert_eq!(visible_from(&[], 10.0, 20.0, 10.0), 0);
    }

    #[test]
    fn long_press_lists_ancestors_nearest_first() {
        let t = crumbs("Root", Path::new("/m"), Path::new("/m/a/b/c"));
        let j = jump_targets(&t, 2);
        assert_eq!(j.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["b", "a", "Root"]);
        assert_eq!(jump_targets(&t, 0).len(), 1);
    }

    #[test]
    fn taps_find_their_crumb() {
        let w = [50.0, 40.0];
        assert_eq!(hit(10.0, 0.0, &w, 10.0), Some(0));
        assert_eq!(hit(55.0, 0.0, &w, 10.0), None);
        assert_eq!(hit(70.0, 0.0, &w, 10.0), Some(1));
        assert_eq!(hit(500.0, 0.0, &w, 10.0), None);
    }
}
