//! The `smartcards` service: the smart card readers and cards on the machine, as the token daemon and the token
//! binding agent of the reference system serve them. The model is `bifrost_ui` (`smartcard_slot::SlotManager` for the
//! reader slots, `token_watch::TokenWatcher` for the tokens present, `token_registry::TokenRegistry` for the cards
//! registered for sign-in); this module is the driver that feeds it.
//!
//! The driver is the PC/SC daemon of the Arch package set (`pcscd`, with the `pcsc-tools` package that ships
//! `pcsc_scan`). The service runs `pcsc_scan -n` once an app first calls it, reads the reader and card lines it
//! prints ([`ScanParser`]) and turns them into slot reports. A valid card (it answered with an ATR) becomes a token
//! whose ID is `pcsc:<reader>:<ATR hex>`. When `pcsc_scan` is not installed or exits, the service retries after
//! [`RETRY_MS`]. No PIN or key material passes through here: tokens are listed, never opened.
//!
//! Calls (JSON params):
//! - `slots {}` -> `{slots: [{name, state, atr?}]}` with state one of `missing`, `empty`, `probing`, `mute`, `valid`
//! - `tokens {}` -> `{tokens: [{token_id, slot, driver, registered}]}`
//! - `registered {}` -> `{tokens: [id], required}`: the cards registered for sign-in and whether sign-in needs one
//! - `register {token_id}` / `unregister {token_id}` -> `{registered}`: a card must be present to be registered
//! - `policy {required}` -> `{required}`: whether sign-in needs a registered card
//!
//! Events to every app: `token_inserted` and `token_removed`, each `{token_id, slot}`.
//!
//! The registrations and the policy are saved in `smartcards/registered.txt` under the data directory: one token ID
//! per line and a `policy=required` line when sign-in needs a card.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::smartcard_slot::{SlotEvent, SlotManager, SlotState};
use bifrost_ui::token_registry::{AuthPolicy, TokenRegistry};
use bifrost_ui::token_watch::{TokenError, TokenInfo, TokenWatcher, WatchEvent};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The name the service is registered under.
pub const SERVICE: &str = "smartcards";
/// The driver name shown for a token.
pub const DRIVER: &str = "pcsc";
/// The program that reports readers and cards.
pub const SCAN_PROGRAM: &str = "pcsc_scan";
/// How long to wait before starting the scan again after it ended or could not start.
pub const RETRY_MS: f64 = 30_000.0;

/// What a reader reported: the slot's state and, for a valid card, its answer to reset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub reader: String,
    pub state: SlotState,
    pub atr: Option<Vec<u8>>,
}

/// Reads the output of `pcsc_scan -n`. A reader block is a ` Reader 0: <name>` line, an `Event number` line, a
/// `Card state:` line and, when a card answered, an `ATR:` line of hex bytes.
#[derive(Debug, Default)]
pub struct ScanParser {
    reader: Option<String>,
    /// a card is inserted and its ATR line has not come yet
    waiting_for_atr: bool,
}

impl ScanParser {
    /// Feeds one output line; returns the reports it completes (a block with a card inserted completes with its ATR
    /// line, or when the next reader block starts without one, as a mute card).
    pub fn line(&mut self, line: &str) -> Vec<Report> {
        let t = line.trim();
        let mut out = Vec::new();
        if let Some(rest) = t.strip_prefix("Reader ") {
            out.extend(self.flush_mute());
            self.reader = rest.split_once(": ").map(|(_, name)| name.trim().to_string()).filter(|n| !n.is_empty());
            return out;
        }
        let Some(reader) = self.reader.clone() else { return out };
        if let Some(state) = t.strip_prefix("Card state:") {
            self.waiting_for_atr = false;
            if state.contains("Unresponsive") || state.contains("Unpowered") {
                out.push(Report { reader, state: SlotState::MuteCard, atr: None });
            } else if state.contains("Card inserted") {
                self.waiting_for_atr = true;
            } else if state.contains("unavailable") || state.contains("Reader unknown") {
                out.push(Report { reader, state: SlotState::Missing, atr: None });
            } else if state.contains("Card removed") {
                out.push(Report { reader, state: SlotState::Empty, atr: None });
            }
        } else if let Some(hex) = t.strip_prefix("ATR:") {
            if self.waiting_for_atr {
                self.waiting_for_atr = false;
                out.push(match parse_hex(hex) {
                    Some(atr) => Report { reader, state: SlotState::ValidCard, atr: Some(atr) },
                    None => Report { reader, state: SlotState::MuteCard, atr: None },
                });
            }
        }
        out
    }

    fn flush_mute(&mut self) -> Option<Report> {
        if !std::mem::take(&mut self.waiting_for_atr) {
            return None;
        }
        Some(Report { reader: self.reader.clone()?, state: SlotState::MuteCard, atr: None })
    }
}

/// `3B 7F 96` as bytes. None for empty text or a byte that is not hex.
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let bytes: Option<Vec<u8>> = text.split_whitespace().map(|b| u8::from_str_radix(b, 16).ok()).collect();
    bytes.filter(|b| !b.is_empty())
}

/// The token ID of the card with answer `atr` in `reader`.
pub fn token_id(reader: &str, atr: &[u8]) -> String {
    let hex: String = atr.iter().map(|b| format!("{b:02X}")).collect();
    format!("{DRIVER}:{reader}:{hex}")
}

/// A change in the tokens present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardEvent {
    Inserted(TokenInfo),
    Removed(TokenInfo),
}

/// The readers, the tokens and the sign-in registrations, with the rules that tie a report to them.
#[derive(Debug, Default)]
pub struct Cards {
    slots: SlotManager,
    tokens: TokenWatcher,
    registry: TokenRegistry,
    /// the token ID in each slot
    in_slot: BTreeMap<String, String>,
}

impl Cards {
    /// Applies one report: an unknown reader becomes a slot, a valid card becomes a token, and a card that went away
    /// (or was swapped for another) takes its token with it. Returns the token changes in order.
    pub fn apply(&mut self, report: Report) -> Vec<CardEvent> {
        if self.slots.slot(&report.reader).is_none() && self.slots.add_slot(&report.reader).is_err() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let old_atr = self.slots.slot(&report.reader).and_then(|s| s.atr()).map(<[u8]>::to_vec);
        let swapped = report.state == SlotState::ValidCard && old_atr.is_some() && old_atr != report.atr;
        let changed = match self.slots.report(&report.reader, report.state, report.atr.clone()) {
            Ok(event) => event,
            Err(_) => return out,
        };
        if swapped || matches!(changed, Some(SlotEvent::CardRemoved)) || report.state != SlotState::ValidCard {
            if let Some(id) = self.in_slot.remove(&report.reader) {
                if let Ok(WatchEvent::Removed(info)) = self.tokens.remove(&id) {
                    out.push(CardEvent::Removed(info));
                }
            }
        }
        if report.state == SlotState::ValidCard && !self.in_slot.contains_key(&report.reader) {
            if let Some(atr) = &report.atr {
                let info = TokenInfo { token_id: token_id(&report.reader, atr), driver_name: DRIVER.to_string(), slot_name: report.reader.clone() };
                if let Ok(WatchEvent::Inserted(info)) = self.tokens.insert(info) {
                    self.in_slot.insert(report.reader.clone(), info.token_id.clone());
                    out.push(CardEvent::Inserted(info));
                }
            }
        }
        out
    }

    /// Registers a card that is present for sign-in. Returns true when it was new.
    pub fn register(&mut self, token_id: &str) -> Result<bool, TokenError> {
        if self.tokens.token_info(token_id).is_none() {
            return Err(TokenError::TokenNotFound);
        }
        self.registry.register(token_id)
    }

    pub fn unregister(&mut self, token_id: &str) -> Result<(), TokenError> {
        self.registry.unregister(token_id)
    }

    pub fn set_required(&mut self, required: bool) {
        self.registry.set_policy(if required { AuthPolicy::Required } else { AuthPolicy::Optional });
    }

    pub fn required(&self) -> bool {
        self.registry.policy() == AuthPolicy::Required
    }

    /// The registrations as the text saved on disk.
    pub fn registry_text(&self) -> String {
        let mut text: String = self.registry.registered().iter().map(|id| format!("{id}\n")).collect();
        if self.required() {
            text.push_str("policy=required\n");
        }
        text
    }

    /// Loads registrations from the saved text; lines that are not token IDs are skipped.
    pub fn load_registry(&mut self, text: &str) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if line == "policy=required" {
                self.set_required(true);
            } else if line.starts_with(&format!("{DRIVER}:")) {
                let _ = self.registry.register(line);
            }
        }
    }
}

/// A running scan, as the service drives it.
pub trait Scan {
    /// The output lines since the last call.
    fn take_lines(&mut self) -> Vec<String>;
    /// True once the scan has ended.
    fn finished(&mut self) -> bool;
}

/// Starts a scan; an error says why it could not.
pub trait ScanSpawner {
    fn start(&mut self) -> Result<Box<dyn Scan>, String>;
}

struct PcscSpawner;

struct PcscScan {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
}

impl ScanSpawner for PcscSpawner {
    fn start(&mut self) -> Result<Box<dyn Scan>, String> {
        let mut child = Command::new(SCAN_PROGRAM)
            .arg("-n")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("smartcards: {SCAN_PROGRAM}: {e}"))?;
        let lines = Arc::new(Mutex::new(Vec::new()));
        if let Some(out) = child.stdout.take() {
            let shared = Arc::clone(&lines);
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Ok(mut l) = shared.lock() {
                        l.push(line);
                    }
                }
            });
        }
        Ok(Box::new(PcscScan { child, lines }))
    }
}

impl Scan for PcscScan {
    fn take_lines(&mut self) -> Vec<String> {
        self.lines.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default()
    }

    fn finished(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PcscScan {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct SmartCards {
    store: PathBuf,
    spawner: Box<dyn ScanSpawner>,
    scan: Option<Box<dyn Scan>>,
    parser: ScanParser,
    cards: Cards,
    /// a call has been made: the scan runs from now on
    wanted: bool,
    next_try: f64,
}

impl SmartCards {
    /// The service with its registrations under `data_dir`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with(data_dir.join("smartcards").join("registered.txt"), Box::new(PcscSpawner))
    }

    fn with(store: PathBuf, spawner: Box<dyn ScanSpawner>) -> Self {
        let mut cards = Cards::default();
        cards.load_registry(&std::fs::read_to_string(&store).unwrap_or_default());
        SmartCards { store, spawner, scan: None, parser: ScanParser::default(), cards, wanted: false, next_try: 0.0 }
    }

    /// Starts the scan when it is wanted and not running, then reads what it printed. Returns the events.
    pub fn poll(&mut self, t: f64) -> Vec<Event> {
        if !self.wanted {
            return Vec::new();
        }
        if self.scan.is_none() && t >= self.next_try {
            match self.spawner.start() {
                Ok(scan) => self.scan = Some(scan),
                Err(_) => self.next_try = t + RETRY_MS,
            }
        }
        let Some(scan) = self.scan.as_mut() else { return Vec::new() };
        let lines = scan.take_lines();
        let ended = scan.finished();
        let mut events = Vec::new();
        for line in lines {
            for report in self.parser.line(&line) {
                events.extend(self.cards.apply(report).into_iter().map(card_event));
            }
        }
        if ended {
            // the readers are gone with the scan: report every slot empty so no stale token stays
            self.scan = None;
            self.next_try = t + RETRY_MS;
            for name in self.cards.slots.slot_names().into_iter().map(str::to_string).collect::<Vec<_>>() {
                events.extend(self.cards.apply(Report { reader: name, state: SlotState::Empty, atr: None }).into_iter().map(card_event));
            }
        }
        events
    }

    fn save(&self) -> Result<(), String> {
        if let Some(dir) = self.store.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("smartcards: {e}"))?;
        }
        std::fs::write(&self.store, self.cards.registry_text()).map_err(|e| format!("smartcards: {e}"))
    }

    /// Answers one call.
    pub fn handle(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        self.wanted = true;
        match method {
            "slots" => {
                let slots = self.cards.slots.slot_names().into_iter().filter_map(|n| self.cards.slots.slot(n)).map(|s| {
                    let mut f = vec![("name", Json::Str(s.name().to_string())), ("state", Json::Str(state_name(s.state()).to_string()))];
                    if let Some(atr) = s.atr() {
                        f.push(("atr", Json::Str(atr.iter().map(|b| format!("{b:02X}")).collect())));
                    }
                    obj(f)
                });
                Ok(obj(vec![("slots", Json::Arr(slots.collect()))]))
            }
            "tokens" => {
                let tokens = self.cards.tokens.token_ids().into_iter().filter_map(|id| self.cards.tokens.token_info(id)).map(|i| {
                    obj(vec![
                        ("token_id", Json::Str(i.token_id.clone())),
                        ("slot", Json::Str(i.slot_name.clone())),
                        ("driver", Json::Str(i.driver_name.clone())),
                        ("registered", Json::Bool(self.cards.registry.is_registered(&i.token_id))),
                    ])
                });
                Ok(obj(vec![("tokens", Json::Arr(tokens.collect()))]))
            }
            "registered" => Ok(obj(vec![
                ("tokens", Json::Arr(self.cards.registry.registered().into_iter().map(|t| Json::Str(t.to_string())).collect())),
                ("required", Json::Bool(self.cards.required())),
            ])),
            "register" | "unregister" => {
                let id = params.str_field("token_id").filter(|i| !i.is_empty()).ok_or("smartcards: a token_id is needed")?;
                let result = if method == "register" { self.cards.register(&id).map(|_| ()) } else { self.cards.unregister(&id) };
                result.map_err(|e| format!("smartcards: {e}"))?;
                self.save()?;
                Ok(obj(vec![("registered", Json::Bool(method == "register"))]))
            }
            "policy" => {
                let Some(Json::Bool(required)) = params.get("required") else { return Err("smartcards: policy needs required: true or false".into()) };
                self.cards.set_required(*required);
                self.save()?;
                Ok(obj(vec![("required", Json::Bool(*required))]))
            }
            _ => Err(format!("smartcards has no method {method}")),
        }
    }
}

impl SystemService for SmartCards {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(method, params)
    }

    fn tick(&mut self, _shell: &mut Shell, t: f64) -> Vec<Event> {
        self.poll(t)
    }
}

fn card_event(e: CardEvent) -> Event {
    let (name, info) = match e {
        CardEvent::Inserted(i) => ("token_inserted", i),
        CardEvent::Removed(i) => ("token_removed", i),
    };
    Event { app: None, event: name.to_string(), body: obj(vec![("token_id", Json::Str(info.token_id)), ("slot", Json::Str(info.slot_name))]) }
}

fn state_name(s: SlotState) -> &'static str {
    match s {
        SlotState::Missing => "missing",
        SlotState::Empty => "empty",
        SlotState::Probing => "probing",
        SlotState::MuteCard => "mute",
        SlotState::ValidCard => "valid",
    }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The data directory the shell keeps its files in (`$XDG_DATA_HOME/springboard`).
pub fn data_dir() -> PathBuf {
    super::recognizer::user_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSERT: &str = " Reader 0: Alcor Micro AU9560 00 00\n  Event number: 1\n  Card state: Card inserted, \n  ATR: 3B 7F 96 00\n";
    const REMOVE: &str = " Reader 0: Alcor Micro AU9560 00 00\n  Event number: 2\n  Card state: Card removed, \n";

    fn reports(text: &str) -> Vec<Report> {
        let mut p = ScanParser::default();
        text.lines().flat_map(|l| p.line(l)).collect()
    }

    #[test]
    fn a_card_with_an_answer_is_a_valid_card_and_a_removal_is_an_empty_slot() {
        let r = reports(&format!("PC/SC device scanner\nScanning present readers...\n{INSERT}{REMOVE}"));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], Report { reader: "Alcor Micro AU9560 00 00".into(), state: SlotState::ValidCard, atr: Some(vec![0x3B, 0x7F, 0x96, 0x00]) });
        assert_eq!(r[1].state, SlotState::Empty);
    }

    #[test]
    fn an_unresponsive_card_and_a_card_without_an_atr_line_are_mute() {
        let r = reports(" Reader 0: R\n  Card state: Card inserted, Unresponsive card, \n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].state, SlotState::MuteCard);
        let r = reports(" Reader 0: R\n  Card state: Card inserted, \n Reader 1: S\n  Card state: Card removed, \n");
        assert_eq!(r.iter().map(|r| (r.reader.as_str(), r.state)).collect::<Vec<_>>(), vec![("R", SlotState::MuteCard), ("S", SlotState::Empty)]);
    }

    #[test]
    fn hex_bytes_parse_and_bad_text_does_not() {
        assert_eq!(parse_hex(" 3B 0a ff"), Some(vec![0x3B, 0x0A, 0xFF]));
        assert_eq!(parse_hex("3B zz"), None);
        assert_eq!(parse_hex("   "), None);
    }

    #[test]
    fn a_card_becomes_a_token_and_leaving_removes_it() {
        let mut c = Cards::default();
        let ev = c.apply(reports(INSERT).remove(0));
        assert_eq!(ev.len(), 1);
        let CardEvent::Inserted(info) = &ev[0] else { panic!("an insert") };
        assert_eq!(info.token_id, "pcsc:Alcor Micro AU9560 00 00:3B7F9600");
        assert_eq!(c.tokens.token_ids().len(), 1);
        let ev = c.apply(reports(REMOVE).remove(0));
        assert!(matches!(ev.as_slice(), [CardEvent::Removed(i)] if i.token_id == info.token_id));
        assert!(c.tokens.token_ids().is_empty());
    }

    #[test]
    fn swapping_the_card_removes_the_old_token_and_adds_the_new() {
        let mut c = Cards::default();
        let r = |atr: u8| Report { reader: "R".into(), state: SlotState::ValidCard, atr: Some(vec![atr]) };
        c.apply(r(1));
        let ev = c.apply(r(2));
        assert!(matches!(ev.as_slice(), [CardEvent::Removed(_), CardEvent::Inserted(_)]));
        assert_eq!(c.tokens.token_ids(), vec!["pcsc:R:02"]);
        assert!(c.apply(r(2)).is_empty(), "the same card again is no change");
    }

    #[test]
    fn only_a_present_card_can_be_registered_and_the_registry_round_trips() {
        let mut c = Cards::default();
        assert_eq!(c.register("pcsc:R:01"), Err(TokenError::TokenNotFound));
        c.apply(Report { reader: "R".into(), state: SlotState::ValidCard, atr: Some(vec![1]) });
        assert_eq!(c.register("pcsc:R:01"), Ok(true));
        c.set_required(true);
        let text = c.registry_text();
        assert_eq!(text, "pcsc:R:01\npolicy=required\n");
        let mut d = Cards::default();
        d.load_registry(&format!("{text}garbage line\n"));
        assert!(d.registry.is_registered("pcsc:R:01"));
        assert!(d.required());
        assert_eq!(d.unregister("pcsc:R:02"), Err(TokenError::TokenNotFound));
    }

    struct Fake {
        lines: Vec<String>,
        done: bool,
    }
    impl Scan for Fake {
        fn take_lines(&mut self) -> Vec<String> {
            std::mem::take(&mut self.lines)
        }
        fn finished(&mut self) -> bool {
            self.done
        }
    }
    struct FakeSpawner(Option<Fake>);
    impl ScanSpawner for FakeSpawner {
        fn start(&mut self) -> Result<Box<dyn Scan>, String> {
            self.0.take().map(|f| Box::new(f) as Box<dyn Scan>).ok_or_else(|| "no pcsc_scan".to_string())
        }
    }

    fn service(fake: Option<Fake>) -> SmartCards {
        let dir = std::env::temp_dir().join(format!("smartcards-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        SmartCards::with(dir.join("registered.txt"), Box::new(FakeSpawner(fake)))
    }

    #[test]
    fn nothing_runs_until_an_app_calls_and_then_the_scan_feeds_the_events() {
        let lines = INSERT.lines().map(str::to_string).collect();
        let mut s = service(Some(Fake { lines, done: false }));
        assert!(s.poll(0.0).is_empty(), "no scan before a call");
        let none = s.handle("tokens", &Json::Obj(vec![])).unwrap();
        assert_eq!(none.get("tokens"), Some(&Json::Arr(vec![])));
        let ev = s.poll(1.0);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event, "token_inserted");
        assert_eq!(ev[0].body.str_field("slot").as_deref(), Some("Alcor Micro AU9560 00 00"));
        let slots = s.handle("slots", &Json::Obj(vec![])).unwrap();
        let first = &slots.get("slots").and_then(Json::as_array).unwrap()[0];
        assert_eq!(first.str_field("state").as_deref(), Some("valid"));
        assert_eq!(first.str_field("atr").as_deref(), Some("3B7F9600"));
    }

    #[test]
    fn when_the_scan_ends_every_token_goes_and_the_scan_is_retried_later() {
        let lines = INSERT.lines().map(str::to_string).collect();
        let mut s = service(Some(Fake { lines, done: true }));
        s.handle("slots", &Json::Obj(vec![])).unwrap();
        let ev = s.poll(1.0);
        assert_eq!(ev.iter().map(|e| e.event.as_str()).collect::<Vec<_>>(), vec!["token_inserted", "token_removed"]);
        assert!(s.scan.is_none());
        assert!(s.poll(2.0).is_empty(), "not retried before the delay");
        assert!(s.next_try >= 1.0 + RETRY_MS);
    }

    #[test]
    fn registering_saves_and_a_new_service_reads_it_back() {
        let lines = INSERT.lines().map(str::to_string).collect();
        let mut s = service(Some(Fake { lines, done: false }));
        s.handle("slots", &Json::Obj(vec![])).unwrap();
        s.poll(1.0);
        let id = "pcsc:Alcor Micro AU9560 00 00:3B7F9600";
        let params = Json::Obj(vec![("token_id".into(), Json::Str(id.into()))]);
        assert!(s.handle("register", &params).is_ok());
        s.handle("policy", &Json::Obj(vec![("required".into(), Json::Bool(true))])).unwrap();
        let again = SmartCards::with(s.store.clone(), Box::new(FakeSpawner(None)));
        assert!(again.cards.registry.is_registered(id));
        assert!(again.cards.required());
        assert!(s.handle("register", &Json::Obj(vec![("token_id".into(), Json::Str("pcsc:X:00".into()))])).is_err());
        assert!(s.handle("nope", &Json::Obj(vec![])).is_err());
    }
}
