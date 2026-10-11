//! App extensions: an app ships a scene, a custom UI that another app presents inside its own window. The host asks the
//! registry what it may present for an extension point, and the user sees one browser list per host to approve, enable or
//! disable each extension. Nothing is enabled until the user turns it on.
//!
//! The registry keeps the user's choice per (app, scene) even while that scene is not installed, so the choice survives an
//! update or a restart: the shell stores [`Registry::to_lines`] in its prefs and loads it with [`Registry::apply_lines`].
//! Pure data, no I/O. This models what the host may present, not out-of-process hosting: a scene here is a declaration.

use std::collections::BTreeMap;

/// A scene an extension offers: the extension point it plugs into, its identifier within the shipping app, and the title
/// the browser list shows. Points and identifiers are lowercase tokens (letters, digits, `.`, `-`, `_`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub point: String,
    pub identifier: String,
    pub title: String,
}

fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
}

/// Normalises a lookup key: the same spelling a [`Scene`] stores.
fn key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// App ids never contain whitespace or control characters, so they can sit in a tab-separated line.
fn is_app_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Scene {
    /// `None` when the point or identifier is not a token, or the title is empty.
    pub fn new(point: &str, identifier: &str, title: &str) -> Option<Scene> {
        let point = key(point);
        let identifier = key(identifier);
        let title = title.trim();
        if !valid_token(&point) || !valid_token(&identifier) || title.is_empty() {
            return None;
        }
        Some(Scene { point, identifier, title: title.into() })
    }
}

/// A scene one app declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub app: String,
    pub scene: Scene,
}

/// One row of the browser list: the extension and whether the user has it enabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub extension: &'a Extension,
    pub enabled: bool,
}

/// One browser section: every declared scene for one extension point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section<'a> {
    pub point: String,
    pub entries: Vec<Entry<'a>>,
}

/// What a host shows for an extension point: the enabled scene, or its placeholder when none is enabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presentation<'a> {
    Remote(&'a Extension),
    Placeholder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    /// Declared scenes, in declaration order.
    declared: Vec<Extension>,
    /// The user's choice per (app, scene identifier). Kept even for a scene that is not declared right now.
    choices: BTreeMap<(String, String), bool>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a scene `app` ships. Declaring a scene again replaces it (its title may have changed) and keeps the user's
    /// choice. `false` when the app id is not usable.
    pub fn declare(&mut self, app: &str, scene: Scene) -> bool {
        let app = app.trim();
        if !is_app_id(app) {
            return false;
        }
        match self.declared.iter().position(|e| e.app == app && e.scene.identifier == scene.identifier) {
            Some(i) => self.declared[i].scene = scene,
            None => self.declared.push(Extension { app: app.into(), scene }),
        }
        true
    }

    /// Drops a declared scene, for example when the app is removed. The user's choice stays for a later declaration.
    pub fn remove(&mut self, app: &str, identifier: &str) -> bool {
        let (app, identifier) = (app.trim(), key(identifier));
        let before = self.declared.len();
        self.declared.retain(|e| !(e.app == app && e.scene.identifier == identifier));
        self.declared.len() != before
    }

    /// Records the user's choice. It applies as soon as the scene is declared.
    pub fn set_enabled(&mut self, app: &str, identifier: &str, enabled: bool) {
        self.choices.insert((app.trim().into(), key(identifier)), enabled);
    }

    /// Extensions are off unless the user has enabled them.
    pub fn is_enabled(&self, app: &str, identifier: &str) -> bool {
        self.choices.get(&(app.trim().to_string(), key(identifier))).copied().unwrap_or(false)
    }

    pub fn declared(&self) -> &[Extension] {
        &self.declared
    }

    /// The enabled scenes for one extension point, in declaration order.
    pub fn enabled_for(&self, point: &str) -> Vec<&Extension> {
        let point = key(point);
        self.declared
            .iter()
            .filter(|e| e.scene.point == point && self.is_enabled(&e.app, &e.scene.identifier))
            .collect()
    }

    /// What a host presents for `point`: the first enabled scene in declaration order, or the placeholder.
    pub fn present(&self, point: &str) -> Presentation<'_> {
        match self.enabled_for(point).into_iter().next() {
            Some(e) => Presentation::Remote(e),
            None => Presentation::Placeholder,
        }
    }

    /// The browser list for a host that accepts `points`: one section per point in the given order, each scene sorted by
    /// title (case-insensitive, then identifier), enabled and disabled alike. A point with no declared scene has no section.
    pub fn browser(&self, points: &[&str]) -> Vec<Section<'_>> {
        let mut sections = Vec::new();
        for &point in points {
            let point = key(point);
            let mut entries: Vec<Entry> = self
                .declared
                .iter()
                .filter(|e| e.scene.point == point)
                .map(|e| Entry { extension: e, enabled: self.is_enabled(&e.app, &e.scene.identifier) })
                .collect();
            if entries.is_empty() {
                continue;
            }
            entries.sort_by(|a, b| {
                let (x, y) = (&a.extension.scene, &b.extension.scene);
                x.title.to_lowercase().cmp(&y.title.to_lowercase()).then_with(|| x.identifier.cmp(&y.identifier))
            });
            sections.push(Section { point, entries });
        }
        sections
    }

    /// The user's choices as `app<TAB>identifier<TAB>1|0` lines, for the shell's prefs.
    pub fn to_lines(&self) -> String {
        let mut out = String::new();
        for ((app, identifier), on) in &self.choices {
            out.push_str(&format!("{app}\t{identifier}\t{}\n", if *on { 1 } else { 0 }));
        }
        out
    }

    /// Loads choices written by [`Registry::to_lines`]. Malformed lines are skipped.
    pub fn apply_lines(&mut self, text: &str) {
        for line in text.lines() {
            let mut fields = line.split('\t');
            let (Some(app), Some(identifier), Some(on), None) = (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let on = match on {
                "1" => true,
                "0" => false,
                _ => continue,
            };
            let identifier = key(identifier);
            if is_app_id(app) && valid_token(&identifier) {
                self.choices.insert((app.into(), identifier), on);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(point: &str, identifier: &str, title: &str) -> Scene {
        Scene::new(point, identifier, title).unwrap()
    }

    #[test]
    fn scenes_are_validated_and_normalised() {
        let s = Scene::new(" Share.Preview ", "Quick-Note", "  Quick Note ").unwrap();
        assert_eq!((s.point.as_str(), s.identifier.as_str(), s.title.as_str()), ("share.preview", "quick-note", "Quick Note"));
        assert!(Scene::new("", "a", "A").is_none(), "empty point");
        assert!(Scene::new("share", "has space", "A").is_none(), "identifier is a token");
        assert!(Scene::new("share", "a", "   ").is_none(), "empty title");
    }

    #[test]
    fn a_new_extension_is_off_until_the_user_enables_it() {
        let mut r = Registry::new();
        assert!(r.declare("notes", scene("share.preview", "quick", "Quick Note")));
        assert!(r.enabled_for("share.preview").is_empty());
        assert_eq!(r.present("share.preview"), Presentation::Placeholder);
        r.set_enabled("notes", "quick", true);
        match r.present("share.preview") {
            Presentation::Remote(e) => assert_eq!(e.app, "notes"),
            other => panic!("expected the enabled scene, got {other:?}"),
        }
        assert!(!r.declare("", scene("share.preview", "x", "X")), "empty app id is refused");
    }

    #[test]
    fn the_choice_outlives_an_uninstalled_scene() {
        let mut r = Registry::new();
        r.set_enabled("notes", "quick", true);
        r.declare("notes", scene("share.preview", "quick", "Quick Note"));
        assert_eq!(r.enabled_for("share.preview").len(), 1, "a choice made before the declaration applies");
        assert!(r.remove("notes", "quick"));
        assert!(r.enabled_for("share.preview").is_empty());
        assert!(!r.remove("notes", "quick"), "already removed");
        r.declare("notes", scene("share.preview", "quick", "Quick Note"));
        assert!(r.is_enabled("notes", "quick"), "reinstalling keeps the choice");
    }

    #[test]
    fn redeclaring_updates_the_title_and_keeps_the_choice() {
        let mut r = Registry::new();
        r.declare("notes", scene("share.preview", "quick", "Quick Note"));
        r.set_enabled("notes", "quick", true);
        r.declare("notes", scene("share.preview", "quick", "Quick Memo"));
        assert_eq!(r.declared().len(), 1);
        assert_eq!(r.declared()[0].scene.title, "Quick Memo");
        assert!(r.is_enabled("notes", "quick"));
    }

    #[test]
    fn present_picks_the_first_enabled_scene_in_declaration_order() {
        let mut r = Registry::new();
        r.declare("notes", scene("share.preview", "first", "First"));
        r.declare("mail", scene("share.preview", "second", "Second"));
        r.set_enabled("mail", "second", true);
        match r.present("share.preview") {
            Presentation::Remote(e) => assert_eq!(e.scene.identifier, "second"),
            other => panic!("expected the second scene, got {other:?}"),
        }
        r.set_enabled("notes", "first", true);
        match r.present("share.preview") {
            Presentation::Remote(e) => assert_eq!(e.scene.identifier, "first", "declaration order wins"),
            other => panic!("expected the first scene, got {other:?}"),
        }
        assert_eq!(r.present("other.point"), Presentation::Placeholder);
    }

    #[test]
    fn browser_lists_points_in_order_with_disabled_scenes_sorted_by_title() {
        let mut r = Registry::new();
        r.declare("notes", scene("share.preview", "b", "beta"));
        r.declare("notes", scene("share.preview", "a", "Alpha"));
        r.declare("notes", scene("quick.action", "c", "Gamma"));
        r.set_enabled("notes", "a", true);
        let sections = r.browser(&["share.preview", "quick.action", "missing.point"]);
        assert_eq!(sections.len(), 2, "a point with no scene has no section");
        assert_eq!(sections[0].point, "share.preview");
        let titles: Vec<&str> = sections[0].entries.iter().map(|e| e.extension.scene.title.as_str()).collect();
        assert_eq!(titles, ["Alpha", "beta"]);
        assert!(sections[0].entries[0].enabled);
        assert!(!sections[0].entries[1].enabled, "disabled scenes stay in the list");
        assert_eq!(sections[1].point, "quick.action");
    }

    #[test]
    fn choices_round_trip_through_lines_and_malformed_lines_are_skipped() {
        let mut r = Registry::new();
        r.set_enabled("notes", "quick", true);
        r.set_enabled("mail", "archive", false);
        let text = r.to_lines();
        let mut loaded = Registry::new();
        loaded.apply_lines(&format!("bad line\na\tb\t2\n\t\t1\n{text}"));
        assert!(loaded.is_enabled("notes", "quick"));
        assert!(!loaded.is_enabled("mail", "archive"));
        assert!(!loaded.is_enabled("a", "b"), "a value other than 0 or 1 is skipped");
        assert_eq!(loaded.to_lines(), text);
    }
}
