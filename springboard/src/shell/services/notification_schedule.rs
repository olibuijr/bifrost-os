//! The `notification-schedule` service: scheduled user notifications with two kinds of trigger.
//!
//! - An interval trigger (`{"kind":"interval","seconds":N,"repeats":bool}`) is handed to the registry that already
//!   keeps the time-interval requests of the wire path (`usernotif::Registry`, with its repeat grid).
//! - A calendar trigger (`{"kind":"calendar","hour":H,"minute":M,"weekdays":[1..7],"repeats":bool}`) is kept here. The
//!   next time comes from the alarm model (`clock::alarm::Alarm`, local civil time, so it follows the zone's offset).
//!   Weekdays are numbered from 1 = Sunday; an empty list means every day. `tick` delivers it through the same
//!   path as any due request (`Shell::usernotif_fire`).
//!
//! Both kinds form one pending list per app, and a request with an identifier replaces the earlier pending request of
//! either kind. Delivery is the normal notification UI, so the `notifications` permission is required for every method.
//! Pending requests of both kinds are saved to `pending-notifications.ini` beside the notification store and read back
//! on the first tick after the data directory is set. A restored request keeps its trigger time: one that passed
//! while the shell was off fires on the first tick. The saved form keeps the text, badge and category of a request;
//! its sound, user info and interruption level are not saved, so a restored request is delivered without them.
//!
//! Methods (JSON params; the app is the caller):
//! - `add`: `{ "id", "title", "subtitle"?, "body"?, "thread"?, "category"?, "badge"?, "sound"?, "userInfo"?, "interruptionLevel"?, "relevanceScore"?, "trigger" }`. Answers `{"added": id}`.
//! - `remove`: `{ "ids": [..] }`; no ids (or an empty list) removes all pending. Answers `{"removed": n}`.
//! - `pending`: answers `{"pending": [ids]}`, the pending identifiers of both kinds.
use super::{Event, Shell, SystemService};
use crate::shell::clock::alarm::Alarm;
use crate::shell::clock::offset_fn;
use crate::shell::prefs::write_atomic;
use crate::shell::usernotif::{self, Calendar};
use bifrost_core::json::Json;
use bifrost_proto::rich::{MAX_NOTIF_ID, MAX_NOTIF_TEXT, MAX_NOTIF_USER_INFO, MAX_NOTIF_USER_INFO_VALUE};
use bifrost_proto::{NotifRequest, NotifStatus, NotifTrigger};

/// The longest interval one request may name (about 49 days, the millisecond delay of the wire message is a `u32`).
const MAX_INTERVAL_SECS: f64 = 4_294_967.0;

/// The `notification-schedule` service. The calendar and interval requests are kept by the shell's registry
/// (`usernotif::Registry`, see `Shell::usernotif_schedule`); this service parses the JSON and saves and restores them.
#[derive(Default)]
pub struct NotificationSchedule {
    /// the saved file has been read (it is read once, when the data directory is known)
    restored: bool,
    /// the text last written, so an unchanged list is not written again
    saved: String,
}

/// One pending request as the saved file keeps it.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    kind: Kind,
    app: String,
    req: NotifRequest,
    /// Unix milliseconds of the next delivery
    due_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Interval,
    Calendar { hour: u32, minute: u32, weekdays: u8, repeats: bool },
}

impl SystemService for NotificationSchedule {
    fn name(&self) -> &'static str {
        "notification-schedule"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        if !usernotif::permitted(app_id) {
            return Err("the app has no notifications permission".into());
        }
        match method {
            "add" => self.add(shell, app_id, params, t),
            "remove" => self.remove(shell, app_id, params),
            "pending" => Ok(Json::Obj(vec![("pending".into(), Json::Arr(shell.usernotif_pending(app_id).into_iter().map(Json::Str).collect()))])),
            _ => Err(format!("notification-schedule has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        if !self.restored {
            self.restore(shell, t);
        }
        if !shell.usernotif.calendars.is_empty() {
            self.fire(shell, t);
        }
        self.persist(shell, t);
        Vec::new()
    }
}

impl NotificationSchedule {
    /// Delivers the calendar requests that are due and keeps the repeating ones for their next time.
    fn fire(&mut self, shell: &mut Shell, t: f64) {
        let now = shell.wall.now(t);
        let offset = offset_fn(None, shell.wall.offset);
        let mut fired = Vec::new();
        let mut kept = Vec::with_capacity(shell.usernotif.calendars.len());
        for mut c in std::mem::take(&mut shell.usernotif.calendars) {
            if c.due > now {
                kept.push(c);
                continue;
            }
            fired.push((c.app.clone(), c.req.clone()));
            if c.repeats {
                // the first time after now: a shell that slept past several times delivers once
                if let Some(next) = c.alarm.regular_fire(now, &offset) {
                    c.due = next;
                    kept.push(c);
                }
            }
        }
        shell.usernotif.calendars = kept;
        for (app, req) in fired {
            shell.usernotif_fire(&app, req, t);
        }
    }

    /// Reads the requests an earlier shell saved. Nothing is read until the data directory is set.
    fn restore(&mut self, shell: &mut Shell, t: f64) {
        let Some(path) = shell.pending_path() else { return };
        self.restored = true;
        let now_ms = shell.wall.now_ms(t);
        for row in decode(&std::fs::read_to_string(&path).unwrap_or_default()) {
            match row.kind {
                Kind::Interval => shell.usernotif.restore_pending(&row.app, row.req, t + (row.due_ms - now_ms) as f64),
                Kind::Calendar { hour, minute, weekdays, repeats } => {
                    let alarm = Alarm { repeat: weekdays, ..Alarm::new(hour, minute) };
                    shell.usernotif.calendars.push(Calendar { app: row.app, req: row.req, alarm, repeats, due: row.due_ms.div_euclid(1000) });
                }
            }
        }
    }

    /// Writes the pending requests of both kinds when they changed since the last write.
    fn persist(&mut self, shell: &Shell, t: f64) {
        if !self.restored {
            return;
        }
        let Some(path) = shell.pending_path() else { return };
        let now_ms = shell.wall.now_ms(t);
        let mut rows: Vec<Row> = shell.usernotif.pending_rows(t, now_ms).into_iter().map(|(app, req, due_ms)| Row { kind: Kind::Interval, app, req, due_ms }).collect();
        rows.extend(shell.usernotif.calendars.iter().map(|c| Row {
            kind: Kind::Calendar { hour: c.alarm.hour, minute: c.alarm.min, weekdays: c.alarm.repeat, repeats: c.repeats },
            app: c.app.clone(),
            req: c.req.clone(),
            due_ms: c.due * 1000,
        }));
        let text = encode(&rows);
        if text != self.saved {
            match write_atomic(&path, &text) {
                Ok(()) => self.saved = text,
                Err(e) => eprintln!("springboard: pending-notifications.ini: {e}"),
            }
        }
    }

    fn add(&mut self, shell: &mut Shell, app: &str, params: &Json, t: f64) -> Result<Json, String> {
        let req = request(params)?;
        let trigger = trigger(params.get("trigger").ok_or("trigger is required")?)?;
        let id = req.id.clone();
        match shell.usernotif_schedule(app, req, trigger, t) {
            NotifStatus::Ok => Ok(Json::Obj(vec![("added".into(), Json::Str(id))])),
            status => Err(refusal(status)),
        }
    }

    fn remove(&mut self, shell: &mut Shell, app: &str, params: &Json) -> Result<Json, String> {
        let ids = string_list(params.get("ids"))?;
        let before = shell.usernotif_pending(app).len();
        shell.usernotif_remove_pending(app, &ids);
        let removed = before.saturating_sub(shell.usernotif_pending(app).len());
        Ok(Json::Obj(vec![("removed".into(), Json::Num(removed as f64))]))
    }
}

const SAVED_HEADER: &str = "# springboard pending notifications: one request per line, fields separated by tabs\n";
/// Fields per saved line: kind, app, due, delay, repeats, badge, hour, minute, weekdays, id, title, subtitle, body,
/// thread, category.
const SAVED_FIELDS: usize = 15;

/// Text in a saved field: tabs and line breaks become spaces, so a line keeps its fields.
fn saved_text(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn encode(rows: &[Row]) -> String {
    let mut out = String::from(SAVED_HEADER);
    for r in rows {
        let (kind, hour, minute, weekdays, repeats) = match r.kind {
            Kind::Interval => ("i", 0, 0, 0, r.req.repeats),
            Kind::Calendar { hour, minute, weekdays, repeats } => ("c", hour, minute, weekdays, repeats),
        };
        let badge = r.req.badge.map_or(-1, i64::from);
        let fields = [
            kind.to_string(),
            saved_text(&r.app),
            r.due_ms.to_string(),
            r.req.delay_ms.to_string(),
            u8::from(repeats).to_string(),
            badge.to_string(),
            hour.to_string(),
            minute.to_string(),
            weekdays.to_string(),
            saved_text(&r.req.id),
            saved_text(&r.req.title),
            saved_text(&r.req.subtitle),
            saved_text(&r.req.body),
            saved_text(&r.req.thread),
            saved_text(&r.req.category),
        ];
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    out
}

/// The rows of a saved file; a line that does not parse is skipped.
fn decode(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != SAVED_FIELDS {
                return None;
            }
            let due_ms: i64 = f[2].parse().ok()?;
            let delay_ms: u32 = f[3].parse().ok()?;
            let repeats = f[4] == "1";
            let badge: i64 = f[5].parse().ok()?;
            let hour: u32 = f[6].parse().ok()?;
            let minute: u32 = f[7].parse().ok()?;
            let weekdays: u8 = f[8].parse().ok()?;
            let kind = match f[0] {
                "i" => Kind::Interval,
                "c" => Kind::Calendar { hour, minute, weekdays, repeats },
                _ => return None,
            };
            let req = NotifRequest {
                id: f[9].into(),
                title: f[10].into(),
                subtitle: f[11].into(),
                body: f[12].into(),
                thread: f[13].into(),
                category: f[14].into(),
                delay_ms,
                badge: u32::try_from(badge).ok(),
                repeats,
                ..Default::default()
            };
            Some(Row { kind, app: f[1].to_string(), req, due_ms })
        })
        .collect()
}

/// The text of a refusal, as the request status names it.
fn refusal(status: NotifStatus) -> String {
    match status {
        NotifStatus::Ok => "ok".into(),
        NotifStatus::NotPermitted => "the app has no notifications permission".into(),
        NotifStatus::Param => "the notification request is not valid".into(),
        NotifStatus::Full => "too many pending notification requests".into(),
    }
}

/// The content of an `add` request, checked against the wire limits.
fn request(params: &Json) -> Result<NotifRequest, String> {
    let (sound_kind, sound_name, sound_volume) = sound(params.get("sound"))?;
    let req = NotifRequest {
        sound_kind,
        sound_name,
        sound_volume,
        user_info: user_info(params.get("userInfo"))?,
        interruption_level: interruption_level(params.get("interruptionLevel"))?,
        relevance_milli: relevance(params.get("relevanceScore"))?,
        id: text(params, "id", MAX_NOTIF_ID)?,
        title: text(params, "title", MAX_NOTIF_TEXT)?,
        subtitle: text(params, "subtitle", MAX_NOTIF_TEXT)?,
        body: text(params, "body", MAX_NOTIF_TEXT * 4)?,
        thread: text(params, "thread", MAX_NOTIF_ID)?,
        category: text(params, "category", MAX_NOTIF_ID)?,
        badge: match params.get("badge") {
            None | Some(Json::Null) => None,
            Some(_) => Some(integer(params, "badge", f64::from(u32::MAX))? as u32),
        },
        ..Default::default()
    };
    Ok(req)
}

/// `content.sound`: `"default"`, `"critical"`, `{"critical":0.5}` or `{"named":"file.wav"}`.
fn sound(v: Option<&Json>) -> Result<(u8, String, u8), String> {
    match v {
        None | Some(Json::Null) => Ok((0, String::new(), 0)),
        Some(Json::Str(s)) if s == "default" => Ok((1, String::new(), 0)),
        Some(Json::Str(s)) if s == "critical" => Ok((3, String::new(), 0)),
        Some(o @ Json::Obj(_)) => {
            if let Some(name) = o.get("named").and_then(Json::as_str) {
                let bare = !name.is_empty() && name.len() <= 255 && !name.contains('/') && !name.contains("..");
                return if bare { Ok((2, name.to_string(), 0)) } else { Err("a sound name is a bare file name of 1 to 255 bytes".into()) };
            }
            match o.get("critical").and_then(Json::number) {
                Some(vol) if (0.0..=1.0).contains(&vol) => Ok((4, String::new(), (vol * 100.0).round() as u8)),
                Some(_) => Err("the critical sound volume is from 0 to 1".into()),
                None => Err("sound must name a file or a critical volume".into()),
            }
        }
        Some(_) => Err("sound must be \"default\", \"critical\" or an object".into()),
    }
}

/// `content.userInfo`: an object of text values, bounded like the wire.
fn user_info(v: Option<&Json>) -> Result<Vec<(String, String)>, String> {
    match v {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Obj(entries)) if entries.len() <= MAX_NOTIF_USER_INFO => entries
            .iter()
            .map(|(k, val)| match val {
                Json::Str(s) if !k.is_empty() && k.len() <= MAX_NOTIF_ID && s.len() <= MAX_NOTIF_USER_INFO_VALUE => Ok((k.clone(), s.clone())),
                _ => Err("a userInfo value must be text of at most 512 bytes under a key of 1 to 128".to_string()),
            })
            .collect(),
        Some(Json::Obj(_)) => Err(format!("userInfo takes at most {MAX_NOTIF_USER_INFO} entries")),
        Some(_) => Err("userInfo must be an object".into()),
    }
}

/// `content.interruptionLevel`: passive, active (the default), timeSensitive or critical.
fn interruption_level(v: Option<&Json>) -> Result<u8, String> {
    Ok(match v {
        None | Some(Json::Null) => 1,
        Some(Json::Str(s)) => match s.as_str() {
            "passive" => 0,
            "active" => 1,
            "timeSensitive" => 2,
            "critical" => 3,
            other => return Err(format!("unknown interruptionLevel {other}")),
        },
        Some(_) => return Err("interruptionLevel must be text".into()),
    })
}

/// `content.relevanceScore` from 0 to 1, in thousandths.
fn relevance(v: Option<&Json>) -> Result<u16, String> {
    match v {
        None | Some(Json::Null) => Ok(0),
        Some(j) => match j.number() {
            Some(n) if (0.0..=1.0).contains(&n) => Ok((n * 1000.0).round() as u16),
            _ => Err("relevanceScore must be a number from 0 to 1".into()),
        },
    }
}

/// The trigger of an `add` request.
fn trigger(t: &Json) -> Result<NotifTrigger, String> {
    let repeats = match t.get("repeats") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err("trigger.repeats must be true or false".into()),
    };
    match t.str_field("kind").as_deref() {
        Some("interval") => {
            let secs = t.get("seconds").and_then(Json::number).ok_or("trigger.seconds must be a number")?;
            if secs.fract() != 0.0 || secs < 1.0 || secs > MAX_INTERVAL_SECS {
                return Err(format!("trigger.seconds must be a whole number from 1 to {}", MAX_INTERVAL_SECS as u32));
            }
            if repeats && secs < 60.0 {
                return Err("trigger.seconds must be at least 60 for a repeating trigger".into());
            }
            Ok(NotifTrigger::Interval { seconds: secs as u32, repeats })
        }
        Some("calendar") => {
            let hour = integer(t, "hour", 23.0)?;
            let minute = integer(t, "minute", 59.0)?;
            let mut weekdays = 0u8;
            match t.get("weekdays") {
                None | Some(Json::Null) => {}
                Some(Json::Arr(list)) => {
                    for d in list {
                        let w = d.number().filter(|w| w.fract() == 0.0 && (1.0..=7.0).contains(w)).ok_or("trigger.weekdays must be numbers from 1 (Sunday) to 7 (Saturday)")?;
                        weekdays |= 1 << (w as u32 - 1);
                    }
                }
                Some(_) => return Err("trigger.weekdays must be a list".into()),
            }
            Ok(NotifTrigger::Calendar { hour: hour as u8, minute: minute as u8, weekdays, repeats })
        }
        _ => Err("trigger.kind must be interval or calendar".into()),
    }
}

/// An optional text field: missing is empty, a non-text value or one over `max` bytes is refused.
fn text(j: &Json, key: &str, max: usize) -> Result<String, String> {
    match j.get(key) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Str(s)) if s.len() <= max => Ok(s.clone()),
        Some(Json::Str(_)) => Err(format!("{key} is longer than {max} bytes")),
        Some(_) => Err(format!("{key} must be text")),
    }
}

/// A whole number from 0 to `max`.
fn integer(j: &Json, key: &str, max: f64) -> Result<u32, String> {
    let v = j.get(key).and_then(Json::number).ok_or_else(|| format!("{key} must be a number"))?;
    if v.fract() != 0.0 || v < 0.0 || v > max {
        return Err(format!("{key} must be a whole number from 0 to {}", max as u64));
    }
    Ok(v as u32)
}

/// A list of identifiers; missing or null is empty.
fn string_list(v: Option<&Json>) -> Result<Vec<String>, String> {
    match v {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Arr(list)) => list.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| "ids must be text".to_string())).collect(),
        Some(_) => Err("ids must be a list".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::usernotif::Added;

    fn json(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn request_content_carries_sound_user_info_level_and_score() {
        let r = request(&json(r#"{"id":"a","title":"T","sound":{"named":"chime.wav"},"userInfo":{"room":"7"},"interruptionLevel":"timeSensitive","relevanceScore":0.5}"#)).unwrap();
        assert_eq!((r.sound_kind, r.sound_name.as_str()), (2, "chime.wav"));
        assert_eq!(r.user_info, vec![("room".to_string(), "7".to_string())]);
        assert_eq!((r.interruption_level, r.relevance_milli), (2, 500));
        let d = request(&json(r#"{"id":"a","title":"T"}"#)).unwrap();
        assert_eq!((d.sound_kind, d.interruption_level, d.relevance_milli), (0, 1, 0));
        assert_eq!(request(&json(r#"{"id":"a","sound":{"critical":0.25}}"#)).unwrap().sound_volume, 25);
        assert!(request(&json(r#"{"id":"a","sound":{"named":"../x.wav"}}"#)).is_err());
        assert!(request(&json(r#"{"id":"a","userInfo":{"k":5}}"#)).is_err());
        assert!(request(&json(r#"{"id":"a","interruptionLevel":"loud"}"#)).is_err());
        assert!(request(&json(r#"{"id":"a","relevanceScore":1.5}"#)).is_err());
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-pending")).join(std::process::id().to_string()).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A test shell at Unix time `unix` (monotonic 0) whose data directory is `dir`.
    fn shell_in(dir: &std::path::Path, unix: i64) -> Shell {
        let mut sh = crate::shell::tests::shell();
        sh.set_wall(unix, 0, 0.0);
        let _ = sh.set_data_dir(dir.to_str().unwrap(), 0.0);
        sh
    }

    #[test]
    fn saved_rows_round_trip_and_skip_lines_that_do_not_parse() {
        let rows = vec![
            Row {
                kind: Kind::Interval,
                app: "is.example.chat".into(),
                req: NotifRequest { id: "a".into(), title: "T\tab".into(), body: "B\nC".into(), thread: "room".into(), category: "MSG".into(), delay_ms: 90_000, badge: Some(3), repeats: true, ..Default::default() },
                due_ms: 1_700_000_090_000,
            },
            Row {
                kind: Kind::Calendar { hour: 8, minute: 30, weekdays: 0b10_0010, repeats: true },
                app: "is.example.news".into(),
                req: NotifRequest { id: "b".into(), title: "Morning".into(), ..Default::default() },
                due_ms: 1_700_001_000_000,
            },
        ];
        let back = decode(&format!("{}garbage line\n", encode(&rows)));
        assert_eq!(back.len(), 2, "the bad line is skipped");
        assert_eq!((back[0].req.title.as_str(), back[0].req.body.as_str()), ("T ab", "B C"), "tabs and breaks become spaces");
        assert_eq!((back[0].kind, back[0].req.badge, back[0].req.repeats), (Kind::Interval, Some(3), true));
        assert_eq!(back[0].req.thread, "room");
        assert_eq!((back[1].kind, back[1].due_ms, back[1].req.badge), (rows[1].kind, 1_700_001_000_000, None));
    }

    #[test]
    fn pending_requests_survive_a_restart_and_keep_their_trigger_times() {
        let dir = scratch_dir("restart");
        // first shell (Unix 1_000_000 s at monotonic 0): an interval request 120 s out, a calendar request at 1_003_600
        let mut first = shell_in(&dir, 1_000_000);
        let mut svc = NotificationSchedule::default();
        let req = |id: &str| NotifRequest { id: id.into(), title: "Timer".into(), ..Default::default() };
        assert_eq!(first.usernotif.add("is.example.timer", NotifRequest { delay_ms: 120_000, ..req("timer") }, 0.0), Added::Scheduled);
        first.usernotif.calendars.push(Calendar { app: "is.example.news".into(), req: req("morning"), alarm: Alarm { repeat: 0, ..Alarm::new(8, 30) }, repeats: false, due: 1_003_600 });
        svc.tick(&mut first, 1_000.0);
        let saved = std::fs::read_to_string(dir.join("pending-notifications.ini")).unwrap();
        assert_eq!(saved.lines().filter(|l| !l.starts_with('#')).count(), 2, "both kinds are saved");

        // restart 60 s later: the timer is due 60 s after the restart, the calendar request keeps its date
        let mut second = shell_in(&dir, 1_000_060);
        let mut back = NotificationSchedule::default();
        back.tick(&mut second, 0.0);
        assert_eq!(second.usernotif.next_due(), Some(60_000.0));
        assert!(second.usernotif.due(59_999.0).is_empty());
        let fired = second.usernotif.due(60_000.0);
        assert_eq!(fired.len(), 1);
        assert_eq!((fired[0].0.as_str(), fired[0].1.id.as_str()), ("is.example.timer", "timer"));
        assert_eq!(second.usernotif.calendars.len(), 1);
        assert_eq!((second.usernotif.calendars[0].app.as_str(), second.usernotif.calendars[0].due), ("is.example.news", 1_003_600));
        assert_eq!((second.usernotif.calendars[0].alarm.hour, second.usernotif.calendars[0].alarm.min), (8, 30));
    }

    #[test]
    fn a_request_whose_time_passed_while_the_shell_was_off_fires_on_the_first_tick() {
        let dir = scratch_dir("missed");
        let mut first = shell_in(&dir, 1_000_000);
        let mut svc = NotificationSchedule::default();
        assert_eq!(first.usernotif.add("is.example.timer", NotifRequest { id: "late".into(), title: "Late".into(), delay_ms: 30_000, ..Default::default() }, 0.0), Added::Scheduled);
        svc.tick(&mut first, 0.0);
        // back 10 minutes later: the 30 s timer is long past, so it is due at once
        let mut second = shell_in(&dir, 1_000_600);
        NotificationSchedule::default().tick(&mut second, 0.0);
        assert_eq!(second.usernotif.next_due(), Some(-570_000.0), "its trigger is 570 s before now");
        let fired = second.usernotif.due(0.0);
        assert_eq!(fired.len(), 1, "the missed request is delivered on the first tick");
        assert_eq!(fired[0].1.id, "late");
    }

    #[test]
    fn a_cleared_request_is_not_saved_again() {
        let dir = scratch_dir("cleared");
        let mut sh = shell_in(&dir, 1_000_000);
        let mut svc = NotificationSchedule::default();
        assert_eq!(sh.usernotif.add("is.example.timer", NotifRequest { id: "gone".into(), title: "x".into(), delay_ms: 5_000, ..Default::default() }, 0.0), Added::Scheduled);
        svc.tick(&mut sh, 0.0);
        sh.usernotif.remove("is.example.timer", false, &["gone".to_string()]);
        svc.tick(&mut sh, 1.0);
        let saved = std::fs::read_to_string(dir.join("pending-notifications.ini")).unwrap();
        assert_eq!(saved.lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    #[test]
    fn an_interval_trigger_is_a_whole_number_of_seconds() {
        assert_eq!(trigger(&json(r#"{"kind":"interval","seconds":90,"repeats":true}"#)), Ok(NotifTrigger::Interval { seconds: 90, repeats: true }));
        assert_eq!(trigger(&json(r#"{"kind":"interval","seconds":5}"#)), Ok(NotifTrigger::Interval { seconds: 5, repeats: false }));
        assert!(trigger(&json(r#"{"kind":"interval","seconds":0}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"interval","seconds":30,"repeats":true}"#)).is_err(), "a repeating interval is at least 60 s");
        assert!(trigger(&json(r#"{"kind":"interval","seconds":60,"repeats":true}"#)).is_ok());
        assert!(trigger(&json(r#"{"kind":"interval","seconds":1.5}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"interval"}"#)).is_err());
    }

    #[test]
    fn a_calendar_trigger_maps_weekdays_to_the_alarm_mask() {
        // 1 = Sunday is bit 0, as in the alarm model; Monday and Friday are bits 1 and 5
        assert_eq!(
            trigger(&json(r#"{"kind":"calendar","hour":8,"minute":30,"weekdays":[2,6,2],"repeats":true}"#)),
            Ok(NotifTrigger::Calendar { hour: 8, minute: 30, weekdays: 0b10_0010, repeats: true })
        );
        assert_eq!(trigger(&json(r#"{"kind":"calendar","hour":7,"minute":0}"#)), Ok(NotifTrigger::Calendar { hour: 7, minute: 0, weekdays: 0, repeats: false }));
        assert!(trigger(&json(r#"{"kind":"calendar","hour":24,"minute":0}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":60}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":1,"weekdays":[0]}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"calendar","hour":1,"minute":1,"weekdays":[8]}"#)).is_err());
        assert!(trigger(&json(r#"{"kind":"weekly"}"#)).is_err());
    }

    #[test]
    fn a_daily_calendar_time_is_the_next_one_strictly_after_now() {
        let offset = |_: i64| 0;
        // 1970-01-01 08:00 UTC, a Thursday, for 08:30 every day
        let daily = Alarm::new(8, 30);
        assert_eq!(daily.regular_fire(28_800, &offset), Some(30_600));
        // at 08:30 itself the next one is tomorrow
        assert_eq!(daily.regular_fire(30_600, &offset), Some(30_600 + 86_400));
    }

    #[test]
    fn a_weekday_calendar_time_skips_the_other_days() {
        let offset = |_: i64| 0;
        // Mondays only (bit 1); Thursday 1970-01-01 08:00 leads to Monday 1970-01-05 08:30
        let monday = Alarm { repeat: 0b10, ..Alarm::new(8, 30) };
        assert_eq!(monday.regular_fire(28_800, &offset), Some(4 * 86_400 + 30_600));
    }

    #[test]
    fn requests_are_checked_against_the_wire_limits() {
        let ok = request(&json(r#"{"id":"a","title":"T","body":"B","badge":3}"#)).unwrap();
        assert_eq!((ok.id.as_str(), ok.title.as_str(), ok.body.as_str(), ok.badge), ("a", "T", "B", Some(3)));
        assert!(request(&json(r#"{"title":"T"}"#)).is_ok(), "the empty identifier is refused by the registry, not here");
        let long = format!(r#"{{"id":"{}"}}"#, "x".repeat(MAX_NOTIF_ID + 1));
        assert!(request(&json(&long)).is_err());
        assert!(request(&json(r#"{"id":"a","title":3}"#)).is_err());
    }

    #[test]
    fn remove_ids_are_text_or_empty() {
        assert_eq!(string_list(None), Ok(Vec::new()));
        assert_eq!(string_list(Some(&json(r#"["a","b"]"#))), Ok(vec!["a".to_string(), "b".to_string()]));
        assert!(string_list(Some(&json(r#"[1]"#))).is_err());
        assert!(string_list(Some(&json(r#""a""#))).is_err());
    }

    #[test]
    fn a_wire_calendar_request_is_saved_and_fires_after_a_restart() {
        let dir = scratch_dir("wire-calendar");
        let mut first = shell_in(&dir, 1_000_000);
        let req = NotifRequest { id: "morning".into(), title: "Morning".into(), ..Default::default() };
        let daily = NotifTrigger::Calendar { hour: 8, minute: 30, weekdays: 0, repeats: true };
        assert_eq!(first.usernotif_schedule("is.example.news", req, daily, 0.0), NotifStatus::Ok);
        NotificationSchedule::default().tick(&mut first, 1.0);
        let saved = std::fs::read_to_string(dir.join("pending-notifications.ini")).unwrap();
        assert!(saved.lines().any(|l| l.starts_with("c\tis.example.news\t")), "the calendar request is saved");

        // restart 60 s later: the request keeps its next 08:30 (UTC, the shell's zone offset is 0 here)
        let mut second = shell_in(&dir, 1_000_060);
        NotificationSchedule::default().tick(&mut second, 0.0);
        let offset = |_: i64| 0;
        let expected = Alarm::new(8, 30).regular_fire(1_000_000, &offset).unwrap();
        assert_eq!(second.usernotif.calendars.len(), 1);
        assert_eq!((second.usernotif.calendars[0].due, second.usernotif.calendars[0].repeats), (expected, true));
    }

    #[test]
    fn the_pending_list_and_removal_cover_both_kinds() {
        let dir = scratch_dir("wire-pending");
        let mut sh = shell_in(&dir, 1_000_000);
        let app = "is.example.timer";
        let at_8_30 = NotifTrigger::Calendar { hour: 8, minute: 30, weekdays: 0, repeats: false };
        let cal = NotifRequest { id: "cal".into(), title: "C".into(), ..Default::default() };
        let int = NotifRequest { id: "int".into(), title: "I".into(), ..Default::default() };
        assert_eq!(sh.usernotif_schedule(app, cal, at_8_30, 0.0), NotifStatus::Ok);
        assert_eq!(sh.usernotif_schedule(app, int, NotifTrigger::Interval { seconds: 120, repeats: false }, 0.0), NotifStatus::Ok);
        assert_eq!(sh.usernotif_pending(app), vec!["int".to_string(), "cal".to_string()], "interval first, then calendar");

        // the same identifier replaces the pending request of either kind
        let again = NotifRequest { id: "int".into(), title: "I".into(), ..Default::default() };
        assert_eq!(sh.usernotif_schedule(app, again, NotifTrigger::Calendar { hour: 9, minute: 0, weekdays: 0, repeats: false }, 0.0), NotifStatus::Ok);
        assert_eq!(sh.usernotif_pending(app), vec!["cal".to_string(), "int".to_string()]);

        sh.usernotif_remove_pending(app, &["cal".to_string()]);
        assert_eq!(sh.usernotif_pending(app), vec!["int".to_string()]);
        sh.usernotif_remove_pending(app, &[]);
        assert!(sh.usernotif_pending(app).is_empty());
    }
}
