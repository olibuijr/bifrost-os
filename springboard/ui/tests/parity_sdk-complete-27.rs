//! Parity sdk-complete-27: headless tests of the store purchase flow (store failures and retries) and of subscription
//! groups (tier changes between products of one service). Pure state and ordering: no store, no network, no clock.

use bifrost_ui::purchase::{Failure, Outcome, Price, Purchase, Step};
use bifrost_ui::subscription::{Change, Group};

fn pro_purchase() -> Purchase {
    Purchase::new("pro", Price::new(1000, 2, "EUR"))
}

#[test]
fn a_store_failure_ends_the_attempt_without_an_outcome() {
    let mut p = pro_purchase();
    p.begin();
    assert!(p.fail(Failure::NetworkUnavailable));
    assert_eq!(*p.step(), Step::Failed(Failure::NetworkUnavailable));
    assert_eq!(p.failure(), Some(Failure::NetworkUnavailable));
    assert!(p.outcome().is_none(), "a failure is not a cancellation or a success");
    assert!(!p.may_install());
}

#[test]
fn a_failure_is_refused_before_the_sheet_and_after_a_final_result() {
    let mut p = pro_purchase();
    assert!(!p.fail(Failure::Unknown), "no failure before the sheet is shown");
    p.begin();
    assert!(p.complete(Outcome::Success(3)));
    assert!(!p.fail(Failure::Unknown), "a finished purchase does not fail");
    assert_eq!(p.outcome(), Some(&Outcome::Success(3)));
}

#[test]
fn a_failure_can_arrive_while_a_purchase_is_pending() {
    let mut p = pro_purchase();
    p.begin();
    assert!(p.complete(Outcome::Pending));
    assert!(p.fail(Failure::StoreUnavailable));
    assert_eq!(p.failure(), Some(Failure::StoreUnavailable));
}

#[test]
fn transient_failures_can_be_retried_and_the_retry_starts_from_idle() {
    let mut p = pro_purchase();
    p.begin();
    p.fail(Failure::StoreUnavailable);
    assert!(p.retry());
    assert_eq!(*p.step(), Step::Idle);
    assert!(p.begin(), "the retry shows the confirmation sheet again");
    assert!(p.complete(Outcome::Success(12)));
    assert!(p.may_install());
}

#[test]
fn a_restriction_or_an_unsold_product_cannot_be_retried() {
    for failure in [Failure::NotAllowed, Failure::ProductUnavailable] {
        let mut p = pro_purchase();
        p.begin();
        p.fail(failure);
        assert!(!p.retry(), "{failure:?} is not retried");
        assert_eq!(*p.step(), Step::Failed(failure));
    }
}

#[test]
fn retry_is_refused_unless_the_attempt_has_failed() {
    let mut p = pro_purchase();
    assert!(!p.retry(), "an idle attempt has nothing to retry");
    p.begin();
    assert!(!p.retry(), "a shown sheet is not a failure");
}

#[test]
fn the_failure_kinds_split_into_retryable_and_final() {
    assert!(Failure::NetworkUnavailable.can_retry());
    assert!(Failure::StoreUnavailable.can_retry());
    assert!(Failure::Unknown.can_retry());
    assert!(!Failure::NotAllowed.can_retry());
    assert!(!Failure::ProductUnavailable.can_retry());
}

#[test]
fn a_group_orders_its_products_by_level() {
    let mut g = Group::new();
    assert!(g.add("plus", 2));
    assert!(g.add("basic", 1));
    assert!(g.add("max", 3));
    assert_eq!(g.products(), vec!["basic", "plus", "max"]);
    assert_eq!(g.level_of("max"), Some(3));
    assert_eq!(g.level_of("other"), None);
}

#[test]
fn a_group_refuses_a_second_entry_for_a_product_or_a_level() {
    let mut g = Group::new();
    assert!(g.add("basic", 1));
    assert!(!g.add("basic", 5), "a product appears once");
    assert!(!g.add("plus", 1), "a level names one product");
    assert_eq!(g.products(), vec!["basic"]);
}

#[test]
fn a_move_to_a_higher_tier_is_an_upgrade_that_takes_effect_now() {
    let mut g = Group::new();
    g.add("basic", 1);
    g.add("plus", 2);
    let change = g.change("basic", "plus").unwrap();
    assert_eq!(change, Change::Upgrade);
    assert!(change.takes_effect_now());
}

#[test]
fn a_move_to_a_lower_tier_is_a_downgrade_that_waits_for_renewal() {
    let mut g = Group::new();
    g.add("basic", 1);
    g.add("plus", 2);
    let change = g.change("plus", "basic").unwrap();
    assert_eq!(change, Change::Downgrade);
    assert!(!change.takes_effect_now());
}

#[test]
fn the_same_product_is_no_change_and_outside_products_have_no_change() {
    let mut g = Group::new();
    g.add("basic", 1);
    assert_eq!(g.change("basic", "basic"), Some(Change::Same));
    assert!(Change::Same.takes_effect_now());
    assert_eq!(g.change("basic", "elsewhere"), None);
    assert_eq!(g.change("elsewhere", "basic"), None);
}
