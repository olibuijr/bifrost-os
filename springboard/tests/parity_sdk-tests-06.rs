//! Parity sdk-tests-06: headless tests of the Night Shift model. The schedule window, the saved settings, the rows of
//! the Night Shift page, the published channel multipliers and the tint of a frame buffer. The tint runs on memory
//! buffers only; no frame is presented and no display is opened.

use springboard::gfx::Damage;
use springboard::shell::list::Accessory;
use springboard::shell::night_shift::{multipliers, published, tint, tint_in_place, NightEdit, NightShift};
use springboard::shell::pages::Action;
use springboard::shell::prefs::Prefs;

#[test]
fn a_scheduled_window_that_crosses_midnight_covers_its_evening_and_morning() {
    let n = NightShift { scheduled: true, ..Default::default() };
    assert!(n.active(22 * 60), "the start minute is inside the window");
    assert!(n.active(23 * 60));
    assert!(n.active(60));
    assert!(!n.active(7 * 60), "the end minute is outside the window");
    assert!(!n.active(12 * 60));
}

#[test]
fn a_window_that_is_not_scheduled_never_applies_by_the_clock() {
    let n = NightShift::default();
    assert!(!n.scheduled);
    assert!(!n.active(23 * 60));
}

#[test]
fn manual_enable_applies_at_any_time_of_day() {
    let n = NightShift { manual: true, ..Default::default() };
    assert!(n.active(12 * 60));
    assert!(n.active(3 * 60));
}

#[test]
fn an_empty_window_with_equal_start_and_end_never_covers_a_minute() {
    let n = NightShift { scheduled: true, start: 600, end: 600, ..Default::default() };
    assert!(!n.active(600));
    assert!(!n.active(0));
}

#[test]
fn the_row_label_says_on_when_manual_and_scheduled_when_only_the_window_is_on() {
    assert_eq!(NightShift::default().label(), "Off");
    assert_eq!(NightShift { scheduled: true, ..Default::default() }.label(), "Scheduled");
    assert_eq!(NightShift { scheduled: true, manual: true, ..Default::default() }.label(), "On");
}

#[test]
fn a_time_picked_for_from_or_to_wraps_past_a_full_day() {
    let mut n = NightShift::default();
    n.set_edge(true, 1500);
    assert_eq!(n.start, 60);
    n.set_edge(false, 1440);
    assert_eq!(n.end, 0);
    n.set_edge(false, 420);
    assert_eq!(n.end, 420);
}

#[test]
fn loading_from_empty_preferences_gives_the_default_settings() {
    assert_eq!(NightShift::load(&Prefs::memory()), NightShift::default());
}

#[test]
fn saved_settings_load_back_unchanged() {
    let mut p = Prefs::memory();
    let n = NightShift { scheduled: true, manual: false, start: 1300, end: 400, warmth: 0.75 };
    n.save(&mut p);
    assert_eq!(NightShift::load(&p), n);
}

#[test]
fn loaded_warmth_is_clamped_to_the_unit_range_and_minutes_wrap() {
    let mut p = Prefs::memory();
    p.set("night_warmth", "3.0").unwrap();
    p.set("night_from", "1500").unwrap();
    let n = NightShift::load(&p);
    assert_eq!(n.warmth, 1.0);
    assert_eq!(n.start, 60);
}

#[test]
fn the_settings_page_lists_the_schedule_rows_the_manual_switch_and_the_warmth_slider() {
    let n = NightShift { scheduled: true, manual: true, start: 22 * 60, end: 7 * 60, warmth: 0.5 };
    let spec = n.spec(true);
    assert_eq!(spec.title, "Night Shift");
    assert_eq!(spec.groups.len(), 3);
    let schedule = &spec.groups[0].items;
    assert_eq!(schedule.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), vec!["Scheduled", "From", "To"]);
    assert_eq!(schedule[0].accessory, Accessory::Switch(true));
    assert_eq!(schedule[0].action, Action::Night(NightEdit::Scheduled));
    assert_eq!(schedule[1].value, springboard::status::clock_in(22, 0, true));
    assert_eq!(schedule[2].action, Action::Night(NightEdit::To));
    assert_eq!(spec.groups[1].items[0].accessory, Accessory::Switch(true));
    assert!(spec.groups[1].footer.as_deref().unwrap_or("").contains("midnight"));
    assert_eq!(spec.groups[2].header.as_deref(), Some("COLOR TEMPERATURE"));
}

#[test]
fn published_settings_set_the_channel_multipliers_by_warmth_and_the_clock() {
    NightShift { manual: true, warmth: 0.5, ..Default::default() }.publish();
    assert_eq!(published(), NightShift { manual: true, warmth: 0.5, ..Default::default() });
    assert_eq!(multipliers(12 * 60), Some([256, 220, 176]), "red stays, green and blue fall with warmth");

    NightShift { manual: true, warmth: 1.0, ..Default::default() }.publish();
    assert_eq!(multipliers(12 * 60), Some([256, 184, 97]));

    NightShift { manual: true, warmth: 0.0, ..Default::default() }.publish();
    assert_eq!(multipliers(12 * 60), None, "zero warmth leaves the screen untouched");

    NightShift { scheduled: true, warmth: 0.5, ..Default::default() }.publish();
    assert_eq!(multipliers(12 * 60), None, "outside the window the screen is untouched");
    assert_eq!(multipliers(23 * 60), Some([256, 220, 176]));

    NightShift::default().publish();
    assert_eq!(multipliers(23 * 60), None);
}

#[test]
fn tinting_a_buffer_in_place_scales_green_and_blue_and_keeps_alpha() {
    let mut buf = [0x80FF_FFFFu32];
    let mut all = Damage::new(1, 1);
    all.all();
    tint_in_place(&mut buf, 1, &all, [256, 220, 176]);
    assert_eq!(buf[0], 0x80FF_DBAF);
}

#[test]
fn tinting_copies_the_source_pixels_through_the_multipliers() {
    let src = [0xFFFF_FFFFu32, 0xFF00_00FF];
    let mut dst = [0u32; 2];
    let mut all = Damage::new(2, 1);
    all.all();
    tint(&src, &mut dst, 2, &all, [256, 200, 100]);
    assert_eq!(dst, [0xFFFF_C763, 0xFF00_0063]);
}

#[test]
fn tinting_writes_only_the_damaged_pixels() {
    let previous = [0u32; 6];
    let mut current = [0u32; 6];
    current[0] = 0xFFFF_FFFF;
    let damage = Damage::between(&previous, &current, 3, 2);
    let mut dst = [0xFF00_0000u32; 6];
    tint(&current, &mut dst, 3, &damage, [256, 220, 176]);
    assert_eq!(dst[0], 0xFFFF_DBAF, "the changed pixel is tinted");
    assert_eq!(dst[1], 0xFF00_0000, "an undamaged pixel is not written, although its source would tint to zero");
}

#[test]
fn an_empty_damage_region_changes_nothing() {
    let src = [0xFFFF_FFFFu32; 2];
    let mut dst = [0x1234_5678u32; 2];
    let damage = Damage::new(2, 1);
    tint(&src, &mut dst, 2, &damage, [256, 100, 100]);
    assert_eq!(dst, [0x1234_5678, 0x1234_5678]);
}
