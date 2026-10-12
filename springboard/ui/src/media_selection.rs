//! Language options and accessibility track choice as a pure model. A language option group holds the audio or
//! caption choices of a playing item and allows one active option at a time (the options, the default option and
//! whether an empty selection is allowed). Track choice follows the viewer's audible and caption preferences
//! (described video, dialogue transcription, music and sound descriptions) and the viewer's language.

/// Accessibility characteristics a track can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Characteristic {
    /// A narration of the picture for people who cannot see it (described video).
    DescribesVideo,
    /// Spoken dialogue written out as captions.
    TranscribesDialog,
    /// Music and sound effects described in text.
    DescribesMusicAndSound,
}

/// One language option: an identifier, a language tag, a display name and what the option carries.
#[derive(Clone, Debug, PartialEq)]
pub struct LanguageOption {
    pub identifier: String,
    pub language_tag: String,
    pub display_name: String,
    pub characteristics: Vec<Characteristic>,
}

/// Why a selection change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    /// The index is past the last option.
    OutOfRange,
    /// The group requires a selection and the request would clear it.
    Required,
}

/// A group of options of which at most one is active (or exactly one, when empty selection is not allowed).
#[derive(Clone, Debug, PartialEq)]
pub struct LanguageOptionGroup {
    options: Vec<LanguageOption>,
    allow_empty: bool,
    selected: Option<usize>,
}

impl LanguageOptionGroup {
    /// `default` names the option selected at the start, and an out-of-range default is ignored. When the group does
    /// not allow an empty selection and no valid default exists, the first option is selected.
    pub fn new(options: Vec<LanguageOption>, default: Option<usize>, allow_empty: bool) -> Self {
        let valid_default = default.filter(|i| *i < options.len());
        let selected = match valid_default {
            Some(i) => Some(i),
            None if !allow_empty && !options.is_empty() => Some(0),
            None => None,
        };
        LanguageOptionGroup { options, allow_empty, selected }
    }

    pub fn options(&self) -> &[LanguageOption] {
        &self.options
    }

    pub fn allow_empty(&self) -> bool {
        self.allow_empty
    }

    /// The index of the active option, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_option(&self) -> Option<&LanguageOption> {
        self.selected.map(|i| &self.options[i])
    }

    /// Makes option `index` the active one.
    pub fn select(&mut self, index: usize) -> Result<(), SelectionError> {
        if index >= self.options.len() {
            return Err(SelectionError::OutOfRange);
        }
        self.selected = Some(index);
        Ok(())
    }

    /// Clears the selection. Refused when the group requires one.
    pub fn clear(&mut self) -> Result<(), SelectionError> {
        if !self.allow_empty {
            return Err(SelectionError::Required);
        }
        self.selected = None;
        Ok(())
    }
}

/// One audio or caption track of a playing item.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub language_tag: String,
    pub characteristics: Vec<Characteristic>,
}

/// The primary language subtag of a tag, lowercased: "en-US" gives "en".
fn primary(tag: &str) -> String {
    tag.split(|c: char| c == '-' || c == '_')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Picks the track to play. A track that describes the picture is eligible only when `preferred` asks for it. The
/// tracks in the viewer's language (primary subtag match) come first when there are any; among them the track that
/// matches the most preferred characteristics wins, and a tie goes to the earlier track. Returns None when no track is
/// eligible.
pub fn choose_track(tracks: &[Track], preferred: &[Characteristic], language: &str) -> Option<usize> {
    let wanted = primary(language);
    let described_ok = preferred.contains(&Characteristic::DescribesVideo);
    let eligible: Vec<usize> = (0..tracks.len())
        .filter(|&i| described_ok || !tracks[i].characteristics.contains(&Characteristic::DescribesVideo))
        .collect();
    let in_language: Vec<usize> = eligible
        .iter()
        .copied()
        .filter(|&i| primary(&tracks[i].language_tag) == wanted)
        .collect();
    let pool = if in_language.is_empty() { eligible } else { in_language };
    let mut best: Option<(usize, usize)> = None; // (index, matched preferences)
    for i in pool {
        let score = preferred
            .iter()
            .filter(|c| tracks[i].characteristics.contains(*c))
            .count();
        if let Some((_, s)) = best {
            if s >= score {
                continue;
            }
        }
        best = Some((i, score));
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(id: &str, tag: &str) -> LanguageOption {
        LanguageOption {
            identifier: id.to_string(),
            language_tag: tag.to_string(),
            display_name: id.to_string(),
            characteristics: Vec::new(),
        }
    }

    fn track(tag: &str, chars: &[Characteristic]) -> Track {
        Track { language_tag: tag.to_string(), characteristics: chars.to_vec() }
    }

    #[test]
    fn a_default_option_is_selected_and_an_out_of_range_default_is_ignored() {
        let opts = || vec![opt("a", "en"), opt("b", "is"), opt("c", "fr")];
        assert_eq!(LanguageOptionGroup::new(opts(), Some(1), false).selected(), Some(1));
        assert_eq!(LanguageOptionGroup::new(opts(), Some(9), false).selected(), Some(0));
        assert_eq!(LanguageOptionGroup::new(opts(), None, true).selected(), None);
    }

    #[test]
    fn a_group_that_requires_a_selection_refuses_to_clear_it() {
        let mut g = LanguageOptionGroup::new(vec![opt("a", "en"), opt("b", "is"), opt("c", "fr")], Some(1), false);
        assert_eq!(g.clear(), Err(SelectionError::Required));
        assert_eq!(g.select(5), Err(SelectionError::OutOfRange));
        assert_eq!(g.select(2), Ok(()));
        assert_eq!(g.selected_option().map(|o| o.identifier.as_str()), Some("c"));
        let mut open = LanguageOptionGroup::new(vec![opt("a", "en")], None, true);
        assert_eq!(open.clear(), Ok(()));
        assert_eq!(open.selected(), None);
    }

    #[test]
    fn described_video_is_chosen_only_when_asked_for() {
        let tracks = vec![
            track("en", &[]),
            track("en", &[Characteristic::DescribesVideo]),
            track("is", &[]),
        ];
        assert_eq!(choose_track(&tracks, &[], "en"), Some(0), "the described track is not eligible");
        assert_eq!(choose_track(&tracks, &[Characteristic::DescribesVideo], "en"), Some(1));
    }

    #[test]
    fn a_track_in_the_viewers_language_wins_by_primary_subtag() {
        let tracks = vec![track("is", &[]), track("en-US", &[])];
        assert_eq!(choose_track(&tracks, &[], "en-GB"), Some(1), "en-US matches en-GB on the primary subtag");
        let no_match = vec![track("en", &[]), track("is", &[])];
        assert_eq!(choose_track(&no_match, &[], "fr"), Some(0), "without a language match every eligible track counts");
    }

    #[test]
    fn captions_follow_the_transcription_preference_and_ties_go_to_the_earlier_track() {
        let tracks = vec![track("en", &[]), track("en", &[Characteristic::TranscribesDialog])];
        assert_eq!(choose_track(&tracks, &[], "en"), Some(0));
        assert_eq!(choose_track(&tracks, &[Characteristic::TranscribesDialog], "en"), Some(1));
        let twins = vec![track("en", &[Characteristic::DescribesMusicAndSound]), track("en", &[Characteristic::DescribesMusicAndSound])];
        assert_eq!(choose_track(&twins, &[Characteristic::DescribesMusicAndSound], "en"), Some(0));
    }

    #[test]
    fn no_tracks_or_only_ineligible_tracks_choose_nothing() {
        assert_eq!(choose_track(&[], &[], "en"), None);
        let only_described = vec![track("en", &[Characteristic::DescribesVideo])];
        assert_eq!(choose_track(&only_described, &[], "en"), None);
    }
}
