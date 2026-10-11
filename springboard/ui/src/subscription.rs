//! Subscription periods and renewal state for auto-renewing subscriptions: the duration between renewals (days, weeks,
//! months or years), and where a subscription stands on a given day (paid, in a billing grace period, retrying a failed
//! charge, or expired). Days count from 1970-01-01 as in `calendar`. Pure: the store that records the dates is the app's.

use crate::calendar::{add_months, from_days, to_days};

/// The unit of a period (`SubscriptionPeriod.Unit`: day, week, month, year).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

/// The duration between two renewals, e.g. `Period::new(Unit::Month, 1)` for a monthly subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub unit: Unit,
    /// At least 1: a period of zero days would renew on the day it ends, forever.
    pub value: u32,
}

impl Period {
    pub fn new(unit: Unit, value: u32) -> Self {
        Period { unit, value: value.max(1) }
    }

    /// The day the period after `day` ends. Months and years clamp to the end of a shorter month (January 31 plus one
    /// month is February 28 or 29), as `calendar::add_months` does.
    pub fn add_to(self, day: i64) -> i64 {
        let n = self.value as i64;
        match self.unit {
            Unit::Day => day + n,
            Unit::Week => day + 7 * n,
            Unit::Month => to_days(add_months(from_days(day), self.value as i32)),
            Unit::Year => to_days(add_months(from_days(day), 12 * self.value as i32)),
        }
    }
}

/// Where a subscription stands on a day. The variant names are BifrOSt's; they cover the states a store reports for a
/// subscription (current, grace period, billing retry, expired).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The paid period runs past the day.
    Subscribed,
    /// The paid period has ended, but the billing grace period still gives access.
    InGracePeriod,
    /// The paid period has ended and a failed charge is being retried, with no grace period left.
    InBillingRetry,
    /// The paid period has ended with no grace period and no retry.
    Expired,
}

/// The renewal facts the store keeps for one subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Renewal {
    /// The day the paid period ends (the expiration date); the subscription is current before this day.
    pub expires: i64,
    /// The last day of a billing grace period after `expires`, if the store grants one.
    pub grace_until: Option<i64>,
    /// The store is retrying a failed renewal charge.
    pub billing_retry: bool,
    /// Whether the customer keeps automatic renewal on.
    pub auto_renew: bool,
}

impl Renewal {
    /// A subscription paid until `expires`, renewing automatically, with no grace period and no retry.
    pub fn new(expires: i64) -> Self {
        Renewal { expires, grace_until: None, billing_retry: false, auto_renew: true }
    }

    /// The state on `day`.
    pub fn state(&self, day: i64) -> State {
        if day < self.expires {
            State::Subscribed
        } else if self.grace_until.is_some_and(|g| day <= g) {
            State::InGracePeriod
        } else if self.billing_retry {
            State::InBillingRetry
        } else {
            State::Expired
        }
    }

    /// Whether the customer has access on `day`: the paid period, or the grace period after it.
    pub fn grants_access(&self, day: i64) -> bool {
        matches!(self.state(day), State::Subscribed | State::InGracePeriod)
    }

    /// The day the period after the current one ends, when automatic renewal is on.
    pub fn next_expiration(&self, period: Period) -> Option<i64> {
        self.auto_renew.then(|| period.add_to(self.expires))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Date;

    fn day(year: i32, month: u32, d: u32) -> i64 {
        to_days(Date { year, month, day: d })
    }

    #[test]
    fn a_month_clamps_to_the_end_of_a_shorter_month() {
        let next = Period::new(Unit::Month, 1).add_to(day(2026, 1, 31));
        assert_eq!(from_days(next), Date { year: 2026, month: 2, day: 28 });
        let leap = Period::new(Unit::Month, 1).add_to(day(2028, 1, 31));
        assert_eq!(from_days(leap), Date { year: 2028, month: 2, day: 29 });
    }

    #[test]
    fn a_year_from_leap_day_clamps_in_a_common_year() {
        let next = Period::new(Unit::Year, 1).add_to(day(2028, 2, 29));
        assert_eq!(from_days(next), Date { year: 2029, month: 2, day: 28 });
    }

    #[test]
    fn days_and_weeks_are_plain_day_counts() {
        assert_eq!(Period::new(Unit::Day, 3).add_to(10), 13);
        assert_eq!(Period::new(Unit::Week, 2).add_to(10), 24);
    }

    #[test]
    fn a_zero_period_is_clamped_to_one_unit() {
        let p = Period::new(Unit::Week, 0);
        assert_eq!(p.value, 1);
        assert_eq!(p.add_to(10), 17);
    }

    #[test]
    fn the_state_follows_the_paid_period_then_expires() {
        let r = Renewal::new(100);
        assert_eq!(r.state(99), State::Subscribed);
        assert_eq!(r.state(100), State::Expired);
        assert!(r.grants_access(99));
        assert!(!r.grants_access(100));
    }

    #[test]
    fn a_grace_period_keeps_access_and_a_retry_without_grace_does_not() {
        let graced = Renewal { grace_until: Some(105), ..Renewal::new(100) };
        assert_eq!(graced.state(102), State::InGracePeriod);
        assert!(graced.grants_access(102));
        assert_eq!(graced.state(106), State::Expired);

        let retrying = Renewal { billing_retry: true, ..Renewal::new(100) };
        assert_eq!(retrying.state(101), State::InBillingRetry);
        assert!(!retrying.grants_access(101));
    }

    #[test]
    fn the_next_expiration_is_one_period_on_only_while_auto_renew_is_on() {
        let month = Period::new(Unit::Day, 30);
        assert_eq!(Renewal::new(100).next_expiration(month), Some(130));
        let off = Renewal { auto_renew: false, ..Renewal::new(100) };
        assert_eq!(off.next_expiration(month), None);
    }
}
