//! A ledger of the customer's purchases of the app's products: the transaction record, the latest purchase of each product,
//! the purchases that entitle the customer on a day, refunds, and purchases the app has not finished yet. The store records
//! the purchases; this model answers what they entitle. Days count from 1970-01-01. Pure: nothing is persisted here.

use crate::subscription::Renewal;

/// One purchase of a product on a day. A subscription carries its renewal facts; a one-time purchase has none.
#[derive(Clone, Debug, PartialEq)]
pub struct Transaction {
    pub id: u64,
    pub product: String,
    /// The day of the purchase; the purchase entitles nothing before it.
    pub purchased: i64,
    pub renewal: Option<Renewal>,
    /// The earliest day the store refunded or revoked the purchase.
    pub revoked: Option<i64>,
    /// The app has delivered the content and called finish.
    pub finished: bool,
}

impl Transaction {
    /// Whether this purchase entitles the customer on `day`.
    pub fn entitles(&self, day: i64) -> bool {
        if day < self.purchased || self.revoked.is_some_and(|r| r <= day) {
            return false;
        }
        match &self.renewal {
            None => true,
            Some(renewal) => renewal.grants_access(day),
        }
    }
}

/// The purchases recorded for this app's customer, with ids that are never reused.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledger {
    transactions: Vec<Transaction>,
    next_id: u64,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger { transactions: Vec::new(), next_id: 1 }
    }

    /// Records a purchase and returns its id.
    pub fn record(&mut self, product: &str, purchased: i64, renewal: Option<Renewal>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.transactions.push(Transaction {
            id,
            product: product.to_string(),
            purchased,
            renewal,
            revoked: None,
            finished: false,
        });
        id
    }

    pub fn get(&self, id: u64) -> Option<&Transaction> {
        self.transactions.iter().find(|t| t.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Transaction> {
        self.transactions.iter_mut().find(|t| t.id == id)
    }

    /// Records a refund or revocation on `day`. A purchase keeps its earliest revocation day. False for an unknown id.
    pub fn revoke(&mut self, id: u64, day: i64) -> bool {
        match self.get_mut(id) {
            Some(t) => {
                t.revoked = Some(t.revoked.map_or(day, |r| r.min(day)));
                true
            }
            None => false,
        }
    }

    /// Marks the purchase delivered (`finish()`). False for an unknown id.
    pub fn finish(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(t) => {
                t.finished = true;
                true
            }
            None => false,
        }
    }

    /// Every purchase of `product`, newest first (`all(for:)`).
    pub fn all_for(&self, product: &str) -> Vec<&Transaction> {
        let mut v: Vec<&Transaction> = self.transactions.iter().filter(|t| t.product == product).collect();
        v.sort_by(|a, b| (b.purchased, b.id).cmp(&(a.purchased, a.id)));
        v
    }

    /// The most recent purchase of `product` (`latest(for:)`).
    pub fn latest_for(&self, product: &str) -> Option<&Transaction> {
        self.all_for(product).into_iter().next()
    }

    /// The latest purchase of each product that entitles the customer on `day` (`currentEntitlements`). An older purchase
    /// never stands in for a newer one that was refunded or has expired. Products come in name order.
    pub fn current_entitlements(&self, day: i64) -> Vec<&Transaction> {
        let mut products: Vec<&str> = self.transactions.iter().map(|t| t.product.as_str()).collect();
        products.sort_unstable();
        products.dedup();
        products
            .into_iter()
            .filter_map(|p| self.latest_for(p))
            .filter(|t| t.entitles(day))
            .collect()
    }

    /// Whether the customer is entitled to `product` on `day`.
    pub fn is_entitled(&self, product: &str, day: i64) -> bool {
        self.current_entitlements(day).iter().any(|t| t.product == product)
    }

    /// Purchases the app has not finished yet (`unfinished`), oldest first.
    pub fn unfinished(&self) -> Vec<&Transaction> {
        let mut v: Vec<&Transaction> = self.transactions.iter().filter(|t| !t.finished).collect();
        v.sort_by_key(|t| t.id);
        v
    }

    /// Merges the purchase records the store lists for the customer (`AppStore.sync`). A record matches a local purchase
    /// when the product and the purchase day are equal. An unmatched record is added as a new purchase. A matched record
    /// takes the store's renewal facts when it has any, and its revocation day when that day is earlier than the local one.
    /// Local purchases the store does not list are kept.
    pub fn sync(&mut self, remote: &[RemoteRecord]) -> SyncReport {
        let mut report = SyncReport::default();
        for r in remote {
            match self.transactions.iter().position(|t| t.product == r.product && t.purchased == r.purchased) {
                Some(i) => {
                    let t = &mut self.transactions[i];
                    if r.renewal.is_some() {
                        t.renewal = r.renewal;
                    }
                    if let Some(day) = r.revoked {
                        if t.revoked.map_or(true, |local| day < local) {
                            t.revoked = Some(day);
                            report.revoked += 1;
                        }
                    }
                }
                None => {
                    let id = self.record(&r.product, r.purchased, r.renewal);
                    report.added += 1;
                    if let Some(day) = r.revoked {
                        self.revoke(id, day);
                        report.revoked += 1;
                    }
                }
            }
        }
        report
    }
}

/// One purchase record as the store lists it for the customer, to be merged into a ledger by [`Ledger::sync`].
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteRecord {
    pub product: String,
    pub purchased: i64,
    pub renewal: Option<Renewal>,
    pub revoked: Option<i64>,
}

/// What a sync changed: purchases added, and revocation days recorded or moved earlier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub revoked: usize,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_assigns_increasing_ids_that_are_not_reused() {
        let mut l = Ledger::new();
        assert_eq!(l.record("pro", 1, None), 1);
        assert_eq!(l.record("theme", 2, None), 2);
        assert_eq!(l.get(2).map(|t| t.product.as_str()), Some("theme"));
        assert!(l.get(3).is_none());
    }

    #[test]
    fn a_one_time_purchase_entitles_from_its_day() {
        let mut l = Ledger::new();
        l.record("pro", 10, None);
        assert!(!l.is_entitled("pro", 9));
        assert!(l.is_entitled("pro", 10));
        assert!(l.is_entitled("pro", 5000));
    }

    #[test]
    fn current_entitlements_keeps_only_the_latest_purchase_of_each_product() {
        let mut l = Ledger::new();
        l.record("pro", 10, Some(Renewal::new(40)));
        l.record("pro", 50, Some(Renewal::new(80)));
        l.record("theme", 20, None);

        let now = l.current_entitlements(55);
        let names: Vec<&str> = now.iter().map(|t| t.product.as_str()).collect();
        assert_eq!(names, vec!["pro", "theme"]);
        assert_eq!(now[0].purchased, 50, "the renewed purchase stands for the product");

        assert!(!l.is_entitled("pro", 90), "the latest pro subscription has expired");
        assert!(l.is_entitled("theme", 90));
    }

    #[test]
    fn a_revocation_ends_the_entitlement_from_its_day_and_the_first_one_stands() {
        let mut l = Ledger::new();
        let id = l.record("pro", 1, None);
        assert!(l.revoke(id, 30));
        assert!(l.revoke(id, 40), "a later revocation is accepted but does not move the day");
        assert!(l.is_entitled("pro", 29));
        assert!(!l.is_entitled("pro", 30));
        assert!(!l.is_entitled("pro", 35));
        assert!(!l.revoke(99, 30), "an unknown id is refused");
    }

    #[test]
    fn a_grace_period_keeps_access_after_the_paid_period() {
        let mut l = Ledger::new();
        let grace = Renewal { grace_until: Some(105), ..Renewal::new(100) };
        l.record("pro", 1, Some(grace));
        assert!(l.is_entitled("pro", 102));
        assert!(!l.is_entitled("pro", 106));
    }

    #[test]
    fn finish_marks_one_purchase_and_unfinished_lists_the_rest() {
        let mut l = Ledger::new();
        let a = l.record("pro", 1, None);
        l.record("theme", 2, None);
        assert!(l.finish(a));
        assert!(!l.finish(9));
        let ids: Vec<u64> = l.unfinished().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2]);
    }

    #[test]
    fn all_for_is_newest_first_and_latest_for_is_its_first_entry() {
        let mut l = Ledger::new();
        let first = l.record("pro", 1, None);
        let newest = l.record("pro", 5, None);
        let middle = l.record("pro", 3, None);
        let ids: Vec<u64> = l.all_for("pro").iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![newest, middle, first]);
        assert_eq!(l.latest_for("pro").map(|t| t.id), Some(newest));
        assert!(l.latest_for("missing").is_none());
    }

    fn remote(product: &str, purchased: i64, renewal: Option<Renewal>, revoked: Option<i64>) -> RemoteRecord {
        RemoteRecord { product: product.to_string(), purchased, renewal, revoked }
    }

    #[test]
    fn sync_adds_unknown_records_and_keeps_local_purchases_the_store_does_not_list() {
        let mut l = Ledger::new();
        l.record("theme", 2, None);
        let report = l.sync(&[remote("pro", 5, None, None)]);
        assert_eq!(report, SyncReport { added: 1, revoked: 0 });
        assert!(l.is_entitled("pro", 6));
        assert!(l.is_entitled("theme", 6), "a purchase the store does not list is kept");
    }

    #[test]
    fn sync_matches_by_product_and_day_so_a_second_sync_adds_nothing() {
        let mut l = Ledger::new();
        let records = [remote("pro", 5, None, None)];
        l.sync(&records);
        let again = l.sync(&records);
        assert_eq!(again, SyncReport::default());
        assert_eq!(l.all_for("pro").len(), 1);
    }

    #[test]
    fn sync_takes_the_store_revocation_only_when_it_is_earlier() {
        let mut l = Ledger::new();
        let id = l.record("pro", 5, None);
        l.revoke(id, 20);
        let earlier = l.sync(&[remote("pro", 5, None, Some(10))]);
        assert_eq!(earlier.revoked, 1);
        assert!(!l.is_entitled("pro", 12));
        let later = l.sync(&[remote("pro", 5, None, Some(30))]);
        assert_eq!(later.revoked, 0, "a later store revocation does not move the local day");
        assert!(l.is_entitled("pro", 9));
    }

    #[test]
    fn sync_refreshes_the_renewal_of_a_matched_subscription() {
        let mut l = Ledger::new();
        l.record("pro", 5, Some(Renewal::new(30)));
        l.sync(&[remote("pro", 5, Some(Renewal::new(60)), None)]);
        assert!(l.is_entitled("pro", 45), "the renewed period from the store is used");
    }
}
