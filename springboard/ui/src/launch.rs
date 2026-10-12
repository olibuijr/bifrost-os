//! Default applications: which app opens a file type or a URL scheme, and which app acts as viewer, editor or shell.
//!
//! Apps declare what they can open, as Golden Gate apps do in their bundle's document types (content type with a
//! handler rank) and URL types. The registry answers `urlForApplication(toOpen:)`, lists every candidate
//! for an "Open With" menu, and keeps the user's explicit choice per type (`setDefaultApplication(at:toOpenContentType:)`,
//! Files's Get Info > Open with > Change All). Without a choice the best declaration wins: the most specific type,
//! then the higher rank, then the app registered first. Role handlers (`Role`) keep one default per
//! type and role, so an editor can differ from the viewer. Pure data, no I/O: the shell stores `to_lines` in its prefs.
use crate::uttype;
use std::cell::RefCell;
use std::collections::HashMap;

/// The roles an app can take on an item: view it, edit it, or act as the shell for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Editor,
    Shell,
}

impl Role {
    /// The word used in the preferences file.
    pub fn name(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Shell => "shell",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s.trim().to_ascii_lowercase().as_str() {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "shell" => Some(Role::Shell),
            _ => None,
        }
    }
}

/// `LSHandlerRank`. `None` means the app can open the type but never becomes the default by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    None,
    Alternate,
    Default,
    Owner,
}

impl Rank {
    pub fn parse(s: &str) -> Option<Rank> {
        match s.trim().to_ascii_lowercase().as_str() {
            "owner" => Some(Rank::Owner),
            "default" => Some(Rank::Default),
            "alternate" => Some(Rank::Alternate),
            "none" => Some(Rank::None),
            _ => None,
        }
    }
}

/// What one app declares it can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handler {
    pub app: String,
    /// (uniform type identifier, rank)
    pub types: Vec<(String, Rank)>,
    /// URL schemes without the colon ("https", "mailto")
    pub schemes: Vec<String>,
    /// (uniform type identifier, role): roles the app can take on for a type. Separate from `types`.
    pub roles: Vec<(String, Role)>,
}

impl Handler {
    pub fn new(app: &str) -> Self {
        Handler { app: app.into(), types: Vec::new(), schemes: Vec::new(), roles: Vec::new() }
    }
    pub fn opens(mut self, uti: &str, rank: Rank) -> Self {
        self.types.push((uti.into(), rank));
        self
    }
    /// Declares that the app can act as `role` for `uti` (and its subtypes).
    pub fn role(mut self, role: Role, uti: &str) -> Self {
        self.roles.push((uti.into(), role));
        self
    }
    pub fn scheme(mut self, s: &str) -> Self {
        self.schemes.push(s.to_ascii_lowercase());
        self
    }
}

/// One row of an Open With menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub app: String,
    pub is_default: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SetError {
    /// The app is not registered, or does not declare the type.
    CannotOpen,
}

#[derive(Default, Clone)]
pub struct Registry {
    handlers: Vec<Handler>,
    types: HashMap<String, String>,
    schemes: HashMap<String, String>,
    /// (type, role) -> the user's chosen app for that role
    roles: HashMap<(String, Role), String>,
    /// type -> index of its default handler; cleared by every change (a lookup walks the type tree for each app)
    resolved: RefCell<HashMap<String, Option<usize>>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Registers (or replaces, keeping its place in the order) an app's declarations.
    pub fn register(&mut self, h: Handler) {
        self.resolved.get_mut().clear();
        match self.handlers.iter_mut().find(|x| x.app == h.app) {
            Some(slot) => *slot = h,
            None => self.handlers.push(h),
        }
    }

    /// An uninstalled app disappears from every list; a user choice naming it falls back to the declarations.
    pub fn unregister(&mut self, app: &str) {
        self.resolved.get_mut().clear();
        self.handlers.retain(|h| h.app != app);
    }

    pub fn is_registered(&self, app: &str) -> bool {
        self.handlers.iter().any(|h| h.app == app)
    }

    /// Every app that can open `uti` (`urlsForApplicationsToOpenContentType:`), best first: most specific declaration,
    /// then rank, then registration order. An app appears once, by its best declaration.
    pub fn apps_for_type(&self, uti: &str) -> Vec<&str> {
        let mut found: Vec<(usize, std::cmp::Reverse<Rank>, usize, &str)> = Vec::new();
        for (order, h) in self.handlers.iter().enumerate() {
            let best = h.types.iter().filter_map(|(t, r)| uttype::distance(uti, t).map(|d| (d, std::cmp::Reverse(*r)))).min();
            if let Some((d, r)) = best {
                found.push((d, r, order, h.app.as_str()));
            }
        }
        found.sort();
        found.into_iter().map(|f| f.3).collect()
    }

    /// `urlForApplication(toOpen:)` for a type: the user's choice when that app still can open it, else the best
    /// declaration whose rank is not `None`.
    pub fn default_for_type(&self, uti: &str) -> Option<&str> {
        if let Some(hit) = self.resolved.borrow().get(uti) {
            return hit.map(|i| self.handlers[i].app.as_str());
        }
        let apps = self.apps_for_type(uti);
        let chosen = self.types.get(uti).and_then(|a| apps.iter().find(|x| *x == a).copied());
        let app = chosen.or_else(|| apps.into_iter().find(|a| self.rank_for(a, uti) > Rank::None));
        let index = app.and_then(|a| self.handlers.iter().position(|h| h.app == a));
        self.resolved.borrow_mut().insert(uti.to_string(), index);
        index.map(|i| self.handlers[i].app.as_str())
    }

    fn rank_for(&self, app: &str, uti: &str) -> Rank {
        self.handlers.iter().filter(|h| h.app == app).flat_map(|h| h.types.iter()).filter(|(t, _)| uttype::distance(uti, t).is_some()).map(|(_, r)| *r).max().unwrap_or(Rank::None)
    }

    /// The default app for a file name, by its extension.
    pub fn default_for_file(&self, name: &str) -> Option<&str> {
        self.default_for_type(uttype::type_for_extension(name.rsplit_once('.').map_or("", |(_, e)| e))?)
    }

    /// The Open With menu of a file: every candidate, the default marked and first.
    pub fn open_with(&self, name: &str) -> Vec<Choice> {
        let Some(uti) = name.rsplit_once('.').and_then(|(_, e)| uttype::type_for_extension(e)) else { return Vec::new() };
        let default = self.default_for_type(uti);
        let mut v: Vec<Choice> = self.apps_for_type(uti).into_iter().map(|a| Choice { app: a.into(), is_default: Some(a) == default }).collect();
        v.sort_by_key(|c| !c.is_default);
        v
    }

    /// `setDefaultApplication(at:toOpenContentType:)`. The app must declare the type (or an ancestor of it).
    pub fn set_default_for_type(&mut self, uti: &str, app: &str) -> Result<(), SetError> {
        if !self.apps_for_type(uti).contains(&app) {
            return Err(SetError::CannotOpen);
        }
        self.resolved.get_mut().clear();
        self.types.insert(uti.into(), app.into());
        Ok(())
    }

    /// Drops the user's choice for a type (the declarations decide again).
    pub fn reset_type(&mut self, uti: &str) {
        self.resolved.get_mut().clear();
        self.types.remove(uti);
    }

    /// The user's own choice for a type, whether or not the app is still installed.
    pub fn user_choice(&self, uti: &str) -> Option<&str> {
        self.types.get(uti).map(String::as_str)
    }

    /// Every app that declares `role` for `uti` (all role handlers for a type): nearest declaration first,
    /// then registration order. Role declarations are separate from `opens`, so an app can view a type without
    /// being offered as its editor.
    pub fn apps_for_role(&self, uti: &str, role: Role) -> Vec<&str> {
        let mut found = Vec::new();
        for (order, h) in self.handlers.iter().enumerate() {
            let nearest = h.roles.iter().filter(|(_, r)| *r == role).filter_map(|(t, _)| uttype::distance(uti, t)).min();
            if let Some(d) = nearest {
                found.push((d, order, h.app.as_str()));
            }
        }
        found.sort();
        found.into_iter().map(|f| f.2).collect()
    }

    /// Default role handler for a type: the user's choice for this role when that app still declares it,
    /// else the first app for the role.
    pub fn default_for_role(&self, uti: &str, role: Role) -> Option<&str> {
        let apps = self.apps_for_role(uti, role);
        let chosen = self.roles.get(&(uti.to_string(), role)).and_then(|a| apps.iter().find(|x| *x == a).copied());
        chosen.or_else(|| apps.first().copied())
    }

    /// Sets the default role handler. The app must declare the role for the type (or an ancestor of it).
    pub fn set_default_for_role(&mut self, uti: &str, role: Role, app: &str) -> Result<(), SetError> {
        if !self.apps_for_role(uti, role).contains(&app) {
            return Err(SetError::CannotOpen);
        }
        self.roles.insert((uti.to_string(), role), app.to_string());
        Ok(())
    }

    /// Drops the user's choice for one role of a type.
    pub fn reset_role(&mut self, uti: &str, role: Role) {
        self.roles.remove(&(uti.to_string(), role));
    }

    /// The user's own choice for a role, whether or not the app is still installed.
    pub fn user_role_choice(&self, uti: &str, role: Role) -> Option<&str> {
        self.roles.get(&(uti.to_string(), role)).map(String::as_str)
    }

    pub fn apps_for_scheme(&self, scheme: &str) -> Vec<&str> {
        let s = scheme.to_ascii_lowercase();
        self.handlers.iter().filter(|h| h.schemes.contains(&s)).map(|h| h.app.as_str()).collect()
    }

    /// `urlForApplication(toOpen:)` for a URL scheme: the user's choice, else the first registered app.
    pub fn default_for_scheme(&self, scheme: &str) -> Option<&str> {
        let s = scheme.to_ascii_lowercase();
        let apps = self.apps_for_scheme(&s);
        self.schemes.get(&s).and_then(|a| apps.iter().find(|x| *x == a)).copied().or_else(|| apps.first().copied())
    }

    /// `setDefaultApplication(at:toOpenURLsWithScheme:)`.
    pub fn set_default_for_scheme(&mut self, scheme: &str, app: &str) -> Result<(), SetError> {
        let s = scheme.to_ascii_lowercase();
        if !self.apps_for_scheme(&s).contains(&app) {
            return Err(SetError::CannotOpen);
        }
        self.schemes.insert(s, app.into());
        Ok(())
    }

    pub fn reset_scheme(&mut self, scheme: &str) {
        self.schemes.remove(&scheme.to_ascii_lowercase());
    }

    /// The user's choices as `type <uti> <app>` / `scheme <scheme> <app>` lines, sorted, for the prefs file.
    pub fn to_lines(&self) -> String {
        let mut v: Vec<String> = self
            .types
            .iter()
            .map(|(k, a)| format!("type {k} {a}"))
            .chain(self.schemes.iter().map(|(k, a)| format!("scheme {k} {a}")))
            .chain(self.roles.iter().map(|((k, r), a)| format!("role {k} {} {a}", r.name())))
            .collect();
        v.sort();
        v.into_iter().map(|l| l + "\n").collect()
    }

    /// `to_lines` on one line (`;` between choices), for a single-line preference value.
    pub fn to_pref(&self) -> String {
        self.to_lines().trim_end().replace('\n', ";")
    }

    /// Reads `to_pref` output.
    pub fn load_pref(&mut self, text: &str) {
        self.load_lines(&text.replace(';', "\n"));
    }

    /// Reads `to_lines` output; unknown or malformed lines are skipped. Choices are kept even for apps that are not
    /// registered yet (they are checked when asked), so a store app installed later picks its choice up again.
    pub fn load_lines(&mut self, text: &str) {
        for line in text.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            match p.as_slice() {
                ["type", k, a] => {
                    self.resolved.get_mut().clear();
                    self.types.insert(k.to_string(), a.to_string());
                }
                ["scheme", k, a] => {
                    self.schemes.insert(k.to_ascii_lowercase(), a.to_string());
                }
                ["role", k, r, a] => {
                    if let Some(role) = Role::parse(r) {
                        self.roles.insert((k.to_string(), role), a.to_string());
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        let mut r = Registry::new();
        r.register(Handler::new("photos").opens("public.image", Rank::Default));
        r.register(Handler::new("preview").opens("public.png", Rank::Alternate).opens("com.adobe.pdf", Rank::Owner));
        r.register(Handler::new("notes").opens("public.text", Rank::Default));
        r.register(Handler::new("editor").opens("public.plain-text", Rank::Alternate).opens("public.rust-source", Rank::None));
        r.register(Handler::new("browser").opens("public.html", Rank::Owner).scheme("http").scheme("HTTPS"));
        r.register(Handler::new("lite").scheme("https"));
        r
    }

    #[test]
    fn the_most_specific_declaration_wins_then_rank_then_order() {
        let r = reg();
        // preview names PNG itself; photos only the image family, so preview is nearer even with the lower rank
        assert_eq!(r.apps_for_type("public.png"), ["preview", "photos"]);
        assert_eq!(r.default_for_type("public.png"), Some("preview"));
        assert_eq!(r.default_for_type("public.jpeg"), Some("photos"));
        assert_eq!(r.default_for_type("com.adobe.pdf"), Some("preview"));
        assert_eq!(r.default_for_type("public.mp3"), None);
    }

    #[test]
    fn rank_none_is_listed_but_never_the_default() {
        let r = reg();
        assert_eq!(r.apps_for_type("public.rust-source"), ["editor", "notes"], "exact (None) before ancestors");
        assert_eq!(r.default_for_type("public.rust-source"), Some("editor"), "editor also inherits Alternate from plain-text");
        let mut r2 = Registry::new();
        r2.register(Handler::new("only").opens("public.rust-source", Rank::None));
        assert_eq!(r2.apps_for_type("public.rust-source"), ["only"]);
        assert_eq!(r2.default_for_type("public.rust-source"), None);
    }

    #[test]
    fn the_users_choice_beats_the_declarations_and_survives_only_while_valid() {
        let mut r = reg();
        assert_eq!(r.set_default_for_type("public.png", "photos"), Ok(()));
        assert_eq!(r.default_for_type("public.png"), Some("photos"));
        assert_eq!(r.default_for_type("public.jpeg"), Some("photos"), "other types unchanged");
        assert_eq!(r.set_default_for_type("public.png", "browser"), Err(SetError::CannotOpen));
        assert_eq!(r.set_default_for_type("public.png", "ghost"), Err(SetError::CannotOpen));
        r.unregister("photos");
        assert_eq!(r.default_for_type("public.png"), Some("preview"), "uninstalled choice falls back");
        assert_eq!(r.user_choice("public.png"), Some("photos"), "and is remembered for a reinstall");
        r.reset_type("public.png");
        assert_eq!(r.user_choice("public.png"), None);
    }

    #[test]
    fn open_with_lists_the_default_first() {
        let mut r = reg();
        let names = |r: &Registry, f: &str| r.open_with(f).into_iter().map(|c| (c.app, c.is_default)).collect::<Vec<_>>();
        assert_eq!(names(&r, "a.PNG"), [("preview".into(), true), ("photos".into(), false)]);
        r.set_default_for_type("public.png", "photos").unwrap();
        assert_eq!(names(&r, "a.png"), [("photos".into(), true), ("preview".into(), false)]);
        assert!(r.open_with("noextension").is_empty());
        assert_eq!(r.default_for_file("notes.txt"), Some("editor"), "plain-text is nearer than text");
        assert_eq!(r.default_for_file("a.unknownext"), None);
    }

    #[test]
    fn schemes_have_defaults_too() {
        let mut r = reg();
        assert_eq!(r.apps_for_scheme("HTTPS"), ["browser", "lite"]);
        assert_eq!(r.default_for_scheme("https"), Some("browser"));
        r.set_default_for_scheme("https", "lite").unwrap();
        assert_eq!(r.default_for_scheme("https"), Some("lite"));
        assert_eq!(r.default_for_scheme("http"), Some("browser"));
        assert_eq!(r.set_default_for_scheme("mailto", "lite"), Err(SetError::CannotOpen));
        assert_eq!(r.default_for_scheme("ftp"), None);
    }

    #[test]
    fn choices_round_trip_through_lines() {
        let mut r = reg();
        r.set_default_for_type("public.png", "photos").unwrap();
        r.set_default_for_scheme("https", "lite").unwrap();
        let text = r.to_lines();
        assert_eq!(text, "scheme https lite\ntype public.png photos\n");
        let mut fresh = reg();
        fresh.load_lines(&(text + "garbage line\ntype only-two\n"));
        assert_eq!(fresh.default_for_type("public.png"), Some("photos"));
        assert_eq!(fresh.default_for_scheme("https"), Some("lite"));
        assert_eq!(r.to_pref(), "scheme https lite;type public.png photos");
        let mut again = reg();
        again.load_pref(&r.to_pref());
        assert_eq!(again.default_for_type("public.png"), Some("photos"));
    }

    #[test]
    fn re_registering_keeps_the_place_in_the_order() {
        let mut r = reg();
        r.register(Handler::new("photos").opens("public.image", Rank::Default).opens("public.gif", Rank::Owner));
        assert_eq!(r.default_for_type("com.compuserve.gif"), Some("photos"));
        assert_eq!(r.apps_for_type("public.jpeg"), ["photos"]);
        assert!(r.is_registered("photos") && !r.is_registered("nope"));
        assert_eq!(Rank::parse("Owner"), Some(Rank::Owner));
        assert_eq!(Rank::parse("x"), None);
    }

    fn roles_reg() -> Registry {
        let mut r = Registry::new();
        r.register(Handler::new("photos").role(Role::Editor, "public.image").role(Role::Viewer, "public.image"));
        r.register(Handler::new("pixel").role(Role::Editor, "public.png"));
        r.register(Handler::new("preview").role(Role::Viewer, "public.png"));
        r
    }

    #[test]
    fn each_role_has_its_own_nearest_default() {
        let r = roles_reg();
        // pixel declares PNG itself; photos only the image family, so pixel is nearer for the editor role
        assert_eq!(r.apps_for_role("public.png", Role::Editor), ["pixel", "photos"]);
        assert_eq!(r.default_for_role("public.png", Role::Editor), Some("pixel"));
        assert_eq!(r.default_for_role("public.png", Role::Viewer), Some("preview"));
        assert_eq!(r.default_for_role("public.jpeg", Role::Viewer), Some("photos"));
        assert_eq!(r.default_for_role("public.png", Role::Shell), None, "no app declares the shell role");
        assert_eq!(r.default_for_type("public.png"), None, "role declarations do not change the type default");
    }

    #[test]
    fn a_role_choice_is_kept_per_role_and_only_while_valid() {
        let mut r = roles_reg();
        assert_eq!(r.set_default_for_role("public.png", Role::Editor, "photos"), Ok(()));
        assert_eq!(r.default_for_role("public.png", Role::Editor), Some("photos"));
        assert_eq!(r.default_for_role("public.png", Role::Viewer), Some("preview"), "other roles unchanged");
        assert_eq!(r.set_default_for_role("public.png", Role::Editor, "preview"), Err(SetError::CannotOpen));
        r.unregister("photos");
        assert_eq!(r.default_for_role("public.png", Role::Editor), Some("pixel"), "uninstalled choice falls back");
        assert_eq!(r.user_role_choice("public.png", Role::Editor), Some("photos"), "and is remembered");
        r.reset_role("public.png", Role::Editor);
        assert_eq!(r.user_role_choice("public.png", Role::Editor), None);
    }

    #[test]
    fn role_choices_round_trip_and_bad_lines_are_skipped() {
        let mut r = roles_reg();
        r.set_default_for_role("public.png", Role::Editor, "photos").unwrap();
        let text = r.to_lines();
        assert_eq!(text, "role public.png editor photos\n");
        let mut fresh = roles_reg();
        fresh.load_lines(&(text + "role public.png bogus photos\nrole public.png viewer\n"));
        assert_eq!(fresh.user_role_choice("public.png", Role::Editor), Some("photos"));
        assert_eq!(fresh.user_role_choice("public.png", Role::Viewer), None);
        assert_eq!(fresh.default_for_role("public.png", Role::Viewer), Some("preview"));
    }

    #[test]
    fn role_names_parse_case_insensitively() {
        assert_eq!(Role::parse(" Editor "), Some(Role::Editor));
        assert_eq!(Role::parse("VIEWER"), Some(Role::Viewer));
        assert_eq!(Role::parse("shell").map(Role::name), Some("shell"));
        assert_eq!(Role::parse("owner"), None);
    }
}
