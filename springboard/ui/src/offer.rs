//! Subscription offers (`Product.SubscriptionOffer` and its `OfferType`): an introductory, promotional or win-back offer
//! configured for an auto-renewable subscription, with its discounted price, period and number of periods. The model
//! decides which offer a customer may be shown, from two facts the app knows: whether the customer has subscribed in the
//! subscription group, and the day the last subscription lapsed. The store keeps the offers and signs promotional ones
//! on its own servers; this module does not sign anything. Days count from 1970-01-01. Pure.

use crate::purchase::Price;
use crate::subscription::Period;

/// The kind of offer (`OfferType`). A win-back offer is shown only within `within_days` of the day the customer's last
/// subscription lapsed. The window length is BifrOSt's model field, set by the app from its own product plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// For customers who have not subscribed in the group before.
    Introductory,
    /// Offered to any customer; the store signs it, so it is shown only when the store presents it.
    Promotional,
    /// For customers whose subscription lapsed recently.
    WinBack { within_days: i64 },
}

/// One offer of a subscription: its identifier, kind, discounted price per period, and how long it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub id: String,
    pub kind: Kind,
    pub price: Price,
    pub period: Period,
    /// The number of periods the discounted price applies to. At least 1.
    pub periods: u32,
}

impl Offer {
    pub fn new(id: &str, kind: Kind, price: Price, period: Period, periods: u32) -> Self {
        Offer { id: id.to_string(), kind, price, period, periods: periods.max(1) }
    }

    /// The day the discounted periods end, when the offer starts on `start` (the first day of its first period).
    pub fn ends_on(&self, start: i64) -> i64 {
        (0..self.periods).fold(start, |day, _| self.period.add_to(day))
    }

    /// Whether this offer may be shown to `customer` on `today`.
    pub fn applies(&self, customer: &Customer, today: i64) -> bool {
        match self.kind {
            Kind::Introductory => !customer.subscribed_in_group,
            Kind::Promotional => true,
            Kind::WinBack { within_days } => match customer.lapsed_on {
                Some(lapsed) => today >= lapsed && today - lapsed <= within_days,
                None => false,
            },
        }
    }
}

/// What the app knows about the customer for the subscription group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Customer {
    /// The customer has held a subscription in the group before (any state).
    pub subscribed_in_group: bool,
    /// The day the customer's last subscription in the group ended, if it has ended.
    pub lapsed_on: Option<i64>,
}

/// The first offer in `offers` that applies to `customer` on `today`. The app lists its offers in the order it prefers;
/// this model does not rank them.
pub fn choose<'a>(offers: &'a [Offer], customer: &Customer, today: i64) -> Option<&'a Offer> {
    offers.iter().find(|o| o.applies(customer, today))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::Unit;

    fn price() -> Price {
        Price::new(300, 2, "EUR")
    }

    #[test]
    fn an_offer_of_three_weekly_periods_ends_twenty_one_days_after_its_start() {
        let offer = Offer::new("trial", Kind::Introductory, price(), Period::new(Unit::Day, 7), 3);
        assert_eq!(offer.ends_on(10), 31);
    }

    #[test]
    fn an_introductory_offer_is_only_for_a_customer_new_to_the_group() {
        let offer = Offer::new("intro", Kind::Introductory, price(), Period::new(Unit::Month, 1), 1);
        let new = Customer { subscribed_in_group: false, lapsed_on: None };
        let returning = Customer { subscribed_in_group: true, lapsed_on: Some(100) };
        assert!(offer.applies(&new, 110));
        assert!(!offer.applies(&returning, 110));
    }

    #[test]
    fn a_promotional_offer_applies_to_any_customer() {
        let offer = Offer::new("promo", Kind::Promotional, price(), Period::new(Unit::Year, 1), 1);
        let returning = Customer { subscribed_in_group: true, lapsed_on: None };
        assert!(offer.applies(&returning, 5));
        assert!(offer.applies(&Customer::default(), 5));
    }

    #[test]
    fn a_win_back_offer_is_shown_only_inside_its_window_after_the_lapse() {
        let offer = Offer::new("back", Kind::WinBack { within_days: 30 }, price(), Period::new(Unit::Month, 1), 2);
        let lapsed = Customer { subscribed_in_group: true, lapsed_on: Some(100) };
        assert!(offer.applies(&lapsed, 130), "the last day of the window");
        assert!(!offer.applies(&lapsed, 131), "one day after the window");
        assert!(!offer.applies(&lapsed, 99), "before the lapse there is nothing to win back");
        assert!(!offer.applies(&Customer { subscribed_in_group: false, lapsed_on: None }, 110));
    }

    #[test]
    fn choose_returns_the_first_offer_in_the_app_order_that_applies() {
        let intro = Offer::new("intro", Kind::Introductory, price(), Period::new(Unit::Month, 1), 1);
        let back = Offer::new("back", Kind::WinBack { within_days: 30 }, price(), Period::new(Unit::Month, 1), 1);
        let offers = vec![intro, back];

        let lapsed = Customer { subscribed_in_group: true, lapsed_on: Some(100) };
        assert_eq!(choose(&offers, &lapsed, 110).map(|o| o.id.as_str()), Some("back"));

        let new = Customer { subscribed_in_group: false, lapsed_on: None };
        assert_eq!(choose(&offers, &new, 110).map(|o| o.id.as_str()), Some("intro"));

        let old = Customer { subscribed_in_group: true, lapsed_on: Some(10) };
        assert!(choose(&offers, &old, 110).is_none());
    }
}
