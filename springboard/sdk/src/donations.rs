//! Donations and discovery (`IntentDonationManager`, `RelevantIntentManager`, `RelevantIntent`).
//!
//! An app tells the system which intents people run: [`DonationLog::donate`] records one run, and
//! [`DonationLog::relevant`] ranks the donated intents for suggestion. A donation fades with age (half-life of one
//! week), so recent use outranks old use. The log is plain data: an app keeps it in its data directory with
//! [`DonationLog::to_json`] and reads it back with [`DonationLog::from_json`]. Nothing here talks to the shell yet;
//! the shell's suggestion list (Search) is the place that will read the ranking. See docs/parity/app-intents.md.

use bifrost_core::json::Json;

/// A donation's weight halves every week.
const HALF_LIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// The log keeps this many donations; the oldest recorded ones go first.
const MAX_DONATIONS: usize = 500;

/// One run of an intent, optionally about an entity (`donate(intent:)` with a related `AppEntity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Donation {
    pub intent_id: String,
    pub entity: Option<String>,
    /// unix seconds
    pub at: u64,
}

/// `RelevantIntent`: one suggestion, with the decayed number of donations behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct Relevant {
    pub intent_id: String,
    pub entity: Option<String>,
    pub score: f64,
}

/// The donated runs of one app, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DonationLog {
    donations: Vec<Donation>,
}

impl DonationLog {
    pub fn new() -> DonationLog {
        DonationLog::default()
    }
    pub fn len(&self) -> usize {
        self.donations.len()
    }
    pub fn is_empty(&self) -> bool {
        self.donations.is_empty()
    }
    /// Records one run of `intent_id` at unix second `at`. Past the cap the earliest-recorded donation is dropped.
    pub fn donate(&mut self, intent_id: &str, entity: Option<&str>, at: u64) {
        self.donations.push(Donation { intent_id: intent_id.into(), entity: entity.map(str::to_string), at });
        self.cap();
    }
    /// `deleteDonations(matching:)`: removes every donation `pred` selects and returns how many went.
    pub fn delete_matching(&mut self, pred: impl Fn(&Donation) -> bool) -> usize {
        let before = self.donations.len();
        self.donations.retain(|d| !pred(d));
        before - self.donations.len()
    }
    /// `updateRelevantIntents`: the donated (intent, entity) pairs ranked for suggestion at unix second `now`, at most
    /// `limit` of them. A pair scores the sum of `0.5^(age / half-life)` over its donations, so a donation made now
    /// counts 1 and one a week old counts 0.5. Equal scores are ordered by intent id, then entity.
    pub fn relevant(&self, now: u64, limit: usize) -> Vec<Relevant> {
        let mut out: Vec<Relevant> = Vec::new();
        for d in &self.donations {
            let age = now.saturating_sub(d.at) as f64;
            let weight = 0.5f64.powf(age / HALF_LIFE_SECS);
            match out.iter_mut().find(|r| r.intent_id == d.intent_id && r.entity == d.entity) {
                Some(r) => r.score += weight,
                None => out.push(Relevant { intent_id: d.intent_id.clone(), entity: d.entity.clone(), score: weight }),
            }
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.intent_id.cmp(&b.intent_id)).then_with(|| a.entity.cmp(&b.entity)));
        out.truncate(limit);
        out
    }
    /// The log as a JSON array of `{"intent", "entity" (text or null), "at"}`.
    pub fn to_json(&self) -> Json {
        Json::Arr(
            self.donations
                .iter()
                .map(|d| {
                    Json::Obj(vec![
                        ("intent".into(), Json::Str(d.intent_id.clone())),
                        ("entity".into(), d.entity.clone().map_or(Json::Null, Json::Str)),
                        ("at".into(), Json::Num(d.at as f64)),
                    ])
                })
                .collect(),
        )
    }
    /// Reads a log written by `to_json`. One malformed entry fails the whole read (`None`), so a damaged file is not
    /// half-loaded.
    pub fn from_json(j: &Json) -> Option<DonationLog> {
        let mut log = DonationLog::new();
        for e in j.as_array()? {
            let intent_id = e.get("intent")?.as_str()?.to_string();
            let entity = match e.get("entity")? {
                Json::Null => None,
                Json::Str(s) => Some(s.clone()),
                _ => return None,
            };
            let at = e.get("at")?.as_u64()?;
            log.donations.push(Donation { intent_id, entity, at });
        }
        log.cap();
        Some(log)
    }
    fn cap(&mut self) {
        if self.donations.len() > MAX_DONATIONS {
            let extra = self.donations.len() - MAX_DONATIONS;
            self.donations.drain(..extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json;

    const WEEK: u64 = 7 * 24 * 3600;

    #[test]
    fn donate_counts_and_reports_empty() {
        let mut log = DonationLog::new();
        assert!(log.is_empty());
        log.donate("add-note", None, 1_000);
        log.donate("add-note", Some("Home"), 1_000);
        assert_eq!(log.len(), 2);
        assert!(!log.is_empty());
    }

    #[test]
    fn recent_use_outranks_old_volume() {
        let now = 10 * WEEK;
        let mut log = DonationLog::new();
        log.donate("open-note", None, now);
        for _ in 0..3 {
            log.donate("old-search", None, now - 4 * WEEK); // 3 * 0.5^4 = 0.1875 < 1
        }
        let r = log.relevant(now, 10);
        assert_eq!(r[0].intent_id, "open-note");
        assert!((r[0].score - 1.0).abs() < 1e-9, "a donation made now weighs 1");
        assert_eq!(r[1].intent_id, "old-search");
        assert!((r[1].score - 0.1875).abs() < 1e-9);
    }

    #[test]
    fn enough_old_donations_can_still_win() {
        let now = 10 * WEEK;
        let mut log = DonationLog::new();
        log.donate("fresh", None, now);
        for _ in 0..20 {
            log.donate("habit", None, now - 4 * WEEK); // 20 * 0.0625 = 1.25
        }
        assert_eq!(log.relevant(now, 1)[0].intent_id, "habit");
    }

    #[test]
    fn entities_are_scored_apart_from_their_intent() {
        let now = 5 * WEEK;
        let mut log = DonationLog::new();
        log.donate("open", Some("Work"), now);
        log.donate("open", Some("Work"), now);
        log.donate("open", Some("Home"), now);
        let r = log.relevant(now, 10);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].entity.as_deref(), r[0].score), (Some("Work"), 2.0));
        assert_eq!((r[1].entity.as_deref(), r[1].score), (Some("Home"), 1.0));
    }

    #[test]
    fn equal_scores_order_by_id_and_limit_truncates() {
        let now = WEEK;
        let mut log = DonationLog::new();
        log.donate("zeta", None, now);
        log.donate("alpha", None, now);
        log.donate("mid", None, now);
        let ids: Vec<String> = log.relevant(now, 10).into_iter().map(|r| r.intent_id).collect();
        assert_eq!(ids, vec!["alpha", "mid", "zeta"]);
        assert_eq!(log.relevant(now, 2).len(), 2);
        assert!(log.relevant(now, 0).is_empty());
    }

    #[test]
    fn future_donations_count_as_fresh() {
        let mut log = DonationLog::new();
        log.donate("later", None, 2 * WEEK);
        assert!((log.relevant(WEEK, 1)[0].score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn delete_matching_removes_selected_donations() {
        let mut log = DonationLog::new();
        log.donate("a", None, 1);
        log.donate("b", None, 2);
        log.donate("a", Some("x"), 3);
        let removed = log.delete_matching(|d| d.intent_id == "a");
        assert_eq!(removed, 2);
        assert_eq!(log.len(), 1);
        assert_eq!(log.relevant(2, 5)[0].intent_id, "b");
        assert_eq!(log.delete_matching(|_| false), 0);
    }

    #[test]
    fn log_keeps_only_the_newest_donations() {
        let mut log = DonationLog::new();
        log.donate("first", None, 0);
        for i in 0..MAX_DONATIONS as u64 {
            log.donate("later", None, i + 1);
        }
        assert_eq!(log.len(), MAX_DONATIONS);
        assert!(log.relevant(1_000, 10).iter().all(|r| r.intent_id == "later"), "the earliest donation was dropped");
    }

    #[test]
    fn log_round_trips_through_json_and_rejects_damage() {
        let mut log = DonationLog::new();
        log.donate("open", Some("Work"), 42);
        log.donate("add", None, 43);
        let text = crate::control::encode(&log.to_json());
        assert_eq!(text, r#"[{"intent":"open","entity":"Work","at":42},{"intent":"add","entity":null,"at":43}]"#);
        let back = DonationLog::from_json(&json::parse(&text).unwrap()).unwrap();
        assert_eq!(back, log);
        assert!(DonationLog::from_json(&json::parse(r#"[{"intent":"x","entity":3,"at":1}]"#).unwrap()).is_none());
        assert!(DonationLog::from_json(&json::parse(r#"[{"intent":"x","entity":null,"at":-1}]"#).unwrap()).is_none(), "time is not negative");
        assert!(DonationLog::from_json(&json::parse(r#"{"intent":"x"}"#).unwrap()).is_none(), "a log is an array");
    }
}
