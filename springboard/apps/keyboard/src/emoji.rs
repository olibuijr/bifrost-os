//! The keyboard's emoji page: categories of up to one screenful each and a
//! most-recently-used list that tops the category tabs.

/// Emoji shown per screen: 3 rows of 11.
pub const PER_PAGE: usize = 33;

pub struct Category {
    /// the tab's label
    pub icon: &'static str,
    pub emoji: &'static [&'static str],
}

pub const CATEGORIES: [Category; 9] = [
    Category {
        icon: "🙂",
        emoji: &[
            "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "🙂", "🙃", "😉", "😊", "😇", "🥰", "😍", "🤩", "😘", "😋", "😛", "😜", "🤪", "🤔", "🤨", "😐", "😶", "🙄", "😏", "😴", "😢", "😭", "😡",
            "🥳", "😎",
        ],
    },
    Category {
        icon: "👋",
        emoji: &[
            "👍", "👎", "👌", "✌", "🤞", "🤟", "🤘", "👈", "👉", "👆", "👇", "👋", "🤚", "🖐", "✋", "🖖", "👏", "🙌", "🤝", "🙏", "💪", "👀", "👂", "👃", "👄", "🧠", "👶", "🧑", "👨", "👩", "🧓",
            "💃", "🕺",
        ],
    },
    Category {
        icon: "🐶",
        emoji: &[
            "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵", "🐔", "🐧", "🐦", "🦆", "🦅", "🦉", "🐴", "🦄", "🐝", "🦋", "🐌", "🐢", "🐍", "🐙", "🐬", "🐳",
            "🌲", "🌹",
        ],
    },
    Category {
        icon: "🍎",
        emoji: &[
            "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍒", "🍑", "🥭", "🍍", "🥝", "🍅", "🥑", "🥕", "🌽", "🥔", "🍞", "🧀", "🥚", "🍳", "🥓", "🍔", "🍟", "🍕", "🌭", "🌮", "🍣", "🍦", "🍫",
            "☕", "🍺",
        ],
    },
    Category {
        icon: "⚽",
        emoji: &[
            "⚽", "🏀", "🏈", "⚾", "🎾", "🏐", "🏉", "🎱", "🏓", "🏸", "🥊", "⛳", "🎯", "🎮", "🎲", "🎸", "🎹", "🥁", "🎤", "🎧", "🎬", "🎨", "🏆", "🥇", "🥈", "🥉", "🏃", "🚴", "🏊", "⛷", "🏂",
            "🎣", "🧩",
        ],
    },
    Category {
        icon: "🚗",
        emoji: &[
            "🚗", "🚕", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚚", "🚜", "🏍", "🚲", "✈", "🚀", "🚁", "⛵", "🚢", "🚂", "🏠", "🏢", "🏥", "🏫", "⛪", "🌋", "🏔", "🌅", "🌃", "🌉", "🗺", "⛺", "🗽",
            "⌚", "📱",
        ],
    },
    Category {
        icon: "💡",
        emoji: &[
            "💡", "🔦", "🔋", "💻", "🖥", "⌨", "🖱", "📷", "📞", "📺", "📻", "⏰", "🔑", "🔒", "🔨", "🔧", "💰", "💳", "📦", "📧", "📝", "📚", "📌", "✂", "🔍", "🎁", "🎈", "🎉", "🧲", "🧪", "💊",
            "🛒", "🧹",
        ],
    },
    Category {
        icon: "❤",
        emoji: &[
            "❤", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "💔", "💕", "💖", "💗", "💯", "💢", "💥", "💫", "💤", "🔥", "✨", "⭐", "🌟", "⚡", "☀", "🌙", "☁", "🌈", "❄", "☔", "✅", "❌", "❓",
            "❗", "⚠",
        ],
    },
    Category {
        icon: "🏁",
        emoji: &[
            "🏁", "🚩", "🎌", "🏴", "🏳", "🇮🇸", "🇺🇸", "🇬🇧", "🇩🇪", "🇫🇷", "🇪🇸", "🇮🇹", "🇳🇴", "🇸🇪", "🇩🇰", "🇫🇮", "🇳🇱", "🇧🇪", "🇨🇭", "🇦🇹", "🇵🇹", "🇮🇪", "🇵🇱", "🇨🇦", "🇲🇽", "🇧🇷", "🇦🇷",
            "🇯🇵", "🇰🇷", "🇨🇳", "🇮🇳", "🇦🇺", "🇳🇿",
        ],
    },
];

/// A tab of the emoji page: Recents first, then the categories.
pub const TABS: usize = CATEGORIES.len() + 1;
/// Recents kept.
const MAX_RECENT: usize = PER_PAGE;

#[derive(Default)]
pub struct Emoji {
    /// 0 is Recents, 1.. index `CATEGORIES`
    tab: usize,
    /// newest first
    pub(super) recent: Vec<String>,
    /// search state, persistence and logic live in `kb_emoji`
    pub(super) query: String,
    pub(super) searching: bool,
    pub(super) path: Option<std::path::PathBuf>,
}

impl Emoji {
    pub fn tab(&self) -> usize {
        self.tab
    }

    /// Switches tab (clamped).
    pub fn set_tab(&mut self, tab: usize) {
        self.query.clear();
        self.tab = tab.min(TABS - 1);
    }

    /// The tab's label.
    pub fn icon(tab: usize) -> &'static str {
        match tab {
            0 => "🕘",
            n => CATEGORIES[(n - 1).min(CATEGORIES.len() - 1)].icon,
        }
    }

    /// The emoji on the current tab (at most `PER_PAGE`).
    pub fn page(&self) -> Vec<String> {
        if !self.query.is_empty() {
            return self.results();
        }
        match self.tab {
            0 => self.recent.clone(),
            n => CATEGORIES[n - 1].emoji.iter().map(|e| e.to_string()).collect(),
        }
    }

    /// Records a typed emoji at the head of Recents.
    pub fn used(&mut self, emoji: &str) {
        self.recent.retain(|e| e != emoji);
        self.recent.insert(0, emoji.to_string());
        self.recent.truncate(MAX_RECENT);
        self.save();
    }

    pub fn recent(&self) -> &[String] {
        &self.recent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_fit_one_page_and_have_no_duplicates() {
        for (i, c) in CATEGORIES.iter().enumerate() {
            assert!(!c.emoji.is_empty() && c.emoji.len() <= PER_PAGE, "category {i} has {}", c.emoji.len());
            let mut seen = std::collections::HashSet::new();
            assert!(c.emoji.iter().all(|e| seen.insert(*e)), "duplicate in category {i}");
        }
    }

    #[test]
    fn recents_are_newest_first_unique_and_capped() {
        let mut e = Emoji::default();
        assert!(e.page().is_empty(), "recents start empty");
        e.used("😀");
        e.used("🔥");
        e.used("😀");
        assert_eq!(e.recent(), ["😀", "🔥"]);
        for i in 0..40 {
            e.used(&format!("e{i}"));
        }
        assert_eq!(e.recent().len(), MAX_RECENT);
        assert_eq!(e.recent()[0], "e39");
    }

    #[test]
    fn tabs_switch_and_clamp() {
        let mut e = Emoji::default();
        e.set_tab(3);
        assert_eq!(e.page(), CATEGORIES[2].emoji);
        e.set_tab(99);
        assert_eq!(e.tab(), TABS - 1);
        assert_eq!(Emoji::icon(0), "🕘");
        assert_eq!(Emoji::icon(1), "🙂");
    }
}
