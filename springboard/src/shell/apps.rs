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
    AppDef { id: AppId::Terminal, name: "Term", icon: "is.olibuijr.term", icon_image: None },
    AppDef { id: AppId::Deildu, name: "Deildu", icon: "is.olibuijr.deildu", icon_image: None },
    AppDef { id: AppId::Photos, name: "Photos", icon: "bifrost.photos", icon_image: None },
    AppDef { id: AppId::Files, name: "Files", icon: "bifrost.files", icon_image: None },
    AppDef { id: AppId::Camera, name: "Camera", icon: "bifrost.camera", icon_image: Some(camera::icon) },
    AppDef { id: AppId::Browser, name: "Browser", icon: "is.olibuijr.browser", icon_image: Some(browser::icon) },
    AppDef { id: AppId::Magnifier, name: "Magnifier", icon: "bifrost.magnifier", icon_image: Some(magnifier::icon) },
    AppDef { id: AppId::AppStore, name: "App Store", icon: "bifrost.appstore", icon_image: Some(store::icon) },
];
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
    let mut v = APPS.to_vec();
    v[files_index].id = files_id;
    if let Some(slot)=browser_slot {
        let a=&installed[slot];
        v[browser_index]=AppDef {id:browser_id,name:intern(&a.manifest.name),icon:intern(&a.manifest.id),icon_image:None};
    }
    for (i, a) in installed.iter().enumerate().take(200) {
        if files_slot == Some(i) || browser_slot == Some(i) { continue; }
        v.push(AppDef { id: AppId::External(i as u8), name: intern(&a.manifest.name), icon: intern(&a.manifest.id), icon_image: None });
    }
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

