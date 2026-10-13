//! Glue to two shell services. `notification-schedule` fires a daily reminder for each shortcut that has a time, and
//! `login-items` opens the app at login when the user turns the row on. The window only changes the list and records the
//! login toggle; this module makes the calls. They happen on a real change (a list edit, a tap, a status report), never
//! per frame, and a missing service only logs.

use springboard::shortcuts::{Daily, Shortcut};
use std::path::{Path, PathBuf};
use bifrost_app::login_items_approval as login;
use bifrost_app::notification_schedule::{self as reminder, Schedule};
use bifrost_app::Ctx;

/// Identifier prefix of a shortcut's reminder; the rest is the shortcut's name.
pub const REMINDER_PREFIX: &str = "shortcut.";
/// The body of a reminder. The shell cannot run shortcut actions yet, so the text says that too.
const BODY: &str = "The scheduled time has come. This device cannot run shortcut actions yet.";

/// One scheduled reminder: the shortcut it is for and the local time it fires at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub name: String,
    pub at: Daily,
}

/// The reminders a list asks for, in list order.
pub fn reminders(list: &[Shortcut]) -> Vec<Reminder> {
    list.iter().filter_map(|s| s.schedule.map(|at| Reminder { name: s.name.clone(), at })).collect()
}

/// The request for one reminder: a calendar trigger at its time on every day, repeating.
pub fn request(r: &Reminder) -> Schedule {
    Schedule::calendar(&format!("{REMINDER_PREFIX}{}", r.name), &r.name, r.at.hour, r.at.minute, &[], true).body(BODY)
}

/// Brings the shell's reminders in line with `list` and returns what is scheduled now. `sent` is what the last sync
/// scheduled; `None` means nothing is known (an earlier run may have left reminders), so the app's pending reminders are
/// cleared and the list sent again. Nothing is sent when the reminders did not change.
pub fn sync_reminders(ctx: &Ctx, sent: Option<&[Reminder]>, list: &[Shortcut]) -> Vec<Reminder> {
    let next = reminders(list);
    if sent == Some(next.as_slice()) {
        return next;
    }
    reminder::remove(ctx, &[]);
    for r in &next {
        reminder::add(ctx, &request(r));
    }
    next
}

/// Asks the shell for the app's Login Items status; the answer comes back as a service reply.
pub fn ask_login(ctx: &Ctx) -> u32 {
    login::status(ctx)
}

/// Asks to open the app at login (`on`) or to stop doing so. Approval is the user's, in Settings.
pub fn set_login(ctx: &Ctx, on: bool) -> u32 {
    if on {
        login::register(ctx)
    } else {
        login::unregister(ctx)
    }
}

/// The folder inside the app's data directory that Save File steps write to.
pub const SAVED_DIR: &str = "Saved";

/// Writes `content` to `Saved/<name>` in the app's data directory and returns the path. The name must be a plain file
/// name: not empty, not starting with a dot, without separators or control characters.
pub fn save_file(data_dir: &Path, name: &str, content: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
        return Err(format!("\"{name}\" is not a usable file name"));
    }
    let dir = data_dir.join(SAVED_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create the folder for saved files: {e}"))?;
    let path = dir.join(name);
    std::fs::write(&path, content).map_err(|e| format!("could not save {name}: {e}"))?;
    Ok(path)
}

/// A battery charge from the text of a `capacity` file: a whole percent, 0 to 100.
pub fn parse_capacity(text: &str) -> Option<u8> {
    text.trim().parse::<u8>().ok().filter(|p| *p <= 100)
}

/// The charge of the first battery under a power-supply directory (`BAT0`, `BAT1`, ...), or `None` without one.
pub fn battery_in(root: &Path) -> Option<u8> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("BAT"))
        .collect();
    names.sort();
    names.into_iter().find_map(|n| parse_capacity(&std::fs::read_to_string(root.join(n).join("capacity")).ok()?))
}

/// The battery charge of this machine, or `None` on one without a battery.
pub fn battery_percent() -> Option<u8> {
    battery_in(Path::new("/sys/class/power_supply"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};
    use bifrost_core::json::Json;

    fn ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn with_time(name: &str, hour: u8, minute: u8) -> Shortcut {
        Shortcut { name: name.into(), color: 0, actions: Vec::new(), schedule: Some(Daily { hour, minute }), ..Shortcut::default() }
    }

    fn plain(name: &str) -> Shortcut {
        Shortcut { name: name.into(), color: 0, actions: Vec::new(), schedule: None, ..Shortcut::default() }
    }

    /// (service, method, body) of each call the ctx sent, in order.
    fn calls(ctx: &Ctx) -> Vec<(String, String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } => Some((service, method, body)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_shortcuts_with_a_time_have_reminders_in_list_order() {
        let list = vec![plain("Bedtime"), with_time("Morning", 7, 0), with_time("Evening", 21, 0)];
        let r = reminders(&list);
        assert_eq!(r, vec![Reminder { name: "Morning".into(), at: Daily { hour: 7, minute: 0 } }, Reminder { name: "Evening".into(), at: Daily { hour: 21, minute: 0 } }]);
    }

    #[test]
    fn a_reminder_is_a_daily_repeating_calendar_request() {
        let j = request(&Reminder { name: "Bedtime".into(), at: Daily { hour: 22, minute: 30 } }).to_json();
        assert_eq!(j.get("id").and_then(Json::as_str), Some("shortcut.Bedtime"));
        assert_eq!(j.get("title").and_then(Json::as_str), Some("Bedtime"));
        let t = j.get("trigger").unwrap();
        assert_eq!(t.get("kind").and_then(Json::as_str), Some("calendar"));
        assert_eq!(t.get("hour").and_then(Json::number), Some(22.0));
        assert_eq!(t.get("minute").and_then(Json::number), Some(30.0));
        assert_eq!(t.get("weekdays"), Some(&Json::Arr(Vec::new())), "no weekdays means every day");
        assert_eq!(t.get("repeats"), Some(&Json::Bool(true)));
    }

    #[test]
    fn a_first_sync_clears_the_app_pending_reminders_then_adds_each() {
        let ctx = ctx();
        let list = vec![with_time("Morning", 7, 0)];
        let now = sync_reminders(&ctx, None, &list);
        assert_eq!(now, reminders(&list));
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str()), ("notification-schedule", "remove"));
        assert_eq!((sent[1].0.as_str(), sent[1].1.as_str()), ("notification-schedule", "add"));
        assert!(sent[1].2.contains("\"shortcut.Morning\""));
    }

    #[test]
    fn an_unchanged_list_sends_nothing() {
        let ctx = ctx();
        let list = vec![with_time("Morning", 7, 0), plain("Quiet")];
        let sent = reminders(&list);
        let again = sync_reminders(&ctx, Some(sent.as_slice()), &list);
        assert_eq!(again, sent);
        assert!(calls(&ctx).is_empty());
    }

    #[test]
    fn a_changed_time_clears_and_resends_the_reminders() {
        let ctx = ctx();
        let before = reminders(&[with_time("Morning", 7, 0)]);
        sync_reminders(&ctx, Some(before.as_slice()), &[with_time("Morning", 8, 0)]);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].1, "remove");
        let body = bifrost_core::json::parse(&sent[1].2).unwrap();
        assert_eq!(body.get("trigger").and_then(|t| t.get("hour")).and_then(Json::number), Some(8.0));
    }

    #[test]
    fn removing_the_last_reminder_only_clears() {
        let ctx = ctx();
        let before = reminders(&[with_time("Morning", 7, 0)]);
        let now = sync_reminders(&ctx, Some(before.as_slice()), &[plain("Morning")]);
        assert!(now.is_empty());
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "remove");
    }

    #[test]
    fn the_login_toggle_registers_or_unregisters() {
        let ctx = ctx();
        set_login(&ctx, true);
        set_login(&ctx, false);
        let sent = calls(&ctx);
        assert_eq!(sent[0], ("login-items".into(), "register".into(), "null".into()));
        assert_eq!(sent[1], ("login-items".into(), "unregister".into(), "null".into()));
        ask_login(&ctx);
        assert_eq!(calls(&ctx)[0].1, "status");
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shortcuts-services-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_saved_file_lands_in_the_saved_folder() {
        let d = scratch("save");
        let path = save_file(&d, " note.txt ", "hello").unwrap();
        assert_eq!(path, d.join("Saved").join("note.txt"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "hello");
        save_file(&d, "note.txt", "again").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("Saved/note.txt")).unwrap(), "again", "a second save replaces the file");
    }

    #[test]
    fn a_file_name_that_could_leave_the_folder_is_refused() {
        let d = scratch("names");
        for bad in ["", "  ", ".hidden", "..", "a/b", "a\\b", "tab\there"] {
            assert!(save_file(&d, bad, "x").is_err(), "{bad:?}");
        }
        assert!(!d.join("Saved").exists(), "nothing was created");
    }

    #[test]
    fn the_battery_charge_is_read_from_the_first_battery() {
        let d = scratch("battery");
        assert_eq!(battery_in(&d), None);
        assert_eq!(battery_in(&d.join("missing")), None);
        for (name, cap) in [("AC", "1"), ("BAT1", "55\n"), ("BAT0", "37\n")] {
            std::fs::create_dir_all(d.join(name)).unwrap();
            std::fs::write(d.join(name).join("capacity"), cap).unwrap();
        }
        assert_eq!(battery_in(&d), Some(37));
        assert_eq!(parse_capacity("100"), Some(100));
        assert_eq!(parse_capacity("101"), None);
        assert_eq!(parse_capacity("full"), None);
    }
}
