//! Per-item tags and last-used dates that a file-manager extension keeps with each file: the tag data
//! (`tagDataForItem(with:)`, `setTagData(_:forItemWith:completion:)`), the last-used date (`lastUsedDateForItem(with:)`,
//! `setLastUsedDate(_:forItemWith:completion:)`) and the attribute read (`values(forAttributes:forItemWith:completion:)`).
//! Pure model.
//!
//! Tags are labels: each is trimmed, non-empty, at most [`MAX_TAG_LEN`] characters, and listed once, in the order given.
//! A file holds at most [`MAX_TAGS`] tags. A last-used date is seconds since the Unix epoch. Items are named by absolute
//! paths without a trailing slash (the root is `/`). An item with no tags and no date is forgotten.

use std::collections::BTreeMap;

/// Longest tag label, in characters, after trimming.
pub const MAX_TAG_LEN: usize = 64;
/// Most tags one item may hold.
pub const MAX_TAGS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrError {
    /// The path is empty, not absolute, or ends in a slash (other than the root).
    BadPath,
    /// A tag is blank or longer than [`MAX_TAG_LEN`].
    BadTag,
    /// More than [`MAX_TAGS`] distinct tags were given.
    TooManyTags,
}

/// The attributes an item can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribute {
    Tags,
    LastUsed,
}

/// A value read back for an attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Tags(Vec<String>),
    /// Seconds since the Unix epoch.
    LastUsed(i64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Item {
    tags: Vec<String>,
    last_used: Option<i64>,
}

/// The attributes of every item an extension has set.
#[derive(Clone, Debug, Default)]
pub struct Store {
    items: BTreeMap<String, Item>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the tags of `path`. An empty list clears them. Blank or over-long labels, and more than [`MAX_TAGS`]
    /// distinct labels, are refused and leave the stored tags unchanged.
    pub fn set_tags(&mut self, path: &str, tags: &[&str]) -> Result<(), AttrError> {
        check_path(path)?;
        let mut clean: Vec<String> = Vec::new();
        for raw in tags {
            let label = raw.trim();
            if label.is_empty() || label.chars().count() > MAX_TAG_LEN {
                return Err(AttrError::BadTag);
            }
            if !clean.iter().any(|t| t.as_str() == label) {
                clean.push(label.to_string());
            }
        }
        if clean.len() > MAX_TAGS {
            return Err(AttrError::TooManyTags);
        }
        self.items.entry(path.to_string()).or_default().tags = clean;
        self.prune(path);
        Ok(())
    }

    /// The tags of `path` in the order they were set. Empty when the item has none or the path is malformed.
    pub fn tags(&self, path: &str) -> &[String] {
        match self.items.get(path) {
            Some(item) => item.tags.as_slice(),
            None => &[],
        }
    }

    /// Sets the last-used date of `path`, as seconds since the Unix epoch.
    pub fn set_last_used(&mut self, path: &str, unix_secs: i64) -> Result<(), AttrError> {
        check_path(path)?;
        self.items.entry(path.to_string()).or_default().last_used = Some(unix_secs);
        Ok(())
    }

    /// The last-used date of `path`, if one was set.
    pub fn last_used(&self, path: &str) -> Option<i64> {
        self.items.get(path)?.last_used
    }

    /// Clears one attribute of `path`. `true` when it was set.
    pub fn remove(&mut self, path: &str, attr: Attribute) -> bool {
        let Some(item) = self.items.get_mut(path) else { return false };
        let was_set = match attr {
            Attribute::Tags => !item.tags.is_empty() && { item.tags.clear(); true },
            Attribute::LastUsed => item.last_used.take().is_some(),
        };
        self.prune(path);
        was_set
    }

    /// The attributes of `path` that are set, in the order asked. Each attribute appears once, even if it is asked for
    /// twice, and an attribute that is not set is left out.
    pub fn values(&self, path: &str, attrs: &[Attribute]) -> Vec<(Attribute, Value)> {
        let Some(item) = self.items.get(path) else { return Vec::new() };
        let mut out: Vec<(Attribute, Value)> = Vec::new();
        for &attr in attrs {
            if out.iter().any(|(a, _)| *a == attr) {
                continue;
            }
            match attr {
                Attribute::Tags if !item.tags.is_empty() => out.push((attr, Value::Tags(item.tags.clone()))),
                Attribute::LastUsed => {
                    if let Some(secs) = item.last_used {
                        out.push((attr, Value::LastUsed(secs)));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Drops an item that has neither tags nor a date, so the store does not grow with empty entries.
    fn prune(&mut self, path: &str) {
        if self.items.get(path).map_or(false, |i| i.tags.is_empty() && i.last_used.is_none()) {
            self.items.remove(path);
        }
    }
}

fn check_path(path: &str) -> Result<(), AttrError> {
    if path.starts_with('/') && (path == "/" || !path.ends_with('/')) {
        Ok(())
    } else {
        Err(AttrError::BadPath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_trimmed_listed_once_and_kept_in_order() {
        let mut s = Store::new();
        s.set_tags("/a", &["  red ", "blue", "red", "green"]).unwrap();
        assert_eq!(s.tags("/a"), &["red".to_string(), "blue".to_string(), "green".to_string()][..]);
    }

    #[test]
    fn a_bad_tag_leaves_the_stored_tags_alone() {
        let mut s = Store::new();
        s.set_tags("/a", &["keep"]).unwrap();
        assert_eq!(s.set_tags("/a", &["ok", "   "]), Err(AttrError::BadTag));
        let long = "x".repeat(MAX_TAG_LEN + 1);
        assert_eq!(s.set_tags("/a", &[long.as_str()]), Err(AttrError::BadTag));
        assert_eq!(s.tags("/a"), &["keep".to_string()][..]);
    }

    #[test]
    fn an_item_holds_at_most_max_tags_distinct_labels() {
        let mut s = Store::new();
        let names: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        assert_eq!(s.set_tags("/a", &refs), Err(AttrError::TooManyTags));
        assert!(s.tags("/a").is_empty());
        assert_eq!(s.set_tags("/a", &refs[..MAX_TAGS]), Ok(()));
        assert_eq!(s.tags("/a").len(), MAX_TAGS);
    }

    #[test]
    fn paths_must_be_absolute_without_a_trailing_slash() {
        let mut s = Store::new();
        assert_eq!(s.set_tags("docs", &["x"]), Err(AttrError::BadPath));
        assert_eq!(s.set_last_used("", 5), Err(AttrError::BadPath));
        assert_eq!(s.set_last_used("/docs/", 5), Err(AttrError::BadPath));
        assert_eq!(s.set_last_used("/", 5), Ok(()));
        assert_eq!(s.last_used("/"), Some(5));
    }

    #[test]
    fn last_used_round_trips_and_can_be_removed() {
        let mut s = Store::new();
        s.set_last_used("/a/b", 1_700_000_000).unwrap();
        assert_eq!(s.last_used("/a/b"), Some(1_700_000_000));
        assert!(s.remove("/a/b", Attribute::LastUsed));
        assert_eq!(s.last_used("/a/b"), None);
        assert!(!s.remove("/a/b", Attribute::LastUsed));
    }

    #[test]
    fn values_return_only_what_is_set_in_the_order_asked() {
        let mut s = Store::new();
        s.set_tags("/f", &["work"]).unwrap();
        assert_eq!(
            s.values("/f", &[Attribute::LastUsed, Attribute::Tags]),
            vec![(Attribute::Tags, Value::Tags(vec!["work".to_string()]))]
        );
        s.set_last_used("/f", 42).unwrap();
        assert_eq!(
            s.values("/f", &[Attribute::LastUsed, Attribute::Tags, Attribute::LastUsed]),
            vec![(Attribute::LastUsed, Value::LastUsed(42)), (Attribute::Tags, Value::Tags(vec!["work".to_string()]))]
        );
        assert!(s.values("/missing", &[Attribute::Tags]).is_empty());
    }

    #[test]
    fn an_item_with_no_attributes_is_forgotten() {
        let mut s = Store::new();
        s.set_tags("/g", &["x"]).unwrap();
        s.set_last_used("/g", 9).unwrap();
        assert!(s.remove("/g", Attribute::Tags));
        assert_eq!(s.last_used("/g"), Some(9));
        assert!(s.remove("/g", Attribute::LastUsed));
        assert!(s.items.is_empty());
        assert!(!s.remove("/g", Attribute::Tags));
    }
}
