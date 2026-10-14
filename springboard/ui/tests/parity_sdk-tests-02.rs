//! Headless tests for the parity rows of the SDK model layer: contact access, partial contact fetches, labeled values
//! and dates. Everything here is pure in-memory state: no display, no device, no store file and no clock.

use bifrost_ui::contact_access::{Access, Choice, Error as AccessError, Status};
use bifrost_ui::contact_fetch::{
    project, DateComponents, Error as FetchError, Key, Keys, LabeledValue, Record, Value, LABEL_HOME, LABEL_OTHER,
    LABEL_PHONE_MAIN, LABEL_PHONE_MOBILE, LABEL_WORK,
};
use bifrost_ui::fetch::{fetch, fetch_indices, Field, FetchRequest, Op, Predicate};
use std::cmp::Ordering;
use std::collections::BTreeSet;

fn picked(ids: &[u64]) -> Choice {
    Choice::Picked(ids.iter().copied().collect())
}

#[test]
fn headless_status_wire_names_are_fixed() {
    let names: Vec<&str> =
        [Status::NotDetermined, Status::Restricted, Status::Denied, Status::Authorized, Status::Limited]
            .into_iter()
            .map(|s| s.name())
            .collect();
    assert_eq!(names, ["not_determined", "restricted", "denied", "authorized", "limited"]);
}

#[test]
fn headless_a_default_app_starts_undecided_with_nothing_picked() {
    assert_eq!(Access::default(), Access::new());
    assert_eq!(Access::default().status(), Status::NotDetermined);
    assert!(Access::default().picked().is_empty());
}

#[test]
fn headless_a_full_grant_can_be_taken_back_in_settings() {
    let mut a = Access::new();
    assert_eq!(a.request(Choice::All), Ok(Status::Authorized));
    assert_eq!(a.change_in_settings(Choice::Deny), Ok(Status::Denied));
    assert!(!a.can_read(1));
    assert_eq!(a.check_write(), Err(AccessError::NotAuthorized));
    assert_eq!(a.change_in_settings(picked(&[4])), Ok(Status::Limited));
    assert_eq!(a.visible(&[4, 5]), vec![4]);
}

#[test]
fn headless_a_limited_grant_becomes_full_access_only_in_settings() {
    let mut a = Access::new();
    a.request(picked(&[1])).unwrap();
    assert_eq!(a.request(Choice::All), Err(AccessError::AlreadyDecided));
    assert_eq!(a.change_in_settings(Choice::All), Ok(Status::Authorized));
    assert_eq!(a.check_write(), Ok(()));
    assert!(a.picked().is_empty());
}

#[test]
fn headless_a_new_pick_in_settings_replaces_the_old_one() {
    let mut a = Access::new();
    a.request(picked(&[1, 2])).unwrap();
    assert_eq!(a.change_in_settings(picked(&[9])), Ok(Status::Limited));
    assert_eq!(a.visible(&[1, 2, 9]), vec![9]);
    assert_eq!(a.picked().iter().copied().collect::<Vec<u64>>(), vec![9]);
}

#[test]
fn headless_an_empty_settings_pick_keeps_the_current_grant() {
    let mut a = Access::new();
    a.request(picked(&[3])).unwrap();
    assert_eq!(a.change_in_settings(picked(&[])), Err(AccessError::NothingPicked));
    assert_eq!(a.status(), Status::Limited);
    assert!(a.can_read(3));
}

#[test]
fn headless_a_denied_app_sees_no_contacts() {
    let mut a = Access::new();
    a.request(Choice::Deny).unwrap();
    assert!(a.visible(&[1, 2, 3]).is_empty());
    assert_eq!(a.check_write(), Err(AccessError::NotAuthorized));
}

#[test]
fn headless_visible_keeps_the_order_given_and_drops_unpicked_ids() {
    let mut a = Access::new();
    a.request(picked(&[8, 2, 5])).unwrap();
    assert_eq!(a.visible(&[5, 1, 8, 2, 9]), vec![5, 8, 2]);
    let all: BTreeSet<u64> = a.picked().clone();
    assert_eq!(all.len(), 3);
}

#[test]
fn headless_fetch_keys_need_one_key_and_duplicates_collapse() {
    assert_eq!(Keys::new(&[]), Err(FetchError::NoKeys));
    assert_eq!(Keys::new(&[Key::Phones, Key::Phones]), Keys::new(&[Key::Phones]));
    assert!(Keys::all().contains(Key::Dates));
}

#[test]
fn headless_key_wire_names_are_fixed_and_read_back() {
    let names: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(
        names,
        [
            "given_name",
            "family_name",
            "organization",
            "phones",
            "emails",
            "address",
            "note",
            "urls",
            "birthday",
            "dates"
        ]
    );
    assert_eq!(Key::from_name("phones"), Some(Key::Phones));
    assert_eq!(Key::from_name("nickname"), None);
}

fn ada() -> Record {
    let mut r = Record::new(12);
    r.set(Key::GivenName, Value::Text("Ada".into()));
    r.set(
        Key::Phones,
        Value::Labeled(LabeledValue::numbered(vec![
            LabeledValue::new(LABEL_PHONE_MOBILE, "+354 555 0100"),
            LabeledValue::new(LABEL_PHONE_MAIN, "+354 555 0101"),
        ])),
    );
    r.set(Key::Organization, Value::Text("   ".into()));
    r.set(Key::Birthday, Value::Date(DateComponents::new(None, 12, 10).unwrap()));
    r
}

#[test]
fn headless_a_partial_record_holds_only_the_fetched_fields() {
    let keys = Keys::new(&[Key::Phones, Key::GivenName]).unwrap();
    let p = project(&ada(), &keys);
    assert_eq!(p.id, 12);
    assert_eq!(p.fetched().collect::<Vec<Key>>(), vec![Key::GivenName, Key::Phones]);
    assert_eq!(p.text(Key::GivenName), Ok("Ada"));
    assert_eq!(p.get(Key::Birthday), Err(FetchError::NotFetched(Key::Birthday)));
    assert_eq!(p.text(Key::Organization), Err(FetchError::NotFetched(Key::Organization)));
}

#[test]
fn headless_a_fetched_field_the_record_lacks_reads_as_absent() {
    let p = project(&ada(), &Keys::new(&[Key::Organization, Key::Note]).unwrap());
    assert_eq!(p.get(Key::Organization), Ok(None));
    assert_eq!(p.text(Key::Note), Ok(""));
}

#[test]
fn headless_all_keys_fetch_every_stored_field_and_blank_ones_stay_absent() {
    let p = project(&ada(), &Keys::all());
    assert_eq!(p.fetched().count(), Key::ALL.len());
    assert_eq!(p.get(Key::Organization), Ok(None));
    assert_eq!(p.get(Key::Birthday), Ok(Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap()))));
}

#[test]
fn headless_setting_a_blank_value_removes_the_field() {
    let mut r = ada();
    r.set(Key::GivenName, Value::Text("  ".into()));
    assert_eq!(r.get(Key::GivenName), None);
    r.set(Key::Emails, Value::List(vec!["".into(), " ".into()]));
    assert_eq!(r.get(Key::Emails), None);
    r.set(Key::Dates, Value::Dates(vec![]));
    assert_eq!(r.get(Key::Dates), None);
    r.set(Key::Phones, Value::Labeled(vec![LabeledValue::new(LABEL_HOME, "")]));
    assert_eq!(r.get(Key::Phones), None);
}

#[test]
fn headless_labeled_values_number_from_one_and_keep_their_labels() {
    let items = LabeledValue::numbered(vec![
        LabeledValue::new(LABEL_WORK, "ada@example.com"),
        LabeledValue::new("lab", "ada@lab.example"),
        LabeledValue::new(LABEL_OTHER, "ada@other.example"),
    ]);
    let ids: Vec<&str> = items.iter().map(|i| i.identifier.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3"]);
    assert_eq!(items[1].label, "lab");
    assert_eq!(items[2].value, "ada@other.example");
}

#[test]
fn headless_predefined_labels_are_plain_words() {
    assert_eq!(LABEL_HOME, "home");
    assert_eq!(LABEL_WORK, "work");
    assert_eq!(LABEL_OTHER, "other");
    assert_eq!(LABEL_PHONE_MOBILE, "mobile");
    assert_eq!(LABEL_PHONE_MAIN, "main");
}

#[test]
fn headless_date_components_reject_out_of_range_parts() {
    assert_eq!(DateComponents::new(Some(2026), 0, 1), None);
    assert_eq!(DateComponents::new(Some(2026), 13, 1), None);
    assert_eq!(DateComponents::new(Some(2026), 1, 0), None);
    assert_eq!(DateComponents::new(Some(2026), 1, 32), None);
    assert!(DateComponents::new(Some(2026), 1, 1).is_some());
}

#[test]
fn headless_date_text_round_trips_with_and_without_a_year() {
    let dated = DateComponents::new(Some(1815), 12, 10).unwrap();
    assert_eq!(dated.to_text(), "1815-12-10");
    assert_eq!(DateComponents::from_text("1815-12-10"), Some(dated));
    let yearless = DateComponents::new(None, 3, 7).unwrap();
    assert_eq!(yearless.to_text(), "--03-07");
    assert_eq!(DateComponents::from_text("--03-07"), Some(yearless));
    assert_eq!(DateComponents::from_text("1815-13-10"), None);
    assert_eq!(DateComponents::from_text("soon"), None);
}

#[test]
fn headless_access_and_fetch_errors_are_distinct_typed_variants() {
    assert_ne!(AccessError::ReadOnly, AccessError::NotAuthorized);
    assert_ne!(AccessError::NothingPicked, AccessError::AlreadyDecided);
    assert_eq!(FetchError::NotFetched(Key::Note), FetchError::NotFetched(Key::Note));
    assert_ne!(FetchError::NotFetched(Key::Note), FetchError::NotFetched(Key::Urls));
}

// Fetch requests over in-memory records (bifrost_ui::fetch): predicates, sort descriptors, offset and limit.

struct Item {
    name: &'static str,
    rank: i64,
    done: bool,
}

impl bifrost_ui::fetch::Record for Item {
    fn field(&self, key: &str) -> Option<Field> {
        match key {
            "name" => Some(Field::Text(self.name.into())),
            "rank" => Some(Field::Int(self.rank)),
            "done" => Some(Field::Bool(self.done)),
            _ => None,
        }
    }
}

fn items() -> Vec<Item> {
    vec![
        Item { name: "Birch", rank: 2, done: false },
        Item { name: "alder", rank: 5, done: true },
        Item { name: "Cedar", rank: 2, done: true },
        Item { name: "Aspen", rank: 9, done: false },
    ]
}

fn names(found: &[&Item]) -> Vec<&'static str> {
    found.iter().map(|i| i.name).collect()
}

#[test]
fn headless_comparisons_select_matching_records_in_their_original_order() {
    let all = items();
    let req = FetchRequest::new().filter(Predicate::compare("rank", Op::Gt, Field::Int(4)));
    assert_eq!(names(&fetch(&all, &req)), ["alder", "Aspen"]);
}

#[test]
fn headless_a_missing_field_never_matches_a_comparison() {
    let all = items();
    let req = FetchRequest::new().filter(Predicate::compare("colour", Op::Ne, Field::Null));
    assert!(fetch(&all, &req).is_empty());
}

#[test]
fn headless_text_containment_ignores_case_and_never_matches_other_kinds() {
    let all = items();
    let by_name = FetchRequest::new().filter(Predicate::contains("name", "ED"));
    assert_eq!(names(&fetch(&all, &by_name)), ["Cedar"]);
    let by_rank = FetchRequest::new().filter(Predicate::contains("rank", "2"));
    assert!(fetch(&all, &by_rank).is_empty());
}

#[test]
fn headless_not_and_or_combine_predicates() {
    let all = items();
    let open = Predicate::compare("done", Op::Eq, Field::Bool(true)).negate();
    assert_eq!(names(&fetch(&all, &FetchRequest::new().filter(open))), ["Birch", "Aspen"]);
    let either = Predicate::contains("name", "aspen").or(Predicate::compare("rank", Op::Eq, Field::Int(5)));
    assert_eq!(names(&fetch(&all, &FetchRequest::new().filter(either))), ["alder", "Aspen"]);
    let both = Predicate::compare("rank", Op::Ge, Field::Int(2)).and(Predicate::compare("rank", Op::Lt, Field::Int(9)));
    assert_eq!(names(&fetch(&all, &FetchRequest::new().filter(both))), ["Birch", "alder", "Cedar"]);
}

#[test]
fn headless_sorting_uses_each_key_in_turn_and_the_direction_given() {
    let all = items();
    let by_name = FetchRequest::new().sorted_by("name", true);
    assert_eq!(names(&fetch(&all, &by_name)), ["alder", "Aspen", "Birch", "Cedar"]);
    let by_rank = FetchRequest::new().sorted_by("rank", false);
    assert_eq!(names(&fetch(&all, &by_rank)), ["Aspen", "alder", "Birch", "Cedar"]);
    let rank_then_name = FetchRequest::new().sorted_by("rank", false).sorted_by("name", false);
    assert_eq!(names(&fetch(&all, &rank_then_name)), ["Aspen", "alder", "Cedar", "Birch"]);
}

#[test]
fn headless_records_that_tie_keep_their_original_order() {
    let all = items();
    let req = FetchRequest::new().sorted_by("done", true);
    assert_eq!(names(&fetch(&all, &req)), ["Birch", "Aspen", "alder", "Cedar"]);
}

#[test]
fn headless_offset_and_limit_apply_after_sorting() {
    let all = items();
    let req = FetchRequest::new().sorted_by("name", true).offset(1).limit(2);
    assert_eq!(names(&fetch(&all, &req)), ["Aspen", "Birch"]);
    assert_eq!(fetch_indices(&all, &req), vec![3, 0]);
    assert!(fetch_indices(&all, &FetchRequest::new().offset(10)).is_empty());
    assert!(fetch_indices(&all, &FetchRequest::new().limit(0)).is_empty());
}

#[test]
fn headless_an_empty_request_selects_every_record_in_order() {
    let all = items();
    assert_eq!(fetch_indices(&all, &FetchRequest::new()), vec![0, 1, 2, 3]);
}

#[test]
fn headless_values_order_by_kind_then_by_value() {
    assert_eq!(Field::Null.order(&Field::Bool(false)), Ordering::Less);
    assert_eq!(Field::Bool(false).order(&Field::Bool(true)), Ordering::Less);
    assert_eq!(Field::Int(-3).order(&Field::Int(4)), Ordering::Less);
    assert_eq!(Field::Int(100).order(&Field::Text("a".into())), Ordering::Less);
    assert_eq!(Field::Text("Banana".into()).order(&Field::Text("apple".into())), Ordering::Greater);
    assert_eq!(Field::Text("a".into()).order(&Field::Text("A".into())), Ordering::Greater);
}

// Drawings (bifrost_ui::drawing): strokes, points, inks, the stroke eraser, the text form and the rendered raster.
// Headless: the model lives in memory and the raster is checked pixel by pixel, without any display.

use bifrost_ui::drawing::{union, Drawing, DrawingError, Ink, InkKind, Raster, Stroke, StrokePoint};
use bifrost_ui::undo::UndoManager;
use bifrost_ui::Rect;

fn pen(color: u32, width: f32) -> Ink {
    Ink { kind: InkKind::Pen, color, width }
}

fn line(ink: Ink, points: &[(f32, f32)]) -> Stroke {
    Stroke { ink, points: points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect() }
}

fn canvas(w: f32, h: f32) -> Rect {
    Rect { x: 0.0, y: 0.0, w, h }
}

#[test]
fn headless_a_stroke_length_and_bounds_come_from_its_points() {
    let s = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (3.0, 4.0), (3.0, 10.0)]);
    assert_eq!(s.length(), 11.0);
    assert_eq!(s.bounds(), Some(Rect { x: 0.0, y: 0.0, w: 3.0, h: 10.0 }));
    assert_eq!(line(pen(0, 1.0), &[]).bounds(), None);
}

#[test]
fn headless_a_point_without_stylus_data_has_neutral_readings() {
    assert_eq!(
        StrokePoint::at(1.0, 2.0),
        StrokePoint { x: 1.0, y: 2.0, time_offset: 0.0, size: (0.0, 0.0), opacity: 1.0, force: 0.0, azimuth: 0.0, altitude: 0.0 }
    );
}

#[test]
fn headless_slicing_a_stroke_interpolates_its_new_ends() {
    let s = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)]);
    let xs: Vec<f32> = s.slice(0.25, 0.75).points.iter().map(|p| p.x).collect();
    assert_eq!(xs, vec![2.5, 7.5]);
    assert!(s.slice(0.5, 0.5).points.is_empty());
    assert_eq!(s.slice(-1.0, 2.0), s.slice(0.0, 1.0));
    let bent = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)]);
    assert_eq!(bent.slice(0.0, 1.0).points.len(), 3);
}

#[test]
fn headless_resampling_spaces_points_evenly_by_length() {
    let s = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)]);
    let xs: Vec<f32> = s.resampled(5).points.iter().map(|p| p.x).collect();
    assert_eq!(xs, vec![0.0, 2.5, 5.0, 7.5, 10.0]);
    assert!(s.resampled(0).points.is_empty());
    assert_eq!(s.resampled(1).points, vec![StrokePoint::at(0.0, 0.0)]);
    let still = line(pen(0, 1.0), &[(3.0, 3.0), (3.0, 3.0)]);
    assert!(still.resampled(3).points.iter().all(|p| p.x == 3.0 && p.y == 3.0));
}

#[test]
fn headless_blending_two_strokes_mixes_colour_width_and_switches_kind_at_halfway() {
    let a = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)]);
    let b = line(Ink { kind: InkKind::Marker, color: 0xff00_ff00, width: 6.0 }, &[(0.0, 0.0), (10.0, 0.0)]);
    let half = Stroke::blend(&a, &b, 0.5);
    assert_eq!(half.ink, Ink { kind: InkKind::Marker, color: 0xff00_8000, width: 4.0 });
    let early = Stroke::blend(&a, &b, 0.25);
    assert_eq!(early.ink.kind, InkKind::Pen);
    assert_eq!(early.ink.width, 3.0);
    assert_eq!(Stroke::blend(&a, &b, 2.0).ink, b.ink);
}

#[test]
fn headless_blending_two_empty_strokes_keeps_no_points() {
    let a = line(pen(0xff00_0000, 2.0), &[]);
    let b = line(Ink { kind: InkKind::Pencil, color: 0xff00_ff00, width: 6.0 }, &[]);
    let blended = Stroke::blend(&a, &b, 0.8);
    assert!(blended.points.is_empty());
    assert_eq!(blended.ink.kind, InkKind::Pencil);
}

#[test]
fn headless_ink_kinds_round_trip_through_the_text_form() {
    let mut d = Drawing::new();
    d.push(line(Ink { kind: InkKind::Marker, color: 0xff00_ff00, width: 6.0 }, &[(0.0, 0.0), (1.5, 2.0)]));
    let text = d.to_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "bifrost-drawing 1",
            "stroke marker ff00ff00 6",
            "point 0 0 0 0 0 1 0 0 0",
            "point 1.5 2 0 0 0 1 0 0 0",
        ]
    );
    assert_eq!(Drawing::from_text(&text), Ok(d));
    let pencil = Drawing::from_text("bifrost-drawing 1\nstroke pencil ff000000 2").unwrap();
    assert_eq!(pencil.strokes()[0].ink.kind, InkKind::Pencil);
}

#[test]
fn headless_malformed_drawing_text_names_the_bad_line() {
    assert_eq!(Drawing::from_text("hello"), Err(DrawingError::BadHeader));
    assert_eq!(
        Drawing::from_text("bifrost-drawing 1\nstroke ink 00000000 1"),
        Err(DrawingError::Line { line: 2, reason: "unknown ink kind" })
    );
    assert_eq!(
        Drawing::from_text("bifrost-drawing 1\npoint 0 0 0 0 0 1 0 0 0"),
        Err(DrawingError::Line { line: 2, reason: "point before any stroke" })
    );
    assert_eq!(
        Drawing::from_text("bifrost-drawing 1\nstroke pen ff000000 2\npoint NaN 0 0 0 0 1 0 0 0"),
        Err(DrawingError::Line { line: 3, reason: "bad number in point" })
    );
}

#[test]
fn headless_the_eraser_removes_whole_strokes_that_pass_within_its_radius() {
    let mut d = Drawing::from_strokes(vec![
        line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)]),
        line(pen(0xff00_0000, 2.0), &[(20.0, 0.0), (20.0, 10.0)]),
    ]);
    assert_eq!(d.erase_strokes_at(5.0, 1.0, 2.0), 1);
    assert_eq!(d.strokes().len(), 1);
    assert_eq!(d.strokes()[0].points[0].x, 20.0);
    assert_eq!(d.erase_strokes_at(5.0, 50.0, 1.0), 0);
}

#[test]
fn headless_the_eraser_counts_a_stroke_exactly_on_its_radius_and_a_dot_too() {
    let mut edge = Drawing::from_strokes(vec![line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)])]);
    assert_eq!(edge.erase_strokes_at(5.0, 2.0, 2.0), 1);
    let mut dot = Drawing::from_strokes(vec![line(pen(0xff00_0000, 2.0), &[(7.0, 7.0)])]);
    assert_eq!(dot.erase_strokes_at(7.0, 8.0, 1.0), 1);
    assert!(dot.is_empty());
}

#[test]
fn headless_the_eraser_takes_the_whole_stroke_and_ignores_bad_input() {
    let long = line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]);
    let mut d = Drawing::from_strokes(vec![long]);
    assert_eq!(d.erase_strokes_at(10.0, 0.5, 0.5), 1);
    assert!(d.is_empty());
    let mut kept = Drawing::from_strokes(vec![line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)])]);
    assert_eq!(kept.erase_strokes_at(5.0, 0.0, -1.0), 0);
    assert_eq!(kept.erase_strokes_at(f32::NAN, 0.0, 1.0), 0);
    assert_eq!(kept.strokes().len(), 1);
}

#[test]
fn headless_drawing_bounds_and_union_cover_every_point() {
    let d = Drawing::from_strokes(vec![
        line(pen(0, 1.0), &[(1.0, 1.0), (4.0, 2.0)]),
        line(pen(0, 1.0), &[(0.0, 5.0)]),
    ]);
    assert_eq!(d.bounds(), Some(Rect { x: 0.0, y: 1.0, w: 4.0, h: 4.0 }));
    assert_eq!(Drawing::new().bounds(), None);
    assert_eq!(
        union(Rect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 }, Rect { x: 1.0, y: 1.0, w: 4.0, h: 1.0 }),
        Rect { x: 0.0, y: 0.0, w: 5.0, h: 2.0 }
    );
}

#[test]
fn headless_an_empty_drawing_renders_a_blank_raster_of_the_rect_size() {
    let r = Drawing::new().image(canvas(4.0, 3.0), 1.0);
    assert_eq!(r, Raster { width: 4, height: 3, pixels: vec![0; 12] });
}

#[test]
fn headless_scale_multiplies_the_pixel_size_and_rounds_up() {
    let r = Drawing::new().image(canvas(2.25, 1.0), 2.0);
    assert_eq!((r.width, r.height, r.pixels.len()), (5, 2, 10));
}

#[test]
fn headless_an_empty_rect_bad_scale_or_huge_raster_gives_an_empty_raster() {
    let d = Drawing::from_strokes(vec![line(pen(0xffff_0000, 2.0), &[(0.0, 0.0), (2.0, 2.0)])]);
    assert_eq!(d.image(canvas(0.0, 4.0), 1.0), Raster::default());
    assert_eq!(d.image(canvas(4.0, 4.0), 0.0), Raster::default());
    assert_eq!(d.image(canvas(4.0, 4.0), f32::NAN), Raster::default());
    assert_eq!(d.image(canvas(1.0e6, 1.0e6), 1.0), Raster::default());
}

#[test]
fn headless_a_stroke_paints_its_width_around_its_segment_in_pixels() {
    let d = Drawing::from_strokes(vec![line(pen(0xffff_0000, 2.0), &[(1.0, 2.0), (3.0, 2.0)])]);
    let r = d.image(canvas(4.0, 4.0), 1.0);
    assert_eq!((r.width, r.height), (4, 4));
    let at = |x: usize, y: usize| r.pixels[y * r.width + x];
    assert_eq!(at(1, 1), 0xffff_0000);
    assert_eq!(at(2, 1), 0xffff_0000);
    assert_eq!(at(2, 2), 0xffff_0000);
    assert_eq!(at(2, 0), 0);
    assert_eq!(at(2, 3), 0);
    assert_eq!(at(0, 0), 0);
}

#[test]
fn headless_a_translucent_ink_blends_over_the_empty_canvas_at_its_alpha() {
    let d = Drawing::from_strokes(vec![line(pen(0x80ff_0000, 2.0), &[(1.0, 2.0), (3.0, 2.0)])]);
    let r = d.image(canvas(4.0, 4.0), 1.0);
    assert_eq!(r.pixels[4 + 2], 0x80ff_0000);
}

#[test]
fn headless_later_strokes_paint_over_earlier_ones() {
    let d = Drawing::from_strokes(vec![
        line(pen(0xffff_0000, 2.0), &[(1.0, 2.0), (3.0, 2.0)]),
        line(pen(0xff00_00ff, 2.0), &[(1.0, 2.0), (3.0, 2.0)]),
    ]);
    let r = d.image(canvas(4.0, 4.0), 1.0);
    assert_eq!(r.pixels[4 + 2], 0xff00_00ff);
}

#[test]
fn headless_an_erase_on_a_drawing_undoes_and_redoes_through_the_undo_manager() {
    let mut drawing = Drawing::from_strokes(vec![line(pen(0xff00_0000, 2.0), &[(0.0, 0.0), (10.0, 0.0)])]);
    let before = drawing.strokes()[0].clone();
    let mut undo: UndoManager<Drawing> = UndoManager::new(10);
    assert_eq!(drawing.erase_strokes_at(5.0, 0.0, 1.0), 1);
    let restore = before.clone();
    undo.register(
        "Erase",
        move |d: &mut Drawing| d.push(restore.clone()),
        |d: &mut Drawing| {
            d.erase_strokes_at(5.0, 0.0, 1.0);
        },
    );
    assert!(undo.can_undo());
    assert!(!undo.can_redo());
    assert_eq!(undo.undo(&mut drawing), Some("Erase".to_string()));
    assert_eq!(drawing.strokes(), std::slice::from_ref(&before));
    assert!(undo.can_redo());
    assert_eq!(undo.redo(&mut drawing), Some("Erase".to_string()));
    assert!(drawing.is_empty());
}

#[test]
fn headless_a_new_edit_after_an_undo_clears_the_redo_history() {
    let mut drawing = Drawing::new();
    let mut undo: UndoManager<Drawing> = UndoManager::new(10);
    undo.register("Draw", |d: &mut Drawing| {
        d.erase_strokes_at(0.0, 0.0, 1.0);
    }, |d: &mut Drawing| d.push(line(pen(0, 1.0), &[(0.0, 0.0)])));
    drawing.push(line(pen(0, 1.0), &[(0.0, 0.0)]));
    assert_eq!(undo.undo(&mut drawing), Some("Draw".to_string()));
    assert!(undo.can_redo());
    undo.register("Draw", |d: &mut Drawing| {
        d.erase_strokes_at(0.0, 0.0, 1.0);
    }, |d: &mut Drawing| d.push(line(pen(0, 1.0), &[(0.0, 0.0)])));
    assert!(!undo.can_redo());
    assert_eq!(undo.steps(), 1);
}

// Calendar events and reminders (bifrost_ui::calendar_store) behind calendar access. Headless: an in-memory store.

use bifrost_ui::calendar_access::Access;
use bifrost_ui::calendar_error::ErrorCode;
use bifrost_ui::calendar_store::{Event, EventStore, Reminder, ReminderPredicate, Span};

const DAY: i64 = 86_400;

fn full_store() -> EventStore {
    let mut access = Access::new();
    access.request_full_events(true);
    access.request_full_reminders(true);
    EventStore::new(access)
}

#[test]
fn headless_a_new_event_has_no_identifier_until_it_is_saved() {
    let e = Event::new("Lunch", 1000, 4600);
    assert!(e.calendar_item_identifier.is_empty());
    assert!(!e.is_all_day);
    assert_eq!(e.repeat_days, None);
}

#[test]
fn headless_saving_assigns_an_identifier_and_the_event_reads_back() {
    let mut store = full_store();
    let mut lunch = Event::new("Lunch", 1000, 4600);
    store.save_event(&mut lunch, Span::ThisEvent, true).unwrap();
    assert!(!lunch.calendar_item_identifier.is_empty());
    assert_eq!(store.events_between(0, 10_000).unwrap(), vec![lunch]);
}

#[test]
fn headless_an_event_that_ends_before_it_starts_is_refused() {
    let mut store = full_store();
    let mut bad = Event::new("Bad", 5000, 1000);
    assert_eq!(store.save_event(&mut bad, Span::ThisEvent, true), Err(ErrorCode::DatesInverted));
    assert!(store.events_between(0, 10_000).unwrap().is_empty());
}

#[test]
fn headless_an_uncommitted_save_stays_hidden_until_commit() {
    let mut store = full_store();
    let mut e = Event::new("Lunch", 1000, 4600);
    store.save_event(&mut e, Span::ThisEvent, false).unwrap();
    assert!(store.events_between(0, 10_000).unwrap().is_empty());
    store.commit().unwrap();
    assert_eq!(store.events_between(0, 10_000).unwrap().len(), 1);
}

#[test]
fn headless_two_pending_saves_commit_as_one_store_change() {
    let mut store = full_store();
    let mut a = Event::new("A", 0, 1);
    let mut b = Event::new("B", 0, 1);
    store.save_event(&mut a, Span::ThisEvent, false).unwrap();
    store.save_event(&mut b, Span::ThisEvent, false).unwrap();
    store.commit().unwrap();
    store.commit().unwrap();
    assert_eq!(store.change_generation(), 1);
}

#[test]
fn headless_this_event_skips_one_occurrence_of_a_repeating_series() {
    let mut store = full_store();
    let mut standup = Event::new("Standup", 0, 3600);
    standup.repeat_days = Some(7);
    store.save_event(&mut standup, Span::FutureEvents, true).unwrap();
    let second = Event { start_date: 7 * DAY, end_date: 7 * DAY + 3600, ..standup.clone() };
    store.remove_event(&second, Span::ThisEvent, true).unwrap();
    let starts: Vec<i64> = store.events_between(0, 22 * DAY).unwrap().iter().map(|e| e.start_date).collect();
    assert_eq!(starts, vec![0, 14 * DAY, 21 * DAY]);
}

#[test]
fn headless_future_events_ends_the_series_before_the_chosen_occurrence() {
    let mut store = full_store();
    let mut standup = Event::new("Standup", 0, 3600);
    standup.repeat_days = Some(7);
    store.save_event(&mut standup, Span::FutureEvents, true).unwrap();
    let second = Event { start_date: 7 * DAY, end_date: 7 * DAY + 3600, ..standup.clone() };
    store.remove_event(&second, Span::FutureEvents, true).unwrap();
    let starts: Vec<i64> = store.events_between(0, 60 * DAY).unwrap().iter().map(|e| e.start_date).collect();
    assert_eq!(starts, vec![0]);
}

#[test]
fn headless_removing_an_event_twice_reports_an_internal_failure() {
    let mut store = full_store();
    let mut lunch = Event::new("Lunch", 1000, 4600);
    store.save_event(&mut lunch, Span::ThisEvent, true).unwrap();
    store.remove_event(&lunch, Span::ThisEvent, true).unwrap();
    assert!(store.events_between(0, 10_000).unwrap().is_empty());
    assert_eq!(store.remove_event(&lunch, Span::ThisEvent, true), Err(ErrorCode::InternalFailure));
}

#[test]
fn headless_writes_need_a_grant_and_reads_need_full_access() {
    let mut e = Event::new("Lunch", 0, 10);
    let mut none = EventStore::new(Access::new());
    assert_eq!(none.save_event(&mut e, Span::ThisEvent, true), Err(ErrorCode::EventStoreNotAuthorized));
    let mut write_only = Access::new();
    write_only.request_write_only_events(true);
    let mut store = EventStore::new(write_only);
    store.save_event(&mut e, Span::ThisEvent, true).unwrap();
    assert_eq!(store.events_between(0, 100), Err(ErrorCode::EventStoreNotAuthorized));
}

#[test]
fn headless_reminders_save_complete_and_filter_by_completion() {
    let mut store = full_store();
    let mut milk = Reminder::new("Milk");
    let mut tax = Reminder::new("Tax");
    tax.is_completed = true;
    store.save_reminder(&mut milk, true).unwrap();
    store.save_reminder(&mut tax, true).unwrap();
    assert_eq!(store.fetch_reminders(ReminderPredicate::All).unwrap().len(), 2);
    assert_eq!(store.fetch_reminders(ReminderPredicate::Incomplete).unwrap(), vec![milk.clone()]);
    assert_eq!(store.fetch_reminders(ReminderPredicate::Completed).unwrap(), vec![tax]);
    store.remove_reminder(&milk, true).unwrap();
    assert_eq!(store.fetch_reminders(ReminderPredicate::All).unwrap().len(), 1);
}
