//! Settings > Spotlight: which kinds of result Search lists, and the folders it never lists. The choice is kept in the
//! settings store (`settings.ini`) as two keys: `spotlight_off` names the categories switched off (comma separated) and
//! `spotlight_excluded` holds the excluded folders, one escaped field each, joined by tabs. A category that is on is
//! the default, and a folder is excluded with everything under it.
//!
//! The shell keeps the current scope in [`install`]ed memory so Search reads it on every query without touching the
//! disk. [`Scope::hides`] is the one question Search asks, for a hit's category and path.

use super::prefs::{escape_field, unescape_field, Prefs};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const OFF_PREF: &str = "spotlight_off";
pub const EXCLUDED_PREF: &str = "spotlight_excluded";
/// The longest folder path kept in the exclusion list.
const PATH_MAX: usize = 4096;

/// A kind of result Search lists, as Settings > Spotlight names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Applications,
    Settings,
    Contacts,
    CalendarEvents,
    Files,
    Documents,
    Cities,
    Calculator,
}

impl Category {
    /// Every category, in the order the page lists them.
    pub const ALL: [Category; 8] = [Category::Applications, Category::Settings, Category::Contacts, Category::CalendarEvents, Category::Files, Category::Documents, Category::Cities, Category::Calculator];

    /// The row title on the Spotlight page.
    pub fn title(self) -> &'static str {
        match self {
            Category::Applications => "Applications",
            Category::Settings => "Settings",
            Category::Contacts => "Contacts",
            Category::CalendarEvents => "Calendar Events",
            Category::Files => "Files and Folders",
            Category::Documents => "Documents",
            Category::Cities => "Cities",
            Category::Calculator => "Calculator",
        }
    }

    /// The name stored in `spotlight_off`.
    pub fn key(self) -> &'static str {
        match self {
            Category::Applications => "applications",
            Category::Settings => "settings",
            Category::Contacts => "contacts",
            Category::CalendarEvents => "calendar",
            Category::Files => "files",
            Category::Documents => "documents",
            Category::Cities => "cities",
            Category::Calculator => "calculator",
        }
    }

    /// The category with this stored name.
    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key)
    }

    /// The index of this category in [`Category::ALL`] (the page's row index).
    pub fn index(self) -> usize {
        Category::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

/// The categories switched off and the folders excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    off: Vec<Category>,
    excluded: Vec<PathBuf>,
}

impl Scope {
    /// Reads both keys. A missing or unknown name is ignored: the default is on, and a bad folder entry is dropped.
    pub fn load(p: &Prefs) -> Scope {
        let off = p.get(OFF_PREF).unwrap_or("").split(',').filter_map(Category::from_key).collect::<Vec<_>>();
        let excluded = p
            .get(EXCLUDED_PREF)
            .unwrap_or("")
            .split('\t')
            .filter(|f| !f.is_empty())
            .filter_map(|f| unescape_field(f, PATH_MAX))
            .map(PathBuf::from)
            .collect();
        Scope { off, excluded }
    }

    /// Writes both keys. Only the keys that change touch the disk (`Prefs::set`).
    pub fn save(&self, p: &mut Prefs) -> std::io::Result<()> {
        let off: Vec<&str> = self.off.iter().map(|c| c.key()).collect();
        p.set(OFF_PREF, &off.join(","))?;
        let fields: Vec<String> = self.excluded.iter().map(|f| escape_field(&f.to_string_lossy())).collect();
        p.set(EXCLUDED_PREF, &fields.join("\t"))
    }

    /// Whether Search lists results of this category.
    pub fn allows(&self, c: Category) -> bool {
        !self.off.contains(&c)
    }

    /// Switches a category on or off. Returns the new state.
    pub fn set_allowed(&mut self, c: Category, on: bool) -> bool {
        self.off.retain(|x| *x != c);
        if !on {
            self.off.push(c);
        }
        on
    }

    /// The folders Search never lists, in the order they were added.
    pub fn excluded(&self) -> &[PathBuf] {
        &self.excluded
    }

    /// Adds a folder to the exclusions. An empty path, a path already excluded, or one under an excluded folder is not
    /// added again. Returns whether the list changed.
    pub fn exclude(&mut self, folder: &Path) -> bool {
        if folder.as_os_str().is_empty() || folder.to_string_lossy().len() > PATH_MAX || self.excludes(folder) {
            return false;
        }
        self.excluded.push(folder.to_path_buf());
        true
    }

    /// Takes the folder at `index` off the exclusions. Returns whether one was removed.
    pub fn remove_excluded(&mut self, index: usize) -> bool {
        if index < self.excluded.len() {
            self.excluded.remove(index);
            true
        } else {
            false
        }
    }

    /// Whether `path` is an excluded folder or lies under one.
    pub fn excludes(&self, path: &Path) -> bool {
        self.excluded.iter().any(|f| path.starts_with(f))
    }

    /// Whether Search must hide a result of category `c` at `path` (a path only for files and documents).
    pub fn hides(&self, c: Category, path: Option<&Path>) -> bool {
        !self.allows(c) || path.is_some_and(|p| self.excludes(p))
    }
}

static CURRENT: Mutex<Option<Scope>> = Mutex::new(None);

/// The scope Search applies now (the default when none was installed).
pub fn current() -> Scope {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Makes `scope` the one Search applies.
pub fn install(scope: Scope) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(scope);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_default_on_and_round_trip_by_key() {
        let s = Scope::default();
        assert!(Category::ALL.iter().all(|c| s.allows(*c)));
        for c in Category::ALL {
            assert_eq!(Category::from_key(c.key()), Some(c));
            assert_eq!(Category::ALL[c.index()], c);
        }
        assert_eq!(Category::from_key("nope"), None);
    }

    #[test]
    fn switching_a_category_off_hides_its_results_only() {
        let mut s = Scope::default();
        assert!(!s.set_allowed(Category::Files, false));
        assert!(s.hides(Category::Files, None));
        assert!(!s.hides(Category::Contacts, None));
        s.set_allowed(Category::Files, true);
        assert!(!s.hides(Category::Files, None));
    }

    #[test]
    fn an_excluded_folder_hides_everything_under_it() {
        let mut s = Scope::default();
        assert!(s.exclude(Path::new("/home/u/Private")));
        assert!(!s.exclude(Path::new("/home/u/Private")), "not added twice");
        assert!(!s.exclude(Path::new("/home/u/Private/deeper")), "already under an excluded folder");
        assert!(s.hides(Category::Files, Some(Path::new("/home/u/Private/a.txt"))));
        assert!(!s.hides(Category::Files, Some(Path::new("/home/u/Privately.txt"))), "a sibling with the same prefix is not inside");
        assert!(s.remove_excluded(0));
        assert!(!s.remove_excluded(0));
        assert!(!s.hides(Category::Files, Some(Path::new("/home/u/Private/a.txt"))));
    }

    #[test]
    fn scope_saves_and_loads_through_the_settings_store() {
        let mut p = Prefs::memory();
        let mut s = Scope::default();
        s.set_allowed(Category::Cities, false);
        s.set_allowed(Category::Files, false);
        s.exclude(Path::new("/home/u/with\ttab"));
        s.exclude(Path::new("/home/u/Work"));
        s.save(&mut p).unwrap();
        let back = Scope::load(&p);
        assert_eq!(back, s);
        assert!(!back.allows(Category::Cities));
        assert_eq!(back.excluded().len(), 2);
    }

    #[test]
    fn a_bad_value_is_ignored_not_fatal() {
        let mut p = Prefs::memory();
        p.set(OFF_PREF, "files,bogus").unwrap();
        p.set(EXCLUDED_PREF, "/a\t\\q").unwrap();
        let s = Scope::load(&p);
        assert!(!s.allows(Category::Files));
        assert!(s.allows(Category::Contacts));
        assert_eq!(s.excluded(), &[PathBuf::from("/a")]);
    }

    #[test]
    fn install_makes_a_scope_current() {
        let mut s = Scope::default();
        s.set_allowed(Category::Calculator, false);
        install(s.clone());
        assert_eq!(current(), s);
        install(Scope::default());
    }
}
