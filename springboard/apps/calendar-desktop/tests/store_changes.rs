//! The calendar's store-change counter is the shared change model (`StoreChangeHub`, the parity model of the
//! official change notification). Every save or import posts one change, and the alert state rescans only when
//! the generation it is given moves on.

use bifrost_app::controls::calendar_store_changes::StoreChangeHub;
use bifrost_calendar::calendar::alerts::{step, AlertState};
use bifrost_calendar::calendar::ics::IcsCalendar;
use std::cell::Cell;

const NOW: i64 = 1_800_000_000;

#[test]
fn a_store_change_rescans_the_alert_calendar_once_per_generation() {
    let mut hub = StoreChangeHub::new();
    let mut state = AlertState::default();
    let loads = Cell::new(0u32);
    let run = |state: &mut AlertState, hub: &StoreChangeHub, now: i64| {
        step(state, now, hub.generation(), 0, || {
            loads.set(loads.get() + 1);
            IcsCalendar::default()
        })
    };

    // Priming loads the calendar once; a quiet step with the same generation does not load again.
    assert!(run(&mut state, &hub, NOW).is_empty());
    assert_eq!(loads.get(), 1);
    assert!(run(&mut state, &hub, NOW + 10).is_empty());
    assert_eq!(loads.get(), 1, "no change, no rescan");

    // One save moves the generation and rescans once.
    hub.post();
    run(&mut state, &hub, NOW + 20);
    assert_eq!(loads.get(), 2);

    // Several saves between two steps still cost one rescan.
    hub.post();
    hub.post();
    hub.post();
    run(&mut state, &hub, NOW + 30);
    assert_eq!(loads.get(), 3);
    assert_eq!(hub.generation(), 4);
}

#[test]
fn an_observer_learns_about_saves_it_did_not_see() {
    let mut hub = StoreChangeHub::new();
    let observer = hub.subscribe();
    hub.post();
    hub.post();
    assert_eq!(hub.missed(observer), Some(2), "both saves after subscribing are reported");
    assert_eq!(hub.acknowledge(observer), Some(true));
    assert_eq!(hub.missed(observer), Some(0));
}
