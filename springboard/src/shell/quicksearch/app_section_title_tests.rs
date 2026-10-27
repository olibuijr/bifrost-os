use super::app_section_title;

#[test]
fn an_app_section_is_titled_by_the_app_name_and_the_name_is_kept_once() {
    let a = app_section_title("Notes");
    assert_eq!(a, "Notes");
    assert!(std::ptr::eq(a, app_section_title("Notes")), "the same name is kept once");
    assert_eq!(app_section_title("Mail"), "Mail");
}
