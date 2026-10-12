//! A store test session for purchase flows, the counterpart of the StoreKit testing harness (`StoreKit Test`). The
//! session keeps its own clock in days from 1970-01-01, buys products on that day, refunds and finishes purchases, and
//! clears every recorded purchase. It drives the same `entitlement::Ledger` and `purchase::Purchase` models that the app
//! uses, so a test sees the entitlements the app would see. Pure: nothing is charged, stored or sent to a store.

use crate::entitlement::Ledger;
use crate::purchase::{Outcome, Purchase};
use crate::subscription::Renewal;

/// One test session: a clock and the purchases made while it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct TestSession {
    ledger: Ledger,
    day: i64,
}

impl TestSession {
    /// A session with no purchases, starting on `day`.
    pub fn new(day: i64) -> Self {
        TestSession { ledger: Ledger::new(), day }
    }

    /// The session's current day.
    pub fn day(&self) -> i64 {
        self.day
    }

    /// The purchases recorded in this session.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Moves the clock forward by `days` and returns the new day. The clock never runs backwards: a negative step is
    /// ignored, so a renewal that already happened stays happened.
    pub fn advance(&mut self, days: i64) -> i64 {
        self.day = self.day.saturating_add(days.max(0));
        self.day
    }

    /// Records a purchase of `product` on the current day and returns its transaction id. A subscription passes its
    /// renewal facts; a one-time purchase passes `None`.
    pub fn buy(&mut self, product: &str, renewal: Option<Renewal>) -> u64 {
        self.ledger.record(product, self.day, renewal)
    }

    /// Refunds or revokes a purchase on the current day. False for an unknown id.
    pub fn refund(&mut self, id: u64) -> bool {
        self.ledger.revoke(id, self.day)
    }

    /// Marks a purchase delivered. False for an unknown id.
    pub fn finish(&mut self, id: u64) -> bool {
        self.ledger.finish(id)
    }

    /// Whether the customer is entitled to `product` on the current day.
    pub fn entitled(&self, product: &str) -> bool {
        self.ledger.is_entitled(product, self.day)
    }

    /// The ids of the purchases the app has not finished, oldest first.
    pub fn unfinished_ids(&self) -> Vec<u64> {
        self.ledger.unfinished().iter().map(|t| t.id).collect()
    }

    /// Clears every recorded purchase, as a test run does before the next scenario. The next purchase gets id 1.
    pub fn clear(&mut self) {
        self.ledger = Ledger::new();
    }

    /// Runs one purchase attempt to success: shows the sheet, records the purchase on the current day, and applies the
    /// result. False if the attempt has already started.
    pub fn approve(&mut self, attempt: &mut Purchase, renewal: Option<Renewal>) -> bool {
        if !attempt.begin() {
            return false;
        }
        let id = self.buy(&attempt.product, renewal);
        attempt.complete(Outcome::Success(id))
    }

    /// Runs one purchase attempt to a customer cancellation. Nothing is recorded. False if the attempt has already started.
    pub fn cancel(&mut self, attempt: &mut Purchase) -> bool {
        if !attempt.begin() {
            return false;
        }
        attempt.complete(Outcome::UserCancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::purchase::{Price, Step};

    #[test]
    fn a_purchase_is_entitled_from_its_day() {
        let mut s = TestSession::new(10);
        let id = s.buy("pro", None);
        assert_eq!(id, 1);
        assert_eq!(s.ledger().get(1).map(|t| t.purchased), Some(10));
        assert!(s.entitled("pro"));
        assert!(!s.entitled("other"));
    }

    #[test]
    fn the_clock_only_moves_forward() {
        let mut s = TestSession::new(10);
        assert_eq!(s.advance(-5), 10);
        assert_eq!(s.advance(3), 13);
        assert_eq!(s.day(), 13);
    }

    #[test]
    fn a_subscription_ends_on_its_expiry_day() {
        let mut s = TestSession::new(10);
        s.buy("monthly", Some(Renewal::new(40)));
        s.advance(29);
        assert_eq!(s.day(), 39);
        assert!(s.entitled("monthly"));
        s.advance(1);
        assert_eq!(s.day(), 40);
        assert!(!s.entitled("monthly"));
    }

    #[test]
    fn a_grace_period_keeps_access_until_its_last_day() {
        let mut s = TestSession::new(40);
        let renewal = Renewal { expires: 40, grace_until: Some(45), billing_retry: false, auto_renew: true };
        s.buy("monthly", Some(renewal));
        assert!(s.entitled("monthly"));
        s.advance(5);
        assert_eq!(s.day(), 45);
        assert!(s.entitled("monthly"));
        s.advance(1);
        assert!(!s.entitled("monthly"));
    }

    #[test]
    fn a_refund_removes_the_entitlement_from_its_day() {
        let mut s = TestSession::new(10);
        let id = s.buy("pro", None);
        s.advance(2);
        assert!(s.refund(id));
        assert_eq!(s.ledger().get(id).and_then(|t| t.revoked), Some(12));
        assert!(!s.entitled("pro"));
        assert!(!s.refund(99));
    }

    #[test]
    fn finished_purchases_leave_the_unfinished_list() {
        let mut s = TestSession::new(1);
        let first = s.buy("a", None);
        let second = s.buy("b", None);
        assert!(s.finish(first));
        assert_eq!(s.unfinished_ids(), vec![second]);
        assert!(!s.finish(99));
    }

    #[test]
    fn clear_forgets_every_purchase_and_restarts_ids() {
        let mut s = TestSession::new(1);
        s.buy("a", None);
        s.buy("b", None);
        s.clear();
        assert!(!s.entitled("a"));
        assert!(s.unfinished_ids().is_empty());
        assert_eq!(s.buy("a", None), 1);
    }

    #[test]
    fn an_approved_attempt_records_a_purchase_and_may_install() {
        let mut s = TestSession::new(7);
        let mut attempt = Purchase::new("pro", Price::new(990, 0, "ISK"));
        assert!(s.approve(&mut attempt, None));
        assert!(attempt.may_install());
        assert_eq!(attempt.outcome(), Some(&Outcome::Success(1)));
        assert!(s.entitled("pro"));
        assert!(!s.approve(&mut attempt, None));
    }

    #[test]
    fn a_cancelled_attempt_records_nothing() {
        let mut s = TestSession::new(7);
        let mut attempt = Purchase::new("pro", Price::new(990, 0, "ISK"));
        assert!(s.cancel(&mut attempt));
        assert_eq!(attempt.step(), &Step::Finished(Outcome::UserCancelled));
        assert!(!attempt.may_install());
        assert!(!s.entitled("pro"));
    }
}
