//! The purchase flow of a product (`Product.purchase(options:)` and `Product.PurchaseResult`): the price the customer sees,
//! the result of asking the store to sell a product, and the steps an app goes through while the store's confirmation
//! sheet is up. The store shows the sheet and charges the customer; this model only tracks what the app has been told.
//! A paid package may be installed only after a successful purchase. Pure: nothing is charged or stored here.

/// A price in minor units (cents, öre, krónur) with the number of fraction digits its currency uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Price {
    pub minor: i64,
    /// The digits after the decimal point: 2 for EUR or USD, 0 for ISK. Clamped to 0..=6 so the scale fits in `u64`.
    pub fraction_digits: u32,
    /// The ISO 4217 currency code, e.g. "ISK".
    pub currency: String,
}

impl Price {
    pub fn new(minor: i64, fraction_digits: u32, currency: &str) -> Self {
        Price { minor, fraction_digits: fraction_digits.min(6), currency: currency.to_string() }
    }

    /// The price as text, e.g. "12.50 EUR" or "990 ISK". A negative price keeps its sign.
    pub fn display(&self) -> String {
        let scale = 10u64.pow(self.fraction_digits);
        let abs = self.minor.unsigned_abs();
        let mut out = String::new();
        if self.minor < 0 {
            out.push('-');
        }
        out.push_str(&(abs / scale).to_string());
        if self.fraction_digits > 0 {
            let width = self.fraction_digits as usize;
            out.push_str(&format!(".{:0width$}", abs % scale, width = width));
        }
        out.push(' ');
        out.push_str(&self.currency);
        out
    }
}

/// The result of a purchase (`Product.PurchaseResult`). `Success` carries the id of the ledger transaction the store
/// recorded (see `entitlement::Ledger::record`); a pending purchase arrives later as its own transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success(u64),
    UserCancelled,
    /// The purchase needs action from the customer (for example, a parent's approval). It completes later.
    Pending,
}

/// Why the store could not complete a purchase. These are the errors a store reports besides a cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The device could not reach the store. The customer may try again.
    NetworkUnavailable,
    /// The store is busy or down for maintenance. The customer may try again.
    StoreUnavailable,
    /// The store could not finish for a reason it does not name. The customer may try again.
    Unknown,
    /// The customer is not allowed to buy this product (a restriction on the account).
    NotAllowed,
    /// The product is not sold in this storefront or is no longer on sale.
    ProductUnavailable,
}

impl Failure {
    /// Whether the same purchase may be attempted again: only the transient failures.
    pub fn can_retry(self) -> bool {
        matches!(self, Failure::NetworkUnavailable | Failure::StoreUnavailable | Failure::Unknown)
    }
}

/// Where one purchase attempt stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// The confirmation sheet is up and waiting for the customer.
    ConfirmationShown,
    /// The store reported a pending purchase; a final outcome follows.
    Pending,
    /// The purchase has a final outcome: success or a cancellation.
    Finished(Outcome),
    /// The store could not complete the purchase. It may be retried when the failure allows it.
    Failed(Failure),
}

/// One purchase attempt of a product at a price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub product: String,
    pub price: Price,
    step: Step,
}

impl Purchase {
    pub fn new(product: &str, price: Price) -> Self {
        Purchase { product: product.to_string(), price, step: Step::Idle }
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    /// The final outcome, once the purchase has one.
    pub fn outcome(&self) -> Option<&Outcome> {
        match &self.step {
            Step::Finished(o) => Some(o),
            _ => None,
        }
    }

    /// The failure that ended the attempt, if the store could not complete it.
    pub fn failure(&self) -> Option<Failure> {
        match self.step {
            Step::Failed(f) => Some(f),
            _ => None,
        }
    }

    /// Shows the confirmation sheet (`purchase(options:)` was called). False unless the attempt is idle.
    pub fn begin(&mut self) -> bool {
        if self.step == Step::Idle {
            self.step = Step::ConfirmationShown;
            true
        } else {
            false
        }
    }

    /// Applies the store's result. From the sheet, any outcome is accepted; a pending result moves to `Pending`, and a
    /// later result finishes the purchase. A second pending result is refused, and so is any result before `begin`.
    pub fn complete(&mut self, outcome: Outcome) -> bool {
        let next = match (&self.step, &outcome) {
            (Step::ConfirmationShown, Outcome::Pending) => Step::Pending,
            (Step::Pending, Outcome::Pending) => return false,
            (Step::ConfirmationShown | Step::Pending, _) => Step::Finished(outcome.clone()),
            _ => return false,
        };
        self.step = next;
        true
    }

    /// Records that the store could not complete the purchase. Accepted from the sheet or while pending; a failed
    /// attempt is not reopened by a later result.
    pub fn fail(&mut self, failure: Failure) -> bool {
        if matches!(self.step, Step::ConfirmationShown | Step::Pending) {
            self.step = Step::Failed(failure);
            true
        } else {
            false
        }
    }

    /// Starts the purchase again after a failure that allows a retry. False for any other state, and for failures the
    /// store does not allow the customer to retry.
    pub fn retry(&mut self) -> bool {
        match self.step {
            Step::Failed(f) if f.can_retry() => {
                self.step = Step::Idle;
                true
            }
            _ => false,
        }
    }

    /// Whether the app may install the paid package now: only after a successful purchase.
    pub fn may_install(&self) -> bool {
        matches!(self.step, Step::Finished(Outcome::Success(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_price_shows_its_fraction_digits() {
        assert_eq!(Price::new(1250, 2, "EUR").display(), "12.50 EUR");
        assert_eq!(Price::new(500, 2, "EUR").display(), "5.00 EUR");
        assert_eq!(Price::new(990, 0, "ISK").display(), "990 ISK");
    }

    #[test]
    fn a_negative_price_keeps_its_sign_and_leading_zero() {
        assert_eq!(Price::new(-75, 2, "USD").display(), "-0.75 USD");
    }

    #[test]
    fn fraction_digits_are_clamped_to_six() {
        let p = Price::new(5, 9, "XYZ");
        assert_eq!(p.fraction_digits, 6);
        assert_eq!(p.display(), "0.000005 XYZ");
    }

    #[test]
    fn a_purchase_must_show_the_sheet_before_it_completes() {
        let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
        assert!(!p.complete(Outcome::Success(1)), "no result before the sheet");
        assert!(p.begin());
        assert!(!p.begin(), "the sheet is shown once");
        assert!(p.complete(Outcome::Success(7)));
        assert_eq!(p.outcome(), Some(&Outcome::Success(7)));
        assert!(p.may_install());
    }

    #[test]
    fn a_cancelled_purchase_does_not_allow_the_install() {
        let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
        p.begin();
        assert!(p.complete(Outcome::UserCancelled));
        assert_eq!(p.outcome(), Some(&Outcome::UserCancelled));
        assert!(!p.may_install());
    }

    #[test]
    fn a_pending_purchase_finishes_only_when_its_final_result_arrives() {
        let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
        p.begin();
        assert!(p.complete(Outcome::Pending));
        assert_eq!(*p.step(), Step::Pending);
        assert!(p.outcome().is_none());
        assert!(!p.may_install());
        assert!(!p.complete(Outcome::Pending), "a second pending result is refused");
        assert!(p.complete(Outcome::Success(9)));
        assert!(p.may_install());
    }
}
