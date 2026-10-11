//! Caption preferences: the language stack, the display type, the look of caption text, highlights and windows, named
//! profiles that each hold a full set of these, and change reporting. The shell stores the values and feeds them in;
//! this is the pure model the player and the settings page read. No I/O and no system calls.

/// Which settings a read or write addresses. `Default` is the system default and is read-only here; `User` is the
/// person's own choice, which is the active profile when one is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Default,
    User,
}

/// Which captions to show: only forced ones, the ones that match the system language when the audio differs from it,
/// or the most suitable track whatever the language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayType {
    ForcedOnly,
    #[default]
    Automatic,
    AlwaysOn,
}

/// The outside edge of caption text. `Undefined` means the person has not chosen one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeStyle {
    #[default]
    Undefined,
    None,
    Raised,
    Depressed,
    Uniform,
    DropShadow,
}

/// A colour with red, green and blue in 0.0 to 1.0. Opacity is kept apart in `Settings`, as the preference store does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb { r: 1.0, g: 1.0, b: 1.0 };
    pub const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };
}

/// One full set of caption preferences.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub display: DisplayType,
    /// Caption languages, most preferred first (BCP 47 tags such as `en-US`, always normalised).
    pub languages: Vec<String>,
    /// Text size as a multiple of the body size; positive.
    pub relative_size: f32,
    pub text: Rgb,
    pub text_opacity: f32,
    pub edge: EdgeStyle,
    pub highlight: Rgb,
    pub highlight_opacity: f32,
    pub window: Rgb,
    pub window_opacity: f32,
    pub window_radius: f32,
}

impl Default for Settings {
    /// BifrOSt's starting point: automatic display, white text at full opacity, transparent highlight and window.
    fn default() -> Self {
        Settings {
            display: DisplayType::Automatic,
            languages: Vec::new(),
            relative_size: 1.0,
            text: Rgb::WHITE,
            text_opacity: 1.0,
            edge: EdgeStyle::Undefined,
            highlight: Rgb::BLACK,
            highlight_opacity: 0.0,
            window: Rgb::BLACK,
            window_opacity: 0.0,
            window_radius: 0.0,
        }
    }
}

/// A named set of caption preferences the person can switch between.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub settings: Settings,
}

/// What changed, for the change notice. Each change is recorded once per batch, however many writes caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Languages,
    Display,
    Appearance,
    Profiles,
    ActiveProfile,
}

/// Normalises a language tag to `language[-Script][-REGION]` form: the language lower case, a two-letter region upper
/// case and other subtags lower case. Returns `None` for anything that is not a plausible tag.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let text = tag.trim().replace('_', "-");
    let mut parts = text.split('-');
    let language = parts.next()?;
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = language.to_ascii_lowercase();
    for part in parts {
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
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

/// The primary language subtag of a normalised tag (`en` for `en-US`).
fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or("")
}

/// Whether two tags name the same primary language (`en-US` and `en-GB` do; `en` and `is` do not).
fn same_language(a: &str, b: &str) -> bool {
    match (normalize_tag(a), normalize_tag(b)) {
        (Some(x), Some(y)) => primary(&x) == primary(&y),
        _ => false,
    }
}

/// Picks the first track matching the preferences in stack order: an exact tag match first, then the same primary
/// language. Returns the index into `tracks`.
fn best_track(prefs: &[&str], tracks: &[&str]) -> Option<usize> {
    for pref in prefs {
        let Some(want) = normalize_tag(pref) else { continue };
        if let Some(i) = tracks.iter().position(|t| normalize_tag(t).as_deref() == Some(want.as_str())) {
            return Some(i);
        }
        let language = primary(&want);
        if let Some(i) = tracks.iter().position(|t| normalize_tag(t).is_some_and(|n| primary(&n) == language)) {
            return Some(i);
        }
    }
    None
}

/// Clamps an opacity to 0.0 to 1.0; a value that is not finite becomes 0.0.
fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The caption preferences of one person: their own settings, the profiles and the active one, plus the change notice.
#[derive(Clone, Debug, Default)]
pub struct CaptionAppearance {
    user: Settings,
    profiles: Vec<Profile>,
    active: Option<String>,
    changes: Vec<Change>,
    displayed: Vec<String>,
    next_id: u32,
}

impl CaptionAppearance {
    pub fn new() -> Self {
        Self::default()
    }

    /// The settings `domain` reads. `User` is the active profile's settings when one is active, else the person's own.
    pub fn settings(&self, domain: Domain) -> Settings {
        match domain {
            Domain::Default => Settings::default(),
            Domain::User => match self.active_profile() {
                Some(profile) => profile.settings.clone(),
                None => self.user.clone(),
            },
        }
    }

    /// The settings a write to `User` lands in: the active profile when one is set, else the person's own settings.
    fn user_target(&mut self) -> &mut Settings {
        let index = self.active.as_ref().and_then(|id| self.profiles.iter().position(|p| &p.id == id));
        match index {
            Some(i) => &mut self.profiles[i].settings,
            None => &mut self.user,
        }
    }

    /// Applies `edit` to the target of `domain`. Records `change` only when a value actually moved. `Default` is read-only.
    fn edit(&mut self, domain: Domain, change: Change, edit: impl FnOnce(&mut Settings)) -> bool {
        if domain == Domain::Default {
            return false;
        }
        let target = self.user_target();
        let before = target.clone();
        edit(target);
        if *target == before {
            return false;
        }
        self.changes.push(change);
        true
    }

    /// Adds a caption language to the top of the stack. A language already there moves to the top. Returns false when
    /// the tag is not valid, the domain is read-only or nothing changed.
    pub fn add_selected_language(&mut self, domain: Domain, tag: &str) -> bool {
        let Some(tag) = normalize_tag(tag) else { return false };
        self.edit(domain, Change::Languages, |s| {
            s.languages.retain(|l| *l != tag);
            s.languages.insert(0, tag.clone());
        })
    }

    /// Removes a caption language from the stack; returns whether it was there.
    pub fn remove_selected_language(&mut self, domain: Domain, tag: &str) -> bool {
        let Some(tag) = normalize_tag(tag) else { return false };
        self.edit(domain, Change::Languages, |s| s.languages.retain(|l| *l != tag))
    }

    /// The caption languages in preference order.
    pub fn selected_languages(&self, domain: Domain) -> Vec<String> {
        self.settings(domain).languages
    }

    pub fn set_display_type(&mut self, domain: Domain, display: DisplayType) -> bool {
        self.edit(domain, Change::Display, |s| s.display = display)
    }

    /// Sets the text size. A size that is not positive and finite is refused.
    pub fn set_relative_size(&mut self, domain: Domain, size: f32) -> bool {
        if !size.is_finite() || size <= 0.0 {
            return false;
        }
        self.edit(domain, Change::Appearance, |s| s.relative_size = size)
    }

    pub fn set_text(&mut self, domain: Domain, color: Rgb, opacity: f32, edge: EdgeStyle) -> bool {
        self.edit(domain, Change::Appearance, |s| {
            s.text = color;
            s.text_opacity = unit(opacity);
            s.edge = edge;
        })
    }

    pub fn set_highlight(&mut self, domain: Domain, color: Rgb, opacity: f32) -> bool {
        self.edit(domain, Change::Appearance, |s| {
            s.highlight = color;
            s.highlight_opacity = unit(opacity);
        })
    }

    pub fn set_window(&mut self, domain: Domain, color: Rgb, opacity: f32, radius: f32) -> bool {
        if !radius.is_finite() || radius < 0.0 {
            return false;
        }
        self.edit(domain, Change::Appearance, |s| {
            s.window = color;
            s.window_opacity = unit(opacity);
            s.window_radius = radius;
        })
    }

    pub fn profile_ids(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.id.clone()).collect()
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn active_profile_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    fn active_profile(&self) -> Option<&Profile> {
        self.active.as_deref().and_then(|id| self.profile(id))
    }

    /// Creates a profile that starts from the person's own settings. Returns its id, or `None` for an empty name.
    pub fn add_profile(&mut self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.next_id += 1;
        let id = format!("profile-{}", self.next_id);
        self.profiles.push(Profile { id: id.clone(), name: name.to_string(), settings: self.user.clone() });
        self.changes.push(Change::Profiles);
        Some(id)
    }

    pub fn rename_profile(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        match self.profiles.iter_mut().find(|p| p.id == id) {
            Some(profile) if profile.name != name => {
                profile.name = name.to_string();
                self.changes.push(Change::Profiles);
                true
            }
            _ => false,
        }
    }

    /// Applies `edit` to the settings of one profile; records a profile change only if a value moved.
    pub fn edit_profile(&mut self, id: &str, edit: impl FnOnce(&mut Settings)) -> bool {
        let Some(profile) = self.profiles.iter_mut().find(|p| p.id == id) else { return false };
        let before = profile.settings.clone();
        edit(&mut profile.settings);
        if profile.settings == before {
            return false;
        }
        self.changes.push(Change::Profiles);
        true
    }

    /// Removes a profile. Removing the active one makes the person's own settings active again.
    pub fn remove_profile(&mut self, id: &str) -> bool {
        let Some(index) = self.profiles.iter().position(|p| p.id == id) else { return false };
        self.profiles.remove(index);
        self.changes.push(Change::Profiles);
        if self.active.as_deref() == Some(id) {
            self.active = None;
            self.changes.push(Change::ActiveProfile);
        }
        true
    }

    /// Makes a profile active, or none with `None`. An id that is not a profile is refused.
    pub fn set_active_profile(&mut self, id: Option<&str>) -> bool {
        if let Some(id) = id {
            if self.profile(id).is_none() {
                return false;
            }
        }
        if self.active.as_deref() == id {
            return false;
        }
        self.active = id.map(str::to_string);
        self.changes.push(Change::ActiveProfile);
        true
    }

    /// Returns the changes since the last call, each kind once and in the order first made, and clears the notice.
    pub fn take_changes(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        for change in std::mem::take(&mut self.changes) {
            if !out.contains(&change) {
                out.push(change);
            }
        }
        out
    }

    /// Records the caption tracks that were shown. Returns true when they differ from the last report.
    pub fn report_displayed(&mut self, tracks: &[&str]) -> bool {
        let next: Vec<String> = tracks.iter().filter_map(|t| normalize_tag(t)).collect();
        if next == self.displayed {
            return false;
        }
        self.displayed = next;
        true
    }

    pub fn displayed(&self) -> &[String] {
        &self.displayed
    }

    /// Chooses the caption track to show for `domain`, given the audio language, the system language and the tracks the
    /// media offers (tags in media order). Returns the index into `tracks`, or `None` for no captions.
    /// `ForcedOnly` shows nothing, since forced-subtitle flags are not modelled here. `Automatic` shows the system-language
    /// track only when the audio is in another language. `AlwaysOn` follows the stack and falls back to the first track.
    pub fn choose_track(&self, domain: Domain, audio_language: &str, system_language: &str, tracks: &[&str]) -> Option<usize> {
        let settings = self.settings(domain);
        match settings.display {
            DisplayType::ForcedOnly => None,
            DisplayType::Automatic => {
                if same_language(audio_language, system_language) {
                    return None;
                }
                best_track(&[system_language], tracks)
            }
            DisplayType::AlwaysOn => {
                let prefs: Vec<&str> = settings.languages.iter().map(String::as_str).collect();
                best_track(&prefs, tracks).or(if tracks.is_empty() { None } else { Some(0) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_stack_normalises_moves_to_top_and_refuses_bad_tags() {
        let mut c = CaptionAppearance::new();
        assert!(c.add_selected_language(Domain::User, "en_us"));
        assert!(c.add_selected_language(Domain::User, "is"));
        assert_eq!(c.selected_languages(Domain::User), vec!["is", "en-US"]);
        // Adding a language already in the stack moves it to the top; adding the top one again changes nothing.
        assert!(c.add_selected_language(Domain::User, "en-US"));
        assert_eq!(c.selected_languages(Domain::User), vec!["en-US", "is"]);
        assert!(!c.add_selected_language(Domain::User, "en-US"));
        assert!(!c.add_selected_language(Domain::User, "english"));
        assert!(!c.add_selected_language(Domain::Default, "de"));
        assert!(c.remove_selected_language(Domain::User, "is"));
        assert!(!c.remove_selected_language(Domain::User, "is"));
    }

    #[test]
    fn change_notice_fires_only_on_real_changes_and_is_drained_once() {
        let mut c = CaptionAppearance::new();
        assert!(c.set_display_type(Domain::User, DisplayType::AlwaysOn));
        assert!(!c.set_display_type(Domain::User, DisplayType::AlwaysOn));
        assert!(c.set_relative_size(Domain::User, 1.5));
        assert!(!c.set_relative_size(Domain::User, -2.0));
        assert_eq!(c.take_changes(), vec![Change::Display, Change::Appearance]);
        assert!(c.take_changes().is_empty());
    }

    #[test]
    fn profiles_copy_settings_and_the_active_one_drives_the_user_domain() {
        let mut c = CaptionAppearance::new();
        c.set_display_type(Domain::User, DisplayType::AlwaysOn);
        let id = c.add_profile("  Night  ").unwrap();
        assert_eq!(c.profile(&id).unwrap().name, "Night");
        assert_eq!(c.profile(&id).unwrap().settings.display, DisplayType::AlwaysOn);
        assert!(c.add_profile("   ").is_none());

        assert!(c.set_active_profile(Some(&id)));
        assert!(!c.set_active_profile(Some(&id)));
        assert!(c.set_display_type(Domain::User, DisplayType::ForcedOnly));
        assert_eq!(c.settings(Domain::User).display, DisplayType::ForcedOnly);
        // The write went to the profile; the person's own settings stay as they were.
        assert_eq!(c.profile(&id).unwrap().settings.display, DisplayType::ForcedOnly);
        assert!(c.edit_profile(&id, |s| s.relative_size = 2.0));
        assert!(c.set_active_profile(None));
        assert_eq!(c.settings(Domain::User).display, DisplayType::AlwaysOn);
        assert_eq!(c.settings(Domain::User).relative_size, 1.0);

        assert!(c.set_active_profile(Some(&id)));
        assert!(c.remove_profile(&id));
        assert_eq!(c.active_profile_id(), None);
        assert!(!c.set_active_profile(Some("missing")));
    }

    #[test]
    fn automatic_shows_the_system_language_only_for_foreign_audio() {
        let mut c = CaptionAppearance::new();
        let tracks = ["de-DE", "is-IS", "en-GB"];
        assert_eq!(c.choose_track(Domain::User, "en-US", "en-US", &tracks), None);
        assert_eq!(c.choose_track(Domain::User, "en-US", "is-IS", &tracks), Some(1));
        assert_eq!(c.choose_track(Domain::User, "de", "en-US", &tracks), Some(2));
        assert_eq!(c.choose_track(Domain::User, "fr", "fi-FI", &tracks), None);
        c.set_display_type(Domain::User, DisplayType::ForcedOnly);
        assert_eq!(c.choose_track(Domain::User, "de", "en-US", &tracks), None);
    }

    #[test]
    fn always_on_follows_the_stack_then_the_first_track() {
        let mut c = CaptionAppearance::new();
        c.set_display_type(Domain::User, DisplayType::AlwaysOn);
        let tracks = ["fr-FR", "de-AT", "en-US"];
        assert_eq!(c.choose_track(Domain::User, "ja", "en-US", &tracks), Some(0));
        c.add_selected_language(Domain::User, "en");
        c.add_selected_language(Domain::User, "de-DE");
        // The stack puts de-DE first; its primary match de-AT wins before the lower-ranked en.
        assert_eq!(c.choose_track(Domain::User, "ja", "en-US", &tracks), Some(1));
        c.remove_selected_language(Domain::User, "de-DE");
        assert_eq!(c.choose_track(Domain::User, "ja", "en-US", &tracks), Some(2));
        assert_eq!(c.choose_track(Domain::User, "ja", "en-US", &[]), None);
    }

    #[test]
    fn tags_normalise_and_displayed_report_only_when_it_differs() {
        assert_eq!(normalize_tag(" zh_hant_tw ").as_deref(), Some("zh-hant-TW"));
        assert_eq!(normalize_tag("EN-gb").as_deref(), Some("en-GB"));
        assert_eq!(normalize_tag("e"), None);
        assert_eq!(normalize_tag("en--GB"), None);
        assert_eq!(normalize_tag("en-"), None);

        let mut c = CaptionAppearance::new();
        assert!(c.report_displayed(&["en-us", "is"]));
        assert!(!c.report_displayed(&["en-US", "is"]));
        assert_eq!(c.displayed().len(), 2);
        assert_eq!(c.displayed()[0], "en-US");
    }

    #[test]
    fn appearance_values_are_clamped_and_refused_when_invalid() {
        let mut c = CaptionAppearance::new();
        assert!(c.set_text(Domain::User, Rgb { r: 1.0, g: 0.9, b: 0.0 }, 3.0, EdgeStyle::Raised));
        let s = c.settings(Domain::User);
        assert_eq!(s.text_opacity, 1.0);
        assert_eq!(s.edge, EdgeStyle::Raised);
        assert!(!c.set_window(Domain::User, Rgb::BLACK, 0.5, -1.0));
        assert!(c.set_highlight(Domain::User, Rgb::BLACK, 2.0));
        assert_eq!(c.settings(Domain::User).highlight_opacity, 1.0);
        assert!(c.set_highlight(Domain::User, Rgb::BLACK, f32::NAN));
        assert_eq!(c.settings(Domain::User).highlight_opacity, 0.0);
        assert!(!c.set_relative_size(Domain::User, f32::INFINITY));
        assert!(!c.set_text(Domain::Default, Rgb::WHITE, 1.0, EdgeStyle::None));
    }
}
