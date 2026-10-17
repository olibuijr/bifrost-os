//! Dictation commands: spoken punctuation and editing words turned into text edits (std only, pure).
//!
//! A recognizer returns words. A person dictating also says what is not a word: "comma", "new paragraph", "caps on",
//! "scratch that". `Interpreter` reads each recognized phrase, carries out the commands in it and keeps the text so far,
//! with the spacing and capitals a typist would produce: punctuation attaches to the word before it, a sentence end
//! capitalizes the next word, an opening bracket or quote attaches to the word after it, a new line starts a sentence.
//!
//! * Punctuation: period, comma, question mark, exclamation mark, colon, semicolon, ellipsis, hyphen, quotes, brackets.
//! * Layout: new line, new paragraph.
//! * Case: cap (next word), caps on, caps off.
//! * Editing: no space (join the next word), scratch that (take back the last phrase), literal (say the next word as
//!   a word, so "literal period" types the word).
//!
//! Command words are matched ignoring case and the punctuation a recognizer adds around them ("Period." is the command).
//! The tables are per language; a phrase in a language with no table is typed as it is. `feed_edit` returns the change
//! as backspaces plus an insert, so a control that cannot be rewritten (a key stream into another app) can follow
//! the text exactly, a "scratch that" included.

/// The language of the spoken commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Icelandic,
}

/// What a command word does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Punctuation that attaches to the word before it; `capitalize` starts a sentence.
    Attach { text: &'static str, capitalize: bool },
    /// An opening bracket: a space before it, none after.
    Open(&'static str),
    /// A mark with no space on either side (a hyphen).
    Join(&'static str),
    /// A line break; the next word starts a sentence and takes no space.
    Break(&'static str),
    /// A quote that opens when none is open and closes otherwise.
    Quote,
    OpenQuote,
    CloseQuote,
    Capitalize,
    CapsOn,
    CapsOff,
    NoSpace,
    Literal,
    Scratch,
}

const fn attach(text: &'static str) -> Action {
    Action::Attach { text, capitalize: false }
}

const fn sentence(text: &'static str) -> Action {
    Action::Attach { text, capitalize: true }
}

const ENGLISH: &[(&str, Action)] = &[
    ("period", sentence(".")),
    ("full stop", sentence(".")),
    ("comma", attach(",")),
    ("question mark", sentence("?")),
    ("exclamation mark", sentence("!")),
    ("exclamation point", sentence("!")),
    ("colon", attach(":")),
    ("semicolon", attach(";")),
    ("ellipsis", attach("\u{2026}")),
    ("dot dot dot", attach("\u{2026}")),
    ("hyphen", Action::Join("-")),
    ("dash", Action::Join("-")),
    ("open parenthesis", Action::Open("(")),
    ("close parenthesis", attach(")")),
    ("open bracket", Action::Open("[")),
    ("close bracket", attach("]")),
    ("open quote", Action::OpenQuote),
    ("close quote", Action::CloseQuote),
    ("end quote", Action::CloseQuote),
    ("quote", Action::Quote),
    ("new line", Action::Break("\n")),
    ("new paragraph", Action::Break("\n\n")),
    ("cap", Action::Capitalize),
    ("caps on", Action::CapsOn),
    ("caps off", Action::CapsOff),
    ("no space", Action::NoSpace),
    ("scratch that", Action::Scratch),
    ("literal", Action::Literal),
];

const ICELANDIC: &[(&str, Action)] = &[
    ("punktur", sentence(".")),
    ("komma", attach(",")),
    ("spurningarmerki", sentence("?")),
    ("upphrópunarmerki", sentence("!")),
    ("tvípunktur", attach(":")),
    ("semíkomma", attach(";")),
    ("bandstrik", Action::Join("-")),
    ("opna sviga", Action::Open("(")),
    ("loka sviga", attach(")")),
    ("opna gæsalappir", Action::OpenQuote),
    ("loka gæsalöppum", Action::CloseQuote),
    ("ný lína", Action::Break("\n")),
    ("ný málsgrein", Action::Break("\n\n")),
    ("há stafur", Action::Capitalize),
    ("hástafir á", Action::CapsOn),
    ("hástafir af", Action::CapsOff),
    ("ekkert bil", Action::NoSpace),
    ("taktu það til baka", Action::Scratch),
    ("bókstaflega", Action::Literal),
];

/// The most phrases of undo kept.
const HISTORY: usize = 64;
/// The longest command, in words.
const LONGEST: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    text: String,
    caps_next: bool,
    all_caps: bool,
    /// The next word takes no space before it.
    glue: bool,
    /// The next word is a word even if it names a command.
    literal: bool,
    quote_open: bool,
}

impl State {
    fn new() -> State {
        State { text: String::new(), caps_next: true, all_caps: false, glue: false, literal: false, quote_open: false }
    }
}

/// What a phrase changed: remove `backspaces` characters from the end, then add `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub backspaces: usize,
    pub insert: String,
}

/// Dictated text, built phrase by phrase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interpreter {
    language: Language,
    state: State,
    history: Vec<State>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Interpreter::new(Language::English)
    }
}

/// A token lowercased, without the sentence punctuation around it.
fn normalized(word: &str) -> String {
    word.trim_matches(|c: char| matches!(c, '.' | ',' | '?' | '!' | ';' | ':' | '"')).to_lowercase()
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Interpreter {
    pub fn new(language: Language) -> Interpreter {
        Interpreter { language, state: State::new(), history: Vec::new() }
    }

    /// The text so far.
    pub fn text(&self) -> &str {
        &self.state.text
    }

    /// Returns the text and starts over (capital first word, empty undo).
    pub fn take_text(&mut self) -> String {
        self.history.clear();
        std::mem::replace(&mut self.state, State::new()).text
    }

    fn table(&self) -> &'static [(&'static str, Action)] {
        match self.language {
            Language::English => ENGLISH,
            Language::Icelandic => ICELANDIC,
        }
    }

    /// The command at the start of `words`, with how many words it uses (the longest match wins).
    fn command(&self, words: &[&str]) -> Option<(Action, usize)> {
        for n in (1..=LONGEST.min(words.len())).rev() {
            let phrase = words[..n].iter().map(|w| normalized(w)).collect::<Vec<_>>().join(" ");
            if let Some((_, action)) = self.table().iter().find(|(name, _)| *name == phrase) {
                return Some((*action, n));
            }
        }
        None
    }

    /// A space, unless the text is empty, ends a line, or the next word is glued on.
    fn separate(&mut self) {
        let s = &mut self.state;
        if !(s.text.is_empty() || s.glue || s.text.ends_with(char::is_whitespace)) {
            s.text.push(' ');
        }
        s.glue = false;
    }

    fn word(&mut self, word: &str) {
        let shaped = if self.state.all_caps {
            word.to_uppercase()
        } else if self.state.caps_next {
            capitalized(word)
        } else {
            word.to_string()
        };
        self.separate();
        self.state.caps_next = shaped.ends_with(['.', '?', '!']);
        self.state.text.push_str(&shaped);
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Attach { text, capitalize } => {
                self.state.text.push_str(text);
                self.state.glue = false;
                self.state.caps_next = capitalize;
            }
            Action::Open(text) => {
                self.separate();
                self.state.text.push_str(text);
                self.state.glue = true;
            }
            Action::Join(text) => {
                self.state.text.push_str(text);
                self.state.glue = true;
            }
            Action::Break(text) => {
                while self.state.text.ends_with(' ') {
                    self.state.text.pop();
                }
                self.state.text.push_str(text);
                self.state.glue = true;
                self.state.caps_next = true;
            }
            Action::Quote => self.apply(if self.state.quote_open { Action::CloseQuote } else { Action::OpenQuote }),
            Action::OpenQuote => {
                self.separate();
                self.state.text.push('"');
                self.state.glue = true;
                self.state.quote_open = true;
            }
            Action::CloseQuote => {
                self.state.text.push('"');
                self.state.glue = false;
                self.state.quote_open = false;
            }
            Action::Capitalize => self.state.caps_next = true,
            Action::CapsOn => self.state.all_caps = true,
            Action::CapsOff => self.state.all_caps = false,
            Action::NoSpace => self.state.glue = true,
            Action::Literal => self.state.literal = true,
            // handled by `feed`, which owns the undo history
            Action::Scratch => {}
        }
    }

    /// Reads one recognized phrase.
    pub fn feed(&mut self, phrase: &str) {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        if words.is_empty() {
            return;
        }
        let mut before = self.state.clone();
        let mut i = 0;
        while i < words.len() {
            if self.state.literal {
                self.state.literal = false;
                self.word(words[i]);
                i += 1;
                continue;
            }
            match self.command(&words[i..]) {
                Some((Action::Scratch, used)) => {
                    if self.state != before {
                        // words of this phrase came before the command: they go too
                        self.state = before.clone();
                    } else if let Some(previous) = self.history.pop() {
                        self.state = previous;
                        before = self.state.clone();
                    }
                    i += used;
                }
                Some((action, used)) => {
                    self.apply(action);
                    i += used;
                }
                None => {
                    self.word(words[i]);
                    i += 1;
                }
            }
        }
        if self.state != before {
            self.history.push(before);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
    }

    /// Reads one recognized phrase and returns how the text changed.
    pub fn feed_edit(&mut self, phrase: &str) -> Edit {
        let old = self.state.text.clone();
        self.feed(phrase);
        let new = &self.state.text;
        let common = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
        let backspaces = old.chars().count() - common;
        let insert: String = new.chars().skip(common).collect();
        Edit { backspaces, insert }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(phrases: &[&str]) -> String {
        let mut i = Interpreter::default();
        for p in phrases {
            i.feed(p);
        }
        i.text().to_string()
    }

    #[test]
    fn sentence_punctuation_attaches_and_capitalizes_the_next_word() {
        assert_eq!(said(&["hello world period how are you question mark"]), "Hello world. How are you?");
        assert_eq!(said(&["stop exclamation mark go on"]), "Stop! Go on");
        assert_eq!(said(&["apples comma oranges and pears"]), "Apples, oranges and pears");
        assert_eq!(said(&["note colon one semicolon two"]), "Note: one; two");
        assert_eq!(said(&["well ellipsis maybe"]), "Well\u{2026} maybe");
    }

    #[test]
    fn commands_match_without_case_or_recognizer_punctuation() {
        assert_eq!(said(&["Hello Period"]), "Hello.");
        assert_eq!(said(&["Hello, Comma. world."]), "Hello,, world.");
        assert_eq!(said(&["Thanks", "Period."]), "Thanks.");
    }

    #[test]
    fn a_command_can_span_phrases() {
        assert_eq!(said(&["hello", "period", "again"]), "Hello. Again");
    }

    #[test]
    fn lines_and_paragraphs_start_sentences_without_a_space() {
        assert_eq!(said(&["first line new line second line"]), "First line\nSecond line");
        assert_eq!(said(&["one period new paragraph two"]), "One.\n\nTwo");
    }

    #[test]
    fn case_commands() {
        assert_eq!(said(&["caps on nasa caps off launched it"]), "NASA launched it");
        assert_eq!(said(&["meet me in cap paris"]), "Meet me in Paris");
        assert_eq!(said(&["hello cap world"]), "Hello World");
    }

    #[test]
    fn quotes_and_brackets_hug_what_they_hold() {
        assert_eq!(said(&["she said open quote hello close quote"]), "She said \"hello\"");
        assert_eq!(said(&["she said quote hello quote period"]), "She said \"hello\".");
        assert_eq!(said(&["see open parenthesis figure two close parenthesis"]), "See (figure two)");
        assert_eq!(said(&["a open bracket b close bracket"]), "A [b]");
    }

    #[test]
    fn hyphen_and_no_space_join_words() {
        assert_eq!(said(&["well hyphen known"]), "Well-known");
        assert_eq!(said(&["web no space site"]), "Website");
    }

    #[test]
    fn literal_types_the_command_word() {
        assert_eq!(said(&["the literal period of time"]), "The period of time");
        assert_eq!(said(&["say literal comma"]), "Say comma");
    }

    #[test]
    fn scratch_that_takes_back_the_last_phrase() {
        assert_eq!(said(&["hello there", "scratch that", "goodbye"]), "Goodbye");
        assert_eq!(said(&["one period", "two three", "scratch that", "four"]), "One. Four");
        // words before the command in the same phrase go too
        assert_eq!(said(&["alpha", "beta gamma scratch that delta"]), "Alpha delta");
        // nothing to take back
        assert_eq!(said(&["scratch that", "hello"]), "Hello");
        assert_eq!(said(&["hello", "scratch that", "scratch that", "again"]), "Again");
    }

    #[test]
    fn other_words_pass_through() {
        assert_eq!(said(&["the quick brown fox"]), "The quick brown fox");
        assert_eq!(said(&["it is 5 o'clock"]), "It is 5 o'clock");
        assert_eq!(said(&["", "   "]), "");
        // a recognizer that already punctuates keeps its marks and the capital that follows
        assert_eq!(said(&["Hello, world. how are you"]), "Hello, world. How are you");
    }

    #[test]
    fn icelandic_commands() {
        let mut i = Interpreter::new(Language::Icelandic);
        i.feed("halló heimur punktur ný lína góðan dag komma vinur");
        assert_eq!(i.text(), "Halló heimur.\nGóðan dag, vinur");
        let mut en_in_is = Interpreter::new(Language::Icelandic);
        en_in_is.feed("hello period");
        assert_eq!(en_in_is.text(), "Hello period", "English commands are not read in an Icelandic session");
    }

    #[test]
    fn the_edit_follows_the_text_including_a_scratch() {
        let mut i = Interpreter::default();
        assert_eq!(i.feed_edit("hello world"), Edit { backspaces: 0, insert: "Hello world".into() });
        assert_eq!(i.feed_edit("period"), Edit { backspaces: 0, insert: ".".into() });
        assert_eq!(i.feed_edit("how are you"), Edit { backspaces: 0, insert: " How are you".into() });
        let e = i.feed_edit("scratch that");
        assert_eq!(e, Edit { backspaces: " How are you".chars().count(), insert: String::new() });
        assert_eq!(i.text(), "Hello world.");
        // replaying the edits on a plain buffer gives the interpreter's text
        let mut field = String::new();
        let mut j = Interpreter::default();
        for p in ["one comma two", "scratch that", "three period four"] {
            let e = j.feed_edit(p);
            for _ in 0..e.backspaces {
                field.pop();
            }
            field.push_str(&e.insert);
        }
        assert_eq!(field, j.text());
        assert_eq!(field, "Three. Four");
    }

    #[test]
    fn taking_the_text_starts_over_with_a_capital() {
        let mut i = Interpreter::default();
        i.feed("hello period");
        assert_eq!(i.take_text(), "Hello.");
        assert_eq!(i.text(), "");
        i.feed("again");
        assert_eq!(i.text(), "Again");
        i.feed("scratch that");
        i.feed("scratch that");
        assert_eq!(i.text(), "");
    }

    #[test]
    fn multi_byte_text_is_counted_in_characters() {
        let mut i = Interpreter::default();
        i.feed("þetta er gott");
        assert_eq!(i.text(), "Þetta er gott");
        let e = i.feed_edit("scratch that");
        assert_eq!(e.backspaces, "Þetta er gott".chars().count());
    }

    #[test]
    fn a_dictation_session_can_read_commands() {
        use crate::dictation_entry::{Availability, Dictation};
        let mut d = Dictation::with_commands(Language::English);
        assert!(!d.deliver("dropped while idle"));
        d.start(Availability::Available).unwrap();
        assert!(d.deliver("dear sam comma"));
        assert!(d.deliver("see you soon period"));
        assert!(d.deliver("scratch that"));
        assert!(d.deliver("thanks exclamation mark"));
        d.stop();
        assert_eq!(d.text(), "Dear sam, thanks!");
        assert_eq!(d.take_text(), "Dear sam, thanks!");
        assert_eq!(d.text(), "");
        // a plain session still types phrases as they come
        let mut plain = Dictation::new();
        plain.start(Availability::Available).unwrap();
        plain.deliver("hello comma");
        assert_eq!(plain.text(), "hello comma");
    }
}
