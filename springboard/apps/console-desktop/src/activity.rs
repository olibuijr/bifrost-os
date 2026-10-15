//! Activities: the messages of one systemd unit invocation (`_SYSTEMD_INVOCATION_ID`). View > Show Only This Activity
//! narrows the table to the invocation of the selected message. The grouped Activities outline is not built. No I/O here.

use crate::model::LogEntry;

/// Whether `entry` is shown under the activity filter: the invocation the filter names, or every message without one.
pub fn shows(filter: Option<&str>, entry: &LogEntry) -> bool {
    match filter {
        None => true,
        Some(id) => entry.invocation == id,
    }
}

/// The filter View > Show Only This Activity sets for `entry`: its invocation, or None when the message has none.
pub fn filter_for(entry: &LogEntry) -> Option<String> {
    (!entry.invocation.is_empty()).then(|| entry.invocation.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Console;
    use crate::model::Level;

    fn entry(invocation: &str, message: &str) -> LogEntry {
        LogEntry { level: Level::Notice, message: message.into(), invocation: invocation.into(), ..LogEntry::default() }
    }

    #[test]
    fn no_filter_shows_every_message_with_or_without_an_invocation() {
        assert!(shows(None, &entry("", "boot")));
        assert!(shows(None, &entry("abc", "unit")));
    }

    #[test]
    fn a_filter_shows_only_the_messages_of_its_invocation() {
        assert!(shows(Some("abc"), &entry("abc", "unit")));
        assert!(!shows(Some("abc"), &entry("def", "other unit")));
        assert!(!shows(Some("abc"), &entry("", "no unit")));
    }

    #[test]
    fn a_message_without_an_invocation_has_no_activity_to_filter_by() {
        assert_eq!(filter_for(&entry("", "kernel")), None);
        assert_eq!(filter_for(&entry("abc", "unit")), Some("abc".to_string()));
    }

    #[test]
    fn show_only_this_activity_narrows_the_table_and_a_second_toggle_restores_it() {
        let mut c = Console::new(800.0, 600.0, Vec::new());
        let first = c.buffer.push(entry("abc", "one"));
        c.buffer.push(entry("def", "two"));
        c.rebuild();
        c.selected = Some(first);
        c.toggle_activity();
        assert_eq!(c.visible, vec![first]);
        c.toggle_activity();
        assert_eq!(c.visible.len(), 2);
    }

    #[test]
    fn a_selected_message_without_an_invocation_leaves_the_table_unfiltered() {
        let mut c = Console::new(800.0, 600.0, Vec::new());
        let kernel = c.buffer.push(entry("", "kernel"));
        c.buffer.push(entry("def", "two"));
        c.rebuild();
        c.selected = Some(kernel);
        c.toggle_activity();
        assert!(c.activity.is_none());
        assert_eq!(c.visible.len(), 2);
    }
}
