//! Parity sdk-complete-09: headless tests for the image catalogue, template tinting, file icons and colour list order.
//! No test draws to a screen or touches a device.

use bifrost_app::color_list::ColorList;
use bifrost_app::graphics_color::Color;
use bifrost_app::image_object::{tint_template, ImageCatalog, ImageObject, ImageRep};
use bifrost_app::workspace::{icon_for_content_type, icon_for_file};

fn named_image(name: &str) -> ImageObject {
    let mut image = ImageObject::named(name);
    image.add_rep(ImageRep::bitmap_at_72_dpi(16, 16).unwrap());
    image
}

#[test]
fn a_catalogue_finds_a_registered_image_by_its_name() {
    let mut catalog = ImageCatalog::new();
    assert!(catalog.register(named_image("Logo")));
    assert_eq!(catalog.named("Logo").and_then(ImageObject::name), Some("Logo"));
    assert!(catalog.named("logo").is_none(), "names are matched exactly");
}

#[test]
fn a_catalogue_refuses_an_unnamed_image_and_a_second_image_with_the_same_name() {
    let mut catalog = ImageCatalog::new();
    assert!(!catalog.register(ImageObject::new()), "an image without a name cannot be looked up");
    assert!(catalog.register(named_image("Logo")));
    assert!(!catalog.register(named_image("Logo")));
    assert_eq!(catalog.names(), vec!["Logo"]);
}

#[test]
fn a_catalogue_lists_names_in_registration_order_and_removes_one() {
    let mut catalog = ImageCatalog::new();
    catalog.register(named_image("Zoom"));
    catalog.register(named_image("Apple"));
    assert_eq!(catalog.names(), vec!["Zoom", "Apple"]);
    assert!(catalog.remove("Zoom"));
    assert!(!catalog.remove("Zoom"));
    assert_eq!(catalog.names(), vec!["Apple"]);
}

#[test]
fn a_template_keeps_the_source_alpha_and_takes_the_tint_colour() {
    let tint = 0xff10_2030;
    let source = [0x00ff_ffff, 0x80ff_ffff, 0xffff_ffff];
    assert_eq!(tint_template(&source, tint), vec![0x0010_2030, 0x8010_2030, 0xff10_2030]);
}

#[test]
fn a_template_is_scaled_by_the_tint_alpha_in_pixel_terms() {
    // A half-transparent tint halves the opaque source pixels: 255 * 128 / 255 = 128.
    let tint = 0x8010_2030;
    assert_eq!(tint_template(&[0xffff_ffff], tint), vec![0x8010_2030]);
    assert!(tint_template(&[], tint).is_empty());
}

#[test]
fn a_file_icon_follows_the_content_type_of_its_extension() {
    assert_eq!(icon_for_file("report.pdf"), "doc.richtext");
    assert_eq!(icon_for_file("song.mp3"), "music.note");
    assert_eq!(icon_for_file("clip.mp4"), "film");
    assert_eq!(icon_for_file("bundle.zip"), "archivebox");
    assert_eq!(icon_for_file("notes.txt"), "doc.text");
}

#[test]
fn a_file_with_an_unknown_extension_gets_the_plain_document_glyph() {
    assert_eq!(icon_for_file("mystery.zzz"), "doc");
    assert_eq!(icon_for_file("no-extension"), "doc");
}

#[test]
fn a_folder_content_type_gets_the_folder_glyph() {
    assert_eq!(icon_for_content_type("public.folder"), "folder");
    assert_eq!(icon_for_content_type("public.unknown-type"), "doc");
}

fn red() -> Color {
    Color::from_rgba8(255, 0, 0, 255)
}

fn blue() -> Color {
    Color::from_rgba8(0, 0, 255, 255)
}

#[test]
fn a_colour_moves_to_a_new_position_in_its_list() {
    let mut list = ColorList::new("Brand");
    list.set_color("a", red());
    list.set_color("b", blue());
    list.set_color("c", red());
    assert!(list.move_color("c", 0));
    assert_eq!(list.keys(), vec!["c", "a", "b"]);
    assert!(list.move_color("c", 2));
    assert_eq!(list.keys(), vec!["a", "b", "c"]);
}

#[test]
fn a_colour_move_refuses_a_missing_key_or_a_position_past_the_end() {
    let mut list = ColorList::new("Brand");
    list.set_color("a", red());
    list.set_color("b", blue());
    assert!(!list.move_color("missing", 0));
    assert!(!list.move_color("a", 2), "the list holds two colours, so position 2 does not exist");
    assert_eq!(list.keys(), vec!["a", "b"]);
}
