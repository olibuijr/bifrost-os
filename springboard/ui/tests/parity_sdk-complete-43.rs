//! Parity sdk-complete-43: headless tests of the page annotation model: markup kinds, editing an annotation in place
//! and finding annotations by kind. No PDF file is read or written.

use bifrost_ui::annotation::{Annotation, PageAnnotations, Subtype};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn links_widgets_and_popups_are_not_markup_and_the_rest_are() {
    for markup in [Subtype::Highlight, Subtype::Ink, Subtype::Square, Subtype::Stamp, Subtype::Text, Subtype::FreeText] {
        assert!(markup.is_markup(), "{markup:?} is made by a reader");
    }
    for other in [Subtype::Link, Subtype::Popup, Subtype::Widget] {
        assert!(!other.is_markup(), "{other:?} is not markup");
    }
}

#[test]
fn contents_can_be_replaced_in_place_and_keep_the_stacking_position() {
    let mut page = PageAnnotations::new();
    let a = page.add(Annotation::new(Subtype::Text, rect(0.0, 0.0, 10.0, 10.0)));
    let b = page.add(Annotation::new(Subtype::Text, rect(20.0, 0.0, 10.0, 10.0)));
    assert!(page.set_contents(a, "Check the total"));
    assert_eq!(page.get(a).unwrap().contents, "Check the total");
    assert_eq!(page.ids(), vec![a, b], "editing does not restack");
    assert!(!page.set_contents(99, "nobody"), "an unknown id is refused");
}

#[test]
fn a_moved_annotation_keeps_its_size_and_is_hit_at_its_new_place() {
    let mut page = PageAnnotations::new();
    let a = page.add(Annotation::new(Subtype::Square, rect(0.0, 0.0, 10.0, 20.0)));
    assert!(page.move_to(a, 100.0, 50.0));
    let moved = page.get(a).unwrap().bounds;
    assert_eq!((moved.x, moved.y, moved.w, moved.h), (100.0, 50.0, 10.0, 20.0));
    assert_eq!(page.annotation_at(5.0, 5.0), None, "the old place is empty now");
    assert_eq!(page.annotation_at(105.0, 60.0), Some(a));
    assert!(!page.move_to(7, 0.0, 0.0));
}

#[test]
fn the_colour_of_an_annotation_can_be_changed() {
    let mut page = PageAnnotations::new();
    let a = page.add(Annotation::new(Subtype::Highlight, rect(0.0, 0.0, 10.0, 10.0)));
    assert!(page.set_color(a, 0xffff_ff00));
    assert_eq!(page.get(a).unwrap().color, 0xffff_ff00);
    assert!(!page.set_color(42, 0));
}

#[test]
fn annotations_of_one_kind_are_listed_in_stacking_order() {
    let mut page = PageAnnotations::new();
    let first = page.add(Annotation::new(Subtype::Highlight, rect(0.0, 0.0, 5.0, 5.0)));
    let _link = page.add(Annotation::new(Subtype::Link, rect(0.0, 0.0, 5.0, 5.0)));
    let second = page.add(Annotation::new(Subtype::Highlight, rect(9.0, 0.0, 5.0, 5.0)));
    assert_eq!(page.ids_of(Subtype::Highlight), vec![first, second]);
    assert!(page.ids_of(Subtype::Circle).is_empty());
}

mod plugin {
    use bifrost_ui::plugin_host::{Bus, HostError, Node, ParameterSpec, ParameterTree, Preset, PresetStore, Unit};

    fn cutoff() -> ParameterSpec {
        ParameterSpec::new(1, "cutoff", "Cutoff", 20.0, 20_000.0, 1_000.0, Unit::Hertz).unwrap()
    }

    fn tree() -> ParameterTree {
        let mute = ParameterSpec::new(2, "mute", "Mute", 0.0, 1.0, 0.0, Unit::Boolean).unwrap().read_only();
        let filter = Node::Group { identifier: "filter".into(), name: "Filter".into(), children: vec![Node::Parameter(cutoff())] };
        ParameterTree::new(vec![filter, Node::Parameter(mute)]).unwrap()
    }

    #[test]
    fn a_written_value_is_clamped_to_the_parameter_range() {
        let mut t = tree();
        assert_eq!(t.set_value(1, 50_000.0), Ok(20_000.0));
        assert_eq!(t.value(1), Some(20_000.0));
        assert_eq!(t.set_value(1, -5.0), Ok(20.0));
    }

    #[test]
    fn writes_to_unknown_read_only_or_non_finite_parameters_are_refused() {
        let mut t = tree();
        assert_eq!(t.set_value(9, 1.0), Err(HostError::UnknownParameter));
        assert_eq!(t.set_value(2, 1.0), Err(HostError::ReadOnly));
        assert_eq!(t.set_value(1, f32::NAN), Err(HostError::InvalidValue));
        assert_eq!(t.value(1), Some(1_000.0), "a refused write changes nothing");
    }

    #[test]
    fn the_tree_lists_addresses_in_tree_order_and_reports_changes_once() {
        let mut t = tree();
        assert_eq!(t.addresses(), &[1u64, 2]);
        t.set_value(1, 500.0).unwrap();
        assert_eq!(t.take_changes(), vec![(1, 500.0)]);
        assert!(t.take_changes().is_empty());
    }

    #[test]
    fn a_preset_saved_from_the_tree_selects_its_values_back() {
        let mut t = tree();
        t.set_value(1, 300.0).unwrap();
        let mut presets = PresetStore::new(Vec::new()).unwrap();
        let number = presets.save_user("Warm", t.snapshot()).unwrap();
        assert_eq!(number, 0);
        t.set_value(1, 9_000.0).unwrap();
        presets.select(number, &mut t).unwrap();
        assert_eq!(t.value(1), Some(300.0));
        assert_eq!(presets.current(), Some(0));
    }

    #[test]
    fn a_saved_state_restores_past_the_read_only_parameters_it_holds() {
        let mut t = tree();
        let saved = t.snapshot();
        assert_eq!(saved, vec![(1, 1_000.0), (2, 0.0)]);
        t.set_value(1, 40.0).unwrap();
        t.restore(&saved).unwrap();
        assert_eq!(t.value(1), Some(1_000.0));
    }

    #[test]
    fn a_restore_with_an_unknown_address_changes_nothing() {
        let mut t = tree();
        t.set_value(1, 40.0).unwrap();
        assert_eq!(t.restore(&[(1, 2_000.0), (99, 1.0)]), Err(HostError::UnknownParameter));
        assert_eq!(t.value(1), Some(40.0));
    }

    #[test]
    fn factory_presets_cannot_be_deleted_and_user_numbers_follow_the_highest() {
        let factory = Preset { number: 5, name: "Room".into(), state: vec![(1, 800.0)], factory: false };
        let mut presets = PresetStore::new(vec![factory]).unwrap();
        assert!(presets.preset(5).unwrap().factory);
        assert_eq!(presets.delete_user(5), Err(HostError::FactoryPreset));
        assert_eq!(presets.save_user("Mine", vec![]).unwrap(), 6);
        presets.delete_user(6).unwrap();
        assert_eq!(presets.delete_user(6), Err(HostError::UnknownPreset));
    }

    #[test]
    fn a_bus_takes_only_the_channel_counts_it_lists_while_enabled() {
        let mut bus = Bus::new("Main", &[2, 1, 2]).unwrap();
        assert!(bus.accepts(2) && bus.accepts(1));
        assert!(!bus.accepts(6));
        assert_eq!(bus.maximum_channels(), 2);
        bus.enabled = false;
        assert!(!bus.accepts(2));
        assert_eq!(Bus::new("Empty", &[]), Err(HostError::InvalidChannelList));
    }
}

mod eventkit {
    use bifrost_ui::calendar_access::Access;
    use bifrost_ui::calendar_store::{Availability, Event, EventStore, Span};

    fn store_with_full_access() -> EventStore {
        let mut access = Access::new();
        access.request_full_events(true);
        EventStore::new(access)
    }

    #[test]
    fn a_new_event_is_busy_and_has_no_url() {
        let event = Event::new("Review", 100, 200);
        assert_eq!(event.availability, Availability::Busy);
        assert_eq!(event.url, None);
    }

    #[test]
    fn a_saved_event_keeps_its_url_and_availability() {
        let mut store = store_with_full_access();
        let mut event = Event::new("Focus", 100, 200);
        event.url = Some("https://example.com/room".into());
        event.availability = Availability::Free;
        store.save_event(&mut event, Span::ThisEvent, true).unwrap();
        let fetched = store.events_between(0, 1000).unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].url.as_deref(), Some("https://example.com/room"));
        assert_eq!(fetched[0].availability, Availability::Free);
    }

    #[test]
    fn an_event_without_access_is_not_saved() {
        let mut store = EventStore::new(Access::new());
        let mut event = Event::new("Hidden", 100, 200);
        assert!(store.save_event(&mut event, Span::ThisEvent, true).is_err());
        assert!(store.events_between(0, 1000).is_err());
    }
}
