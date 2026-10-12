//! Badges that a file-manager extension puts on files and folders: the badge images it registers
//! (`setBadgeImage(_:label:forBadgeIdentifier:)`), the badge it assigns to a path (`setBadgeIdentifier(_:for:)`), and the
//! file manager's question for one path (`requestBadgeIdentifier(for:)`). Pure model.
//!
//! An extension registers each badge identifier once, with an image and a label; registering it again replaces both.
//! It then assigns an identifier to an absolute path. Only registered identifiers are accepted, so a path can never point
//! at a badge that has no image. [`BadgeSet::request`] runs the file manager's question for one path: the answer is stored
//! when it names a registered badge, and a path with no answer keeps no badge.

/// Longest badge identifier, in bytes.
pub const MAX_ID_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BadgeError {
    /// The identifier is empty, longer than [`MAX_ID_LEN`], or contains whitespace or `/`.
    BadIdentifier,
    /// The path is empty or not absolute.
    BadPath,
    /// The badge image name is empty.
    EmptyImage,
    /// No image was registered for the identifier.
    UnknownBadge,
}

/// One registered badge: the identifier, the image the file manager draws, and the label of its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub identifier: String,
    pub image: String,
    /// The tooltip label. An empty label means no label.
    pub label: String,
}

/// The badges an extension has registered and the paths it has marked.
#[derive(Clone, Debug, Default)]
pub struct BadgeSet {
    badges: Vec<Badge>,
    /// `(normalized path, badge identifier)`, one entry per marked path.
    assigned: Vec<(String, String)>,
}

impl BadgeSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registered badges, in registration order.
    pub fn badges(&self) -> &[Badge] {
        &self.badges
    }

    /// `setBadgeImage(_:label:forBadgeIdentifier:)`: registers the badge, or replaces its image and label. Paths that
    /// already carry the identifier keep it and show the new image.
    pub fn set_image(&mut self, identifier: &str, image: &str, label: &str) -> Result<(), BadgeError> {
        valid_id(identifier)?;
        if image.trim().is_empty() {
            return Err(BadgeError::EmptyImage);
        }
        let badge = Badge { identifier: identifier.to_string(), image: image.to_string(), label: label.to_string() };
        match self.badges.iter().position(|b| b.identifier == identifier) {
            Some(i) => self.badges[i] = badge,
            None => self.badges.push(badge),
        }
        Ok(())
    }

    /// `setBadgeIdentifier(_:for:)`: marks `path` with a registered badge, replacing any badge it had.
    pub fn set_badge(&mut self, path: &str, identifier: &str) -> Result<(), BadgeError> {
        let key = normalize(path)?;
        if !self.badges.iter().any(|b| b.identifier == identifier) {
            return Err(BadgeError::UnknownBadge);
        }
        match self.assigned.iter().position(|(p, _)| *p == key) {
            Some(i) => self.assigned[i].1 = identifier.to_string(),
            None => self.assigned.push((key, identifier.to_string())),
        }
        Ok(())
    }

    /// Removes the badge from `path`. `true` when it had one.
    pub fn clear(&mut self, path: &str) -> bool {
        let Ok(key) = normalize(path) else { return false };
        let before = self.assigned.len();
        self.assigned.retain(|(p, _)| *p != key);
        self.assigned.len() != before
    }

    /// The badge on `path`, if the path is marked and its badge is still registered.
    pub fn badge_for(&self, path: &str) -> Option<&Badge> {
        let key = normalize(path).ok()?;
        let (_, id) = self.assigned.iter().find(|(p, _)| *p == key)?;
        self.badges.iter().find(|b| b.identifier == *id)
    }

    /// `requestBadgeIdentifier(for:)`: asks `answer` for the badge of `path`. A registered identifier is stored and
    /// `Ok(true)` is returned. `None` clears any badge the path had and returns `Ok(false)`. An identifier that is not
    /// registered is refused with [`BadgeError::UnknownBadge`] and leaves the path as it was.
    pub fn request(&mut self, path: &str, answer: impl FnOnce(&str) -> Option<String>) -> Result<bool, BadgeError> {
        let key = normalize(path)?;
        match answer(key.as_str()) {
            Some(id) => {
                self.set_badge(&key, &id)?;
                Ok(true)
            }
            None => {
                self.clear(&key);
                Ok(false)
            }
        }
    }
}

fn valid_id(id: &str) -> Result<(), BadgeError> {
    if id.is_empty() || id.len() > MAX_ID_LEN || id.chars().any(|c| c.is_whitespace() || c == '/') {
        Err(BadgeError::BadIdentifier)
    } else {
        Ok(())
    }
}

/// An absolute path without trailing slashes, except the root itself, which stays `/`.
fn normalize(path: &str) -> Result<String, BadgeError> {
    if !path.starts_with('/') {
        return Err(BadgeError::BadPath);
    }
    let trimmed = path.trim_end_matches('/');
    Ok(if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_checked() {
        let mut s = BadgeSet::new();
        assert_eq!(s.set_image("", "img", ""), Err(BadgeError::BadIdentifier));
        assert_eq!(s.set_image("has space", "img", ""), Err(BadgeError::BadIdentifier));
        assert_eq!(s.set_image("a/b", "img", ""), Err(BadgeError::BadIdentifier));
        assert_eq!(s.set_image(&"x".repeat(MAX_ID_LEN + 1), "img", ""), Err(BadgeError::BadIdentifier));
        assert_eq!(s.set_image(&"x".repeat(MAX_ID_LEN), "img", ""), Ok(()));
        assert_eq!(s.set_image("synced", "   ", ""), Err(BadgeError::EmptyImage));
        assert_eq!(s.badges().len(), 1);
    }

    #[test]
    fn a_path_needs_a_registered_badge() {
        let mut s = BadgeSet::new();
        assert_eq!(s.set_badge("/a/b", "synced"), Err(BadgeError::UnknownBadge));
        s.set_image("synced", "sync-ok", "Synced").unwrap();
        assert_eq!(s.set_badge("/a/b", "synced"), Ok(()));
        assert_eq!(s.badge_for("/a/b").map(|b| b.image.as_str()), Some("sync-ok"));
    }

    #[test]
    fn paths_are_absolute_and_trailing_slashes_do_not_matter() {
        let mut s = BadgeSet::new();
        s.set_image("synced", "sync-ok", "Synced").unwrap();
        assert_eq!(s.set_badge("docs", "synced"), Err(BadgeError::BadPath));
        assert_eq!(s.set_badge("", "synced"), Err(BadgeError::BadPath));
        assert_eq!(s.set_badge("/docs/", "synced"), Ok(()));
        assert!(s.badge_for("/docs").is_some());
        assert!(s.badge_for("/").is_none());
        assert!(s.badge_for("relative/docs").is_none());
    }

    #[test]
    fn a_new_badge_replaces_the_old_one_and_clear_removes_it() {
        let mut s = BadgeSet::new();
        s.set_image("synced", "sync-ok", "Synced").unwrap();
        s.set_image("error", "sync-err", "Error").unwrap();
        s.set_badge("/docs", "synced").unwrap();
        s.set_badge("/docs", "error").unwrap();
        assert_eq!(s.badge_for("/docs").map(|b| b.identifier.as_str()), Some("error"));
        assert!(s.clear("/docs/"));
        assert!(!s.clear("/docs"));
        assert!(s.badge_for("/docs").is_none());
    }

    #[test]
    fn re_registering_an_image_keeps_the_paths_marked_with_it() {
        let mut s = BadgeSet::new();
        s.set_image("synced", "sync-ok", "Synced").unwrap();
        s.set_badge("/a", "synced").unwrap();
        s.set_image("synced", "sync-ok-2", "Up to date").unwrap();
        assert_eq!(s.badges().len(), 1);
        let b = s.badge_for("/a").unwrap();
        assert_eq!((b.image.as_str(), b.label.as_str()), ("sync-ok-2", "Up to date"));
    }

    #[test]
    fn the_file_manager_question_stores_only_registered_answers() {
        let mut s = BadgeSet::new();
        s.set_image("synced", "sync-ok", "Synced").unwrap();
        assert_eq!(s.request("/x", |p| if p == "/x" { Some("synced".to_string()) } else { None }), Ok(true));
        assert!(s.badge_for("/x").is_some());
        assert_eq!(s.request("/x", |_| None), Ok(false));
        assert!(s.badge_for("/x").is_none());
        assert_eq!(s.request("/y", |_| Some("missing".to_string())), Err(BadgeError::UnknownBadge));
        assert!(s.badge_for("/y").is_none());
        assert_eq!(s.request("nope", |_| None), Err(BadgeError::BadPath));
    }
}
