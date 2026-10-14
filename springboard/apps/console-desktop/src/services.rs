//! The shell services the Console uses. The `power` service gives the thermal state, Low Power Mode and the battery; the
//! footer shows them (`PowerNote`). The `pasteboard` service takes a copied log line on the general board. The models
//! and the text are pure (no I/O); `main.rs` makes the calls on real events only: the window opening, a pushed change,
//! and the Log menu's Copy Message. An older shell answers "no service ..." and the Console keeps working without it.

use crate::draw::full_date;
use crate::model::{Level, LogEntry};
use bifrost_app::pasteboard_types::{self, Board, Item};
use bifrost_app::power_service::{Battery, Change, Snapshot};
use bifrost_app::power_sources::PowerState;
use bifrost_app::process_info::ThermalState;
use bifrost_app::Ctx;

/// What the power service last said, as the footer shows it. Empty until a reply or an event arrives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerNote {
    thermal: Option<ThermalState>,
    low_power: bool,
    battery: Option<Battery>,
}

impl PowerNote {
    /// A full reply to `power_service::snapshot`.
    pub fn apply_snapshot(&mut self, s: Snapshot) {
        self.thermal = Some(s.thermal_state);
        self.low_power = s.low_power_mode;
        self.battery = s.battery;
    }

    /// One pushed change; it replaces only the part it names.
    pub fn apply_change(&mut self, change: Change) {
        match change {
            Change::ThermalState(t) => self.thermal = Some(t),
            Change::LowPowerMode(on) => self.low_power = on,
            Change::Battery(b) => self.battery = b,
        }
    }

    /// The footer text, joined with " · ". Empty when nothing is known (no battery and no thermal reply).
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = self.thermal {
            parts.push(format!("Thermal {}", thermal_label(t)));
        }
        if let Some(b) = &self.battery {
            parts.push(battery_text(b));
        }
        if self.low_power {
            parts.push("Low Power Mode".to_string());
        }
        parts.join(" · ")
    }
}

/// The thermal state as a word for the footer.
pub fn thermal_label(t: ThermalState) -> &'static str {
    match t {
        ThermalState::Nominal => "Nominal",
        ThermalState::Fair => "Fair",
        ThermalState::Serious => "Serious",
        ThermalState::Critical => "Critical",
    }
}

/// "Battery 42%, discharging, 1 h 15 min left". The time is the estimate for the direction the battery moves.
pub fn battery_text(b: &Battery) -> String {
    let mut out = match b.percent {
        Some(p) => format!("Battery {p}%"),
        None => "Battery".to_string(),
    };
    let word = match b.state {
        PowerState::Charging => Some("charging"),
        PowerState::Discharging => Some("discharging"),
        PowerState::Full => Some("full"),
        PowerState::NotCharging => Some("not charging"),
        PowerState::Unknown => None,
    };
    if let Some(word) = word {
        out.push_str(", ");
        out.push_str(word);
    }
    if let Some(m) = b.time_remaining() {
        let tail = if b.state == PowerState::Charging { "to full" } else { "left" };
        out.push_str(&format!(", {} {tail}", duration(m)));
    }
    out
}

/// Minutes as "45 min" or "1 h 15 min".
fn duration(minutes: u32) -> String {
    if minutes >= 60 {
        format!("{} h {} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

/// One log line as plain text for the pasteboard: "date process[pid] Level subsystem:category message". Empty fields
/// are left out; a date of 0 (unknown) is left out.
pub fn copy_line(e: &LogEntry) -> String {
    let mut out = String::new();
    if e.date > 0 {
        out.push_str(&full_date(e.date));
        out.push(' ');
    }
    if !e.process.is_empty() {
        out.push_str(&e.process);
        if e.pid > 0 {
            out.push_str(&format!("[{}]", e.pid));
        }
        out.push(' ');
    }
    out.push_str(Level::label(e.level));
    out.push(' ');
    if !e.subsystem.is_empty() {
        out.push_str(&e.subsystem);
        if !e.category.is_empty() {
            out.push(':');
            out.push_str(&e.category);
        }
        out.push(' ');
    }
    out.push_str(&e.message);
    out
}

/// Writes `text` as one plain-text item to the general board (`pasteboard` service, `write`). Returns the request id;
/// its reply is an error on a shell without the service, and the caller then falls back to the plain clipboard.
pub fn copy_to_general(ctx: &Ctx, text: &str) -> u32 {
    Board::general().write(ctx, &[Item::default().with(pasteboard_types::STRING, text.as_bytes())])
}

/// A context with no shell behind it. The fonts come from the same asset folder the other apps' tests read.
#[cfg(test)]
pub(crate) fn test_ctx() -> Ctx {
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::Configure;
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::power_service;
    use bifrost_app::proto::ToShell;

    #[test]
    fn the_power_snapshot_is_one_call_to_the_power_service() {
        let ctx = test_ctx();
        let id = power_service::snapshot(&ctx);
        let sent = ctx.take_sent();
        assert_eq!(sent, vec![ToShell::ServiceCall { request: id, service: "power".into(), method: "snapshot".into(), body: "null".into() }]);
    }

    fn battery(percent: Option<u8>, state: PowerState, empty: Option<u32>, full: Option<u32>) -> Battery {
        Battery { percent, state, plugged_in: None, minutes_to_empty: empty, minutes_to_full: full }
    }

    #[test]
    fn nothing_is_shown_until_the_service_answers() {
        assert_eq!(PowerNote::default().line(), "");
    }

    #[test]
    fn a_snapshot_and_its_changes_update_only_their_part() {
        let mut p = PowerNote::default();
        p.apply_snapshot(Snapshot {
            thermal_state: ThermalState::Fair,
            low_power_mode: false,
            battery: Some(battery(Some(42), PowerState::Discharging, Some(75), None)),
        });
        assert_eq!(p.line(), "Thermal Fair · Battery 42%, discharging, 1 h 15 min left");
        p.apply_change(Change::LowPowerMode(true));
        p.apply_change(Change::ThermalState(ThermalState::Serious));
        assert_eq!(p.line(), "Thermal Serious · Battery 42%, discharging, 1 h 15 min left · Low Power Mode");
        p.apply_change(Change::Battery(None));
        assert_eq!(p.line(), "Thermal Serious · Low Power Mode", "a machine with no battery shows no battery");
    }

    #[test]
    fn a_pushed_battery_event_decodes_and_reads_as_the_footer_text() {
        let mut p = PowerNote::default();
        let body = bifrost_core::json::parse(r#"{"battery":{"present":true,"percent":10,"state":"discharging"}}"#).unwrap();
        let change = bifrost_app::power_service::decode_change("power", bifrost_app::power_service::BATTERY_DID_CHANGE, &body).unwrap();
        p.apply_change(change);
        assert_eq!(p.line(), "Battery 10%, discharging");
        assert!(bifrost_app::power_service::decode_change("pasteboard", "changed", &body).is_none());
    }

    #[test]
    fn charging_full_and_unknown_batteries_read_naturally() {
        assert_eq!(battery_text(&battery(Some(50), PowerState::Charging, None, Some(30))), "Battery 50%, charging, 30 min to full");
        assert_eq!(battery_text(&battery(Some(100), PowerState::Full, None, None)), "Battery 100%, full");
        assert_eq!(battery_text(&battery(Some(20), PowerState::NotCharging, None, None)), "Battery 20%, not charging");
        assert_eq!(battery_text(&battery(None, PowerState::Unknown, None, None)), "Battery");
        assert_eq!(battery_text(&battery(Some(5), PowerState::Discharging, Some(125), None)), "Battery 5%, discharging, 2 h 5 min left");
    }

    #[test]
    fn a_copied_line_has_its_time_process_level_subsystem_and_message() {
        let e = LogEntry {
            date: 0,
            process: "bifrost-notes".into(),
            pid: 4242,
            subsystem: "is.olibuijr.notes".into(),
            category: "sync".into(),
            level: Level::Error,
            message: "sync failed".into(),
            invocation: String::new(),
        };
        assert_eq!(copy_line(&e), "bifrost-notes[4242] Error is.olibuijr.notes:sync sync failed");
        let bare = LogEntry { message: "hi".into(), ..LogEntry::default() };
        assert_eq!(copy_line(&bare), "Default hi", "empty fields are left out");
    }

    #[test]
    fn copy_sends_one_pasteboard_write_to_the_general_board() {
        let ctx = test_ctx();
        let id = copy_to_general(&ctx, "hello log");
        let sent = ctx.take_sent();
        let [ToShell::ServiceCall { request, service, method, body }] = sent.as_slice() else {
            panic!("one service call expected");
        };
        assert_eq!(*request, id);
        assert_eq!((service.as_str(), method.as_str()), ("pasteboard", "write"));
        assert!(body.contains("general"), "the general board");
        assert!(body.contains("public.utf8-plain-text") && body.contains("hello log"), "a plain-text item");
    }
}
