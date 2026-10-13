//! Message categories (View > Category): the list can show only the personal mail, the receipts and confirmations, the
//! updates from services, or the promotions. A message is put in a category from its headers alone, so it works before
//! the text is loaded and needs no server: the bulk-mail headers (`List-Unsubscribe`, `List-Id`, `Precedence`,
//! `Auto-Submitted`), the sender's address and the subject.

use crate::message::{address_of, Summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
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

/// Words in a subject that always mark a receipt, a shipment or a security message.
const SURE_TRANSACTION_WORDS: [&str; 8] =
    ["receipt", "invoice", "order confirmation", "verification code", "password reset", "statement", "shipped", "tracking"];

/// Words that mark a transaction unless a bulk mailing uses them to sell something.
const TRANSACTION_WORDS: [&str; 8] = ["payment", "delivery", "booking", "reservation", "confirmation", "ticket", "your order", "order #"];

/// Words in a subject that mark an offer.
const PROMOTION_WORDS: [&str; 10] = ["% off", "sale", "discount", "deal", "offer", "coupon", "free shipping", "limited time", "save up to", "newsletter"];

/// Local parts of a sender's address that belong to a machine rather than a person.
const AUTOMATIC_SENDERS: [&str; 9] = ["noreply", "no-reply", "donotreply", "do-not-reply", "notifications", "notification", "alerts", "updates", "mailer-daemon"];

/// Local parts that belong to marketing.
const MARKETING_SENDERS: [&str; 7] = ["newsletter", "news", "marketing", "promo", "promotions", "deals", "offers"];

fn local_part(summary: &Summary) -> String {
    address_of(&summary.from).split('@').next().unwrap_or("").to_lowercase()
}

/// The category of a message, from its headers.
pub fn classify(summary: &Summary) -> Category {
    let subject = summary.subject.to_lowercase();
    let sender = local_part(summary);
    let bulk = !summary.list_unsubscribe.is_empty() || !summary.list_id.is_empty() || matches!(summary.precedence.as_str(), "bulk" | "list" | "junk");
    let selling = bulk && PROMOTION_WORDS.iter().any(|w| subject.contains(w));
    if SURE_TRANSACTION_WORDS.iter().any(|w| subject.contains(w)) || (!selling && TRANSACTION_WORDS.iter().any(|w| subject.contains(w))) {
        return Category::Transactions;
    }
    if bulk && (MARKETING_SENDERS.iter().any(|s| sender.contains(s)) || PROMOTION_WORDS.iter().any(|w| subject.contains(w))) {
        return Category::Promotions;
    }
    if bulk || summary.automatic || AUTOMATIC_SENDERS.iter().any(|s| sender.contains(s)) {
        return Category::Updates;
    }
    Category::Primary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(headers: &str) -> Category {
        classify(&Summary::from_headers(&format!("{headers}\r\n\r\n")))
    }

    #[test]
    fn a_letter_from_a_person_is_primary() {
        assert_eq!(category("From: Ana <ana@example.com>\r\nSubject: Lunch tomorrow?"), Category::Primary);
        assert_eq!(category("From: ana@example.com\r\nSubject: Sale of the old car"), Category::Primary, "a word alone is not bulk mail");
    }

    #[test]
    fn receipts_and_bookings_are_transactions_even_from_a_machine() {
        assert_eq!(category("From: shop@store.example\r\nSubject: Your order #1234 has shipped"), Category::Transactions);
        assert_eq!(category("From: noreply@bank.example\r\nSubject: Payment received\r\nAuto-Submitted: auto-generated"), Category::Transactions);
        assert_eq!(category("From: info@air.example\r\nSubject: Booking confirmation"), Category::Transactions);
    }

    #[test]
    fn bulk_mail_with_an_offer_is_a_promotion() {
        assert_eq!(category("From: news@shop.example\r\nSubject: Hello\r\nList-Unsubscribe: <mailto:u@shop.example>"), Category::Promotions);
        assert_eq!(category("From: shop@shop.example\r\nSubject: 20% off everything\r\nPrecedence: bulk"), Category::Promotions);
        assert_eq!(
            category("From: shop@shop.example\r\nSubject: Receipt for the sale\r\nList-Unsubscribe: <u>"),
            Category::Transactions,
            "a receipt stays a receipt"
        );
    }

    #[test]
    fn lists_and_automatic_mail_are_updates() {
        assert_eq!(category("From: dev@lists.example\r\nSubject: [project] new release\r\nList-Id: <project.lists.example>"), Category::Updates);
        assert_eq!(category("From: notifications@forge.example\r\nSubject: Pull request merged"), Category::Updates);
        assert_eq!(category("From: ci@forge.example\r\nSubject: Build failed\r\nAuto-Submitted: auto-generated"), Category::Updates);
    }

    #[test]
    fn a_promotion_and_a_transaction_are_not_confused_by_a_mixed_subject() {
        assert_eq!(category("From: marketing@shop.example\r\nSubject: Free shipping on your next order\r\nList-Unsubscribe: <u>"), Category::Promotions);
    }
}
