//! The tips the app ships with. Each card describes a feature of BifrOSt as the system and its apps build it today.

use crate::model::{Collection, Tip};

pub const TIPS: &[Tip] = &[
    // Getting Started
    Tip {
        id: "getting-started-welcome",
        collection: Collection::GettingStarted,
        title: "Welcome to Tips",
        body: "Tips is a library of short how-to cards about BifrOSt. Pick a collection in the sidebar, or choose All Tips to see every card, and open a card to read it in full.",
        keywords: "help guide start introduction library",
    },
    Tip {
        id: "getting-started-new-markers",
        collection: Collection::GettingStarted,
        title: "What the New marker means",
        body: "A card marked New is one you have not opened yet. Opening a card clears its marker. Choose New in the sidebar to see only the unread cards, and use Mark All Read in the toolbar to clear every marker at once.",
        keywords: "unread badge read seen",
    },
    Tip {
        id: "getting-started-search",
        collection: Collection::GettingStarted,
        title: "Search the whole library",
        body: "Type in the search field above the sidebar list. Every word you type must appear in a card's title, text, keywords or collection name, and case does not matter. Choose All Tips first so the search covers the whole library.",
        keywords: "find filter query words",
    },
    Tip {
        id: "getting-started-reading-order",
        collection: Collection::GettingStarted,
        title: "Read cards in order",
        body: "While a card is open, press Right to read the next card in the list you came from and Left to go back. The list keeps the collection and the search you chose, so Back returns you to the same place.",
        keywords: "arrow keys navigate next previous",
    },
    Tip {
        id: "getting-started-escape",
        collection: Collection::GettingStarted,
        title: "Clear the search with Escape",
        body: "Press Escape in the list to clear the search field. Press Escape while a card is open to return to the list.",
        keywords: "escape keyboard clear back",
    },
    Tip {
        id: "getting-started-on-screen-keyboard",
        collection: Collection::GettingStarted,
        title: "Typing on a touch screen",
        body: "Tap the search field to bring up the on-screen keyboard. Opening a card hides the keyboard again, so the card has the whole window.",
        keywords: "touch keyboard tablet type search",
    },
    // Desktop
    Tip {
        id: "desktop-menu-bar",
        collection: Collection::Desktop,
        title: "The menu bar",
        body: "The bar across the top of the screen shows the menus of the app in front, after the desktop's own menus. A menu item that has a keyboard shortcut shows it beside its name.",
        keywords: "menus top bar shortcuts commands",
    },
    Tip {
        id: "desktop-dock",
        collection: Collection::Desktop,
        title: "Open apps from the Dock",
        body: "The Dock holds the apps you use most. Click an app in the Dock to open it, or to bring its window forward when it is already open.",
        keywords: "launch open app switch dock",
    },
    Tip {
        id: "desktop-window-controls",
        collection: Collection::Desktop,
        title: "Window controls",
        body: "Every window has three controls at its top left: close, minimise and zoom. In a window that uses the full size, the controls sit inside the sidebar and the toolbar shares the top row with the content.",
        keywords: "close minimise zoom title bar traffic lights window",
    },
    Tip {
        id: "desktop-sheets",
        collection: Collection::Desktop,
        title: "Answer a sheet first",
        body: "Some actions ask a question in a sheet that slides down from the top of its window. Choose one of the sheet's buttons to answer it before you go back to the window's contents.",
        keywords: "dialog question alert sheet confirm",
    },
    Tip {
        id: "desktop-shared-clipboard",
        collection: Collection::Desktop,
        title: "Copy between apps",
        body: "Copy in one app and paste in another. Text and numbers travel through the desktop's shared pasteboard, so a result copied from Calculator can be pasted into a note in Stickies.",
        keywords: "copy paste clipboard pasteboard transfer",
    },
    // Windows
    Tip {
        id: "windows-full-size",
        collection: Collection::Windows,
        title: "Full-size windows",
        body: "Apps such as Files, Weather and Tips draw their sidebar under the traffic lights, with no separate title row above it. The toolbar sits beside the sidebar on the same top row.",
        keywords: "unified toolbar sidebar title row layout",
    },
    Tip {
        id: "windows-undo-redo",
        collection: Collection::Windows,
        title: "Undo and Redo",
        body: "Most apps keep an Undo and Redo history in the Edit menu, and the item names the change it reverses, such as Undo Arrange. A run of typing counts as one change.",
        keywords: "undo redo edit history reverse",
    },
    Tip {
        id: "windows-translucent-notes",
        collection: Collection::Windows,
        title: "Translucent windows",
        body: "A sticky note can be made translucent from its Window menu, so you can see what is behind it. Notes that are not in front also fade slightly, so the one you are working in stands out.",
        keywords: "translucent transparent inactive window fade stickies",
    },
    // Files
    Tip {
        id: "files-sidebar-locations",
        collection: Collection::Files,
        title: "Sidebar locations",
        body: "The Files sidebar lists Recents, Desktop, Documents, Downloads, Home, Computer and Trash. Each location has its own glyph, and the selected one is shown on a grey plate.",
        keywords: "sidebar recents desktop documents downloads home computer trash",
    },
    Tip {
        id: "files-path-bar",
        collection: Collection::Files,
        title: "Jump back up a folder path",
        body: "The path bar at the foot of a Files window shows the current folder as a row of segments. Click any segment to go straight to that folder.",
        keywords: "path bar folder navigate breadcrumb parent",
    },
    Tip {
        id: "files-toggle-sidebar",
        collection: Collection::Files,
        title: "Hide the sidebar",
        body: "Use the sidebar button beside the traffic lights, or press Control, Command and S together, to hide the sidebar and give the file list the full width. Use the same keys again to show it.",
        keywords: "sidebar hide show toggle shortcut keys",
    },
    Tip {
        id: "files-search",
        collection: Collection::Files,
        title: "Search in Files",
        body: "The search capsule in the Files toolbar filters the files in view by name. Clear the field to return to the full list.",
        keywords: "search find filter capsule name",
    },
    Tip {
        id: "files-undo-change",
        collection: Collection::Files,
        title: "Undo a file change",
        body: "Undo in the Edit menu reverses the last change Files made, such as a move, a rename or a copy into a folder, so one mistake costs one keypress to take back.",
        keywords: "undo move rename copy mistake restore",
    },
    // Apps
    Tip {
        id: "apps-stickies-new-note",
        collection: Collection::Apps,
        title: "Make a sticky note",
        body: "Stickies keeps floating notes in six colours. Every change is saved as you type, and the notes come back the next time you open the app. Double-press a note's title bar to collapse it out of the way.",
        keywords: "stickies note colour save collapse title bar",
    },
    Tip {
        id: "apps-stickies-duplicate",
        collection: Collection::Apps,
        title: "Duplicate a note",
        body: "Press Command and D in Stickies to copy the focused note. The copy sits a little offset from the original and takes focus, so you can change it straight away.",
        keywords: "stickies duplicate copy note command",
    },
    Tip {
        id: "apps-stickies-find",
        collection: Collection::Apps,
        title: "Find text in every note",
        body: "Command and F in Stickies searches all of your notes. Find Next and Find Previous step through the matches, and Use Selection fills the search with the text you have selected.",
        keywords: "stickies find search notes match selection",
    },
    Tip {
        id: "apps-stickies-text-size",
        collection: Collection::Apps,
        title: "Bigger and smaller text",
        body: "Command and equals makes the text in a note bigger, and Command and minus makes it smaller. The Font menu also sets the text colour, underline and strikethrough, and can save a style as the default for new notes.",
        keywords: "stickies font size bigger smaller colour underline",
    },
    Tip {
        id: "apps-stickies-links",
        collection: Collection::Apps,
        title: "Open a link in a note",
        body: "Web addresses in a note are underlined. Double-press one, or choose Open Link from the Edit menu, to open it in the browser.",
        keywords: "stickies link url web address open",
    },
    Tip {
        id: "apps-stickies-print",
        collection: Collection::Apps,
        title: "Print a note",
        body: "Choose Print from the File menu in Stickies to print the focused note.",
        keywords: "stickies print file paper note",
    },
    Tip {
        id: "apps-calculator-copy",
        collection: Collection::Apps,
        title: "Copy a calculator result",
        body: "Choose Copy from the Edit menu in Calculator to send the number on the display to the shared pasteboard. Paste reads a number from the pasteboard into the calculator.",
        keywords: "calculator copy paste result number display",
    },
    Tip {
        id: "apps-calculator-angles",
        collection: Collection::Apps,
        title: "Radians or degrees",
        body: "The angle setting in Calculator, Rad or Deg, decides how sine, cosine and tangent read their input. The same setting applies when another app asks Calculator to evaluate an expression.",
        keywords: "calculator angle radians degrees trigonometry sine",
    },
    Tip {
        id: "apps-weather-add-place",
        collection: Collection::Apps,
        title: "Add a place to Weather",
        body: "Search for a city in Weather's Locations and choose the result to add it. The first place in the list is the one that the Today widget and the lock screen show.",
        keywords: "weather city location place add forecast",
    },
    Tip {
        id: "apps-weather-units",
        collection: Collection::Apps,
        title: "Celsius or Fahrenheit",
        body: "Open Locations in Weather and choose Celsius or Fahrenheit. The setting changes every temperature that Weather shows, including the temperatures beside each place.",
        keywords: "weather temperature celsius fahrenheit units",
    },
    Tip {
        id: "apps-weather-stocks",
        collection: Collection::Apps,
        title: "Watch stock prices",
        body: "Choose Stocks in the Weather toolbar to open the watchlist. Type a ticker symbol in the field to add it, and remove a symbol with its Remove button.",
        keywords: "weather stocks ticker watchlist quotes market",
    },
    Tip {
        id: "apps-clock-alarms",
        collection: Collection::Apps,
        title: "Alarms ring with the app closed",
        body: "You set alarms and timers in Clock, but the desktop's clock service fires them. An alarm still rings after you quit the Clock app.",
        keywords: "clock alarm timer ring closed background service",
    },
    Tip {
        id: "apps-clock-laps",
        collection: Collection::Apps,
        title: "Copy stopwatch laps",
        body: "Copy Laps in the Stopwatch puts the lap table on the shared pasteboard as plain text, ready to paste into a note or a message.",
        keywords: "clock stopwatch laps copy table time",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_catalogue_has_at_least_twenty_five_tips() {
        assert!(TIPS.len() >= 25, "only {} tips", TIPS.len());
    }

    #[test]
    fn ids_are_unique_and_kebab_case() {
        let mut seen = HashSet::new();
        for tip in TIPS {
            assert!(seen.insert(tip.id), "duplicate id {}", tip.id);
            assert!(
                tip.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "id {} is not kebab-case",
                tip.id
            );
        }
    }

    #[test]
    fn every_tip_has_a_title_and_a_body() {
        for tip in TIPS {
            assert!(!tip.title.trim().is_empty(), "{} has no title", tip.id);
            assert!(tip.body.split_whitespace().count() >= 8, "{} has a stub body", tip.id);
        }
    }

    #[test]
    fn every_collection_has_cards() {
        for collection in Collection::ALL {
            assert!(TIPS.iter().any(|tip| tip.collection == collection), "{:?} is empty", collection);
        }
    }

    #[test]
    fn no_card_uses_a_trademarked_platform_name() {
    let banned = [
        concat!("ap", "ple"), concat!("ma", "c"), concat!("i", "os"), concat!("i", "pad"), concat!("i", "phone"),
        concat!("find", "er"), concat!("spot", "light"), concat!("sa", "fari"), concat!("si", "ri"), concat!("i", "cloud"),
    ];
        for tip in TIPS {
            let text = format!("{} {} {}", tip.title, tip.body, tip.keywords).to_lowercase();
            for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
                assert!(!banned.contains(&word), "{} mentions {}", tip.id, word);
            }
        }
    }
}
