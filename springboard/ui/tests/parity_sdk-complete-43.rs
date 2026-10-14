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
