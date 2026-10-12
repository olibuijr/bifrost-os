//! Speech asset inventory: which recognition models are installed for which locale, which locales the app has
//! reserved, and the install steps that move a model from supported to installed. The models are large downloads that
//! a system service fetches and keeps; this module tracks their state and plans the work. It downloads nothing.

use std::collections::BTreeMap;

/// One model a module needs: the module's name and the locale it recognizes, normalized with `normalize_locale`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Need {
    pub module: String,
    pub locale: String,
}

impl Need {
    pub fn new(module: &str, locale: &str) -> Self {
        let locale = normalize_locale(locale).unwrap_or_else(|| locale.to_string());
        Need { module: module.to_string(), locale }
    }
}

/// The state of a set of needs, the worst one wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// At least one need has no model in the catalog, so it cannot be installed here.
    Unsupported,
    /// At least one model is not installed yet and none is downloading.
    Supported,
    /// A download is in progress.
    Downloading,
    /// Every model is installed. An empty set of needs is installed.
    Installed,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Available,
    Downloading(f32),
    Installed,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// The text is not a language tag such as `en-US`.
    InvalidLocale(String),
    /// The catalog has no model for this need.
    Unsupported(Need),
    /// The need is not in the state the call requires.
    WrongState(Need),
}

/// Reads a language tag such as `en_us` or `EN-gb` and returns it in the form `en-US`: a 2 or 3 letter language in
/// lower case, then subtags, with two-letter region subtags in upper case. Returns `None` for anything else.
pub fn normalize_locale(tag: &str) -> Option<String> {
    let parts: Vec<&str> = tag.trim().split(|c: char| c == '-' || c == '_').collect();
    let lang = parts.first()?;
    if lang.len() < 2 || lang.len() > 3 || !lang.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = lang.to_ascii_lowercase();
    for part in &parts[1..] {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        out.push('-');
        if part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            out.push_str(&part.to_ascii_uppercase());
        } else {
            out.push_str(&part.to_ascii_lowercase());
        }
    }
    Some(out)
}

/// The models the system offers, their install state, and the locales this app has reserved.
pub struct Inventory {
    catalog: BTreeMap<Need, State>,
    reserved: Vec<String>,
    max_reserved: usize,
}

impl Inventory {
    /// An inventory that allows `max_reserved` locale reservations. The system sets the real limit; the caller passes
    /// the number it was given.
    pub fn new(max_reserved: usize) -> Self {
        Inventory { catalog: BTreeMap::new(), reserved: Vec::new(), max_reserved }
    }

    /// Adds a model the system offers. A model that is already installed keeps its state.
    pub fn add_supported(&mut self, module: &str, locale: &str) {
        self.catalog.entry(Need::new(module, locale)).or_insert(State::Available);
    }

    pub fn status_of(&self, need: &Need) -> Status {
        match self.catalog.get(need) {
            None => Status::Unsupported,
            Some(State::Available) | Some(State::Failed(_)) => Status::Supported,
            Some(State::Downloading(_)) => Status::Downloading,
            Some(State::Installed) => Status::Installed,
        }
    }

    /// The state of a set of needs: unsupported beats downloading, downloading beats supported.
    pub fn status(&self, needs: &[Need]) -> Status {
        let states: Vec<Status> = needs.iter().map(|n| self.status_of(n)).collect();
        if states.contains(&Status::Unsupported) {
            Status::Unsupported
        } else if states.contains(&Status::Downloading) {
            Status::Downloading
        } else if states.contains(&Status::Supported) {
            Status::Supported
        } else {
            Status::Installed
        }
    }

    /// The needs that still have to be installed, in the order given and without repeats. A need that is already
    /// downloading is left out. Fails on the first need the catalog does not have.
    pub fn plan(&self, needs: &[Need]) -> Result<Vec<Need>, AssetError> {
        let mut todo: Vec<Need> = Vec::new();
        for need in needs {
            match self.status_of(need) {
                Status::Unsupported => return Err(AssetError::Unsupported(need.clone())),
                Status::Supported => {
                    if !todo.contains(need) {
                        todo.push(need.clone());
                    }
                }
                _ => {}
            }
        }
        Ok(todo)
    }

    /// Starts the download of a need that is supported and not installed. A failed download can be started again.
    pub fn start(&mut self, need: &Need) -> Result<(), AssetError> {
        match self.status_of(need) {
            Status::Unsupported => Err(AssetError::Unsupported(need.clone())),
            Status::Supported => {
                self.catalog.insert(need.clone(), State::Downloading(0.0));
                Ok(())
            }
            _ => Err(AssetError::WrongState(need.clone())),
        }
    }

    /// Records progress of a download, from 0 to 1. Values outside that range are clamped, and NaN counts as 0.
    pub fn set_progress(&mut self, need: &Need, fraction: f32) -> Result<(), AssetError> {
        let value = if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) };
        match self.catalog.get_mut(need) {
            Some(State::Downloading(f)) => {
                *f = value;
                Ok(())
            }
            _ => Err(AssetError::WrongState(need.clone())),
        }
    }

    /// The progress of a download in progress, or `None` when the need is not downloading.
    pub fn download_progress(&self, need: &Need) -> Option<f32> {
        match self.catalog.get(need) {
            Some(State::Downloading(f)) => Some(*f),
            _ => None,
        }
    }

    /// Marks a finished download as installed.
    pub fn finish(&mut self, need: &Need) -> Result<(), AssetError> {
        if self.is_downloading(need) {
            self.catalog.insert(need.clone(), State::Installed);
            Ok(())
        } else {
            Err(AssetError::WrongState(need.clone()))
        }
    }

    /// Marks a download that failed. The need goes back to supported and keeps the reason until it is started again.
    pub fn fail(&mut self, need: &Need, reason: &str) -> Result<(), AssetError> {
        if self.is_downloading(need) {
            self.catalog.insert(need.clone(), State::Failed(reason.to_string()));
            Ok(())
        } else {
            Err(AssetError::WrongState(need.clone()))
        }
    }

    /// The reason the last download of this need failed, if it did and has not been started again.
    pub fn failure(&self, need: &Need) -> Option<&str> {
        match self.catalog.get(need) {
            Some(State::Failed(reason)) => Some(reason.as_str()),
            _ => None,
        }
    }

    fn is_downloading(&self, need: &Need) -> bool {
        matches!(self.catalog.get(need), Some(State::Downloading(_)))
    }

    /// The most locales the app may hold at once.
    pub fn maximum_reserved(&self) -> usize {
        self.max_reserved
    }

    pub fn reserved_locales(&self) -> &[String] {
        &self.reserved
    }

    /// Reserves a locale. Returns `Ok(true)` when it is reserved, including when it already was, and `Ok(false)` when
    /// every slot is taken. Fails when the text is not a language tag.
    pub fn reserve(&mut self, locale: &str) -> Result<bool, AssetError> {
        let tag = normalize_locale(locale).ok_or_else(|| AssetError::InvalidLocale(locale.to_string()))?;
        if self.reserved.contains(&tag) {
            return Ok(true);
        }
        if self.reserved.len() >= self.max_reserved {
            return Ok(false);
        }
        self.reserved.push(tag);
        Ok(true)
    }

    /// Releases a reservation. Returns `true` when the locale was reserved.
    pub fn release(&mut self, locale: &str) -> bool {
        match normalize_locale(locale) {
            Some(tag) => {
                let before = self.reserved.len();
                self.reserved.retain(|t| *t != tag);
                self.reserved.len() != before
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_are_normalized() {
        assert_eq!(normalize_locale("en_us"), Some("en-US".to_string()));
        assert_eq!(normalize_locale(" EN-gb "), Some("en-GB".to_string()));
        assert_eq!(normalize_locale("english"), None);
        assert_eq!(normalize_locale(""), None);
        assert_eq!(normalize_locale("xx!"), None);
    }

    #[test]
    fn reservations_stop_at_the_limit() {
        let mut inv = Inventory::new(2);
        assert_eq!(inv.reserve("en-US"), Ok(true));
        assert_eq!(inv.reserve("sv_IS"), Ok(true));
        assert_eq!(inv.reserve("de-DE"), Ok(false));
        assert_eq!(inv.reserve("en_us"), Ok(true));
        assert_eq!(inv.reserved_locales().len(), 2);
        assert_eq!(inv.reserved_locales()[1], "sv-IS");
        assert_eq!(inv.reserve("xx!"), Err(AssetError::InvalidLocale("xx!".to_string())));
    }

    #[test]
    fn releasing_a_locale_frees_its_slot() {
        let mut inv = Inventory::new(1);
        assert_eq!(inv.reserve("en-US"), Ok(true));
        assert!(!inv.release("de-DE"));
        assert!(inv.release("en_US"));
        assert_eq!(inv.reserve("de-DE"), Ok(true));
        assert_eq!(inv.reserved_locales(), &["de-DE".to_string()]);
    }

    #[test]
    fn status_is_the_worst_of_the_needs() {
        let mut inv = Inventory::new(1);
        inv.add_supported("transcriber", "en-US");
        inv.add_supported("detector", "en-US");
        let t = Need::new("transcriber", "en-US");
        let d = Need::new("detector", "en-US");
        let missing = Need::new("transcriber", "fr-FR");
        assert_eq!(inv.status(&[]), Status::Installed);
        assert_eq!(inv.status(&[t.clone(), d.clone()]), Status::Supported);
        assert_eq!(inv.status(&[t.clone(), missing]), Status::Unsupported);
        inv.start(&t).unwrap();
        assert_eq!(inv.status(&[t.clone(), d.clone()]), Status::Downloading);
        inv.finish(&t).unwrap();
        assert_eq!(inv.status(&[t.clone()]), Status::Installed);
        assert_eq!(inv.status(&[t, d]), Status::Supported);
    }

    #[test]
    fn plan_lists_only_what_is_missing() {
        let mut inv = Inventory::new(1);
        inv.add_supported("transcriber", "en-US");
        inv.add_supported("detector", "en-US");
        let t = Need::new("transcriber", "en-US");
        let d = Need::new("detector", "en-US");
        assert_eq!(inv.plan(&[t.clone(), d.clone(), t.clone()]).unwrap(), vec![t.clone(), d.clone()]);
        inv.start(&t).unwrap();
        inv.finish(&t).unwrap();
        assert_eq!(inv.plan(&[t]).unwrap(), Vec::<Need>::new());
        let fr = Need::new("transcriber", "fr-FR");
        assert_eq!(inv.plan(&[d, fr.clone()]), Err(AssetError::Unsupported(fr)));
    }

    #[test]
    fn download_progress_is_clamped_and_failures_can_retry() {
        let mut inv = Inventory::new(1);
        inv.add_supported("transcriber", "en-US");
        let t = Need::new("transcriber", "en-US");
        assert_eq!(inv.set_progress(&t, 0.5), Err(AssetError::WrongState(t.clone())));
        inv.start(&t).unwrap();
        assert_eq!(inv.start(&t), Err(AssetError::WrongState(t.clone())));
        inv.set_progress(&t, 2.0).unwrap();
        assert_eq!(inv.download_progress(&t), Some(1.0));
        inv.set_progress(&t, -1.0).unwrap();
        assert_eq!(inv.download_progress(&t), Some(0.0));
        inv.fail(&t, "network lost").unwrap();
        assert_eq!(inv.status_of(&t), Status::Supported);
        assert_eq!(inv.failure(&t), Some("network lost"));
        inv.start(&t).unwrap();
        assert_eq!(inv.failure(&t), None);
        inv.finish(&t).unwrap();
        assert_eq!(inv.status_of(&t), Status::Installed);
    }

    #[test]
    fn a_need_outside_the_catalog_cannot_be_started() {
        let mut inv = Inventory::new(1);
        let fr = Need::new("transcriber", "fr-FR");
        assert_eq!(inv.start(&fr), Err(AssetError::Unsupported(fr.clone())));
        assert_eq!(inv.finish(&fr), Err(AssetError::WrongState(fr.clone())));
        assert_eq!(inv.status_of(&fr), Status::Unsupported);
    }
}
