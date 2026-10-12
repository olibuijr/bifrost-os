//! Media sources and the load options that choose them, as a pure model. A source is a provider of media identified by
//! a unique identifier within one library load, and its primary kind is audio, image or movie. The load options say
//! whether folder sources load, which identifiers are included and which are excluded. This module decides which
//! sources a load keeps; it reads no library and no disk.

/// The primary kind of media a source holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceType {
    Audio,
    Image,
    Movie,
}

/// One media source of a library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSource {
    /// Unique within one library load. Never empty.
    pub identifier: String,
    pub kind: SourceType,
    /// True for an app-defined folder source rather than a built-in provider.
    pub is_folder: bool,
}

impl MediaSource {
    /// The identifier is trimmed. None when nothing is left of it.
    pub fn new(identifier: &str, kind: SourceType, is_folder: bool) -> Option<Self> {
        let id = identifier.trim();
        if id.is_empty() {
            None
        } else {
            Some(MediaSource { identifier: id.to_string(), kind, is_folder })
        }
    }
}

/// What a library load keeps: whether folder sources load, an optional include list and an exclude list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadOptions {
    pub load_folders: bool,
    /// None loads every source. Some keeps only the listed identifiers.
    pub include: Option<Vec<String>>,
    /// Identifiers that never load.
    pub exclude: Vec<String>,
}

impl LoadOptions {
    /// Loads every source, folders included.
    pub fn all() -> Self {
        LoadOptions { load_folders: true, include: None, exclude: Vec::new() }
    }

    /// Builds the options from the optional load keys of a library load. An absent folders key loads folders, and an
    /// absent include or exclude key applies no filter.
    pub fn from_keys(folders: Option<bool>, include: Option<Vec<String>>, exclude: Option<Vec<String>>) -> Self {
        LoadOptions { load_folders: folders.unwrap_or(true), include, exclude: exclude.unwrap_or_default() }
    }

    /// The sources a load keeps, in the order given. A folder source is dropped unless folders load. An excluded
    /// identifier is dropped even when it is also included. A repeated identifier keeps its first entry.
    pub fn filter(&self, sources: &[MediaSource]) -> Vec<MediaSource> {
        let mut kept: Vec<MediaSource> = Vec::new();
        for s in sources {
            if s.is_folder && !self.load_folders {
                continue;
            }
            if self.exclude.iter().any(|e| e == &s.identifier) {
                continue;
            }
            if let Some(list) = &self.include {
                if !list.iter().any(|i| i == &s.identifier) {
                    continue;
                }
            }
            if kept.iter().any(|k| k.identifier == s.identifier) {
                continue;
            }
            kept.push(s.clone());
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<MediaSource> {
        vec![
            MediaSource::new("a", SourceType::Audio, false).unwrap(),
            MediaSource::new("b", SourceType::Image, false).unwrap(),
            MediaSource::new("f", SourceType::Movie, true).unwrap(),
        ]
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn names(sources: &[MediaSource]) -> Vec<String> {
        sources.iter().map(|s| s.identifier.clone()).collect()
    }

    #[test]
    fn loading_everything_keeps_every_source_in_order() {
        assert_eq!(names(&LoadOptions::all().filter(&sample())), ids(&["a", "b", "f"]));
    }

    #[test]
    fn folder_sources_are_dropped_when_folders_do_not_load() {
        let opts = LoadOptions { load_folders: false, include: None, exclude: Vec::new() };
        assert_eq!(names(&opts.filter(&sample())), ids(&["a", "b"]));
    }

    #[test]
    fn an_include_list_keeps_only_those_sources_in_source_order() {
        let opts = LoadOptions { load_folders: true, include: Some(ids(&["b", "a"])), exclude: Vec::new() };
        assert_eq!(names(&opts.filter(&sample())), ids(&["a", "b"]), "the order is the source order, not the list order");
    }

    #[test]
    fn an_exclude_list_drops_sources_and_wins_over_the_include_list() {
        let excluded = LoadOptions { load_folders: true, include: None, exclude: ids(&["a"]) };
        assert_eq!(names(&excluded.filter(&sample())), ids(&["b", "f"]));
        let both = LoadOptions { load_folders: true, include: Some(ids(&["a", "b"])), exclude: ids(&["a"]) };
        assert_eq!(names(&both.filter(&sample())), ids(&["b"]));
    }

    #[test]
    fn a_repeated_identifier_keeps_its_first_source() {
        let sources = vec![
            MediaSource::new("a", SourceType::Audio, false).unwrap(),
            MediaSource::new(" a ", SourceType::Image, false).unwrap(),
        ];
        let kept = LoadOptions::all().filter(&sources);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, SourceType::Audio);
    }

    #[test]
    fn absent_load_keys_load_everything_and_present_ones_narrow_the_load() {
        assert_eq!(LoadOptions::from_keys(None, None, None), LoadOptions::all());
        let opts = LoadOptions::from_keys(Some(false), Some(ids(&["a", "f"])), None);
        assert_eq!(names(&opts.filter(&sample())), ids(&["a"]), "the folder is dropped by the folders key");
    }

    #[test]
    fn a_blank_identifier_is_refused_and_a_padded_one_is_trimmed() {
        assert!(MediaSource::new("   ", SourceType::Movie, false).is_none());
        assert_eq!(MediaSource::new(" x ", SourceType::Movie, false).unwrap().identifier, "x");
    }
}
