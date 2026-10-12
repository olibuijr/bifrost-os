//! The kind of a product sold through the store (`Product.ProductType`): a consumable that can be bought again and again,
//! a one-time non-consumable, a non-renewing subscription that lasts a fixed time, or an auto-renewable subscription. The
//! catalogue names the kind in a `product_type` field; the kind decides which purchase rules and entitlement rules apply
//! (see `entitlement` and `subscription`). Pure: nothing is sold here.

/// The product kinds, with the catalogue spelling each one accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductType {
    /// Used up by the customer and bought again (coins, credits). Never entitles the customer to something lasting.
    Consumable,
    /// Bought once and kept for good (a feature unlock).
    NonConsumable,
    /// Bought for a fixed time and then lapses; it does not renew by itself.
    NonRenewable,
    /// Renews on its own every period until the customer cancels.
    AutoRenewable,
}

impl ProductType {
    /// Reads a catalogue value. Accepts `consumable`, `non_consumable`, `non_renewable` and `auto_renewable`, with a dash
    /// or a space in place of an underscore and any letter case; anything else gives `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let norm: String = text.trim().chars().map(|c| if c == '-' || c == ' ' { '_' } else { c.to_ascii_lowercase() }).collect();
        match norm.as_str() {
            "consumable" => Some(ProductType::Consumable),
            "non_consumable" => Some(ProductType::NonConsumable),
            "non_renewable" => Some(ProductType::NonRenewable),
            "auto_renewable" => Some(ProductType::AutoRenewable),
            _ => None,
        }
    }

    /// The catalogue spelling of this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            ProductType::Consumable => "consumable",
            ProductType::NonConsumable => "non_consumable",
            ProductType::NonRenewable => "non_renewable",
            ProductType::AutoRenewable => "auto_renewable",
        }
    }

    /// Whether the customer may buy this product again after using it up.
    pub fn is_repeatable(self) -> bool {
        self == ProductType::Consumable
    }

    /// Whether the product is a subscription, which is tracked by period rather than by a one-time entitlement.
    pub fn is_subscription(self) -> bool {
        matches!(self, ProductType::NonRenewable | ProductType::AutoRenewable)
    }

    /// Whether an entitlement from this product can be restored on a new device. Consumables are never restored: they
    /// were used up when they were bought.
    pub fn is_restorable(self) -> bool {
        self != ProductType::Consumable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_parses_from_its_own_spelling() {
        for t in [ProductType::Consumable, ProductType::NonConsumable, ProductType::NonRenewable, ProductType::AutoRenewable] {
            assert_eq!(ProductType::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn dashes_spaces_and_case_are_accepted() {
        assert_eq!(ProductType::parse(" Auto-Renewable "), Some(ProductType::AutoRenewable));
        assert_eq!(ProductType::parse("NON CONSUMABLE"), Some(ProductType::NonConsumable));
    }

    #[test]
    fn unknown_kinds_are_refused() {
        assert_eq!(ProductType::parse(""), None);
        assert_eq!(ProductType::parse("subscription"), None);
        assert_eq!(ProductType::parse("auto_renewable_plus"), None);
    }

    #[test]
    fn only_consumables_repeat_and_only_they_are_not_restored() {
        assert!(ProductType::Consumable.is_repeatable());
        assert!(!ProductType::Consumable.is_restorable());
        assert!(!ProductType::NonConsumable.is_repeatable());
        assert!(ProductType::NonConsumable.is_restorable());
        assert!(ProductType::AutoRenewable.is_restorable());
    }

    #[test]
    fn subscriptions_are_the_two_period_kinds() {
        assert!(ProductType::NonRenewable.is_subscription());
        assert!(ProductType::AutoRenewable.is_subscription());
        assert!(!ProductType::NonConsumable.is_subscription());
        assert!(!ProductType::Consumable.is_subscription());
    }
}
