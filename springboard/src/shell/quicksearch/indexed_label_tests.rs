use super::indexed_label;

#[test]
fn an_indexed_result_shows_its_content_type_after_the_title() {
    assert_eq!(indexed_label("Q1 report", "PDF document"), "Q1 report · PDF document");
    assert_eq!(indexed_label("Notes", ""), "Notes");
}
