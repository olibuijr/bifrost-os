//! Custom vocabulary for typed and spoken requests: the names an app answers to (its name and synonyms) and the
//! phrases that run one of its intents. A request is normalized (case folded, punctuation and runs of spaces turned
//! into single spaces) and then matched exactly against the vocabulary. One match is the answer; several different
//! targets for the same request are reported as ambiguous, so the caller can ask the user which one is meant.
//!
//! The model is pure: it holds strings and returns answers. It does not launch apps or read the intent manifests;
//! the caller adds the names and phrases an app declares.

/// Longest normalized name or phrase, in bytes.
pub const MAX_TEXT: usize = 128;
/// Most names (the app name and its synonyms) one app may register.
pub const MAX_NAMES_PER_APP: usize = 16;
/// Most phrases kept in one vocabulary.
pub const MAX_PHRASES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An empty app or intent id, or text with no letters or digits.
    Empty,
    /// Normalized text longer than [`MAX_TEXT`].
    TooLong,
    TooManyNames,
    TooManyPhrases,
}

/// What a request can point at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Open the app.
    App(String),
    /// Run one intent of an app.
    Intent { app: String, intent: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    NoMatch,
    Match(Target),
    /// More than one different target matches; the caller asks the user which one is meant.
    Ambiguous(Vec<Target>),
}

/// Names and phrases for the apps on one system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vocabulary {
    /// (app id, normalized name): the app name and its synonyms.
    names: Vec<(String, String)>,
    /// (app id, intent id, normalized phrase).
    phrases: Vec<(String, String, String)>,
}

/// Lowercase text with every run of non-alphanumeric characters replaced by one space, and no leading or trailing
/// space. Letters outside ASCII are kept and folded the same way.
pub fn normalize(text: &str) -> String {
    let mut spaced = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                spaced.push(lower);
            }
        } else {
            spaced.push(' ');
        }
    }
    spaced.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Normalizes text for the vocabulary. Empty after normalization is an error, and so is too long.
fn checked(text: &str) -> Result<String, Error> {
    let norm = normalize(text);
    if norm.is_empty() {
        return Err(Error::Empty);
    }
    if norm.len() > MAX_TEXT {
        return Err(Error::TooLong);
    }
    Ok(norm)
}

fn push_unique(hits: &mut Vec<Target>, target: Target) {
    if !hits.contains(&target) {
        hits.push(target);
    }
}

impl Vocabulary {
    pub fn new() -> Vocabulary {
        Vocabulary::default()
    }

    /// Adds a name or synonym for an app. Adding the same name for the same app again does nothing.
    pub fn add_name(&mut self, app: &str, text: &str) -> Result<(), Error> {
        if app.is_empty() {
            return Err(Error::Empty);
        }
        let norm = checked(text)?;
        if self.names.iter().any(|(a, t)| a.as_str() == app && *t == norm) {
            return Ok(());
        }
        if self.names.iter().filter(|(a, _)| a.as_str() == app).count() >= MAX_NAMES_PER_APP {
            return Err(Error::TooManyNames);
        }
        self.names.push((app.to_string(), norm));
        Ok(())
    }

    /// Adds a phrase that runs an intent of an app. The same phrase for the same intent again does nothing.
    pub fn add_phrase(&mut self, app: &str, intent: &str, text: &str) -> Result<(), Error> {
        if app.is_empty() || intent.is_empty() {
            return Err(Error::Empty);
        }
        let norm = checked(text)?;
        if self.phrases.iter().any(|(a, i, t)| a.as_str() == app && i.as_str() == intent && *t == norm) {
            return Ok(());
        }
        if self.phrases.len() >= MAX_PHRASES {
            return Err(Error::TooManyPhrases);
        }
        self.phrases.push((app.to_string(), intent.to_string(), norm));
        Ok(())
    }

    /// The normalized names registered for an app, in the order they were added.
    pub fn names_of(&self, app: &str) -> Vec<&str> {
        self.names.iter().filter(|(a, _)| a.as_str() == app).map(|(_, t)| t.as_str()).collect()
    }

    /// Removes an app's names and phrases and returns how many entries went.
    pub fn remove_app(&mut self, app: &str) -> usize {
        let before = self.names.len() + self.phrases.len();
        self.names.retain(|(a, _)| a.as_str() != app);
        self.phrases.retain(|(a, _, _)| a.as_str() != app);
        before - (self.names.len() + self.phrases.len())
    }

    /// Finds what a request means. App names are checked first, then intent phrases; a target found twice counts
    /// once. A request with no letters or digits matches nothing.
    pub fn lookup(&self, query: &str) -> Lookup {
        let q = normalize(query);
        if q.is_empty() {
            return Lookup::NoMatch;
        }
        let mut hits: Vec<Target> = Vec::new();
        for (app, text) in &self.names {
            if *text == q {
                push_unique(&mut hits, Target::App(app.clone()));
            }
        }
        for (app, intent, text) in &self.phrases {
            if *text == q {
                push_unique(&mut hits, Target::Intent { app: app.clone(), intent: intent.clone() });
            }
        }
        match hits.len() {
            0 => Lookup::NoMatch,
            1 => Lookup::Match(hits.swap_remove(0)),
            _ => Lookup::Ambiguous(hits),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> Target {
        Target::App(id.to_string())
    }

    fn intent(app: &str, intent: &str) -> Target {
        Target::Intent { app: app.to_string(), intent: intent.to_string() }
    }

    #[test]
    fn normalize_folds_case_and_punctuation() {
        assert_eq!(normalize("  Open   NOTES! "), "open notes");
        // the apostrophe and the hyphen are separators; the capital O folds to a lowercase letter
        assert_eq!(normalize("Ólafur's-notes"), "ólafur s notes");
        assert_eq!(normalize(" !! "), "");
    }

    #[test]
    fn a_synonym_matches_the_app_after_normalizing() {
        let mut v = Vocabulary::new();
        v.add_name("notes", "Notes").unwrap();
        v.add_name("notes", "Notebook").unwrap();
        assert_eq!(v.lookup("notebook!"), Lookup::Match(app("notes")));
        assert_eq!(v.lookup("NOTES"), Lookup::Match(app("notes")));
        assert_eq!(v.lookup("calendar"), Lookup::NoMatch);
        assert_eq!(v.names_of("notes"), vec!["notes", "notebook"]);
    }

    #[test]
    fn empty_and_too_long_names_are_refused() {
        let mut v = Vocabulary::new();
        assert_eq!(v.add_name("notes", "  !! "), Err(Error::Empty));
        assert_eq!(v.add_name("", "notes"), Err(Error::Empty));
        assert_eq!(v.add_name("notes", &"a".repeat(129)), Err(Error::TooLong));
        assert_eq!(v.add_name("notes", &"a".repeat(128)), Ok(()));
        assert!(v.names_of("notes").len() == 1);
    }

    #[test]
    fn an_app_holds_at_most_sixteen_names_and_repeats_are_free() {
        let mut v = Vocabulary::new();
        for i in 0..16 {
            v.add_name("music", &format!("name {}", i)).unwrap();
        }
        // a repeat of an existing name is accepted and adds nothing
        assert_eq!(v.add_name("music", "Name 3"), Ok(()));
        assert_eq!(v.names_of("music").len(), 16);
        assert_eq!(v.add_name("music", "name 16"), Err(Error::TooManyNames));
    }

    #[test]
    fn a_phrase_runs_its_intent() {
        let mut v = Vocabulary::new();
        v.add_phrase("notes", "new-note", "New note").unwrap();
        assert_eq!(v.lookup("new  note"), Lookup::Match(intent("notes", "new-note")));
        assert_eq!(v.lookup("new"), Lookup::NoMatch);
    }

    #[test]
    fn the_same_phrase_for_two_apps_is_ambiguous() {
        let mut v = Vocabulary::new();
        v.add_phrase("notes", "new-note", "new note").unwrap();
        v.add_phrase("music", "new-note", "New note").unwrap();
        assert_eq!(
            v.lookup("new note"),
            Lookup::Ambiguous(vec![intent("notes", "new-note"), intent("music", "new-note")])
        );
    }

    #[test]
    fn an_app_name_and_an_intent_phrase_can_collide() {
        let mut v = Vocabulary::new();
        v.add_name("notes", "notes").unwrap();
        v.add_phrase("music", "play", "Notes").unwrap();
        // the app name is listed first because names are checked before phrases
        assert_eq!(v.lookup("notes"), Lookup::Ambiguous(vec![app("notes"), intent("music", "play")]));
    }

    #[test]
    fn remove_app_drops_its_names_and_phrases_only() {
        let mut v = Vocabulary::new();
        v.add_name("notes", "notes").unwrap();
        v.add_name("notes", "notebook").unwrap();
        v.add_phrase("notes", "new-note", "new note").unwrap();
        v.add_name("music", "music").unwrap();
        assert_eq!(v.remove_app("notes"), 3);
        assert_eq!(v.lookup("notes"), Lookup::NoMatch);
        assert_eq!(v.lookup("music"), Lookup::Match(app("music")));
        assert_eq!(v.remove_app("notes"), 0);
    }
}
