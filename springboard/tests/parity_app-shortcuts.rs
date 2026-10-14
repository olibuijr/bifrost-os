//! Parity app-shortcuts: personal automations of the Shortcuts app and the starter gallery. A shortcut can run when an app
//! opens or when the battery charge falls below a level. These tests cover the list file, the trigger rule, the copy and
//! import rules and the gallery, headless: nothing reads a battery or starts an app.

use springboard::shortcuts::{self as model, due, parse, serialize, Action, Automation, Event, Shortcut};

fn with_automation(name: &str, automation: Option<Automation>) -> Shortcut {
    Shortcut { automation, ..Shortcut::new(name, 0, vec![Action::Screenshot]) }
}

#[test]
fn a_new_shortcut_has_no_automation() {
    assert_eq!(Shortcut::new("Plain", 0, Vec::new()).automation, None);
}

#[test]
fn an_automation_survives_the_list_file() {
    let list = vec![
        with_automation("Open Notes", Some(Automation::AppOpens(0))),
        with_automation("Save Power", Some(Automation::BatteryBelow(20))),
        with_automation("Plain", None),
    ];
    assert_eq!(parse(&serialize(&list)), list);
}

#[test]
fn a_file_line_without_the_automation_field_reads_as_none() {
    let list = parse("Old\t1\t\t\t0\t0\t2\n");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].automation, None);
    assert_eq!(list[0].key, 2, "the menu key before it is still read");
}

#[test]
fn an_automation_with_no_menu_key_keeps_a_zero_key_in_its_place() {
    let list = vec![with_automation("Save Power", Some(Automation::BatteryBelow(20)))];
    let line = serialize(&list);
    assert!(line.trim_end().ends_with("\t0\tbattery:20"), "{line}");
    assert_eq!(parse(&line)[0].key, 0);
}

#[test]
fn an_out_of_range_automation_in_the_file_is_ignored() {
    let list = parse("Odd\t1\t\t\t0\t0\t0\tbattery:3\nOdder\t1\t\t\t0\t0\t0\topen:99\n");
    assert_eq!(list.len(), 2, "the shortcuts are kept");
    assert!(list.iter().all(|s| s.automation.is_none()));
}

#[test]
fn a_battery_automation_fires_when_the_charge_falls_below_its_level() {
    let list = vec![with_automation("Save Power", Some(Automation::BatteryBelow(20)))];
    assert_eq!(due(&list, Event::BatteryLevel { before: Some(21), now: 19 }), vec![0]);
    assert_eq!(due(&list, Event::BatteryLevel { before: Some(20), now: 19 }), vec![0], "it was at the level, now below");
}

#[test]
fn a_battery_automation_does_not_fire_while_low_on_the_first_reading_or_when_charging() {
    let list = vec![with_automation("Save Power", Some(Automation::BatteryBelow(20)))];
    assert!(due(&list, Event::BatteryLevel { before: Some(15), now: 14 }).is_empty(), "it was already below");
    assert!(due(&list, Event::BatteryLevel { before: None, now: 10 }).is_empty(), "the first reading crosses nothing");
    assert!(due(&list, Event::BatteryLevel { before: Some(18), now: 25 }).is_empty(), "a rise does not fire");
}

#[test]
fn an_app_automation_fires_only_for_its_app() {
    assert_eq!(model::APP_CHOICES[0], "Notes");
    assert_eq!(model::APP_CHOICES[6], "Photos");
    let list = vec![
        with_automation("Notes Setup", Some(Automation::AppOpens(0))),
        with_automation("Photo Setup", Some(Automation::AppOpens(6))),
    ];
    assert_eq!(due(&list, Event::AppOpened("Notes")), vec![0]);
    assert_eq!(due(&list, Event::AppOpened("Photos")), vec![1]);
    assert!(due(&list, Event::AppOpened("Music")).is_empty());
}

#[test]
fn a_shortcut_without_an_automation_is_never_due() {
    let list = vec![with_automation("Plain", None)];
    assert!(due(&list, Event::AppOpened("Notes")).is_empty());
    assert!(due(&list, Event::BatteryLevel { before: Some(90), now: 1 }).is_empty());
}

#[test]
fn due_lists_every_matching_shortcut_in_list_order() {
    let list = vec![
        with_automation("A", Some(Automation::BatteryBelow(50))),
        with_automation("B", Some(Automation::AppOpens(0))),
        with_automation("C", Some(Automation::BatteryBelow(40))),
    ];
    assert_eq!(due(&list, Event::BatteryLevel { before: Some(60), now: 30 }), vec![0, 2]);
}

#[test]
fn a_copy_of_a_shortcut_does_not_repeat_its_automation() {
    let mut list = vec![with_automation("Notes Setup", Some(Automation::AppOpens(0)))];
    let copy = model::duplicate(&mut list, 0).unwrap();
    assert_eq!(list[copy].automation, None);
    assert_eq!(list[0].automation, Some(Automation::AppOpens(0)), "the original keeps it");
}

#[test]
fn an_imported_shortcut_does_not_bring_its_automation() {
    let list = vec![with_automation("Save Power", Some(Automation::BatteryBelow(20)))];
    let mut target = Vec::new();
    assert_eq!(model::import(&mut target, &model::export(&list)), Ok(1));
    assert_eq!(target[0].automation, None, "an imported automation would run without its owner's choice");
}

#[test]
fn a_starter_added_from_the_gallery_keeps_its_glyph_and_folder() {
    let mut list = Vec::new();
    let at = model::add_starter(&mut list, 10).expect("the eleventh starter exists");
    assert_eq!(list[at].name, "Sign Off");
    assert_eq!(list[at].glyph, 7);
    assert_eq!(list[at].folder, 1);
}
