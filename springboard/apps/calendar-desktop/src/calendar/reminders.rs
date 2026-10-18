//! Dated reminders from the Reminders app, shown read-only in Calendar. Reminders keeps its store as a tab-separated file
//! in its own data folder (`appdata/is.olibuijr.reminders.desktop/reminders.tsv`, format v2 or v3). Calendar reads the
//! reminder lines (`R`) and takes the open ones that have a due day; each becomes an all-day item with a checkbox glyph.
//! The store has no shared API for writing, so completing a reminder stays in Reminders.
use super::overlay;
use super::store::CalendarEvent;
use super::Date;
use std::path::{Path, PathBuf};

/// The Reminders app's data folder name under the shell's appdata folder.
pub const STORE_DIR: &str = "is.olibuijr.reminders.desktop";
/// The store file in that folder.
pub const STORE_FILE: &str = "reminders.tsv";
/// The checkbox glyph that starts each reminder's title in the grids.
pub const GLYPH: &str = "\u{2610}";
/// The store files this reader understands (v3 and v2 have the same reminder lines; v1 is not read).
const HEADERS: [&str; 2] = ["springboard-reminders-v3", "springboard-reminders-v2"];
/// Stores larger than this are not read (the Reminders app has the same limit).
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// One dated reminder: its title, the local day it is due (days since 1970-01-01) and whether it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub title: String,
    pub due: i64,
    pub completed: bool,
}

/// The store file for the Calendar events file `events_path` (both live in the shell's appdata folders).
pub fn store_path(events_path: &Path) -> Option<PathBuf> {
    Some(events_path.parent()?.parent()?.join(STORE_DIR).join(STORE_FILE))
}

/// Reverses the store's field escaping: `\\`, `\t`, `\n` and `\r`. None for a dangling or unknown escape.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}

/// The dated reminders in a store text. Lines that do not parse are skipped, as the Reminders app skips them.
pub fn parse(text: &str) -> Vec<Item> {
    let mut lines = text.lines();
    if !lines.next().is_some_and(|h| HEADERS.contains(&h)) {
        return vec![];
    }
    // R id list parent due time done flag prio repeat early title notes url ...
    lines
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.first() != Some(&"R") || f.len() < 12 {
                return None;
            }
            let due: i64 = f[4].parse().ok()?;
            let id: u64 = f[1].parse().ok()?;
            let completed = match f[6] {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let title = unescape(f[11])?;
            Some(Item { id, title, due, completed })
        })
        .collect()
}

/// Reads the store at `path`. A missing, oversized or unreadable store gives no reminders.
pub fn load(path: &Path) -> Vec<Item> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_BYTES => std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default(),
        _ => vec![],
    }
}

/// The open reminders due in the years `first..=last`, as all-day items in calendar `slot`. A done reminder is left out.
pub fn overlay_events(items: &[Item], first: i64, last: i64, slot: usize) -> Vec<CalendarEvent> {
    let (from, to) = (Date { year: first, month: 1, day: 1 }.days(), Date { year: last, month: 12, day: 31 }.days());
    items
        .iter()
        .filter(|r| !r.completed && (from..=to).contains(&r.due))
        .enumerate()
        .map(|(n, r)| CalendarEvent {
            id: overlay::id_for(slot, n),
            date: Date::from_days(r.due),
            minute: 0,
            duration: 1440,
            all_day: true,
            title: format!("{GLYPH} {}", r.title),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: 0, uid: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORE: &str = "springboard-reminders-v3\n\
        L\t1\t0\t0\tReminders\t0\t0\n\
        R\t1\t1\t-\t20470\t-\t0\t0\t0\t-\t-\tCall the dentist\t\t\n\
        R\t2\t1\t-\t20470\t540\t1\t0\t0\t-\t-\tAlready done\t\t\n\
        R\t3\t1\t-\t-\t-\t0\t0\t0\t-\t-\tNo date\t\t\n\
        R\t4\t1\t-\t20900\t-\t0\t0\t0\t-\t-\tBuy milk\\nand eggs\t\t\n\
        R\t5\t1\t-\t20470\t-\t0\t0\t9\t-\t-\tPriority nine is still a line\t\t\n\
        R\t6\t1\t-\t20470\t-\tx\t0\t0\t-\t-\tBad done flag\t\t\n";

    #[test]
    fn dated_reminders_are_read_with_their_day_and_done_flag() {
        let items = parse(STORE);
        assert_eq!(items[0], Item { id: 1, title: "Call the dentist".into(), due: 20470, completed: false });
        assert!(items[1].completed, "the done flag is read");
        assert_eq!(items[2].title, "Buy milk\nand eggs", "escaped newlines are restored");
        assert_eq!(items.len(), 4, "undated and bad-done-flag lines are skipped; the rest are kept");
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 2, 4, 5]);
    }

    #[test]
    fn an_unknown_store_header_gives_nothing() {
        assert!(parse("springboard-reminders-v1\nR\t1\t1\t-\t20470\n").is_empty());
        assert!(parse("").is_empty());
    }

    #[test]
    fn open_dated_reminders_become_all_day_items_with_a_checkbox_glyph() {
        let items = parse(STORE);
        let got = overlay_events(&items, 2026, 2026, 7);
        // 20470 is 2026-01-17; 20900 is in 2027, and the done one is left out
        assert_eq!(got.len(), 2, "done and other-year reminders are not shown");
        let e = &got[0];
        assert_eq!(e.title, "\u{2610} Call the dentist");
        assert!(e.all_day && e.minute == 0 && e.duration == 1440);
        assert_eq!(e.date, Date::from_days(20470));
        assert_eq!(e.date, Date { year: 2026, month: 1, day: 17 });
        assert!(overlay::is_overlay(e.id) && overlay::slot_of(e.id) == 7);
        assert_eq!(got[1].title, "\u{2610} Priority nine is still a line");
    }

    #[test]
    fn each_item_gets_its_own_id_in_the_slot() {
        let items = vec![
            Item { id: 1, title: "A".into(), due: 20470, completed: false },
            Item { id: 2, title: "B".into(), due: 20471, completed: false },
        ];
        let got = overlay_events(&items, 2026, 2027, 3);
        assert_eq!(got.len(), 2);
        assert_ne!(got[0].id, got[1].id);
        assert_eq!(got[1].date, Date::from_days(20471));
    }

    #[test]
    fn the_store_sits_beside_the_calendar_appdata_folder() {
        let p = store_path(Path::new("/root/appdata/is.olibuijr.calendar.desktop/events.tsv")).unwrap();
        assert_eq!(p, Path::new("/root/appdata/is.olibuijr.reminders.desktop/reminders.tsv"));
    }
}
