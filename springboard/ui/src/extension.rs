//! App extensions: the extension points a host app declares, the extensions a package provides for them, and the
//! registry that matches the two.
//!
//! A host declares a [`Point`] whose identifier sits in its own namespace (`com.example.notes` owns
//! `com.example.notes.share`). An extension package declares an [`Identity`] naming its bundle and the point it serves.
//! [`Registry::install`] accepts an identity only when its point is declared and the point's [`Scope`] admits the bundle.
//! [`Registry::discover`] lists the enabled extensions a host can offer, by name. Pure data, no I/O: the shell keeps the
//! `manifest_toml` text in its package records and rebuilds the registry from them. Running an extension is
//! `bifrost_core::extension_process`.

use std::hash::{Hash, Hasher};

/// Why a declaration or install was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not reverse-DNS: needs at least two non-empty labels of ASCII letters, digits or hyphens.
    InvalidBundleId,
    /// The point identifier is not a bundle identifier under its owner's namespace.
    InvalidIdentifier,
    /// A point with this identifier is already declared.
    DuplicatePoint,
    /// No declared point has this identifier.
    UnknownPoint,
    /// The point's scope does not admit this bundle.
    NotAllowed,
}

/// True for reverse-DNS bundle identifiers: at least two labels, each non-empty ASCII letters, digits or `-`.
pub fn valid_bundle_id(s: &str) -> bool {
    s.len() <= 255 && s.split('.').count() >= 2 && s.split('.').all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// `id` lies strictly under the namespace `owner`: `com.example.notes.share` is under `com.example.notes`.
fn under(owner: &str, id: &str) -> bool {
    id.strip_prefix(owner).and_then(|rest| rest.strip_prefix('.')).is_some_and(|rest| !rest.is_empty())
}

/// Which bundles may bind an extension to a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Any package with a valid bundle identifier.
    Any,
    /// Only bundles inside the host's own namespace.
    SameDeveloper,
}

impl Scope {
    /// The manifest spelling: `any` or `same-developer`.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Any => "any",
            Scope::SameDeveloper => "same-developer",
        }
    }
}

/// An extension point a host app declares: its identifier, the host's bundle identifier, and who may bind to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    pub identifier: String,
    pub owner: String,
    pub scope: Scope,
}

impl Point {
    /// `owner` must be a valid bundle identifier and `identifier` a valid one under it.
    pub fn new(owner: &str, identifier: &str, scope: Scope) -> Result<Point, Error> {
        if !valid_bundle_id(owner) {
            return Err(Error::InvalidBundleId);
        }
        if !under(owner, identifier) || !valid_bundle_id(identifier) {
            return Err(Error::InvalidIdentifier);
        }
        Ok(Point { identifier: identifier.into(), owner: owner.into(), scope })
    }

    /// The `[[extension_point]]` table a package manifest carries.
    pub fn manifest_toml(&self) -> String {
        format!("[[extension_point]]\nidentifier = \"{}\"\nowner = \"{}\"\nscope = \"{}\"\n", self.identifier, self.owner, self.scope.name())
    }
}

/// One extension package serving one point. Two identities are equal when they name the same bundle and point;
/// the display name and the enabled flag do not take part.
#[derive(Clone, Debug)]
pub struct Identity {
    pub bundle_id: String,
    pub point: String,
    /// Human-readable name shown in lists.
    pub name: String,
    pub enabled: bool,
}

impl Identity {
    pub fn new(bundle_id: &str, point: &str, name: &str) -> Result<Identity, Error> {
        if !valid_bundle_id(bundle_id) {
            return Err(Error::InvalidBundleId);
        }
        Ok(Identity { bundle_id: bundle_id.into(), point: point.into(), name: name.into(), enabled: true })
    }

    /// The identity starts switched off: installed, but not offered to hosts until enabled.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The `[[extension]]` table a package manifest carries.
    pub fn manifest_toml(&self) -> String {
        format!("[[extension]]\nbundle_id = \"{}\"\npoint = \"{}\"\nname = \"{}\"\n", self.bundle_id, self.point, self.name.replace('"', "'"))
    }
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.bundle_id == other.bundle_id && self.point == other.point
    }
}

impl Eq for Identity {}

impl Hash for Identity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bundle_id.hash(state);
        self.point.hash(state);
    }
}

/// True when `bundle_id` may bind to `point`.
fn admits(point: &Point, bundle_id: &str) -> bool {
    match point.scope {
        Scope::Any => true,
        Scope::SameDeveloper => under(&point.owner, bundle_id),
    }
}

/// The points declared on the system and the extensions installed for them.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    points: Vec<Point>,
    identities: Vec<Identity>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    pub fn declare(&mut self, point: Point) -> Result<(), Error> {
        if self.point(&point.identifier).is_some() {
            return Err(Error::DuplicatePoint);
        }
        self.points.push(point);
        Ok(())
    }

    pub fn point(&self, identifier: &str) -> Option<&Point> {
        self.points.iter().find(|p| p.identifier == identifier)
    }

    /// Installs an identity, or replaces the earlier one for the same bundle and point (an update keeps its place).
    pub fn install(&mut self, identity: Identity) -> Result<(), Error> {
        let point = self.point(&identity.point).ok_or(Error::UnknownPoint)?;
        if !admits(point, &identity.bundle_id) {
            return Err(Error::NotAllowed);
        }
        match self.identities.iter_mut().find(|i| **i == identity) {
            Some(slot) => *slot = identity,
            None => self.identities.push(identity),
        }
        Ok(())
    }

    /// Removes every extension of a package; returns how many were removed.
    pub fn uninstall(&mut self, bundle_id: &str) -> usize {
        let before = self.identities.len();
        self.identities.retain(|i| i.bundle_id != bundle_id);
        before - self.identities.len()
    }

    /// Switches one installed extension on or off. False when it is not installed.
    pub fn set_enabled(&mut self, bundle_id: &str, point: &str, enabled: bool) -> bool {
        match self.identities.iter_mut().find(|i| i.bundle_id == bundle_id && i.point == point) {
            Some(i) => {
                i.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// The enabled extensions for one point, by name then bundle identifier.
    pub fn discover(&self, point: &str) -> Vec<&Identity> {
        self.matching(&[point])
    }

    /// The enabled extensions for any of `points`, by name then bundle identifier.
    pub fn matching(&self, points: &[&str]) -> Vec<&Identity> {
        let mut found: Vec<&Identity> = self.identities.iter().filter(|i| i.enabled && points.contains(&i.point.as_str())).collect();
        found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.bundle_id.cmp(&b.bundle_id)));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn share_point() -> Point {
        Point::new("com.example.notes", "com.example.notes.share", Scope::Any).unwrap()
    }

    #[test]
    fn bundle_ids_need_two_plain_labels() {
        assert!(valid_bundle_id("com.example.notes"));
        assert!(valid_bundle_id("org.my-app.v2"));
        assert!(!valid_bundle_id("notes"));
        assert!(!valid_bundle_id("com..notes"));
        assert!(!valid_bundle_id(".com.notes"));
        assert!(!valid_bundle_id("com.example_notes"));
    }

    #[test]
    fn a_point_must_sit_under_its_owner() {
        assert!(Point::new("com.example.notes", "com.example.notes.share", Scope::Any).is_ok());
        assert_eq!(Point::new("com.example.notes", "com.example.other", Scope::Any), Err(Error::InvalidIdentifier));
        assert_eq!(Point::new("com.example.notes", "com.example.notes", Scope::Any), Err(Error::InvalidIdentifier));
        assert_eq!(Point::new("com.example.notesfoo", "com.example.notes.share", Scope::Any), Err(Error::InvalidIdentifier));
        assert_eq!(Point::new("notes", "notes.share", Scope::Any), Err(Error::InvalidBundleId));
    }

    #[test]
    fn duplicate_points_are_refused() {
        let mut r = Registry::new();
        r.declare(share_point()).unwrap();
        assert_eq!(r.declare(share_point()), Err(Error::DuplicatePoint));
    }

    #[test]
    fn install_needs_a_declared_point_and_an_admitted_bundle() {
        let mut r = Registry::new();
        let any = Identity::new("com.other.clip", "com.example.notes.share", "Clip").unwrap();
        assert_eq!(r.install(any.clone()), Err(Error::UnknownPoint));

        r.declare(share_point()).unwrap();
        r.declare(Point::new("com.example.notes", "com.example.notes.own", Scope::SameDeveloper).unwrap()).unwrap();
        assert!(r.install(any).is_ok());

        let outsider = Identity::new("com.example.notesfoo", "com.example.notes.own", "Lookalike").unwrap();
        assert_eq!(r.install(outsider), Err(Error::NotAllowed));
        let insider = Identity::new("com.example.notes.pdf", "com.example.notes.own", "PDF").unwrap();
        assert!(r.install(insider).is_ok());
    }

    #[test]
    fn discover_lists_only_enabled_extensions_of_the_point_by_name() {
        let mut r = Registry::new();
        r.declare(share_point()).unwrap();
        r.install(Identity::new("com.other.zeta", "com.example.notes.share", "Zeta").unwrap()).unwrap();
        r.install(Identity::new("com.other.alpha", "com.example.notes.share", "Alpha").unwrap()).unwrap();
        r.install(Identity::new("com.other.hidden", "com.example.notes.share", "Hidden").unwrap().disabled()).unwrap();

        let names: Vec<&str> = r.discover("com.example.notes.share").iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Zeta"]);
        assert!(r.discover("com.example.notes.unknown").is_empty());

        assert!(r.set_enabled("com.other.hidden", "com.example.notes.share", true));
        assert_eq!(r.discover("com.example.notes.share").len(), 3);
        assert!(!r.set_enabled("com.other.absent", "com.example.notes.share", true));
    }

    #[test]
    fn reinstalling_a_bundle_replaces_it_and_uninstall_removes_all_its_points() {
        let mut r = Registry::new();
        r.declare(share_point()).unwrap();
        r.declare(Point::new("com.example.notes", "com.example.notes.open", Scope::Any).unwrap()).unwrap();
        r.install(Identity::new("com.other.clip", "com.example.notes.share", "Clip").unwrap()).unwrap();
        r.install(Identity::new("com.other.clip", "com.example.notes.share", "Clip 2").unwrap()).unwrap();
        r.install(Identity::new("com.other.clip", "com.example.notes.open", "Clip").unwrap()).unwrap();
        assert_eq!(r.discover("com.example.notes.share")[0].name, "Clip 2");

        assert_eq!(r.uninstall("com.other.clip"), 2);
        assert!(r.matching(&["com.example.notes.share", "com.example.notes.open"]).is_empty());
    }

    #[test]
    fn identity_equality_ignores_name_and_state() {
        let a = Identity::new("com.other.clip", "com.example.notes.share", "Clip").unwrap();
        let b = Identity::new("com.other.clip", "com.example.notes.share", "Renamed").unwrap().disabled();
        assert_eq!(a, b);
        let set: HashSet<Identity> = [a, b].into_iter().collect();
        assert_eq!(set.len(), 1);
        let other_point = Identity::new("com.other.clip", "com.example.notes.open", "Clip").unwrap();
        assert_ne!(set.iter().next().unwrap(), &other_point);
    }

    #[test]
    fn manifest_text_names_each_field() {
        let p = share_point().manifest_toml();
        assert!(p.starts_with("[[extension_point]]\nidentifier = \"com.example.notes.share\"\n"));
        assert!(p.contains("scope = \"any\"\n"));
        let i = Identity::new("com.other.clip", "com.example.notes.share", "The \"Clip\"").unwrap().manifest_toml();
        assert!(i.contains("name = \"The 'Clip'\"\n"));
    }
}
