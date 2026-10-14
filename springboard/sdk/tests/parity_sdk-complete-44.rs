//! Parity sdk-complete-44: headless tests for event descriptors (value coercion and object specifiers) and for the
//! calendar trigger model of scheduled notifications. Each test drives the public API with concrete values. No
//! device, network or wall clock is used: times are Unix seconds and zone offsets are plain numbers.

use bifrost_app::event_descriptor::{Descriptor, Element, Form, Kind, ObjectSpecifier, ResolveError};
use bifrost_app::usernotif_model::{CalendarTrigger, DateComponents};
use bifrost_core::json::Json;

fn element(class: &str, id: i64, name: Option<&str>, children: Vec<Element>) -> Element {
    Element { class: class.into(), id, name: name.map(String::from), children }
}

/// Two documents: "Notes" holds two paragraphs (the first with two words, the second with one word); "Draft" is empty.
fn tree() -> Vec<Element> {
    let intro = element("paragraph", 10, Some("Intro"), vec![element("word", 100, Some("hello"), vec![]), element("word", 101, Some("world"), vec![])]);
    let body = element("paragraph", 11, None, vec![element("word", 110, Some("again"), vec![])]);
    vec![element("document", 1, Some("Notes"), vec![intro, body]), element("document", 2, Some("Draft"), vec![])]
}

fn doc(form: Form) -> ObjectSpecifier {
    ObjectSpecifier::new("document", form)
}

// Coercion of descriptors

#[test]
fn text_with_digits_coerces_to_an_integer_after_trimming() {
    assert_eq!(Descriptor::Text("42".into()).coerce(Kind::Integer), Ok(Descriptor::Integer(42)));
    assert_eq!(Descriptor::Text(" 7 ".into()).coerce(Kind::Integer), Ok(Descriptor::Integer(7)));
}

#[test]
fn text_that_is_not_a_number_is_refused_for_numeric_kinds() {
    use bifrost_app::event_descriptor::CoercionError;
    assert_eq!(Descriptor::Text("4.5".into()).coerce(Kind::Integer), Err(CoercionError::NotANumber));
    assert_eq!(Descriptor::Text("inf".into()).coerce(Kind::Real), Err(CoercionError::NotANumber));
    assert_eq!(Descriptor::Text("".into()).coerce(Kind::Real), Err(CoercionError::NotANumber));
}

#[test]
fn text_reads_booleans_words_and_refuses_the_rest() {
    use bifrost_app::event_descriptor::CoercionError;
    assert_eq!(Descriptor::Text("Yes".into()).coerce(Kind::Boolean), Ok(Descriptor::Boolean(true)));
    assert_eq!(Descriptor::Text(" false ".into()).coerce(Kind::Boolean), Ok(Descriptor::Boolean(false)));
    assert_eq!(Descriptor::Text("maybe".into()).coerce(Kind::Boolean), Err(CoercionError::NotABoolean));
}

#[test]
fn numbers_convert_between_integer_and_real_and_round_to_nearest() {
    assert_eq!(Descriptor::Integer(3).coerce(Kind::Real), Ok(Descriptor::Real(3.0)));
    assert_eq!(Descriptor::Real(2.6).coerce(Kind::Integer), Ok(Descriptor::Integer(3)));
    assert_eq!(Descriptor::Real(-2.6).coerce(Kind::Integer), Ok(Descriptor::Integer(-3)));
}

#[test]
fn reals_that_do_not_fit_an_integer_are_out_of_range() {
    use bifrost_app::event_descriptor::CoercionError;
    assert_eq!(Descriptor::Real(f64::NAN).coerce(Kind::Integer), Err(CoercionError::OutOfRange));
    assert_eq!(Descriptor::Real(f64::INFINITY).coerce(Kind::Integer), Err(CoercionError::OutOfRange));
    assert_eq!(Descriptor::Real(1e30).coerce(Kind::Integer), Err(CoercionError::OutOfRange));
}

#[test]
fn booleans_and_integers_convert_both_ways() {
    assert_eq!(Descriptor::Boolean(true).coerce(Kind::Integer), Ok(Descriptor::Integer(1)));
    assert_eq!(Descriptor::Integer(0).coerce(Kind::Boolean), Ok(Descriptor::Boolean(false)));
    assert_eq!(Descriptor::Integer(12).coerce(Kind::Text), Ok(Descriptor::Text("12".into())));
    assert_eq!(Descriptor::Boolean(true).coerce(Kind::Text), Ok(Descriptor::Text("true".into())));
}

#[test]
fn lists_and_null_have_no_conversion() {
    use bifrost_app::event_descriptor::CoercionError;
    let list = Descriptor::List(vec![Descriptor::Integer(1)]);
    assert_eq!(list.coerce(Kind::Text), Err(CoercionError::NoConversion));
    assert_eq!(Descriptor::Null.coerce(Kind::Integer), Err(CoercionError::NoConversion));
    assert_eq!(list.coerce(Kind::Boolean), Err(CoercionError::NoConversion));
}

#[test]
fn json_numbers_become_integers_only_when_they_are_whole() {
    assert_eq!(Descriptor::from_json(&Json::Num(3.0)), Some(Descriptor::Integer(3)));
    assert_eq!(Descriptor::from_json(&Json::Num(2.5)), Some(Descriptor::Real(2.5)));
    assert_eq!(Descriptor::from_json(&Json::Str("x".into())), Some(Descriptor::Text("x".into())));
    assert_eq!(Descriptor::from_json(&Json::Null), Some(Descriptor::Null));
    assert_eq!(
        Descriptor::from_json(&Json::Arr(vec![Json::Bool(true)])),
        Some(Descriptor::List(vec![Descriptor::Boolean(true)]))
    );
    assert_eq!(Descriptor::from_json(&Json::Obj(vec![])), None);
}

// Object specifiers

#[test]
fn a_document_is_found_by_its_name() {
    let roots = tree();
    assert_eq!(doc(Form::Name("Draft".into())).resolve(&roots).map(|e| e.id), Ok(2));
}

#[test]
fn an_index_counts_from_one_and_a_negative_index_from_the_end() {
    let roots = tree();
    assert_eq!(doc(Form::Index(1)).resolve(&roots).map(|e| e.id), Ok(1));
    assert_eq!(doc(Form::Index(-1)).resolve(&roots).map(|e| e.id), Ok(2));
    assert_eq!(doc(Form::Index(-2)).resolve(&roots).map(|e| e.id), Ok(1));
}

#[test]
fn an_index_of_zero_or_past_either_end_is_out_of_range() {
    let roots = tree();
    let out = |index: i64| -> Result<i64, ResolveError> { Err(ResolveError::IndexOutOfRange { class: "document".into(), index }) };
    assert_eq!(doc(Form::Index(0)).resolve(&roots).map(|e| e.id), out(0));
    assert_eq!(doc(Form::Index(3)).resolve(&roots).map(|e| e.id), out(3));
    assert_eq!(doc(Form::Index(-3)).resolve(&roots).map(|e| e.id), out(-3));
}

#[test]
fn a_name_that_no_element_has_is_no_such_object() {
    let roots = tree();
    assert_eq!(
        doc(Form::Name("Missing".into())).resolve(&roots).map(|e| e.id),
        Err(ResolveError::NoSuchObject { class: "document".into() })
    );
}

#[test]
fn specifiers_nest_through_their_container() {
    // word 2 of paragraph 1 of document "Notes"
    let spec = ObjectSpecifier::new("word", Form::Index(2))
        .within(ObjectSpecifier::new("paragraph", Form::Index(1)).within(doc(Form::Name("Notes".into()))));
    assert_eq!(spec.resolve(&tree()).map(|e| e.id), Ok(101));
}

#[test]
fn first_and_last_pick_among_elements_of_the_named_class_only() {
    let notes = || doc(Form::Name("Notes".into()));
    let first = ObjectSpecifier::new("paragraph", Form::First).within(notes());
    let last = ObjectSpecifier::new("paragraph", Form::Last).within(notes());
    assert_eq!(first.resolve(&tree()).map(|e| e.id), Ok(10));
    assert_eq!(last.resolve(&tree()).map(|e| e.id), Ok(11));
}

#[test]
fn an_identifier_form_finds_the_element_with_that_identifier() {
    let spec = ObjectSpecifier::new("word", Form::Id(110)).within(ObjectSpecifier::new("paragraph", Form::Index(2)).within(doc(Form::Name("Notes".into()))));
    assert_eq!(spec.resolve(&tree()).map(|e| e.name.clone()), Ok(Some("again".to_string())));
}

#[test]
fn a_missing_container_stops_the_resolution_with_the_container_class() {
    let spec = ObjectSpecifier::new("word", Form::First).within(doc(Form::Name("Missing".into())));
    assert_eq!(spec.resolve(&tree()).map(|e| e.id), Err(ResolveError::NoSuchObject { class: "document".into() }));
}

#[test]
fn a_class_absent_at_the_top_level_is_out_of_range_for_an_index() {
    let spec = ObjectSpecifier::new("paragraph", Form::Index(1));
    assert_eq!(
        spec.resolve(&tree()).map(|e| e.id),
        Err(ResolveError::IndexOutOfRange { class: "paragraph".into(), index: 1 })
    );
}

// Calendar trigger model: the date components a scheduled notification may match

fn utc(_: i64) -> i64 {
    0
}

#[test]
fn a_month_and_day_trigger_fires_on_that_day_of_the_next_year_that_has_it() {
    // 1970-02-14 09:00 UTC is day 44 after the epoch
    let t = CalendarTrigger { date: DateComponents { month: Some(2), day: Some(14), hour: Some(9), minute: Some(0), ..Default::default() }, repeats: false };
    assert_eq!(t.next_trigger_date(0, &utc), Some(44 * 86_400 + 9 * 3_600));
}

#[test]
fn a_leap_day_trigger_skips_the_years_without_one() {
    // 1972-02-29 00:00 UTC is day 789 after the epoch; 1970 and 1971 have no 29 February
    let t = CalendarTrigger { date: DateComponents { month: Some(2), day: Some(29), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: true };
    assert_eq!(t.next_trigger_date(0, &utc), Some(789 * 86_400));
}

#[test]
fn a_year_pinned_trigger_fires_only_in_that_year() {
    // 1971-01-01 00:00 UTC is day 365
    let t = CalendarTrigger { date: DateComponents { year: Some(1971), month: Some(1), day: Some(1), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: false };
    assert_eq!(t.next_trigger_date(0, &utc), Some(365 * 86_400));
}

#[test]
fn a_weekday_trigger_counts_the_weekday_of_each_day() {
    // 1970-01-01 was a Thursday; the next Monday (weekday 2) at 08:30 UTC is 1970-01-05
    let t = CalendarTrigger { date: DateComponents { weekday: Some(2), hour: Some(8), minute: Some(30), ..Default::default() }, repeats: true };
    assert_eq!(t.next_trigger_date(0, &utc), Some(4 * 86_400 + 8 * 3_600 + 30 * 60));
}

#[test]
fn a_trigger_with_an_impossible_component_never_fires() {
    let t = CalendarTrigger { date: DateComponents { hour: Some(24), ..Default::default() }, repeats: false };
    assert_eq!(t.next_trigger_date(0, &utc), None);
}

#[test]
fn a_trigger_follows_the_zone_offset_at_the_time_it_fires() {
    // UTC+1 all year: 09:00 local is 08:00 UTC on 1970-01-01
    let plus_one = |_: i64| -> i64 { 3_600 };
    let t = CalendarTrigger { date: DateComponents { hour: Some(9), minute: Some(0), ..Default::default() }, repeats: true };
    assert_eq!(t.next_trigger_date(0, &plus_one), Some(8 * 3_600));
}
