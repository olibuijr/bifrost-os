//! In-app products listed from the PC App Store catalogue: the entries that declare a product kind (consumable,
//! non-consumable, non-renewable or auto-renewable), with their price, whether the customer already owns them and what
//! the store can offer. The PC App Store takes no payment yet, so a paid product is listed with its price and no
//! purchase action (`Offer::Unavailable`). Pure: the caller passes the catalogue and the entitlement ledger.

use super::api::StoreApp;
use bifrost_ui::entitlement::Ledger;
use bifrost_ui::product::ProductType;

/// What the store offers for a product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// The customer already owns it on this day.
    Owned,
    /// No price: the customer may take it.
    Free,
    /// Priced, and the store cannot take a payment yet.
    Unavailable,
}

/// One row of the product list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    /// "Free" or the price as the customer sees it.
    pub price: String,
    pub product_type: ProductType,
    pub subscription: bool,
    pub restorable: bool,
    pub offer: Offer,
}

/// The products in the catalogue, in catalogue order. Entries without a product kind are not products and are left
/// out. `day` is the day the entitlements are read on, counted from 1970-01-01.
pub fn listing(catalog: &[StoreApp], ledger: &Ledger, day: i64) -> Vec<Listing> {
    catalog
        .iter()
        .filter_map(|app| {
            let product_type = app.product_type?;
            let offer = if ledger.is_entitled(&app.id, day) {
                Offer::Owned
            } else if app.is_paid() {
                Offer::Unavailable
            } else {
                Offer::Free
            };
            Some(Listing {
                id: app.id.clone(),
                name: app.name.clone(),
                price: app.price_text(),
                product_type,
                subscription: product_type.is_subscription(),
                restorable: product_type.is_restorable(),
                offer,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::api::parse_catalog;
    use super::*;

    const SUM: &str = "abababababababababababababababababababababababababababababababab";

    fn catalog() -> Vec<StoreApp> {
        let text = format!(
            r#"{{"apps":[
              {{"id":"pro.unlock","name":"Pro","version":"1.0.0","sha256":"{SUM}","package":"/a","product_type":"non-consumable","price":1250,"currency":"EUR"}},
              {{"id":"pro.free","name":"Free Pack","version":"1.0.0","sha256":"{SUM}","package":"/b","product_type":"consumable"}},
              {{"id":"plain","name":"Plain","version":"1.0.0","sha256":"{SUM}","package":"/c"}},
              {{"id":"pro.sub","name":"Sub","version":"1.0.0","sha256":"{SUM}","package":"/d","product_type":"auto-renewable","price":990,"currency":"EUR"}}
            ]}}"#
        );
        parse_catalog(&text).expect("a catalogue")
    }

    #[test]
    fn only_entries_with_a_product_kind_are_listed_in_catalogue_order() {
        let rows = listing(&catalog(), &Ledger::new(), 100);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["pro.unlock", "pro.free", "pro.sub"]);
    }

    #[test]
    fn a_priced_product_is_listed_with_its_price_and_no_purchase_action() {
        let rows = listing(&catalog(), &Ledger::new(), 100);
        assert_eq!(rows[0].price, "12.50 EUR");
        assert_eq!(rows[0].offer, Offer::Unavailable, "no payments on the PC App Store yet");
        assert!(rows[0].restorable);
        assert!(!rows[0].subscription);
    }

    #[test]
    fn an_unpriced_product_is_free_and_a_subscription_says_so() {
        let rows = listing(&catalog(), &Ledger::new(), 100);
        assert_eq!(rows[1].offer, Offer::Free);
        assert_eq!(rows[1].price, "Free");
        assert!(rows[2].subscription);
    }

    #[test]
    fn an_owned_product_is_marked_owned_on_and_after_its_purchase_day() {
        let mut ledger = Ledger::new();
        ledger.record("pro.unlock", 50, None);
        let rows = listing(&catalog(), &ledger, 100);
        assert_eq!(rows[0].offer, Offer::Owned);
        let before = listing(&catalog(), &ledger, 10);
        assert_eq!(before[0].offer, Offer::Unavailable, "not owned before the purchase day");
    }
}
