//! Duplicates: pictures whose files are the same bytes (found by `library::duplicate_groups`). The Duplicates album
//! lists them; Merge keeps the best copy of each group, folds the favourite mark, the rating and the keywords of the
//! others into it, and moves the rest to Recently Deleted.

use std::collections::HashSet;
use std::path::PathBuf;

/// What decides which copy of a group is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub taken: i64,
    pub favourite: bool,
    pub rating: u8,
    pub edited: bool,
}

/// The outcome for one group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub keep: PathBuf,
    pub drop: Vec<PathBuf>,
    /// any copy was a favourite
    pub favourite: bool,
    /// the best rating of any copy
    pub rating: u8,
}

/// The groups restricted to the pictures in `present`; a group with fewer than two left is not a group any more.
pub fn members(groups: &[Vec<PathBuf>], present: &HashSet<PathBuf>) -> Vec<Vec<PathBuf>> {
    groups.iter().map(|g| g.iter().filter(|p| present.contains(*p)).cloned().collect::<Vec<_>>()).filter(|g| g.len() > 1).collect()
}

/// Which copy of each group to keep: an edited one, then the best rated, then a favourite, then the oldest, then the
/// shortest path (the one that is not "name 2").
pub fn plan(groups: &[Vec<Candidate>]) -> Vec<Merge> {
    groups
        .iter()
        .filter(|g| g.len() > 1)
        .filter_map(|g| {
            let best = g.iter().max_by(|a, b| {
                (a.edited, a.rating, a.favourite)
                    .cmp(&(b.edited, b.rating, b.favourite))
                    .then_with(|| b.taken.cmp(&a.taken))
                    .then_with(|| b.path.as_os_str().len().cmp(&a.path.as_os_str().len()))
                    .then_with(|| b.path.cmp(&a.path))
            })?;
            Some(Merge {
                keep: best.path.clone(),
                drop: g.iter().filter(|c| c.path != best.path).map(|c| c.path.clone()).collect(),
                favourite: g.iter().any(|c| c.favourite),
                rating: g.iter().map(|c| c.rating).max().unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(path: &str, taken: i64) -> Candidate {
        Candidate { path: PathBuf::from(path), taken, favourite: false, rating: 0, edited: false }
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_group_needs_two_pictures_still_present() {
        let groups = vec![paths(&["/a", "/b", "/c"]), paths(&["/d", "/e"])];
        let present: HashSet<PathBuf> = paths(&["/a", "/b", "/d"]).into_iter().collect();
        assert_eq!(members(&groups, &present), vec![paths(&["/a", "/b"])]);
        assert!(members(&groups, &HashSet::new()).is_empty());
    }

    #[test]
    fn the_oldest_and_shortest_named_copy_is_kept_by_default() {
        let merges = plan(&[vec![cand("/p/IMG 2.png", 200), cand("/p/IMG.png", 200), cand("/p/IMG 3.png", 100)]]);
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].keep, PathBuf::from("/p/IMG 3.png"), "the earliest time wins first");
        assert_eq!(merges[0].drop.len(), 2);
        let same_time = plan(&[vec![cand("/p/IMG 2.png", 5), cand("/p/IMG.png", 5)]]);
        assert_eq!(same_time[0].keep, PathBuf::from("/p/IMG.png"), "then the shorter name");
    }

    #[test]
    fn an_edited_or_better_rated_copy_wins_and_marks_are_folded_in() {
        let mut edited = cand("/p/b.png", 900);
        edited.edited = true;
        let mut starred = cand("/p/c.png", 1);
        starred.rating = 4;
        starred.favourite = true;
        let m = plan(&[vec![cand("/p/a.png", 1), edited.clone(), starred.clone()]]);
        assert_eq!(m[0].keep, edited.path, "an edit is never thrown away");
        assert_eq!((m[0].favourite, m[0].rating), (true, 4));
        let m = plan(&[vec![cand("/p/a.png", 1), starred.clone()]]);
        assert_eq!(m[0].keep, starred.path);
        assert_eq!(m[0].drop, paths(&["/p/a.png"]));
    }

    #[test]
    fn single_pictures_and_no_groups_plan_nothing() {
        assert!(plan(&[]).is_empty());
        assert!(plan(&[vec![cand("/p/a.png", 1)]]).is_empty());
        assert!(plan(&[vec![]]).is_empty());
    }
}
