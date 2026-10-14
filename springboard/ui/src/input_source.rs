//! Input sources as the system lists them (the text input source services): every keyboard layout, input mode and
//! palette the system offers, which of them the person enabled, and which one is selected. Enumeration is filtered the
//! way the system call is (all installed sources or only the enabled ones), selection needs a source that is enabled
//! and can be selected, and the choice is saved to and restored from a small text form so it survives a restart.
//! Pure model: it reads no devices and draws nothing.

/// What a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    KeyboardLayout,
    InputMode,
    InputMethod,
    Palette,
}

/// One input source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSource {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    /// BCP 47 language tags the source types.
    pub languages: Vec<String>,
    /// True when the person may turn the source off (the last keyboard source cannot be turned off).
    pub enable_capable: bool,
    /// True when the source can be the selected one (a palette is shown, not selected).
    pub select_capable: bool,
    enabled: bool,
}

impl InputSource {
    pub fn new(id: &str, name: &str, kind: SourceKind, languages: &[&str]) -> Self {
        InputSource {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            languages: languages.iter().map(|l| l.to_string()).collect(),
            enable_capable: true,
            select_capable: kind != SourceKind::Palette,
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Why a change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    Unknown(String),
    NotEnabled(String),
    NotSelectCapable(String),
    /// The source is the selected one or the last enabled keyboard source.
    CannotDisable(String),
}

/// The installed sources, the enabled set and the selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputSources {
    sources: Vec<InputSource>,
    selected: Option<String>,
}

impl InputSources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a source. A source with an id already installed is ignored. The first selectable source added is
    /// enabled and selected, so the list is never without a selection.
    pub fn install(&mut self, mut source: InputSource) {
        if self.find(&source.id).is_some() {
            return;
        }
        if self.selected.is_none() && source.select_capable {
            source.enabled = true;
            self.selected = Some(source.id.clone());
        }
        self.sources.push(source);
    }

    fn find(&self, id: &str) -> Option<&InputSource> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// The list: every installed source, or only the enabled ones.
    pub fn list(&self, include_all_installed: bool) -> Vec<&InputSource> {
        self.sources.iter().filter(|s| include_all_installed || s.enabled).collect()
    }

    /// The sources typing one language (a tag matches on its primary subtag).
    pub fn for_language(&self, language: &str, include_all_installed: bool) -> Vec<&InputSource> {
        let primary = |t: &str| t.split('-').next().unwrap_or("").to_ascii_lowercase();
        let want = primary(language);
        self.list(include_all_installed).into_iter().filter(|s| s.languages.iter().any(|l| primary(l) == want)).collect()
    }

    /// The selected keyboard source.
    pub fn current(&self) -> Option<&InputSource> {
        self.selected.as_deref().and_then(|id| self.find(id))
    }

    pub fn enable(&mut self, id: &str) -> Result<(), SourceError> {
        let s = self.sources.iter_mut().find(|s| s.id == id).ok_or_else(|| SourceError::Unknown(id.to_string()))?;
        s.enabled = true;
        Ok(())
    }

    pub fn disable(&mut self, id: &str) -> Result<(), SourceError> {
        let source = self.find(id).ok_or_else(|| SourceError::Unknown(id.to_string()))?;
        let last_selectable = source.select_capable
            && self.sources.iter().filter(|s| s.enabled && s.select_capable).count() <= 1;
        if !source.enable_capable || self.selected.as_deref() == Some(id) || last_selectable {
            return Err(SourceError::CannotDisable(id.to_string()));
        }
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
            s.enabled = false;
        }
        Ok(())
    }

    /// Selects an enabled, select-capable source.
    pub fn select(&mut self, id: &str) -> Result<(), SourceError> {
        let source = self.find(id).ok_or_else(|| SourceError::Unknown(id.to_string()))?;
        if !source.enabled {
            return Err(SourceError::NotEnabled(id.to_string()));
        }
        if !source.select_capable {
            return Err(SourceError::NotSelectCapable(id.to_string()));
        }
        self.selected = Some(id.to_string());
        Ok(())
    }

    /// Selects the first enabled, select-capable source that types `language` (matched on the primary subtag, as
    /// [`InputSources::for_language`] does). Returns the id it selected, or `None` when no enabled source types that
    /// language, in which case the current selection stays.
    pub fn select_for_language(&mut self, language: &str) -> Option<String> {
        let id = self.for_language(language, false).into_iter().find(|s| s.select_capable).map(|s| s.id.clone())?;
        self.selected = Some(id.clone());
        Some(id)
    }

    /// Selects the next enabled select-capable source, wrapping around. The id it chose, or None with no choice.
    pub fn select_next(&mut self) -> Option<String> {
        let ids: Vec<String> =
            self.sources.iter().filter(|s| s.enabled && s.select_capable).map(|s| s.id.clone()).collect();
        let at = ids.iter().position(|id| Some(id) == self.selected.as_ref())?;
        let next = ids[(at + 1) % ids.len()].clone();
        self.selected = Some(next.clone());
        Some(next)
    }

    /// The saved form: one line `selected <id>`, then one line `enabled <id>` per enabled source.
    pub fn save(&self) -> String {
        let mut out = String::new();
        if let Some(id) = &self.selected {
            out.push_str(&format!("selected {id}\n"));
        }
        for s in self.sources.iter().filter(|s| s.enabled) {
            out.push_str(&format!("enabled {}\n", s.id));
        }
        out
    }

    /// Restores the saved form onto the installed sources. Ids that are not installed are skipped, and a selection
    /// that is not usable leaves the current one in place.
    pub fn restore(&mut self, saved: &str) {
        let mut selected = None;
        for line in saved.lines() {
            match line.split_once(' ') {
                Some(("enabled", id)) => {
                    let _ = self.enable(id);
                }
                Some(("selected", id)) => selected = Some(id.to_string()),
                _ => {}
            }
        }
        if let Some(id) = selected {
            let _ = self.select(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> InputSources {
        let mut s = InputSources::new();
        s.install(InputSource::new("keylayout.US", "U.S.", SourceKind::KeyboardLayout, &["en"]));
        s.install(InputSource::new("keylayout.Icelandic", "Icelandic", SourceKind::KeyboardLayout, &["is"]));
        s.install(InputSource::new("mode.Japanese.Romaji", "Romaji", SourceKind::InputMode, &["ja"]));
        s.install(InputSource::new("mode.Korean.TwoSet", "2-Set", SourceKind::InputMode, &["ko-KR"]));
        s.install(InputSource::new("palette.Characters", "Emoji and Symbols", SourceKind::Palette, &[]));
        s
    }

    #[test]
    fn the_first_selectable_source_is_enabled_and_selected() {
        let s = sources();
        assert_eq!(s.current().map(|c| c.id.as_str()), Some("keylayout.US"));
        assert_eq!(s.list(false).len(), 1);
        assert_eq!(s.list(true).len(), 5);
    }

    #[test]
    fn a_duplicate_install_is_ignored() {
        let mut s = sources();
        s.install(InputSource::new("keylayout.US", "Other", SourceKind::KeyboardLayout, &[]));
        assert_eq!(s.list(true).len(), 5);
    }

    #[test]
    fn selection_needs_an_enabled_selectable_source() {
        let mut s = sources();
        assert_eq!(s.select("mode.Korean.TwoSet"), Err(SourceError::NotEnabled("mode.Korean.TwoSet".into())));
        s.enable("mode.Korean.TwoSet").unwrap();
        s.select("mode.Korean.TwoSet").unwrap();
        assert_eq!(s.current().unwrap().name, "2-Set");
        s.enable("palette.Characters").unwrap();
        assert_eq!(s.select("palette.Characters"), Err(SourceError::NotSelectCapable("palette.Characters".into())));
        assert_eq!(s.select("nope"), Err(SourceError::Unknown("nope".into())));
    }

    #[test]
    fn the_selected_and_the_last_keyboard_source_stay_enabled() {
        let mut s = sources();
        assert_eq!(s.disable("keylayout.US"), Err(SourceError::CannotDisable("keylayout.US".into())));
        s.enable("keylayout.Icelandic").unwrap();
        s.disable("keylayout.Icelandic").unwrap();
        assert_eq!(s.list(false).len(), 1);
    }

    #[test]
    fn the_language_filter_matches_the_primary_subtag() {
        let s = sources();
        assert_eq!(s.for_language("ko", true).len(), 1);
        assert_eq!(s.for_language("ko-KR", true).len(), 1);
        assert_eq!(s.for_language("ko", false).len(), 0, "not enabled");
        assert_eq!(s.for_language("en-GB", true)[0].id, "keylayout.US");
    }

    #[test]
    fn select_next_wraps_over_enabled_selectable_sources() {
        let mut s = sources();
        s.enable("mode.Japanese.Romaji").unwrap();
        s.enable("palette.Characters").unwrap();
        assert_eq!(s.select_next(), Some("mode.Japanese.Romaji".into()));
        assert_eq!(s.select_next(), Some("keylayout.US".into()), "the palette is skipped and the list wraps");
    }

    #[test]
    fn the_choice_survives_a_save_and_restore() {
        let mut s = sources();
        s.enable("mode.Japanese.Romaji").unwrap();
        s.select("mode.Japanese.Romaji").unwrap();
        let saved = s.save();
        let mut fresh = sources();
        fresh.restore(&format!("{saved}enabled gone.Missing\n"));
        assert_eq!(fresh.current().unwrap().id, "mode.Japanese.Romaji");
        assert_eq!(fresh.list(false).len(), 2);
    }
}
