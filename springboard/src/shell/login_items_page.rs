//! Settings > General > Login Items & Extensions. Three groups sit under the open-at-login list:
//! - START ORDER: the order the login items start in (`Shell::login_items_move`; saved with the list in `login.items`).
//! - ALLOW IN THE BACKGROUND: apps that declare a background agent, with a switch per app. The switches the user
//!   turned off are saved in `login.background.off`.
//! - EXTENSIONS: the app extensions the installed manifests declare, grouped by kind.
//!
//! Manifest tables read here (our own names, read from `manifest.toml` next to the app):
//! `[[extension]]` with `kind = "share" | "quick-look" | "file-provider" | "widget"` and a `name`, and
//! `[[background-agent]]` (the app runs a background agent). Unknown kinds are skipped.
use super::apphost::Installed;
use super::list::{Group, Item};
use super::pages::Action;

/// The pref that holds the app ids whose background agent the user turned off, comma separated.
pub const BACKGROUND_OFF_KEY: &str = "login.background.off";

/// The kinds of app extension the page groups by, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionKind {
    Share,
    Preview,
    FileProvider,
    Widget,
}

impl ExtensionKind {
    /// Every kind, in the order the page lists them.
    pub const ALL: [ExtensionKind; 4] = [ExtensionKind::Share, ExtensionKind::Preview, ExtensionKind::FileProvider, ExtensionKind::Widget];

    /// The manifest spelling of the kind, which the extension key uses.
    pub fn key(self) -> &'static str {
        match self {
            ExtensionKind::Share => "share",
            ExtensionKind::Preview => "quick-look",
            ExtensionKind::FileProvider => "file-provider",
            ExtensionKind::Widget => "widget",
        }
    }

    /// The manifest spelling of the kind (`kind = "..."`).
    pub fn parse(text: &str) -> Option<ExtensionKind> {
        match text.trim() {
            "share" => Some(ExtensionKind::Share),
            "quick-look" => Some(ExtensionKind::Preview),
            "file-provider" => Some(ExtensionKind::FileProvider),
            "widget" => Some(ExtensionKind::Widget),
            _ => None,
        }
    }

    /// The group header of the kind.
    pub fn label(self) -> &'static str {
        match self {
            ExtensionKind::Share => "SHARE",
            ExtensionKind::Preview => "PREVIEW",
            ExtensionKind::FileProvider => "FILE PROVIDER",
            ExtensionKind::Widget => "WIDGETS",
        }
    }
}

/// One extension an installed app declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub app_id: String,
    pub app_name: String,
    pub kind: ExtensionKind,
    /// the extension's own name, shown as the row label
    pub name: String,
}

/// An app that declares a background agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundAgent {
    pub app_id: String,
    pub app_name: String,
}

/// What the page lists beyond the login items: the background agents, their switch state, and the extensions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoginExtensions {
    /// sorted by app name
    pub background: Vec<BackgroundAgent>,
    /// app ids whose background agent is turned off
    pub background_off: Vec<String>,
    /// apps that declare background tasks (`[background] tasks`), sorted by app name
    pub refresh: Vec<BackgroundAgent>,
    /// app ids whose Background App Refresh the user turned off (`background.refresh.off`)
    pub refresh_off: Vec<String>,
    /// sorted by app name, then by kind (install order is kept within an app)
    pub extensions: Vec<Extension>,
    /// the keys (`extension_key`) of the extensions the user turned off (`login.extensions.off`)
    pub extensions_off: Vec<String>,
}

/// The pref that holds the keys of the extensions turned off, comma separated.
pub const EXTENSIONS_OFF_KEY: &str = "login.extensions.off";

/// The key one extension is saved under: its app, its kind and its name. A comma or percent sign in the name is
/// escaped, so the pref list stays comma separated.
pub fn extension_key(e: &Extension) -> String {
    let name = e.name.replace('%', "%25").replace(',', "%2C");
    format!("{}/{}/{}", e.app_id, e.kind.key(), name)
}

/// Whether an extension is on: it is not in the turned-off keys.
pub fn extension_enabled(off: &[String], e: &Extension) -> bool {
    !off.contains(&extension_key(e))
}

/// The empty list, for pages that show no extensions (a `const` so a page model can borrow it).
#[cfg_attr(not(test), allow(dead_code))] // the empty set for tests and previews (a static: a const with Vecs cannot be borrowed for 'static)
pub static NO_EXTENSIONS: LoginExtensions = LoginExtensions { background: Vec::new(), background_off: Vec::new(), refresh: Vec::new(), refresh_off: Vec::new(), extensions: Vec::new(), extensions_off: Vec::new() };

/// The tables of one manifest that this page reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestTables {
    pub background: bool,
    pub extensions: Vec<(ExtensionKind, String)>,
}

#[derive(Default)]
struct Table {
    header: String,
    kind: String,
    name: String,
}

impl Table {
    /// Files the finished table: an `[[extension]]` with a known kind, or a `[[background-agent]]`.
    fn finish(self, out: &mut ManifestTables) {
        match self.header.as_str() {
            "[[extension]]" => {
                if let Some(kind) = ExtensionKind::parse(&self.kind) {
                    out.extensions.push((kind, self.name));
                }
            }
            "[[background-agent]]" => out.background = true,
            _ => {}
        }
    }
}

/// Reads the `[[extension]]` and `[[background-agent]]` tables of a manifest. Other tables are ignored.
pub fn parse_tables(text: &str) -> ManifestTables {
    let mut out = ManifestTables::default();
    let mut table = Table::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            std::mem::take(&mut table).finish(&mut out);
            table.header = line.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "kind" => table.kind = value,
            "name" => table.name = value,
            _ => {}
        }
    }
    table.finish(&mut out);
    out
}

/// Builds the page model from `(app id, app name, manifest text)` of each installed app.
pub fn from_manifests<'a, I>(apps: I) -> LoginExtensions
where
    I: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
{
    let mut out = LoginExtensions::default();
    for (id, name, text) in apps {
        let tables = parse_tables(text);
        if tables.background {
            out.background.push(BackgroundAgent { app_id: id.to_string(), app_name: name.to_string() });
        }
        if !super::services::background::parse_tasks(text).is_empty() {
            out.refresh.push(BackgroundAgent { app_id: id.to_string(), app_name: name.to_string() });
        }
        for (kind, label) in tables.extensions {
            let label = if label.is_empty() { kind.label().to_string() } else { label };
            out.extensions.push(Extension { app_id: id.to_string(), app_name: name.to_string(), kind, name: label });
        }
    }
    out.background.sort_by_key(|b| b.app_name.to_lowercase());
    out.refresh.sort_by_key(|b| b.app_name.to_lowercase());
    out.extensions.sort_by_key(|e| e.app_name.to_lowercase());
    out
}

/// The extension model, kept between touches. The page asks on every touch, but the manifests only change when
/// the installed set does (`apphost::refresh` swaps the snapshot), so the files are read again only then.
#[derive(Default)]
pub struct ExtensionsCache {
    snapshot: Option<std::sync::Arc<[Installed]>>,
    value: LoginExtensions,
}

impl ExtensionsCache {
    pub fn get(&mut self) -> LoginExtensions {
        let now = super::apphost::installed();
        if !self.snapshot.as_ref().is_some_and(|s| std::sync::Arc::ptr_eq(s, &now)) {
            self.value = scan_installed(&now);
            self.snapshot = Some(now);
        }
        self.value.clone()
    }
}

/// The model for the installed apps, read from their manifests. Only `ExtensionsCache` calls it, and only when
/// the installed set changed, so the manifest reads stay off the per-touch path.
fn scan_installed(apps: &[Installed]) -> LoginExtensions {
    let texts: Vec<(String, String, String)> = apps
        .iter()
        .filter_map(|a| std::fs::read_to_string(a.dir.join("manifest.toml")).ok().map(|t| (a.manifest.id.clone(), a.manifest.name.clone(), t)))
        .collect();
    from_manifests(texts.iter().map(|(id, name, text)| (id.as_str(), name.as_str(), text.as_str())))
}

/// The ids in the `login.background.off` pref.
pub fn background_off_from_pref(value: Option<&str>) -> Vec<String> {
    value.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// The `login.background.off` pref value for the ids.
pub fn background_off_to_pref(off: &[String]) -> String {
    off.join(",")
}

/// Whether the background agent of `app_id` may run: its switch is on, so its id is not in the turned-off list.
pub fn background_agent_enabled(off: &[String], app_id: &str) -> bool {
    !off.iter().any(|id| id == app_id)
}

/// The background agents whose switch is on, in the order given. A launcher starts these and skips the rest.
pub fn runnable_background_agents<'a>(agents: &'a [BackgroundAgent], off: &[String]) -> Vec<&'a BackgroundAgent> {
    agents.iter().filter(|a| background_agent_enabled(off, &a.app_id)).collect()
}

/// The START ORDER group: one row per login item in start order, each showing its place. Choosing a row moves that
/// app one place earlier. None when there is nothing to order (fewer than two items).
pub fn order_group(items: &[(String, String)]) -> Option<Group> {
    if items.len() < 2 {
        return None;
    }
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, (id, name))| Item::new(name.as_str()).value(format!("{} of {}", i + 1, items.len())).action(Action::LoginItemMove(id.clone())))
        .collect();
    Some(Group::new(rows).header("START ORDER").footer("Apps start in this order when you log in. Choose an app to move it one place earlier."))
}

/// The ALLOW IN THE BACKGROUND group: a switch per app with a background agent. None when no app has one.
pub fn background_group(ext: &LoginExtensions) -> Option<Group> {
    if ext.background.is_empty() {
        return None;
    }
    let rows = ext.background.iter().map(|b| Item::new(b.app_name.as_str()).switch(!ext.background_off.contains(&b.app_id), Action::BackgroundAgentToggle(b.app_id.clone()))).collect();
    Some(Group::new(rows).header("ALLOW IN THE BACKGROUND").footer("Apps with a background agent may run when no window is open. Turn one off to stop its agent until you turn it back on."))
}

/// The per-app list under Background App Refresh: a switch per app that declares background tasks. The system-wide
/// switch is the page's first group. None when no installed app declares a task.
pub fn refresh_group(ext: &LoginExtensions) -> Option<Group> {
    if ext.refresh.is_empty() {
        return None;
    }
    let rows = ext.refresh.iter().map(|b| Item::new(b.app_name.as_str()).app_icon(b.app_id.as_str()).switch(!ext.refresh_off.contains(&b.app_id), Action::RefreshToggle(b.app_id.clone()))).collect();
    Some(Group::new(rows).header("APPS").footer("Apps with background tasks may refresh their content while no window is open. Turn one off to stop its refresh."))
}

/// The EXTENSIONS groups: one group per kind that some app declares. Each row is the extension's name and its app, with
/// a switch that turns the extension off. With no extensions at all, a single group says so.
pub fn extension_groups(ext: &LoginExtensions) -> Vec<Group> {
    if ext.extensions.is_empty() {
        return vec![Group::new(vec![Item::new("No extensions")]).header("EXTENSIONS").footer("Apps add extensions to the share menu, Preview, file providers and widgets in their manifest.")];
    }
    ExtensionKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let rows: Vec<Item> = ext
                .extensions
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| Item::new(e.name.as_str()).value(e.app_name.as_str()).switch(extension_enabled(&ext.extensions_off, e), Action::ExtensionToggle(extension_key(e))))
                .collect();
            if rows.is_empty() {
                None
            } else {
                Some(Group::new(rows).header(kind.label()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_off_extension_is_keyed_by_app_kind_and_name_with_commas_escaped() {
        let e = Extension { app_id: "is.notes".into(), app_name: "Notes".into(), kind: ExtensionKind::Share, name: "Save, then send".into() };
        assert_eq!(extension_key(&e), "is.notes/share/Save%2C then send");
        assert!(extension_enabled(&[], &e));
        let off = vec![extension_key(&e)];
        assert!(!extension_enabled(&off, &e));
        // the pref list is comma separated, so a key with a comma must survive the round trip
        let saved = background_off_to_pref(&off);
        assert_eq!(background_off_from_pref(Some(&saved)), off);
        let groups = extension_groups(&LoginExtensions { extensions: vec![e], extensions_off: off, ..Default::default() });
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].items.len(), 1);
    }

    fn item(id: &str, name: &str) -> (String, String) {
        (id.to_string(), name.to_string())
    }

    #[test]
    fn one_or_no_login_item_has_no_order_to_set() {
        assert!(order_group(&[]).is_none());
        assert!(order_group(&[item("a", "Alpha")]).is_none());
    }

    #[test]
    fn every_login_item_gets_its_place_in_the_order() {
        let g = order_group(&[item("a", "Alpha"), item("b", "Beta"), item("c", "Gamma")]).expect("three items");
        assert_eq!(g.header.as_deref(), Some("START ORDER"));
        assert_eq!(g.items.len(), 3);
        assert_eq!(g.items[0].label, "Alpha");
        assert_eq!(g.items[0].value, "1 of 3");
        assert_eq!(g.items[2].value, "3 of 3");
    }

    const NOTES: &str = "id = \"is.notes\"\nname = \"Notes\"\nexec = \"notes\"\n\n[[intent]]\nid = \"new\"\ntitle = \"New\"\n\n[[extension]]\nkind = \"share\"\nname = \"Save to Notes\"\n\n[[background-agent]]\nname = \"Sync\"\n";
    const PHOTOS: &str = "id = \"is.photos\"\nname = \"Photos\"\n\n[[extension]]\nkind = \"quick-look\"\nname = \"Photo preview\"\n\n[[extension]]\nkind = \"unknown-kind\"\nname = \"Ignored\"\n\n[[extension]]\nkind = \"widget\"\nname = \"Photo wall\"\n";

    #[test]
    fn manifest_tables_read_only_the_extension_and_agent_tables() {
        let t = parse_tables(NOTES);
        assert!(t.background);
        assert_eq!(t.extensions, vec![(ExtensionKind::Share, "Save to Notes".to_string())]);
        let p = parse_tables(PHOTOS);
        assert!(!p.background);
        assert_eq!(p.extensions.len(), 2, "the unknown kind is skipped");
        assert_eq!(p.extensions[1].0, ExtensionKind::Widget);
    }

    #[test]
    fn the_model_lists_agents_and_extensions_sorted_by_app() {
        let ext = from_manifests([("is.photos", "Photos", PHOTOS), ("is.notes", "Notes", NOTES)]);
        assert_eq!(ext.background, vec![BackgroundAgent { app_id: "is.notes".into(), app_name: "Notes".into() }]);
        assert_eq!(ext.extensions.iter().map(|e| e.app_name.as_str()).collect::<Vec<_>>(), ["Notes", "Photos", "Photos"]);
        assert!(ext.background_off.is_empty());
    }

    #[test]
    fn an_extension_without_a_name_takes_its_kind_label() {
        let ext = from_manifests([("a.b", "Alpha", "[[extension]]\nkind = \"file-provider\"\n")]);
        assert_eq!(ext.extensions[0].name, "FILE PROVIDER");
    }

    #[test]
    fn background_group_has_a_switch_state_per_app_and_none_without_agents() {
        assert!(background_group(&NO_EXTENSIONS).is_none());
        let mut ext = from_manifests([("is.notes", "Notes", NOTES)]);
        let g = background_group(&ext).expect("one agent");
        assert_eq!(g.header.as_deref(), Some("ALLOW IN THE BACKGROUND"));
        assert_eq!(g.items.len(), 1);
        ext.background_off.push("is.notes".into());
        assert!(background_group(&ext).is_some(), "an off agent is still listed");
    }

    #[test]
    fn extension_groups_follow_the_kind_order_and_say_when_there_are_none() {
        let ext = from_manifests([("is.photos", "Photos", PHOTOS), ("is.notes", "Notes", NOTES)]);
        let headers: Vec<String> = extension_groups(&ext).iter().filter_map(|g| g.header.clone()).collect();
        assert_eq!(headers, ["SHARE", "PREVIEW", "WIDGETS"]);
        let none = extension_groups(&NO_EXTENSIONS);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].items[0].label, "No extensions");
    }

    #[test]
    fn background_off_pref_round_trips() {
        assert!(background_off_from_pref(None).is_empty());
        let off = background_off_from_pref(Some(" is.a, ,is.b "));
        assert_eq!(off, ["is.a", "is.b"]);
        assert_eq!(background_off_to_pref(&off), "is.a,is.b");
    }
}

#[cfg(test)]
mod background_gate_tests {
    use super::*;

    fn agent(id: &str) -> BackgroundAgent {
        BackgroundAgent { app_id: id.into(), app_name: id.into() }
    }

    #[test]
    fn a_turned_off_background_agent_does_not_run() {
        let agents = [agent("is.a"), agent("is.b"), agent("is.c")];
        let off = background_off_from_pref(Some("is.b, is.c"));
        assert!(background_agent_enabled(&off, "is.a"));
        assert!(!background_agent_enabled(&off, "is.b"));
        let run: Vec<&str> = runnable_background_agents(&agents, &off).iter().map(|a| a.app_id.as_str()).collect();
        assert_eq!(run, ["is.a"]);
    }

    #[test]
    fn with_no_switch_turned_off_every_agent_runs_in_order() {
        let agents = [agent("is.b"), agent("is.a")];
        let run: Vec<&str> = runnable_background_agents(&agents, &[]).iter().map(|a| a.app_id.as_str()).collect();
        assert_eq!(run, ["is.b", "is.a"]);
    }
}
