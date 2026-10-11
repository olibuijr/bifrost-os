//! Which apps appear where, and which bifrost icon/name each Android app wears.

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub pkg: String,
    pub cls: String,
    pub label: String,
}

pub struct Skin {
    /// asset name under assets/icons/, without extension
    pub icon: &'static str,
    pub label: &'static str,
}

const SKINS: &[(&str, &str, &str)] = &[
    ("com.android.settings", "bifrost.settings", "Settings"),
    ("org.lineageos.jelly", "bifrost.browser", "Browser"),
    ("com.android.chrome", "bifrost.browser", "Browser"),
    ("com.android.documentsui", "bifrost.files", "Files"),
    ("com.android.deskclock", "bifrost.clock", "Clock"),
    ("org.lineageos.aperture", "bifrost.camera", "Camera"),
    ("org.lineageos.glimpse", "bifrost.photos", "Photos"),
    ("com.android.gallery3d", "bifrost.photos", "Photos"),
    ("com.android.messaging", "bifrost.messages", "Messages"),
    ("com.android.contacts", "bifrost.people", "Contacts"),
    ("org.fdroid.fdroid", "bifrost.appstore", "App Store"),
    ("com.termux", "ws.hbang.Terminal", "Terminal"),
];

/// Dock order, like the bifrost's default dock.
const DOCK: &[&str] = &["org.lineageos.jelly", "com.android.chrome", "com.android.messaging", "com.android.settings", "com.android.documentsui", "org.fdroid.fdroid", "com.termux"];
const DOCK_MAX: usize = 6;

pub fn skin(pkg: &str) -> Option<Skin> {
    SKINS.iter().find(|s| s.0 == pkg).map(|s| Skin { icon: s.1, label: s.2 })
}

pub fn display_label(e: &Entry) -> &str {
    skin(&e.pkg).map(|s| s.label).unwrap_or(&e.label)
}

/// Splits launchable apps into (dock, grid). The grid is sorted by name.
pub fn arrange(entries: Vec<Entry>, own_pkg: &str) -> (Vec<Entry>, Vec<Entry>) {
    let mut entries: Vec<Entry> = entries.into_iter().filter(|e| e.pkg != own_pkg).collect();
    entries.dedup_by(|a, b| a.pkg == b.pkg && a.cls == b.cls);
    let mut dock = Vec::new();
    for pkg in DOCK {
        if dock.len() == DOCK_MAX {
            break;
        }
        if let Some(i) = entries.iter().position(|e| e.pkg == *pkg) {
            dock.push(entries.remove(i));
        }
    }
    entries.sort_by_key(|e| display_label(e).to_lowercase());
    (dock, entries)
}

/// App Library categories, in card order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Productivity,
    Utilities,
    Entertainment,
    Social,
    Developer,
}

impl Category {
    pub const ALL: [Category; 5] = [Category::Productivity, Category::Utilities, Category::Entertainment, Category::Social, Category::Developer];

    pub fn name(self) -> &'static str {
        match self {
            Category::Productivity => "Productivity",
            Category::Utilities => "Utilities",
            Category::Entertainment => "Entertainment",
            Category::Social => "Social",
            Category::Developer => "Developer",
        }
    }
}

/// Which category an app lands in, keyed by its icon asset name (the
/// built-in apps) or its Android package. Unlisted apps are Utilities.
const CATEGORIES: &[(&str, Category)] = &[
    ("bifrost.notes", Category::Productivity),
    ("bifrost.files", Category::Productivity),
    ("bifrost.calendar", Category::Productivity),
    ("bifrost.reminders", Category::Productivity),
    ("bifrost.settings", Category::Utilities),
    ("bifrost.clock", Category::Utilities),
    ("bifrost.calculator", Category::Utilities),
    ("bifrost.freeform", Category::Productivity),
    ("bifrost.shortcuts", Category::Productivity),
    ("bifrost.browser", Category::Utilities),
    ("bifrost.appstore", Category::Utilities),
    ("bifrost.camera", Category::Utilities),
    ("bifrost.magnifier", Category::Utilities),
    ("is.olibuijr.deildu", Category::Entertainment),
    ("is.olibuijr.news", Category::Entertainment),
    ("is.olibuijr.podcasts", Category::Entertainment),
    ("is.olibuijr.voicememos", Category::Utilities),
    ("is.olibuijr.games", Category::Entertainment),
    ("bifrost.photos", Category::Entertainment),
    ("bifrost.music", Category::Entertainment),
    ("bifrost.contacts", Category::Social),
    ("bifrost.messages", Category::Social),
    ("bifrost.people", Category::Social),
    ("is.olibuijr.term", Category::Developer),
    ("ws.hbang.Terminal", Category::Developer),
    ("com.termux", Category::Developer),
    ("com.android.messaging", Category::Social),
    ("com.android.contacts", Category::Social),
    ("org.fdroid.fdroid", Category::Utilities),
];

pub fn category(key: &str) -> Category {
    CATEGORIES.iter().find(|c| c.0 == key).map_or(Category::Utilities, |c| c.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(pkg: &str, label: &str) -> Entry {
        Entry { pkg: pkg.into(), cls: format!("{pkg}.Main"), label: label.into() }
    }

    #[test]
    fn settings_wears_the_bifrost_icon() {
        let s = skin("com.android.settings").unwrap();
        assert_eq!(s.icon, "bifrost.settings");
        assert_eq!(s.label, "Settings");
        assert!(skin("is.akurai.signage").is_none());
    }

    #[test]
    fn dock_takes_known_apps_in_order_and_grid_is_sorted() {
        let (dock, grid) = arrange(
            vec![e("com.android.settings", "Settings"), e("zz.app", "zebra"), e("org.lineageos.jelly", "Browser"), e("aa.app", "Apricot"), e("is.akurai.springboard", "SpringBoard")],
            "is.akurai.springboard",
        );
        let pk = |v: &[Entry]| v.iter().map(|x| x.pkg.clone()).collect::<Vec<_>>();
        assert_eq!(pk(&dock), ["org.lineageos.jelly", "com.android.settings"]);
        assert_eq!(pk(&grid), ["aa.app", "zz.app"]);
    }

    #[test]
    fn dock_is_capped() {
        let all: Vec<Entry> = DOCK.iter().map(|p| e(p, p)).collect();
        let (dock, grid) = arrange(all, "x");
        assert_eq!(dock.len(), DOCK_MAX);
        assert_eq!(grid.len(), DOCK.len() - DOCK_MAX);
    }

    #[test]
    fn display_label_prefers_skin() {
        assert_eq!(display_label(&e("org.lineageos.jelly", "Browser")), "Browser");
        assert_eq!(display_label(&e("x.y", "Thing")), "Thing");
    }

    #[test]
    fn categories_come_from_the_static_map() {
        assert_eq!(category("is.olibuijr.term"), Category::Developer);
        assert_eq!(category("bifrost.notes"), Category::Productivity);
        assert_eq!(category("com.termux"), Category::Developer);
        assert_eq!(category("unknown.app"), Category::Utilities);
        assert_eq!(Category::ALL.len(), 5);
    }
}
