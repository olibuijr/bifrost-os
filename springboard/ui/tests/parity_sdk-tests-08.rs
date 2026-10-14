//! Parity tests (sdk-tests-08) for the pure models behind the store purchase flow, entitlements and subscriptions, the
//! remote view host state, and the picture-in-picture controller and placement. Host tests with concrete values: no
//! display, no device, no network and no clock. Every model here is state or geometry only; nothing is drawn.

use bifrost_ui::calendar::{from_days, to_days, Date};
use bifrost_ui::entitlement::Ledger;
use bifrost_ui::pip_window::{self, Controller, Corner, Event, State};
use bifrost_ui::purchase::{Outcome, Price, Purchase, Step};
use bifrost_ui::remote_view::{Bounds, Error, Phase, Session};
use bifrost_ui::subscription::{self, Period, Renewal, Unit};
use bifrost_ui::Rect;

fn day(year: i32, month: u32, d: u32) -> i64 {
    to_days(Date { year, month, day: d })
}

fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

// Purchase flow (storekit).

#[test]
fn purchase_price_text_shows_fraction_digits_and_currency() {
    assert_eq!(Price::new(1250, 2, "EUR").display(), "12.50 EUR");
    assert_eq!(Price::new(500, 2, "EUR").display(), "5.00 EUR");
    assert_eq!(Price::new(990, 0, "ISK").display(), "990 ISK");
    assert_eq!(Price::new(-75, 2, "USD").display(), "-0.75 USD");
}

#[test]
fn purchase_price_clamps_fraction_digits_to_six() {
    let p = Price::new(5, 9, "XYZ");
    assert_eq!(p.fraction_digits, 6);
    assert_eq!(p.display(), "0.000005 XYZ");
}

#[test]
fn purchase_sheet_is_shown_once_before_a_result_is_taken() {
    let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
    assert_eq!(p.step(), &Step::Idle);
    assert!(!p.complete(Outcome::Success(1)), "no result before the sheet");
    assert!(p.begin());
    assert!(!p.begin(), "the sheet is shown once");
    assert_eq!(p.step(), &Step::ConfirmationShown);
    assert!(p.complete(Outcome::Success(7)));
    assert_eq!(p.outcome(), Some(&Outcome::Success(7)));
    assert!(p.may_install());
}

#[test]
fn purchase_pending_result_waits_for_its_final_result() {
    let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
    p.begin();
    assert!(p.complete(Outcome::Pending));
    assert_eq!(p.step(), &Step::Pending);
    assert!(p.outcome().is_none());
    assert!(!p.may_install());
    assert!(!p.complete(Outcome::Pending), "a second pending result is refused");
    assert!(p.complete(Outcome::UserCancelled));
    assert_eq!(p.outcome(), Some(&Outcome::UserCancelled));
    assert!(!p.may_install(), "a cancelled pending purchase never allows the install");
}

#[test]
fn purchase_cancelled_at_the_sheet_never_allows_the_install() {
    let mut p = Purchase::new("pro", Price::new(1000, 2, "EUR"));
    p.begin();
    assert!(p.complete(Outcome::UserCancelled));
    assert_eq!(p.outcome(), Some(&Outcome::UserCancelled));
    assert!(!p.may_install());
}

// Transactions and entitlements (storekit).

#[test]
fn ledger_ids_increase_and_are_never_reused() {
    let mut l = Ledger::new();
    assert_eq!(l.record("pro", 1, None), 1);
    assert_eq!(l.record("theme", 2, None), 2);
    assert_eq!(l.get(2).map(|t| t.product.as_str()), Some("theme"));
    assert!(l.get(3).is_none());
}

#[test]
fn ledger_one_time_purchase_entitles_from_its_day_onwards() {
    let mut l = Ledger::new();
    l.record("pro", 10, None);
    assert!(!l.is_entitled("pro", 9));
    assert!(l.is_entitled("pro", 10));
    assert!(l.is_entitled("pro", 5000));
}

#[test]
fn ledger_latest_renewal_stands_for_the_product() {
    let mut l = Ledger::new();
    l.record("pro", 10, Some(Renewal::new(40)));
    l.record("pro", 50, Some(Renewal::new(80)));
    l.record("theme", 20, None);

    let now = l.current_entitlements(55);
    let names: Vec<&str> = now.iter().map(|t| t.product.as_str()).collect();
    assert_eq!(names, vec!["pro", "theme"]);
    assert_eq!(now[0].purchased, 50);
    assert!(!l.is_entitled("pro", 90), "the latest pro subscription expired on day 80");
    assert!(l.is_entitled("theme", 90));
}

#[test]
fn ledger_revocation_ends_access_from_its_day_and_the_first_day_stands() {
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
fn ledger_grace_period_keeps_access_after_the_paid_period() {
    let mut l = Ledger::new();
    let grace = Renewal { grace_until: Some(105), ..Renewal::new(100) };
    l.record("pro", 1, Some(grace));
    assert!(l.is_entitled("pro", 102));
    assert!(!l.is_entitled("pro", 106));
}

#[test]
fn ledger_finish_marks_one_purchase_and_unfinished_lists_the_rest() {
    let mut l = Ledger::new();
    let a = l.record("pro", 1, None);
    l.record("theme", 2, None);
    assert!(l.finish(a));
    assert!(!l.finish(9));
    let ids: Vec<u64> = l.unfinished().iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn ledger_all_for_lists_newest_first_and_latest_is_the_first() {
    let mut l = Ledger::new();
    let first = l.record("pro", 1, None);
    let newest = l.record("pro", 5, None);
    let middle = l.record("pro", 3, None);
    let ids: Vec<u64> = l.all_for("pro").iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![newest, middle, first]);
    assert_eq!(l.latest_for("pro").map(|t| t.id), Some(newest));
    assert!(l.latest_for("missing").is_none());
}

// Subscription status and renewal (storekit).

#[test]
fn subscription_month_clamps_to_the_end_of_a_shorter_month() {
    let next = Period::new(Unit::Month, 1).add_to(day(2026, 1, 31));
    assert_eq!(from_days(next), Date { year: 2026, month: 2, day: 28 });
    let leap = Period::new(Unit::Month, 1).add_to(day(2028, 1, 31));
    assert_eq!(from_days(leap), Date { year: 2028, month: 2, day: 29 });
}

#[test]
fn subscription_year_from_leap_day_clamps_in_a_common_year() {
    let next = Period::new(Unit::Year, 1).add_to(day(2028, 2, 29));
    assert_eq!(from_days(next), Date { year: 2029, month: 2, day: 28 });
}

#[test]
fn subscription_days_and_weeks_are_plain_day_counts() {
    assert_eq!(Period::new(Unit::Day, 3).add_to(10), 13);
    assert_eq!(Period::new(Unit::Week, 2).add_to(10), 24);
}

#[test]
fn subscription_zero_period_is_clamped_to_one_unit() {
    let p = Period::new(Unit::Week, 0);
    assert_eq!(p.value, 1);
    assert_eq!(p.add_to(10), 17);
}

#[test]
fn subscription_state_follows_the_paid_period_then_expires() {
    let r = Renewal::new(100);
    assert_eq!(r.state(99), subscription::State::Subscribed);
    assert_eq!(r.state(100), subscription::State::Expired);
    assert!(r.grants_access(99));
    assert!(!r.grants_access(100));
}

#[test]
fn subscription_grace_period_keeps_access_and_a_retry_without_grace_does_not() {
    let graced = Renewal { grace_until: Some(105), ..Renewal::new(100) };
    assert_eq!(graced.state(102), subscription::State::InGracePeriod);
    assert!(graced.grants_access(102));
    assert_eq!(graced.state(106), subscription::State::Expired);

    let retrying = Renewal { billing_retry: true, ..Renewal::new(100) };
    assert_eq!(retrying.state(101), subscription::State::InBillingRetry);
    assert!(!retrying.grants_access(101));
}

#[test]
fn subscription_next_expiration_moves_one_period_only_while_auto_renew_is_on() {
    let month = Period::new(Unit::Day, 30);
    assert_eq!(Renewal::new(100).next_expiration(month), Some(130));
    let off = Renewal { auto_renew: false, ..Renewal::new(100) };
    assert_eq!(off.next_expiration(month), None);
    let monthly = Renewal::new(day(2026, 1, 31));
    assert_eq!(monthly.next_expiration(Period::new(Unit::Month, 1)), Some(day(2026, 2, 28)));
}

// Extension-based UI in a host app (extensionkit): the host side of a remote view.

#[test]
fn remote_view_bounds_refuse_a_minimum_above_the_maximum() {
    assert_eq!(Bounds::new(200, 50, 100, 300), None);
    assert_eq!(Bounds::new(200, 400, 300, 300), None);
    assert_eq!(Bounds::new(100, 50, 100, 50).map(|b| b.clamp(7, 9)), Some((100, 50)));
}

#[test]
fn remote_view_connected_view_hides_the_placeholder_and_a_second_connect_is_busy() {
    let bounds = Bounds::new(100, 50, 400, 300).unwrap();
    let mut s = Session::new(bounds, 2);
    assert_eq!(s.phase(), Phase::Idle);
    assert!(s.shows_placeholder());
    assert_eq!(s.connect(), Ok(()));
    assert_eq!(s.phase(), Phase::Connecting);
    assert!(s.connected());
    assert_eq!(s.phase(), Phase::Active);
    assert!(!s.shows_placeholder());
    assert!(!s.connected(), "a second connected report changes nothing");
    assert_eq!(s.connect(), Err(Error::Busy));
}

#[test]
fn remote_view_losses_past_the_limit_fail_until_the_host_stops() {
    let bounds = Bounds::new(100, 50, 400, 300).unwrap();
    let mut s = Session::new(bounds, 2);
    s.connect().unwrap();
    assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
    s.connect().unwrap();
    assert_eq!(s.lost(), Phase::Reconnecting { attempt: 2 });
    s.connect().unwrap();
    assert_eq!(s.lost(), Phase::Failed);
    assert_eq!(s.lost(), Phase::Failed);
    assert_eq!(s.connect(), Err(Error::Busy));
    s.stop();
    assert_eq!(s.phase(), Phase::Idle);
    assert_eq!(s.connect(), Ok(()));
}

#[test]
fn remote_view_a_good_connection_resets_the_loss_count() {
    let bounds = Bounds::new(100, 50, 400, 300).unwrap();
    let mut s = Session::new(bounds, 2);
    s.connect().unwrap();
    s.connected();
    assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
    s.connect().unwrap();
    s.connected();
    assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
}

#[test]
fn remote_view_fit_is_clamped_and_only_an_active_view_reports_a_size() {
    let bounds = Bounds::new(100, 50, 400, 300).unwrap();
    let mut s = Session::new(bounds, 2);
    assert_eq!(s.fit(1000, 120), (400, 120));
    assert_eq!(s.size(), (400, 120));
    assert_eq!(s.report(200, 200), None, "not active yet, so the view may not resize itself");
    s.connect().unwrap();
    s.connected();
    assert_eq!(s.report(500, 20), Some((400, 50)));
    assert_eq!(s.report(400, 50), None);
    assert_eq!(s.report(10, 10), Some((100, 50)));
}

// Picture in Picture (avkit): the controller and the window placement.

#[test]
fn pip_window_starts_and_stops_in_order() {
    let mut c = Controller::new();
    assert_eq!(c.start(), None, "not possible yet");
    assert_eq!(c.set_possible(true), None);
    assert_eq!(c.start(), Some(Event::WillStart));
    assert_eq!(c.state(), State::Starting);
    assert!(!c.is_active());
    assert_eq!(c.finish_start(), Some(Event::DidStart));
    assert!(c.is_active() && c.can_stop());
    assert_eq!(c.stop(), Some(Event::WillStop));
    assert!(!c.can_stop(), "a stopping window cannot be stopped twice");
    assert_eq!(c.finish_stop(), Some(Event::DidStop));
    assert_eq!(c.state(), State::Inline);
}

#[test]
fn pip_window_start_is_refused_while_suspended_or_already_running() {
    let mut c = Controller::new();
    c.set_possible(true);
    c.suspend(true);
    assert!(c.is_suspended());
    assert_eq!(c.start(), None, "suspended by the system");
    c.suspend(false);
    assert!(c.start().is_some());
    assert_eq!(c.start(), None, "already starting");
    c.finish_start();
    assert_eq!(c.start(), None, "already active");
    assert_eq!(c.finish_start(), None, "finishing needs a starting window");
}

#[test]
fn pip_losing_the_possibility_takes_the_window_down() {
    let mut c = Controller::new();
    c.set_possible(true);
    c.start();
    c.finish_start();
    assert_eq!(c.set_possible(false), Some(Event::DidStop));
    assert_eq!(c.state(), State::Inline);
    assert!(!c.is_possible());
    assert_eq!(c.set_possible(false), None, "nothing left to take down");
}

#[test]
fn pip_automatic_start_only_when_playing_in_the_background() {
    let mut c = Controller::new();
    c.set_possible(true);
    assert_eq!(c.enter_background(true), None, "automatic start is off");
    c.start_automatically = true;
    assert_eq!(c.enter_background(false), None, "paused");
    assert_eq!(c.enter_background(true), Some(Event::WillStart));
    assert_eq!(c.enter_background(true), None, "already starting");
}

#[test]
fn pip_window_size_clamps_width_and_keeps_the_aspect() {
    assert!(near(pip_window::window_size(1920.0, 1080.0, 16.0 / 9.0), (480.0, 270.0)));
    assert!(near(pip_window::window_size(800.0, 600.0, 16.0 / 9.0), (240.0, 135.0)));
    assert!(near(pip_window::window_size(1000.0, 400.0, 1.0), (200.0, 200.0)));
    assert!(near(pip_window::window_size(1920.0, 1080.0, f32::NAN), (480.0, 270.0)), "no usable aspect uses 16:9");
}

#[test]
fn pip_window_sits_in_the_chosen_corner_with_the_edge_margin() {
    let screen = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
    let size = (480.0, 270.0);
    assert_eq!(pip_window::place(screen, size, Corner::TopLeft), Rect { x: 16.0, y: 16.0, w: 480.0, h: 270.0 });
    assert_eq!(
        pip_window::place(screen, size, Corner::BottomRight),
        Rect { x: 1424.0, y: 794.0, w: 480.0, h: 270.0 }
    );
    assert_eq!(pip_window::place(screen, size, Corner::TopRight).x, 1424.0);
    assert_eq!(pip_window::place(screen, size, Corner::BottomLeft).y, 794.0);
}

#[test]
fn pip_dropped_window_snaps_to_the_nearest_corner() {
    let screen = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
    assert_eq!(pip_window::nearest_corner(screen, 100.0, 100.0), Corner::TopLeft);
    assert_eq!(pip_window::nearest_corner(screen, 1800.0, 100.0), Corner::TopRight);
    assert_eq!(pip_window::nearest_corner(screen, 100.0, 1000.0), Corner::BottomLeft);
    assert_eq!(pip_window::nearest_corner(screen, 960.0, 540.0), Corner::BottomRight);
}
