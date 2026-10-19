//! Built-in and installed app catalogue used by the home screen and desktop.
use super::{apphost, browser, camera, magnifier, store};
use crate::gfx::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppId {
    Settings,
    Terminal,
    Deildu,
    Photos,
    Files,
    Camera,
    Browser,
    Magnifier,
    AppStore,
    /// an installed app (see `apphost`): the index into `apphost::installed()`
    External(u8),
    /// an extra window of an installed app: the app's slot, then the window id the app chose (see `app_window_table`).
    /// The desk treats it as a window of its own; the parent app's semantics apply everywhere else.
    ExternalWindow(u8, u16),
}

impl AppId {
    /// The app this window belongs to: an extra window's parent is `External(slot)`; every other id is its own parent.
    pub fn parent(self) -> AppId {
        match self {
            AppId::ExternalWindow(slot, _) => AppId::External(slot),
            other => other,
        }
    }

    /// True for an extra window (`ExternalWindow`), false for an app's main window or a built-in app.
    pub fn is_extra_window(self) -> bool {
        matches!(self, AppId::ExternalWindow(..))
    }

    /// The slot of the installed app an extra window belongs to.
    pub fn window_slot(self) -> Option<u8> {
        match self {
            AppId::ExternalWindow(slot, _) => Some(slot),
            _ => None,
        }
    }

    /// The app's own id of an extra window (`WindowOpen { id }`), as the desk keys it.
    pub fn window_id(self) -> Option<u16> {
        match self {
            AppId::ExternalWindow(_, id) => Some(id),
            _ => None,
        }
    }

    /// The extra window `id` of the installed app in `slot`.
    pub fn extra_window(slot: u8, id: u16) -> AppId {
        AppId::ExternalWindow(slot, id)
    }
}

#[derive(Clone, Copy)]
pub struct AppDef {
    pub id: AppId,
    pub name: &'static str,
    pub icon: &'static str,
    /// draws the icon at a size in px when no `icons/<icon>.raw` asset exists
    pub icon_image: Option<fn(usize) -> Image>,
}

pub const APPS: &[AppDef] = &[
    AppDef { id: AppId::Settings, name: "Settings", icon: "bifrost.settings", icon_image: None },
    AppDef { id: AppId::Terminal, name: "Term", icon: "is.olibuijr.term", icon_image: Some(term_icon) },
    AppDef { id: AppId::Deildu, name: "Deildu", icon: "is.olibuijr.deildu", icon_image: None },
    AppDef { id: AppId::Photos, name: "Photos", icon: "bifrost.photos", icon_image: None },
    AppDef { id: AppId::Files, name: "Files", icon: "bifrost.files", icon_image: None },
    AppDef { id: AppId::Camera, name: "Camera", icon: "bifrost.camera", icon_image: Some(camera::icon) },
    AppDef { id: AppId::Browser, name: "Browser", icon: "is.olibuijr.browser", icon_image: Some(browser::icon) },
    AppDef { id: AppId::Magnifier, name: "Magnifier", icon: "bifrost.magnifier", icon_image: Some(magnifier::icon) },
    AppDef { id: AppId::AppStore, name: "App Store", icon: "bifrost.appstore", icon_image: Some(store::icon) },
];
/// The Terminal icon at `size` px: BifrOSt's own layered icon (`res/icons/terminal`, the tile from
/// `res/term-icon/icon.svg` with its prompt as a layer), drawn by the shared icon renderer.
pub fn term_icon(size: usize) -> Image {
    bifrost_render::icon_doc::render_builtin(TERM_ICON_JSON, &[("prompt.png", TERM_PROMPT_PNG)], size)
        .unwrap_or_else(|| Image { w: size, h: size, px: vec![0; size * size] })
}

/// BifrOSt's Terminal icon document and its prompt layer (`res/icons/terminal`).
const TERM_ICON_JSON: &str = include_str!("../../res/icons/terminal/icon.json");
const TERM_PROMPT_PNG: &[u8] = include_bytes!("../../res/icons/terminal/prompt.png");

/// Every app on the home screen: the built-in `APPS`, then the installed ones
/// (`apphost::register` must run before the first call to include them).
pub fn apps() -> &'static [AppDef] {
    static ALL: std::sync::RwLock<&'static [AppDef]> = std::sync::RwLock::new(&[]);
    let installed = apphost::installed();
    let desktop = true;
    let files_slot = desktop.then(|| apphost::slot_of("is.olibuijr.files.desktop")).flatten();
    let browser_slot = desktop.then(|| apphost::slot_of("is.olibuijr.browser.desktop")).flatten();
    let browser_index=APPS.iter().position(|a|a.id==AppId::Browser).unwrap();
    let browser_id=browser_slot.map(|i|AppId::External(i as u8)).unwrap_or(AppId::Browser);
    let expected = APPS.len() + installed.len().min(200) - usize::from(files_slot.is_some()) - usize::from(browser_slot.is_some());
    let files_index = APPS.iter().position(|a| a.id == AppId::Files).unwrap();
    let files_id = files_slot.map(|i| AppId::External(i as u8)).unwrap_or(AppId::Files);
    if let Ok(g) = ALL.read() {
        if g.len() == expected && g.get(files_index).is_some_and(|a| a.id == files_id) && g.get(browser_index).is_some_and(|a|a.id==browser_id) {
            return *g;
        }
    }
    // first call, or apps were installed since: rebuild. Strings are interned by
    // content and the slice is only leaked when its contents changed, so repeated
    // rebuilds with the same apps do not grow memory.
    let v = build_list(&installed);
    if let Ok(mut g) = ALL.write() {
        if g.len() == v.len() && g.iter().zip(v.iter()).all(|(x, y)| x.id == y.id && x.name == y.name && x.icon == y.icon) {
            return *g;
        }
        let all: &'static [AppDef] = Box::leak(v.into_boxed_slice());
        *g = all;
        return all;
    }
    Box::leak(v.into_boxed_slice())
}

/// The home-screen list for a set of installed apps: the built-in `APPS`, with the installed Files and Browser
/// taking over their built-in entries (same position and icon key for Files), then every other installed app in
/// slot order. Pure, so the Dock tests can model an install or an in-place update.
pub(super) fn build_list(installed: &[apphost::Installed]) -> Vec<AppDef> {
    let files_slot = installed.iter().position(|a| a.manifest.id == "is.olibuijr.files.desktop");
    let browser_slot = installed.iter().position(|a| a.manifest.id == "is.olibuijr.browser.desktop");
    let browser_index = APPS.iter().position(|a| a.id == AppId::Browser).unwrap();
    let files_index = APPS.iter().position(|a| a.id == AppId::Files).unwrap();
    let mut v = APPS.to_vec();
    if let Some(i) = files_slot { v[files_index].id = AppId::External(i as u8); }
    if let Some(slot) = browser_slot {
        let a = &installed[slot];
        v[browser_index] = AppDef { id: AppId::External(slot as u8), name: intern(&a.manifest.name), icon: intern(&a.manifest.id), icon_image: None };
    }
    for (i, a) in installed.iter().enumerate().take(200) {
        if files_slot == Some(i) || browser_slot == Some(i) { continue; }
        v.push(AppDef { id: AppId::External(i as u8), name: intern(&a.manifest.name), icon: intern(&a.manifest.id), icon_image: None });
    }
    v
}

/// Returns a `'static` copy of `s`, reusing the same allocation for equal strings.
fn intern(s: &str) -> &'static str {
    static STRINGS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut set) = STRINGS.lock() else { return Box::leak(s.to_string().into_boxed_str()) };
    if let Some(found) = set.iter().find(|&&x| x == s) {
        return found;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.push(leaked);
    leaked
}


/// The icon of every app in `list` decoded at `size` px (`apphost::icon_image`, None when the app has no
/// installed icon), in the same order. Each app decodes on its own thread, so the home screen's PNG icons
/// are ready together before the first frame instead of one after another.
pub fn decode_icons(list: &[AppDef], size: usize) -> Vec<Option<Image>> {
    std::thread::scope(|s| {
        let jobs: Vec<_> = list.iter().map(|a| s.spawn(move || apphost::icon_image(a.icon, size))).collect();
        jobs.into_iter().map(|j| j.join().ok().flatten()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_icons_keeps_order_and_length() {
        // no app is registered in a unit test, so every entry decodes to None, one per app, in order
        let icons = decode_icons(APPS, 64);
        assert_eq!(icons.len(), APPS.len());
        assert!(icons.iter().all(Option::is_none));
        assert!(decode_icons(&[], 64).is_empty());
    }

    #[test]
    fn terminal_icon_is_drawn_at_any_size() {
        for size in [32, 100, 160] {
            let img = term_icon(size);
            assert_eq!((img.w, img.h, img.px.len()), (size, size, size * size));
        }
        assert!(APPS.iter().any(|a| a.id == AppId::Terminal && a.icon_image.is_some()));
    }

    #[test]
    fn an_extra_window_belongs_to_its_app_and_is_its_own_desk_key() {
        let w = AppId::ExternalWindow(2, 7);
        assert_eq!(w.parent(), AppId::External(2));
        assert_eq!(AppId::External(2).parent(), AppId::External(2));
        assert_eq!(AppId::Files.parent(), AppId::Files);
        assert!(w.is_extra_window() && !AppId::External(2).is_extra_window());
        assert_eq!((w.window_slot(), w.window_id()), (Some(2), Some(7)));
        assert_eq!((AppId::External(2).window_slot(), AppId::External(2).window_id()), (None, None));
        assert_eq!(AppId::extra_window(2, 7), w);
        // two windows of one app are two desk keys; the same id in another app is another key
        assert_ne!(AppId::extra_window(2, 7), AppId::extra_window(2, 8));
        assert_ne!(AppId::extra_window(2, 7), AppId::extra_window(3, 7));
        assert_ne!(w, AppId::External(2), "a window is not the main window of its app");
    }

    #[test]
    fn app_defs_are_sync_for_the_decode_threads() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<AppDef>();
    }
}
