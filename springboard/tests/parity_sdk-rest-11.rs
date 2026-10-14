//! Parity sdk-rest-11: headless tests of the system sleep block an app holds through its process activities. The
//! shell keeps the registry; the suspend path refuses while it has an entry. No test suspends a machine.

use bifrost_proto::activity::{BACKGROUND, IDLE_SYSTEM_SLEEP_DISABLED, USER_INITIATED};
use springboard::bare::sleep::{app_sleep_blockers, forget_app, note_app_activity};

#[test]
fn an_app_that_holds_idle_system_sleep_disabled_blocks_system_sleep() {
    let app = "is.example.sleep-holder";
    note_app_activity(app, IDLE_SYSTEM_SLEEP_DISABLED);
    assert!(app_sleep_blockers().contains(&format!("app {app}")));
    forget_app(app);
}

#[test]
fn an_app_that_ends_its_sleep_activity_releases_the_block() {
    let app = "is.example.sleep-release";
    note_app_activity(app, IDLE_SYSTEM_SLEEP_DISABLED);
    note_app_activity(app, USER_INITIATED | BACKGROUND);
    assert!(!app_sleep_blockers().contains(&format!("app {app}")));
}

#[test]
fn an_app_that_goes_away_releases_its_block_even_if_it_never_ended_the_activity() {
    let app = "is.example.sleep-gone";
    note_app_activity(app, IDLE_SYSTEM_SLEEP_DISABLED | USER_INITIATED);
    forget_app(app);
    assert!(!app_sleep_blockers().contains(&format!("app {app}")));
}

#[test]
fn an_app_without_the_idle_bit_does_not_block_system_sleep() {
    let app = "is.example.no-sleep-block";
    note_app_activity(app, USER_INITIATED | BACKGROUND);
    assert!(!app_sleep_blockers().contains(&format!("app {app}")));
}
