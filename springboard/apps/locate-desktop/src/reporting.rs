//! When a device reports its position, and which contacts a report is copied to. The model reads no clock and sends
//! nothing: the caller passes `now` and acts on the answer.
//!
//! A device reports at most every 15 minutes while sharing is on, and at once when the owner asks for a locate. With
//! sharing off nothing is reported, and the first time sharing goes off the stored position is deleted from the cloud.
//! A contact copy goes only to a grant that has not ended, so a revoked or expired share gets no further copies.

use crate::share::Shares;

/// The interval between reports while sharing is on.
pub const REPORT_INTERVAL_SECS: i64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to send.
    Nothing,
    /// Seal the position and send it to the cloud (and to the contacts whose grants are live).
    Report,
    /// Delete the stored position from the cloud.
    Clear,
}

/// The throttle and the clear state. A new reporter has not confirmed that the cloud is clear, so the first step with
/// sharing off deletes whatever a previous run left there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reporter {
    last_report: Option<i64>,
    stored: bool,
}

impl Default for Reporter {
    fn default() -> Self {
        Reporter::new()
    }
}

impl Reporter {
    pub fn new() -> Reporter {
        Reporter { last_report: None, stored: true }
    }

    /// Decides the action for one tick. `locate_requested` is true when the owner asked for a locate since the last tick.
    pub fn step(&mut self, sharing: bool, locate_requested: bool, now: i64) -> Action {
        if !sharing {
            if self.stored {
                self.stored = false;
                self.last_report = None;
                return Action::Clear;
            }
            return Action::Nothing;
        }
        let due = self.last_report.map_or(true, |t| now - t >= REPORT_INTERVAL_SECS);
        if locate_requested || due {
            self.last_report = Some(now);
            self.stored = true;
            Action::Report
        } else {
            Action::Nothing
        }
    }
}

/// The share ids a report is copied to at `now`. `links` maps each contact's address to its cloud share id. A grant
/// that has ended, or has no share id, gets no copy. With sharing off there are no copies.
pub fn copy_targets(sharing: bool, shares: &Shares, links: &[(String, String)], now: i64) -> Vec<String> {
    if !sharing {
        return Vec::new();
    }
    shares
        .active(now)
        .into_iter()
        .filter_map(|grant| links.iter().find(|(address, _)| address.trim().to_lowercase() == grant.to).map(|(_, id)| id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::Duration;

    const T0: i64 = 1_700_000_000;

    #[test]
    fn a_device_reports_once_then_waits_fifteen_minutes() {
        let mut r = Reporter::new();
        assert_eq!(r.step(true, false, T0), Action::Report);
        assert_eq!(r.step(true, false, T0 + REPORT_INTERVAL_SECS - 1), Action::Nothing);
        assert_eq!(r.step(true, false, T0 + REPORT_INTERVAL_SECS), Action::Report);
    }

    #[test]
    fn a_locate_request_reports_at_once_inside_the_interval() {
        let mut r = Reporter::new();
        assert_eq!(r.step(true, false, T0), Action::Report);
        assert_eq!(r.step(true, true, T0 + 5), Action::Report);
        assert_eq!(r.step(true, false, T0 + 10), Action::Nothing, "the locate counted as the last report");
    }

    #[test]
    fn a_locate_request_with_sharing_off_reports_nothing() {
        let mut r = Reporter::new();
        assert_eq!(r.step(false, false, T0), Action::Clear);
        assert_eq!(r.step(false, true, T0 + 1), Action::Nothing);
    }

    #[test]
    fn turning_sharing_off_deletes_the_cloud_position_once() {
        let mut r = Reporter::new();
        r.step(true, false, T0);
        assert_eq!(r.step(false, false, T0 + 60), Action::Clear);
        assert_eq!(r.step(false, false, T0 + 120), Action::Nothing, "the delete is sent once");
    }

    #[test]
    fn a_new_reporter_clears_what_an_earlier_run_left_behind() {
        let mut r = Reporter::new();
        assert_eq!(r.step(false, false, T0), Action::Clear);
    }

    #[test]
    fn turning_sharing_back_on_reports_at_once() {
        let mut r = Reporter::new();
        r.step(true, false, T0);
        r.step(false, false, T0 + 60);
        assert_eq!(r.step(true, false, T0 + 120), Action::Report);
    }

    #[test]
    fn copies_go_only_to_live_grants_and_stop_after_revoke_or_expiry() {
        let mut shares = Shares::default();
        shares.share("anna@example.com", Duration::Hour, T0).unwrap();
        shares.share("bo@example.com", Duration::Indefinite, T0).unwrap();
        let links = vec![("Anna@Example.com".to_string(), "sh1".to_string()), ("bo@example.com".to_string(), "sh2".to_string())];
        assert_eq!(copy_targets(true, &shares, &links, T0 + 60), vec!["sh1".to_string(), "sh2".to_string()]);
        assert_eq!(copy_targets(true, &shares, &links, T0 + 3_600), vec!["sh2".to_string()], "the hour is over");
        shares.stop("bo@example.com");
        assert!(copy_targets(true, &shares, &links, T0 + 60).iter().all(|id| id != "sh2"), "a revoked share gets no copy");
    }

    #[test]
    fn no_copies_go_while_sharing_is_off() {
        let mut shares = Shares::default();
        shares.share("anna@example.com", Duration::Indefinite, T0).unwrap();
        let links = vec![("anna@example.com".to_string(), "sh1".to_string())];
        assert!(copy_targets(false, &shares, &links, T0).is_empty());
    }

    #[test]
    fn a_grant_without_a_cloud_share_id_gets_no_copy() {
        let mut shares = Shares::default();
        shares.share("anna@example.com", Duration::Indefinite, T0).unwrap();
        assert!(copy_targets(true, &shares, &[], T0).is_empty());
    }
}
