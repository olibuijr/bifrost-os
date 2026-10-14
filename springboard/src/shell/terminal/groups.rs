//! Window groups: named sets of Terminal tabs. A tab keeps its folder, the profile in use when it was saved and a command
//! typed into it when the group opens. The groups live in the `terminal.window_groups` pref as JSON, newest first. Open
//! window group at launch opens the first one.
use super::*;
use bifrost_core::json::{self, Json};

/// The pref that holds the saved groups.
pub const KEY: &str = "terminal.window_groups";
/// Most groups kept; saving past this drops the oldest.
pub const MAX_GROUPS: usize = 32;
/// Longest group name kept.
const MAX_NAME: usize = 64;

/// One tab of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub dir: String,
    /// the profile to use (an index into the profiles; None is the numbered theme)
    pub profile: Option<usize>,
    /// a command typed into the tab once its shell is in the folder (empty: none)
    pub command: String,
}

/// A named group of tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub tabs: Vec<Tab>,
}

/// The groups as the pref's JSON text.
pub fn encode(groups: &[Group]) -> String {
    Json::Arr(
        groups
            .iter()
            .map(|group| {
                Json::Obj(vec![
                    ("name".to_string(), Json::Str(group.name.clone())),
                    (
                        "tabs".to_string(),
                        Json::Arr(
                            group
                                .tabs
                                .iter()
                                .map(|tab| {
                                    Json::Obj(vec![
                                        ("dir".to_string(), Json::Str(tab.dir.clone())),
                                        ("profile".to_string(), tab.profile.map_or(Json::Null, |p| Json::Num(p as f64))),
                                        ("command".to_string(), Json::Str(tab.command.clone())),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
    .encode()
}

/// The groups of the pref's JSON text. Text that is not a list of groups gives none, and a broken entry is skipped.
pub fn decode(text: &str) -> Vec<Group> {
    let Ok(json) = json::parse(text) else { return Vec::new() };
    json.as_array()
        .unwrap_or(&[])
        .iter()
        .filter_map(|group| {
            let name = group.str_field("name")?;
            let tabs = group
                .get("tabs")?
                .as_array()?
                .iter()
                .filter_map(|tab| {
                    Some(Tab {
                        dir: tab.str_field("dir")?,
                        profile: tab.get("profile").and_then(Json::number).map(|p| p as usize),
                        command: tab.str_field("command").unwrap_or_default(),
                    })
                })
                .collect();
            Some(Group { name, tabs })
        })
        .collect()
}

/// The group a name types as: blanks trimmed, at most 64 characters, no control characters. Err says why.
pub fn group_name(typed: &str) -> Result<String, String> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("Type a name for the window group".into());
    }
    if name.chars().any(char::is_control) {
        return Err("A name cannot hold control characters".into());
    }
    Ok(name.chars().take(MAX_NAME).collect())
}

/// `groups` with `group` at the front: a group of the same name is replaced, and the list is kept to `MAX_GROUPS`.
pub fn saved_with(mut groups: Vec<Group>, group: Group) -> Vec<Group> {
    groups.retain(|g| g.name != group.name);
    groups.insert(0, group);
    groups.truncate(MAX_GROUPS);
    groups
}

impl Shell {
    /// The saved groups, newest first.
    pub(super) fn terminal_groups(&self) -> Vec<Group> {
        decode(self.prefs.get(KEY).unwrap_or(""))
    }

    /// Saves the open tabs as the group `name`: each keeps its folder, and all keep the profile in use. Tabs whose folder
    /// is not known are left out.
    pub(super) fn terminal_save_group_named(&mut self, name: &str) -> String {
        let profile = self.terminal_ui.profile;
        let tabs: Vec<Tab> = self
            .term
            .iter()
            .filter_map(|tab| tab.cwd().map(|dir| Tab { dir, profile, command: String::new() }))
            .collect();
        if tabs.is_empty() {
            return "No tab folders to save".into();
        }
        let count = tabs.len();
        let groups = saved_with(self.terminal_groups(), Group { name: name.to_string(), tabs });
        match self.prefs.set(KEY, &encode(&groups)) {
            Ok(()) => format!("Saved {count} tab{} as {name}", if count == 1 { "" } else { "s" }),
            Err(e) => format!("Could not save window group: {e}"),
        }
    }

    /// Opens the tabs of group `index`: the profile of its first tab is applied, each tab opens in its folder and runs its
    /// command. False when there is no such group.
    pub(super) fn terminal_open_group_tabs(&mut self, index: usize) -> bool {
        let Some(group) = self.terminal_groups().get(index).cloned() else { return false };
        if let Some(first) = group.tabs.first() {
            self.terminal_ui.profile = first.profile;
            self.terminal_appearance();
        }
        for tab in &group.tabs {
            if self.new_terminal_tab(Some(tab.dir.clone())) && !tab.command.is_empty() {
                if let Some(term) = self.term.as_mut() {
                    term.write(format!("{}\r", tab.command).as_bytes());
                }
            }
        }
        true
    }

    /// Removes group `index` from the saved list.
    pub(super) fn terminal_delete_group(&mut self, index: usize) -> String {
        let mut groups = self.terminal_groups();
        if index >= groups.len() {
            return "No window group saved".into();
        }
        let gone = groups.remove(index);
        match self.prefs.set(KEY, &encode(&groups)) {
            Ok(()) => format!("Deleted {}", gone.name),
            Err(e) => format!("Could not delete window group: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, dirs: &[&str]) -> Group {
        Group {
            name: name.to_string(),
            tabs: dirs
                .iter()
                .map(|dir| Tab { dir: dir.to_string(), profile: Some(2), command: "ls".into() })
                .collect(),
        }
    }

    #[test]
    fn groups_round_trip_with_folders_profiles_and_commands() {
        let mut odd = group("Work \"two\"\tline", &["/home/a b", "/tmp/with\ttab", "/x\\y"]);
        odd.tabs[1].profile = None;
        odd.tabs[2].command = String::new();
        let groups = vec![group("Dev", &["/src"]), odd];
        let text = encode(&groups);
        assert!(!text.contains('\n') && !text.contains('\r'), "one pref line");
        assert_eq!(decode(&text), groups);
    }

    #[test]
    fn broken_or_empty_text_gives_no_groups_and_broken_entries_are_skipped() {
        assert!(decode("").is_empty());
        assert!(decode("not json").is_empty());
        assert!(decode("{\"name\":1}").is_empty(), "a group is a list");
        let text = "[{\"name\":\"ok\",\"tabs\":[{\"dir\":\"/a\"},{\"command\":\"x\"}]},{\"tabs\":[]}]";
        let decoded = decode(text);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].tabs, vec![Tab { dir: "/a".into(), profile: None, command: String::new() }]);
    }

    #[test]
    fn a_saved_name_replaces_its_group_and_the_newest_comes_first() {
        let groups = vec![group("A", &["/a"]), group("B", &["/b"])];
        let saved = saved_with(groups, group("B", &["/b2"]));
        assert_eq!(saved.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["B", "A"]);
        assert_eq!(saved[0].tabs[0].dir, "/b2");
        let mut many = Vec::new();
        for i in 0..(MAX_GROUPS + 3) {
            many = saved_with(many, group(&format!("g{i}"), &["/x"]));
        }
        assert_eq!(many.len(), MAX_GROUPS);
        assert_eq!(many[0].name, format!("g{}", MAX_GROUPS + 2));
    }

    #[test]
    fn a_group_name_is_trimmed_and_short() {
        assert_eq!(group_name("  Work  ").as_deref(), Ok("Work"));
        assert!(group_name("   ").is_err());
        assert!(group_name("a\nb").is_err());
        assert_eq!(group_name(&"x".repeat(80)).map(|n| n.chars().count()), Ok(64));
    }
}
