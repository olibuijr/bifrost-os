//! One Dock tile per app id. The Dock lays out indices into `apps()` (built-in and installed entries) and, after a
//! divider, Linux clients keyed by their Wayland app id. An in-place update of an installed app (a new version
//! directory, a flipped `current` link, the old process killed) must not add a tile: whatever names the app, its
//! manifest id, its Wayland app id, a `linux:<id>` pin made while the old instance's window looked like a foreign
//! client, resolves to the one entry of that app. Pure over the registry and the entry list, so it is host-tested.

use super::apphost::Installed;
use super::apps::{AppDef, AppId};

/// The compositor id the shell gives its own windows (`is.olibuijr.bifrost.<frame id>`): never an app of its own.
const OWN_WINDOW_PREFIX: &str = "is.olibuijr.bifrost.";

/// An id compared loosely: case and a trailing `.desktop` (the package suffix) do not tell two apps apart.
fn norm(id: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    lower.strip_suffix(".desktop").map(str::to_string).unwrap_or(lower)
}

/// The icon key of the `defs` entry that stands for `id` (an icon key, a manifest id, a Wayland app id), if any.
pub(super) fn owner_icon(installed: &[Installed], defs: &[AppDef], id: &str) -> Option<&'static str> {
    let n = norm(id);
    if n.is_empty() {
        return None;
    }
    if let Some(d) = defs.iter().find(|d| d.icon == id) {
        return Some(d.icon);
    }
    for (slot, a) in installed.iter().enumerate() {
        let m = &a.manifest;
        if norm(&m.id) == n || (!m.wayland_app_id.is_empty() && norm(&m.wayland_app_id) == n) {
            if let Some(d) = defs.iter().find(|d| d.id == AppId::External(slot as u8)) {
                return Some(d.icon);
            }
        }
    }
    defs.iter().find(|d| norm(d.icon) == n).map(|d| d.icon)
}

/// True when a compositor window with `app_id` is a Linux client of its own: not a window of the shell, of an
/// installed app (even while its process winds down) or of a built-in app.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn is_linux_app(installed: &[Installed], defs: &[AppDef], app_id: &str) -> bool {
    !app_id.starts_with(OWN_WINDOW_PREFIX) && owner_icon(installed, defs, app_id).is_none()
}

/// Rewrites the pinned keys so each app is kept once under its entry's icon key: a `linux:<id>` or a stale key that
/// resolves to an app becomes that app's key (a pin of the shell's own window is dropped), repeats are removed, other
/// keys (a real Linux client, an app that is not installed right now) are kept. True when the list changed.
pub(super) fn normalize_pinned(installed: &[Installed], defs: &[AppDef], pinned: &mut Vec<String>) -> bool {
    let mut out: Vec<String> = Vec::with_capacity(pinned.len());
    for key in pinned.iter() {
        let resolved = match key.strip_prefix("linux:") {
            Some(id) if id.starts_with(OWN_WINDOW_PREFIX) => continue,
            Some(id) => owner_icon(installed, defs, id).map(str::to_string),
            None if defs.iter().any(|d| d.icon == key) => None,
            None => owner_icon(installed, defs, key).map(str::to_string),
        };
        let key = resolved.unwrap_or_else(|| key.clone());
        if !out.contains(&key) {
            out.push(key);
        }
    }
    let changed = out != *pinned;
    *pinned = out;
    changed
}

/// The `defs` entries the Dock shows: those kept in the Dock in their order, then the running ones. An app is one
/// tile however many entries or keys name it (same icon key or same id), and a running entry of a kept app adds no
/// tile (the kept tile shows the running dot).
pub(super) fn dock_app_tiles(defs: &[AppDef], pinned: &[String], running: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut list: Vec<usize> = Vec::new();
    let has = |list: &[usize], i: usize| list.iter().any(|&j| defs[j].icon == defs[i].icon || defs[j].id == defs[i].id);
    for key in pinned {
        if let Some(i) = defs.iter().position(|a| a.icon == key) {
            if !has(&list, i) {
                list.push(i);
            }
        }
    }
    for i in 0..defs.len() {
        if running(i) && !has(&list, i) {
            list.push(i);
        }
    }
    list
}

/// The positions in a list of Linux clients (by app id) that get a tile of their own: one per id, none for an app
/// the Dock already shows under its own entry.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn linux_tiles(installed: &[Installed], defs: &[AppDef], app_ids: &[&str]) -> Vec<usize> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (i, id) in app_ids.iter().enumerate() {
        if seen.contains(id) || !is_linux_app(installed, defs, id) {
            continue;
        }
        seen.push(id);
        out.push(i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::apphost::{merge_installed, Manifest};
    use super::super::apps::{build_list, APPS};
    use super::*;
    use std::path::PathBuf;

    fn pkg(id: &str, version: &str) -> Installed {
        let manifest = Manifest { id: id.into(), version: version.into(), exec: "app".into(), name: id.into(), ..Manifest::default() };
        Installed { manifest, dir: PathBuf::from(format!("/var/lib/springboard/apps/{id}/{version}")) }
    }

    fn tiles_of(installed: &[Installed], pinned: &[String], running: &[&str]) -> Vec<&'static str> {
        let defs = build_list(installed);
        let is_running = |i: usize| running.contains(&defs[i].icon);
        dock_app_tiles(&defs, pinned, is_running).into_iter().map(|i| defs[i].icon).collect()
    }

    fn default_pins(installed: &[Installed]) -> Vec<String> {
        build_list(installed).iter().map(|a| a.icon.to_string()).collect()
    }

    /// Installing v1, keeping it in the Dock, then installing v2 over it (with or without v1 running) is one tile.
    #[test]
    fn an_update_in_place_keeps_one_tile() {
        let v1 = merge_installed(&[], vec![pkg("is.olibuijr.notes.desktop", "1"), pkg("is.olibuijr.console.desktop", "1")]);
        let mut pins = default_pins(&v1);
        pins.push("is.olibuijr.notes.desktop".into()); // a key kept twice
        for running in [&[][..], &["is.olibuijr.notes.desktop"][..]] {
            let before = tiles_of(&v1, &pins, running);
            assert_eq!(before.iter().filter(|&&k| k == "is.olibuijr.notes.desktop").count(), 1);
            // v2 lands in a new version directory; the registry keeps the slot and swaps the record
            let v2 = merge_installed(&v1, vec![pkg("is.olibuijr.notes.desktop", "2"), pkg("is.olibuijr.console.desktop", "2")]);
            assert_eq!(v2.len(), v1.len());
            assert_eq!(tiles_of(&v2, &pins, running), before, "an update adds no tile (running: {running:?})");
        }
    }

    /// A Linux tile pinned while an installed app's window looked foreign, a stale key and a repeat all become the
    /// app's one key; the app's windows never make a Linux tile.
    #[test]
    fn a_stale_or_linux_key_of_an_installed_app_is_the_apps_own_tile() {
        let installed = merge_installed(&[], vec![pkg("is.olibuijr.notes.desktop", "2")]);
        let defs = build_list(&installed);
        let mut pins: Vec<String> =
            ["bifrost.settings", "linux:is.olibuijr.notes", "is.olibuijr.notes.desktop", "linux:is.olibuijr.bifrost.7", "linux:is.olibuijr.notes.desktop", "linux:firefox", "bifrost.settings"]
                .map(String::from)
                .to_vec();
        assert!(normalize_pinned(&installed, &defs, &mut pins));
        assert_eq!(pins, vec!["bifrost.settings", "is.olibuijr.notes.desktop", "linux:firefox"]);
        assert!(!normalize_pinned(&installed, &defs, &mut pins), "idempotent");
        assert!(!is_linux_app(&installed, &defs, "is.olibuijr.notes.desktop"));
        assert!(!is_linux_app(&installed, &defs, "is.olibuijr.notes"), "the package suffix does not make a second app");
        assert!(!is_linux_app(&installed, &defs, "is.olibuijr.bifrost.3"));
        assert!(!is_linux_app(&installed, &defs, "bifrost.camera"), "a built-in app is not a Linux client");
        assert!(is_linux_app(&installed, &defs, "firefox"));
        assert_eq!(linux_tiles(&installed, &defs, &["firefox", "is.olibuijr.notes", "firefox", "foot"]), vec![0, 3]);
    }

    /// The installed Files keeps the built-in Files key; a pin by its manifest id is migrated onto it.
    #[test]
    fn the_installed_files_app_replaces_the_built_in_tile() {
        let installed = merge_installed(&[], vec![pkg("is.olibuijr.files.desktop", "1")]);
        let defs = build_list(&installed);
        let files = APPS.iter().find(|a| a.id == AppId::Files).unwrap().icon;
        let mut pins = vec![files.to_string(), "is.olibuijr.files.desktop".to_string()];
        normalize_pinned(&installed, &defs, &mut pins);
        assert_eq!(pins, vec![files.to_string()]);
        assert_eq!(tiles_of(&installed, &pins, &["is.olibuijr.files.desktop"]).len(), 1);
    }

    /// A running instance of a kept app is the kept tile (it shows the dot), not a tile beside it; an app that is
    /// not kept gets one running tile; two entries with one icon key are one tile whichever of them runs.
    #[test]
    fn a_running_instance_shares_the_pinned_tile() {
        let installed = merge_installed(&[], vec![pkg("is.olibuijr.notes.desktop", "1"), pkg("is.olibuijr.mail.desktop", "1")]);
        let pins = vec!["is.olibuijr.notes.desktop".to_string()];
        assert_eq!(tiles_of(&installed, &pins, &[]), vec!["is.olibuijr.notes.desktop"]);
        assert_eq!(tiles_of(&installed, &pins, &["is.olibuijr.notes.desktop"]), vec!["is.olibuijr.notes.desktop"]);
        assert_eq!(tiles_of(&installed, &pins, &["is.olibuijr.mail.desktop"]), vec!["is.olibuijr.notes.desktop", "is.olibuijr.mail.desktop"]);
        // a registry that lists one icon key twice still gives one tile, and the second entry running counts
        let mut defs = build_list(&installed);
        let twin = AppDef { id: AppId::External(9), ..defs[defs.len() - 1] };
        defs.push(twin);
        let last = defs.len() - 1;
        let list = dock_app_tiles(&defs, &[defs[last].icon.to_string()], |i| i == last);
        assert_eq!(list.len(), 1);
    }
}
