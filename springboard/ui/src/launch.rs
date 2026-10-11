//! Default applications (`NativeWorkspace` / Launch Services): which app opens a file type or a URL scheme.
//!
//! Apps declare what they can open, as Golden Gate apps do in `CFBundleDocumentTypes` (`LSItemContentTypes` with an
//! `LSHandlerRank`) and `CFBundleURLTypes`. The registry answers `urlForApplication(toOpen:)`, lists every candidate
//! for an "Open With" menu, and keeps the user's explicit choice per type (`setDefaultApplication(at:toOpenContentType:)`,
//! Files's Get Info > Open with > Change All). Without a choice the best declaration wins: the most specific type,
//! then the higher rank, then the app registered first. Pure data, no I/O: the shell stores `to_lines` in its prefs.
use crate::uttype;
use std::cell::RefCell;
use std::collections::HashMap;

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
}

impl Handler {
    pub fn new(app: &str) -> Self {
        Handler { app: app.into(), types: Vec::new(), schemes: Vec::new() }
    }
    pub fn opens(mut self, uti: &str, rank: Rank) -> Self {
        self.types.push((uti.into(), rank));
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
        let mut v: Vec<String> = self.types.iter().map(|(k, a)| format!("type {k} {a}")).chain(self.schemes.iter().map(|(k, a)| format!("scheme {k} {a}"))).collect();
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
            let mut p = line.split_whitespace();
            match (p.next(), p.next(), p.next(), p.next()) {
                (Some("type"), Some(k), Some(a), None) => {
                    self.resolved.get_mut().clear();
                    self.types.insert(k.into(), a.into());
                }
                (Some("scheme"), Some(k), Some(a), None) => {
                    self.schemes.insert(k.to_ascii_lowercase(), a.into());
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
}
