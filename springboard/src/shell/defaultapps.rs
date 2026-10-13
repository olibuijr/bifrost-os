//! Default apps: the shell's Launch Services. Built-in apps and installed manifests (`handles`, `schemes`) declare what
//! they open; `bifrost_ui::launch::Registry` resolves the default per type or URL scheme, and the user's choices (the
//! Settings > General > Default Apps page, which is where Golden Gate has Files's Get Info > Open with > Change All) are
//! kept in one preference. Apps are keyed by `AppDef::icon`, the stable identifier (the bundle id for installed apps).
use super::apphost;
use super::apps::{apps, AppId};
use bifrost_proto::WorkspaceStatus;
use bifrost_ui::launch::{Choice, Handler, Rank, Registry};

/// The preference holding the user's choices.
pub const PREF: &str = "default.apps";

/// One row group of the Default Apps page: a family of types (or a URL scheme) that is changed together.
pub struct Category {
    pub label: &'static str,
    /// concrete types set together ("Change All"); the first one decides who is shown as the current default
    pub types: &'static [&'static str],
    pub scheme: Option<&'static str>,
}

pub const CATEGORIES: &[Category] = &[
    Category { label: "Web Browser", types: &["public.html"], scheme: Some("https") },
    Category { label: "Images", types: &["public.png", "public.jpeg", "com.compuserve.gif", "public.webp"], scheme: None },
    Category { label: "Audio", types: &["public.mp3", "public.mpeg-4-audio", "org.xiph.flac", "com.microsoft.waveform-audio", "org.xiph.ogg-audio", "public.aac-audio", "org.xiph.opus-audio"], scheme: None },
    Category { label: "Video", types: &["public.mpeg-4", "bifrost.player-movie"], scheme: None },
    Category { label: "Text", types: &["public.plain-text", "net.daringfireball.markdown", "public.json", "public.xml", "public.yaml", "org.toml.toml"], scheme: None },
    Category { label: "PDF Documents", types: &["com.adobe.pdf"], scheme: None },
    Category { label: "Archives", types: &["public.zip-archive", "public.tar-archive", "org.gnu.gnu-zip-archive"], scheme: None },
    Category { label: "Email", types: &[], scheme: Some("mailto") },
];

/// What each built-in app opens (installed apps declare it in their manifest).
fn builtin(app: AppId, icon: &str) -> Option<Handler> {
    let h = Handler::new(icon);
    Some(match app {
        AppId::Photos => h.opens("public.png", Rank::Owner).opens("public.jpeg", Rank::Owner),
        AppId::Files => h.opens("public.folder", Rank::Owner).opens("public.text", Rank::Alternate).opens("public.png", Rank::Alternate).opens("public.jpeg", Rank::Alternate).opens("public.audio", Rank::Alternate).opens("public.movie", Rank::Alternate),
        AppId::Browser => h.opens("public.html", Rank::Owner).opens("public.xml", Rank::Alternate).scheme("http").scheme("https"),
        AppId::Terminal => h.opens("public.shell-script", Rank::Alternate).opens("public.plain-text", Rank::None),
        AppId::Deildu => h.opens("public.movie", Rank::Alternate),
        _ => return None,
    })
}

/// What the first-party desktop apps open, until their manifests carry `handles` themselves (a manifest, once present, wins).
fn first_party(id: &str) -> Option<Handler> {
    let h = Handler::new(id);
    Some(match id {
        "is.olibuijr.files.desktop" => h.opens("public.folder", Rank::Owner).opens("public.text", Rank::Alternate).opens("public.png", Rank::Alternate).opens("public.jpeg", Rank::Alternate).opens("public.audio", Rank::Alternate).opens("public.movie", Rank::Alternate).opens("public.archive", Rank::Alternate),
        "is.olibuijr.browser.desktop" => h.opens("public.html", Rank::Owner).opens("public.xml", Rank::Alternate).opens("public.svg-image", Rank::Alternate).scheme("http").scheme("https"),
        "is.olibuijr.music.desktop" => h.opens("public.audio", Rank::Default),
        "is.olibuijr.notes.desktop" => h.opens("public.plain-text", Rank::Default).opens("net.daringfireball.markdown", Rank::Default),
        "is.olibuijr.voicememos.desktop" => h.opens("public.mpeg-4-audio", Rank::Alternate),
        _ => return None,
    })
}

/// The ranks of a manifest `handles` entry: "public.png", "public.png:owner".
fn parse_handle(entry: &str) -> Option<(&str, Rank)> {
    let (uti, rank) = entry.split_once(':').map_or((entry, None), |(u, r)| (u, Some(r)));
    let rank = match rank {
        Some(r) => Rank::parse(r)?,
        None => Rank::Default,
    };
    (!uti.is_empty()).then_some((uti, rank))
}

/// The handler an installed app's manifest declares; `None` when it declares nothing.
pub fn manifest_handler(m: &apphost::Manifest) -> Option<Handler> {
    if m.handles.is_empty() && m.schemes.is_empty() {
        return None;
    }
    let mut h = Handler::new(&m.id);
    for e in &m.handles {
        if let Some((uti, rank)) = parse_handle(e) {
            h = h.opens(uti, rank);
        }
    }
    for s in &m.schemes {
        h = h.scheme(s);
    }
    Some(h)
}

/// The registry for the apps on this system now, with the user's saved choices applied.
pub fn registry(saved: Option<&str>) -> Registry {
    let mut r = Registry::new();
    let installed = apphost::installed();
    for def in apps() {
        let handler = match def.id {
            AppId::External(slot) => installed.get(slot as usize).and_then(|a| manifest_handler(&a.manifest).or_else(|| first_party(&a.manifest.id))).map(|mut h| {
                h.app = def.icon.into();
                h
            }),
            id => builtin(id, def.icon),
        };
        if let Some(h) = handler {
            r.register(h);
        }
    }
    if let Some(s) = saved {
        r.load_pref(s);
    }
    r
}

impl super::Shell {
    /// Builds the registry (with the saved choices) on first use and again when apps were installed or removed.
    pub(super) fn ensure_defaults(&mut self) {
        let count = apps().len();
        if self.defaults.as_ref().is_some_and(|(n, ..)| *n == count) {
            return;
        }
        let r = registry(self.prefs.get(PREF));
        let v = views(&r);
        self.defaults = Some((count, r, v));
    }

    /// Applies the Settings page's choices (category index, app key) and saves them.
    pub(super) fn apply_default_changes(&mut self, changes: Vec<(usize, String)>) {
        if changes.is_empty() {
            return;
        }
        self.ensure_defaults();
        let Some((_, r, v)) = &mut self.defaults else { return };
        for (category, app) in changes {
            set_category(r, category, &app);
        }
        *v = views(r);
        let _ = self.prefs.set(PREF, &r.to_pref());
    }

    /// The default app for a file name (the registry's answer, for Files' open and the shell's own dispatch).
    pub fn default_app_for_file(&mut self, name: &str) -> Option<String> {
        self.ensure_defaults();
        self.defaults.as_ref().and_then(|(_, r, _)| r.default_for_file(name)).map(str::to_string)
    }
}

impl super::Shell {
    /// The Open With candidates for a file name, the default first (the `launch` service's `open_with`).
    pub(super) fn open_with_choices(&mut self, name: &str) -> Vec<Choice> {
        self.ensure_defaults();
        self.defaults.as_ref().map_or_else(Vec::new, |(_, r, _)| r.open_with(name))
    }

    /// The app that opens a URL scheme, from the user's choice or the best declaration (the `launch` service).
    pub(super) fn default_app_for_scheme(&mut self, scheme: &str) -> Option<String> {
        self.ensure_defaults();
        self.defaults.as_ref().and_then(|(_, r, _)| r.default_for_scheme(scheme)).map(str::to_string)
    }

    /// Opens a link from the control socket (`open-url`, `bifrost-open-url`): web links go to the browser, any other
    /// scheme to the app the registry names for it (`mailto:`, `bifrost-app:`). Local and script schemes are refused.
    pub fn desktop_open_link(&mut self, url: &str, t: f64) -> std::io::Result<()> {
        let why = match self.workspace_open_url(url, t).0 {
            WorkspaceStatus::Ok => return Ok(()),
            WorkspaceStatus::Denied => "Unsupported link",
            WorkspaceStatus::NotFound => "No app opens this link",
            WorkspaceStatus::Unsupported => "Links are not supported here",
            WorkspaceStatus::Failed => "The link could not be opened",
        };
        Err(std::io::Error::other(why))
    }
}

/// The display name of an app key.
pub fn app_name(key: &str) -> String {
    apps().iter().find(|a| a.icon == key).map_or_else(|| key.to_string(), |a| a.name.to_string())
}

/// A category as the Settings page shows it: its candidates and the current default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryView {
    pub label: &'static str,
    /// (app key, display name) for every app that can open the family's first type, the default first
    pub apps: Vec<(String, String)>,
    pub current: Option<String>,
}

pub fn views(r: &Registry) -> Vec<CategoryView> {
    CATEGORIES
        .iter()
        .map(|c| {
            let (candidates, current): (Vec<&str>, Option<&str>) = match c.scheme {
                Some(s) => (r.apps_for_scheme(s), r.default_for_scheme(s)),
                None => (r.apps_for_type(c.types[0]), r.default_for_type(c.types[0])),
            };
            let mut apps: Vec<(String, String)> = candidates.into_iter().map(|a| (a.to_string(), app_name(a))).collect();
            apps.sort_by_key(|(k, _)| Some(k.as_str()) != current);
            CategoryView { label: c.label, apps, current: current.map(str::to_string) }
        })
        .collect()
}

/// Makes `app` the default for the whole category ("Change All"): every type of the family it can open, and the scheme.
pub fn set_category(r: &mut Registry, category: usize, app: &str) {
    let Some(c) = CATEGORIES.get(category) else { return };
    for t in c.types {
        let _ = r.set_default_for_type(t, app);
    }
    if let Some(s) = c.scheme {
        let _ = r.set_default_for_scheme(s, app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> apphost::Manifest {
        apphost::Manifest::parse(&format!("id = \"is.x.app\"\nname = \"X\"\nexec = \"x\"\n{text}")).unwrap()
    }

    #[test]
    fn manifests_declare_what_they_open_with_optional_ranks() {
        let m = manifest("handles = [\"public.png:owner\", \"public.text\", \"public.mp3:none\", \"bad:rank\"]\nschemes = [\"mailto\"]");
        assert_eq!(m.handles.len(), 4);
        let h = manifest_handler(&m).unwrap();
        assert_eq!(h.types, [("public.png".to_string(), Rank::Owner), ("public.text".to_string(), Rank::Default), ("public.mp3".to_string(), Rank::None)], "a bad rank drops the entry");
        assert_eq!(h.schemes, ["mailto"]);
        assert!(manifest_handler(&manifest("")).is_none());
    }

    #[test]
    fn first_party_apps_declare_sensible_defaults() {
        let mut r = Registry::new();
        for id in ["is.olibuijr.files.desktop", "is.olibuijr.browser.desktop", "is.olibuijr.music.desktop", "is.olibuijr.notes.desktop"] {
            r.register(first_party(id).unwrap());
        }
        assert_eq!(r.default_for_file("song.flac"), Some("is.olibuijr.music.desktop"), "Music (Default) beats Files (Alternate)");
        assert_eq!(r.default_for_file("todo.md"), Some("is.olibuijr.notes.desktop"));
        assert_eq!(r.default_for_file("page.html"), Some("is.olibuijr.browser.desktop"));
        assert_eq!(r.default_for_scheme("https"), Some("is.olibuijr.browser.desktop"));
        assert_eq!(r.default_for_type("public.folder"), Some("is.olibuijr.files.desktop"));
        assert_eq!(r.default_for_file("archive.zip"), Some("is.olibuijr.files.desktop"));
        assert!(first_party("is.x.unknown").is_none());
    }

    #[test]
    fn built_in_apps_are_the_defaults_until_the_user_chooses() {
        let mut r = Registry::new();
        for (id, app) in [("photos", AppId::Photos), ("files", AppId::Files), ("browser", AppId::Browser)] {
            r.register(builtin(app, id).unwrap());
        }
        r.register(manifest_handler(&manifest("handles = [\"public.png:alternate\"]")).unwrap());
        let v = views(&r);
        let images = v.iter().find(|c| c.label == "Images").unwrap();
        assert_eq!(images.current.as_deref(), Some("photos"));
        assert_eq!(images.apps[0].0, "photos", "the default is listed first");
        assert!(images.apps.iter().any(|(k, _)| k == "is.x.app"));
        let web = v.iter().find(|c| c.label == "Web Browser").unwrap();
        assert_eq!(web.current.as_deref(), Some("browser"));
        // Change All moves every type of the family the app can open
        set_category(&mut r, 1, "files");
        assert_eq!(r.default_for_type("public.jpeg"), Some("files"));
        assert_eq!(r.default_for_type("public.png"), Some("files"));
        assert_eq!(r.default_for_type("com.compuserve.gif"), None, "nobody declares GIF, so nothing is set");
        // and it survives a restart through the preference
        let saved = r.to_pref();
        let mut again = Registry::new();
        again.register(builtin(AppId::Photos, "photos").unwrap());
        again.register(builtin(AppId::Files, "files").unwrap());
        again.load_pref(&saved);
        assert_eq!(again.default_for_type("public.png"), Some("files"));
    }

    #[test]
    fn the_shell_resolves_saves_and_restores_the_users_choice() {
        let mut sh = crate::shell::tests::shell();
        assert_eq!(sh.default_app_for_file("holiday.PNG").as_deref(), Some("bifrost.photos"), "Photos owns PNG");
        assert_eq!(sh.default_app_for_file("notes.unknownext"), None);
        let images = CATEGORIES.iter().position(|c| c.label == "Images").unwrap();
        sh.apply_default_changes(vec![(images, "bifrost.files".into())]);
        assert_eq!(sh.default_app_for_file("holiday.png").as_deref(), Some("bifrost.files"));
        assert!(sh.prefs.get(PREF).is_some_and(|v| v.contains("type public.png bifrost.files")), "saved: {:?}", sh.prefs.get(PREF));
        let view = &sh.defaults.as_ref().unwrap().2[images];
        assert_eq!(view.current.as_deref(), Some("bifrost.files"));
        // a fresh shell with the same preference starts with the same default
        let saved = sh.prefs.get(PREF).unwrap().to_string();
        let again = registry(Some(&saved));
        assert_eq!(again.default_for_file("a.jpeg"), Some("bifrost.files"));
    }

    #[test]
    fn the_email_category_names_the_mailto_handler() {
        let mut r = Registry::new();
        r.register(Handler::new("bifrost.mail").scheme("mailto"));
        let v = views(&r);
        let mail = CATEGORIES.iter().position(|c| c.label == "Email").unwrap();
        assert_eq!(v[mail].apps, [("bifrost.mail".to_string(), "bifrost.mail".to_string())]);
        assert_eq!(v[mail].current.as_deref(), Some("bifrost.mail"));
        assert_eq!(r.default_for_scheme("MAILTO"), Some("bifrost.mail"), "schemes are case-insensitive");
    }

    #[test]
    fn open_with_lists_the_default_first_and_links_are_refused_without_a_handler() {
        let mut sh = crate::shell::tests::shell();
        let c = sh.open_with_choices("holiday.PNG");
        assert_eq!(c[0].app, "bifrost.photos");
        assert!(c[0].is_default);
        assert!(c.iter().any(|x| x.app == "bifrost.files" && !x.is_default), "Files can open PNG too: {c:?}");
        assert!(sh.open_with_choices("notes.unknownext").is_empty());
        assert_eq!(sh.default_app_for_scheme("mailto"), None, "nothing declares mailto in the test shell");
        // a local scheme is refused before any app is looked up
        assert_eq!(sh.desktop_open_link("file:///etc/passwd", 0.0).unwrap_err().to_string(), "Unsupported link");
        assert_eq!(sh.desktop_open_link("gopher://x", 0.0).unwrap_err().to_string(), "No app opens this link");
    }

    #[test]
    fn the_settings_page_lists_every_category_with_the_default_checked() {
        use crate::shell::list::Accessory;
        use crate::shell::pages::{spec, Action, Model, PageId};
        let mut r = Registry::new();
        r.register(builtin(AppId::Photos, "bifrost.photos").unwrap());
        r.register(builtin(AppId::Files, "bifrost.files").unwrap());
        let v = views(&r);
        let info = crate::shell::Info::default();
        let opts = crate::shell::options::Options::default();
        let m = Model { info: &info, opts: &opts, a11y: &crate::shell::accessibility::AccessibilitySettings::DEFAULT, airplane: false, haptics: false, wifi_on: true, brightness: 0.5, wallpaper: String::new(), detail: "", now: 0, update_badge: false, dock: Default::default(), defaults: &v, login_items: &[], login_pending: &[], ext: &crate::shell::login_items_page::NO_EXTENSIONS, fonts: &crate::shell::pages::NO_FONTS, refresh: crate::shell::services::display_services::RefreshChoice::Max, sound: &crate::shell::sound_page::NO_SOUND, sharing: None, printers: None, users: &crate::shell::pages::NO_USERS };
        let s = spec(PageId::DefaultApps, &m);
        assert!(s.groups.len() > CATEGORIES.len(), "a group per category plus the footer");
        let images = &s.groups[1];
        assert_eq!(images.items[0].label, "Photos", "the default first");
        assert!(matches!(images.items[0].accessory, Accessory::Check(true)));
        assert!(matches!(images.items[1].accessory, Accessory::Check(false)));
        assert_eq!(images.items[1].action, Action::DefaultApp(1, "bifrost.files".into()));
        assert_eq!(s.groups[0].items[0].label, "No app available", "no browser registered here");
    }
}
