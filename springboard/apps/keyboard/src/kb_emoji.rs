//! Emoji keyboard extras: persisted Frequently Used, and keyword search for
//! the search field above the grid.

use crate::emoji::{Emoji, CATEGORIES, PER_PAGE};
use std::path::PathBuf;

/// Where Frequently Used survives restarts.
pub fn file() -> PathBuf {
    // the app's own data directory (the sandbox does not expose the shell's)
    std::env::var_os("BIFROST_DATA").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir()).join("emoji_recents.txt")
}

/// Names for the emoji people actually look for: (emoji, space separated keywords).
const KEYWORDS: &[(&str, &str)] = &[
    ("😀", "grin smile happy face"), ("😃", "smile happy"), ("😄", "smile happy laugh"), ("😁", "grin beam"), ("😆", "laugh squint"),
    ("😅", "sweat nervous"), ("😂", "joy tears laugh lol"), ("🤣", "rofl laugh floor"), ("🙂", "slight smile"), ("🙃", "upside down"),
    ("😉", "wink"), ("😊", "blush smile"), ("😇", "angel halo innocent"), ("🥰", "love hearts adore"), ("😍", "heart eyes love"),
    ("🤩", "star struck wow"), ("😘", "kiss blow"), ("😋", "yum tasty"), ("😛", "tongue"), ("😜", "wink tongue crazy"),
    ("🤪", "zany crazy"), ("🤔", "think thinking hmm"), ("🤨", "eyebrow skeptical"), ("😐", "neutral"), ("🙄", "eye roll"),
    ("😏", "smirk"), ("😴", "sleep sleeping zzz tired"), ("😢", "cry sad tear"), ("😭", "sob cry loud"), ("😡", "angry mad rage"),
    ("🥳", "party celebrate"), ("😎", "cool sunglasses"), ("👍", "thumbs up yes like ok"), ("👎", "thumbs down no dislike"),
    ("👌", "ok perfect"), ("✌", "victory peace"), ("🤞", "fingers crossed luck"), ("🤘", "rock horns"), ("👋", "wave hello bye hand"),
    ("👏", "clap applause"), ("🙌", "raised hands hooray"), ("🤝", "handshake deal"), ("🙏", "pray please thanks"), ("💪", "muscle strong flex"),
    ("👀", "eyes look"), ("🧠", "brain"), ("👶", "baby"), ("👨", "man"), ("👩", "woman"), ("💃", "dance dancer"), ("🕺", "dance"),
    ("🐶", "dog puppy"), ("🐱", "cat kitten"), ("🐭", "mouse"), ("🐰", "rabbit bunny"), ("🦊", "fox"), ("🐻", "bear"), ("🐼", "panda"),
    ("🐯", "tiger"), ("🦁", "lion"), ("🐮", "cow"), ("🐷", "pig"), ("🐸", "frog"), ("🐵", "monkey"), ("🐔", "chicken"), ("🐧", "penguin"),
    ("🦉", "owl"), ("🐴", "horse"), ("🦄", "unicorn"), ("🐝", "bee"), ("🦋", "butterfly"), ("🐢", "turtle"), ("🐍", "snake"),
    ("🐙", "octopus"), ("🐬", "dolphin"), ("🐳", "whale"), ("🌲", "tree pine"), ("🌹", "rose flower"),
    ("🍎", "apricot fruit red"), ("🍊", "orange"), ("🍋", "lemon"), ("🍌", "banana"), ("🍉", "watermelon"), ("🍇", "grapes"),
    ("🍓", "strawberry"), ("🍒", "cherry"), ("🍑", "peach"), ("🍍", "pineapple"), ("🥑", "avocado"), ("🥕", "carrot"), ("🌽", "corn"),
    ("🍞", "bread"), ("🧀", "cheese"), ("🥚", "egg"), ("🍳", "cooking fried egg"), ("🥓", "bacon"), ("🍔", "burger hamburger"),
    ("🍟", "fries"), ("🍕", "pizza"), ("🌭", "hot dog"), ("🌮", "taco"), ("🍣", "sushi"), ("🍦", "ice cream"), ("🍫", "chocolate"),
    ("☕", "coffee tea hot"), ("🍺", "beer"),
    ("⚽", "soccer football"), ("🏀", "basketball"), ("🏈", "american football"), ("⚾", "baseball"), ("🎾", "tennis"),
    ("🎮", "game video controller"), ("🎲", "dice"), ("🎸", "guitar"), ("🎹", "piano keyboard"), ("🎤", "microphone sing"),
    ("🎧", "headphones music"), ("🎬", "movie film"), ("🎨", "art paint"), ("🏆", "trophy win"), ("🥇", "gold medal first"),
    ("🚗", "car"), ("🚕", "taxi"), ("🚌", "bus"), ("🚓", "police"), ("🚑", "ambulance"), ("🚒", "fire truck"), ("🚲", "bike bicycle"),
    ("✈", "airplane plane flight"), ("🚀", "rocket launch"), ("🚁", "helicopter"), ("⛵", "boat sail"), ("🚂", "train"), ("🏠", "house home"),
    ("🏢", "office building"), ("🏥", "hospital"), ("🌋", "volcano"), ("🌅", "sunrise"), ("🗺", "map world"), ("⌚", "watch time"), ("📱", "phone mobile"),
    ("💡", "idea bulb light"), ("🔋", "battery"), ("💻", "laptop computer"), ("📷", "camera photo"), ("📞", "phone call"), ("📺", "tv television"),
    ("⏰", "alarm clock"), ("🔑", "key"), ("🔒", "lock"), ("🔨", "hammer"), ("🔧", "wrench tool"), ("💰", "money bag"), ("💳", "credit card"),
    ("📦", "package box"), ("📧", "email mail"), ("📝", "memo note write"), ("📚", "books"), ("📌", "pin"), ("✂", "scissors cut"),
    ("🔍", "search magnifier find"), ("🎁", "gift present"), ("🎈", "balloon"), ("🎉", "party tada celebrate"),
    ("❤", "heart love red"), ("🧡", "orange heart"), ("💛", "yellow heart"), ("💚", "green heart"), ("💙", "blue heart"), ("💜", "purple heart"),
    ("🖤", "black heart"), ("🤍", "white heart"), ("💔", "broken heart"), ("💯", "hundred perfect"), ("🔥", "fire hot lit"), ("✨", "sparkles"),
    ("⭐", "star"), ("🌟", "glowing star"), ("⚡", "lightning bolt"), ("☀", "sun sunny"), ("🌙", "moon night"), ("☁", "cloud"),
    ("🌈", "rainbow"), ("❄", "snow snowflake cold"), ("☔", "rain umbrella"), ("✅", "check done yes"), ("❌", "cross no wrong"),
    ("❓", "question"), ("❗", "exclamation"), ("⚠", "warning"),
    ("🏁", "chequered finish race"), ("🚩", "flag red"), ("🏴", "black flag"), ("🏳", "white flag"), ("🇮🇸", "iceland"), ("🇺🇸", "usa united states america"),
    ("🇬🇧", "uk united kingdom britain england"), ("🇩🇪", "germany"), ("🇫🇷", "france"), ("🇪🇸", "spain"), ("🇮🇹", "italy"),
    ("🇳🇴", "norway"), ("🇸🇪", "sweden"), ("🇩🇰", "denmark"), ("🇫🇮", "finland"), ("🇳🇱", "netherlands"), ("🇧🇪", "belgium"),
    ("🇨🇭", "switzerland"), ("🇦🇹", "austria"), ("🇵🇹", "portugal"), ("🇮🇪", "ireland"), ("🇵🇱", "poland"), ("🇨🇦", "canada"),
    ("🇲🇽", "mexico"), ("🇧🇷", "brazil"), ("🇦🇷", "argentina"), ("🇯🇵", "japan"), ("🇰🇷", "korea"), ("🇨🇳", "china"), ("🇮🇳", "india"),
    ("🇦🇺", "australia"), ("🇳🇿", "new zealand"),
];

impl Emoji {
    /// A set whose Frequently Used list is read from, and saved to, `path`.
    pub fn persistent(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut e = Emoji::default();
        if let Ok(text) = std::fs::read_to_string(&path) {
            e.recent = text.lines().filter(|l| !l.is_empty()).take(PER_PAGE).map(String::from).collect();
        }
        e.path = Some(path);
        e
    }

    /// The on-device set; the host and tests stay in memory.
    pub fn on_device() -> Self {
        if cfg!(test) {
            Emoji::default()
        } else {
            Emoji::persistent(file())
        }
    }

    pub(super) fn save(&self) {
        if let Some(p) = &self.path {
            let _ = std::fs::write(p, self.recent.iter().map(|e| format!("{e}\n")).collect::<String>());
        }
    }

    pub fn searching(&self) -> bool {
        self.searching
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn begin_search(&mut self) {
        self.searching = true;
    }

    pub fn end_search(&mut self) {
        self.searching = false;
    }

    pub fn search_type(&mut self, text: &str) {
        self.query.push_str(&text.to_lowercase());
    }

    pub fn search_backspace(&mut self) {
        self.query.pop();
    }

    /// Emoji whose keywords start with every word of the query (at most one page).
    pub fn results(&self) -> Vec<String> {
        let words: Vec<&str> = self.query.split_whitespace().collect();
        let known = CATEGORIES.iter().flat_map(|c| c.emoji.iter().copied());
        known
            .filter(|e| {
                let kw = KEYWORDS.iter().find(|(k, _)| k == e).map_or("", |(_, w)| *w);
                words.iter().all(|q| kw.split_whitespace().any(|w| w.starts_with(q)))
            })
            .take(PER_PAGE)
            .map(String::from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_keyword_prefixes() {
        let mut e = Emoji::default();
        e.search_type("Dog");
        assert_eq!(e.results(), ["🐶", "🌭"]);
        e.search_backspace();
        e.search_backspace();
        e.search_backspace();
        e.search_type("heart");
        assert!(e.results().contains(&"❤".to_string()));
        assert_eq!(e.page(), e.results());
        e.set_tab(1);
        assert!(e.query().is_empty());
    }

    #[test]
    fn recents_persist() {
        let path = std::env::temp_dir().join(format!("bifrost-kb-emoji-recents-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut e = Emoji::persistent(&path);
        e.used("🔥");
        e.used("😀");
        assert_eq!(Emoji::persistent(&path).recent(), ["😀", "🔥"]);
        let _ = std::fs::remove_file(&path);
    }
}
