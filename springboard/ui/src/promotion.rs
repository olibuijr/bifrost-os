//! Promoted purchases (`PurchaseIntent` and `Product.PromotionInfo`): the store can show a product on its own page, and a
//! customer who taps the promotion sends the app a purchase intent. The app then runs its normal purchase flow (see
//! `purchase`) for that product. This module is the app-side queue of those intents and the order in which a store
//! lists its promoted products. It does not show anything and does not charge anyone. Pure.
//!
//! Rules, chosen by BifrOSt and not taken from a store document:
//! - intents are handled first in, first out;
//! - a product that already waits in the queue is not added twice;
//! - at most `MAX_PENDING` intents wait at once; a new one past that is refused, so a flood cannot grow the queue.

use std::collections::VecDeque;

/// The most intents that may wait at once.
pub const MAX_PENDING: usize = 16;

/// One intent: the customer wants to buy this product, sent from a promotion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseIntent {
    pub product: String,
}

/// The app's queue of purchase intents that have not been handled yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PurchaseIntents {
    queue: VecDeque<String>,
}

impl PurchaseIntents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues an intent for `product`. False when the id is empty, the product already waits, or the queue is full.
    pub fn deliver(&mut self, product: &str) -> bool {
        if product.is_empty() || self.queue.len() >= MAX_PENDING || self.queue.iter().any(|p| p == product) {
            return false;
        }
        self.queue.push_back(product.to_string());
        true
    }

    /// The oldest waiting intent, removed from the queue.
    pub fn next(&mut self) -> Option<PurchaseIntent> {
        self.queue.pop_front().map(|product| PurchaseIntent { product })
    }

    /// The oldest waiting intent, left in the queue.
    pub fn peek(&self) -> Option<&str> {
        self.queue.front().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// A product the store promotes, with its place in the store's list (lower comes first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promotion {
    pub product: String,
    pub order: u32,
}

/// The promoted product ids in display order. Ties keep the input order, and a product listed twice keeps its first
/// place only.
pub fn in_display_order(list: &[Promotion]) -> Vec<&str> {
    let mut sorted: Vec<&Promotion> = list.iter().collect();
    sorted.sort_by_key(|p| p.order);
    let mut seen: Vec<&str> = Vec::new();
    for p in sorted {
        if !seen.contains(&p.product.as_str()) {
            seen.push(&p.product);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promo(product: &str, order: u32) -> Promotion {
        Promotion { product: product.to_string(), order }
    }

    #[test]
    fn intents_come_out_first_in_first_out() {
        let mut q = PurchaseIntents::new();
        assert!(q.deliver("pro"));
        assert!(q.deliver("tip"));
        assert_eq!(q.peek(), Some("pro"));
        assert_eq!(q.next(), Some(PurchaseIntent { product: "pro".into() }));
        assert_eq!(q.next(), Some(PurchaseIntent { product: "tip".into() }));
        assert_eq!(q.next(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn an_empty_product_id_is_refused() {
        let mut q = PurchaseIntents::new();
        assert!(!q.deliver(""));
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn a_product_that_already_waits_is_not_queued_twice() {
        let mut q = PurchaseIntents::new();
        assert!(q.deliver("pro"));
        assert!(!q.deliver("pro"));
        assert_eq!(q.len(), 1);
        q.next();
        assert!(q.deliver("pro"), "once handled, the same product may be promoted again");
    }

    #[test]
    fn the_queue_stops_at_its_limit() {
        let mut q = PurchaseIntents::new();
        for i in 0..MAX_PENDING {
            assert!(q.deliver(&format!("p{i}")));
        }
        assert!(!q.deliver("one-too-many"));
        assert_eq!(q.len(), MAX_PENDING);
        q.next();
        assert!(q.deliver("one-too-many"), "a handled intent frees a place");
    }

    #[test]
    fn promoted_products_list_by_order_and_keep_the_first_of_a_duplicate() {
        let list = [promo("b", 2), promo("a", 1), promo("c", 2), promo("a", 5)];
        assert_eq!(in_display_order(&list), vec!["a", "b", "c"]);
    }

    #[test]
    fn an_empty_promotion_list_gives_nothing() {
        assert!(in_display_order(&[]).is_empty());
    }
}
