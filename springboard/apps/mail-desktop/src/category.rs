//! Categories (View > Category). A message is put in one of four groups from its headers and, when those say nothing,
//! from its sender and subject: Primary (mail from people), Transactions (receipts, orders, bills), Updates (automatic
//! notices and mailing lists) and Promotions (bulk mail that sells something).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    Primary,
    Transactions,
    Updates,
    Promotions,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Primary, Category::Transactions, Category::Updates, Category::Promotions];

    pub fn label(self) -> &'static str {
        match self {
            Category::Primary => "Primary",
            Category::Transactions => "Transactions",
            Category::Updates => "Updates",
            Category::Promotions => "Promotions",
        }
    }
}

const TRANSACTION_WORDS: [&str; 12] = [
    "receipt", "invoice", "order", "payment", "confirmation", "shipped", "delivery", "your bill", "statement", "booking", "reservation", "ticket",
];
const PROMOTION_WORDS: [&str; 10] = ["sale", "% off", "discount", "offer", "deal", "coupon", "free shipping", "limited time", "newsletter", "save "];
const AUTOMATED_SENDERS: [&str; 7] = ["noreply", "no-reply", "donotreply", "do-not-reply", "notifications@", "notification@", "mailer-daemon"];

/// The category of a message. `headers` are the parsed header pairs with lower-case names (see
/// [`crate::message::parse_headers`]).
pub fn classify(headers: &[(String, String)], from: &str, subject: &str) -> Category {
    let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.to_lowercase());
    let subject = subject.to_lowercase();
    let from = from.to_lowercase();
    let has_any = |text: &str, words: &[&str]| words.iter().any(|w| text.contains(w));
    let bulk = header("precedence").is_some_and(|p| p.contains("bulk") || p.contains("list") || p.contains("junk"));
    let listed = header("list-id").is_some() || header("list-unsubscribe").is_some();
    let automatic = header("auto-submitted").is_some_and(|v| v != "no") || has_any(&from, &AUTOMATED_SENDERS);
    if has_any(&subject, &TRANSACTION_WORDS) && (automatic || listed || bulk) {
        return Category::Transactions;
    }
    if (listed || bulk) && has_any(&subject, &PROMOTION_WORDS) {
        return Category::Promotions;
    }
    if listed || bulk || automatic {
        return Category::Updates;
    }
    Category::Primary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
    }

    #[test]
    fn mail_from_a_person_is_primary() {
        assert_eq!(classify(&headers(&[]), "Ana <ana@example.com>", "Lunch?"), Category::Primary);
    }

    #[test]
    fn automatic_receipts_are_transactions() {
        assert_eq!(classify(&headers(&[]), "Shop <noreply@shop.example>", "Your order has shipped"), Category::Transactions);
        assert_eq!(classify(&headers(&[("auto-submitted", "auto-generated")]), "x@y.z", "Invoice 12"), Category::Transactions);
    }

    #[test]
    fn a_person_writing_about_an_order_stays_primary() {
        assert_eq!(classify(&headers(&[]), "Bo <bo@example.com>", "About the order"), Category::Primary);
    }

    #[test]
    fn bulk_mail_that_sells_is_a_promotion_and_other_lists_are_updates() {
        let list = headers(&[("list-unsubscribe", "<mailto:u@x>")]);
        assert_eq!(classify(&list, "Shop <hi@shop.example>", "50% off everything"), Category::Promotions);
        assert_eq!(classify(&list, "Club <club@x.example>", "Meeting notes"), Category::Updates);
        assert_eq!(classify(&headers(&[("precedence", "bulk")]), "a@b.c", "Hello"), Category::Updates);
    }

    #[test]
    fn notification_senders_are_updates() {
        assert_eq!(classify(&headers(&[]), "Bot <notifications@forge.example>", "New comment"), Category::Updates);
    }
}
