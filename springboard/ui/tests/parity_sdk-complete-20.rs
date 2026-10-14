//! Parity sdk-complete-20: headless tests of the compose sheet's content (`bifrost_ui::compose`). The sheet can start
//! from a draft, such as the content another app shared into it. No test shows a sheet or posts anything.

use bifrost_ui::compose::{ComposeDraft, ComposeError, ComposeResult, ComposeSheet, SheetState, MAX_IMAGES};

#[test]
fn headless_a_sheet_starts_from_its_draft_with_the_draft_content() {
    let mut draft = ComposeDraft::new();
    draft.set_initial_text("hi");
    draft.add_url("https://example.test/a").expect("a web link");
    let sheet = ComposeSheet::with_draft(draft.clone());
    assert_eq!(sheet.draft(), &draft);
    assert_eq!(sheet.draft().text(), "hi");
    assert_eq!(sheet.draft().urls(), ["https://example.test/a"]);
    assert_eq!(sheet.state(), SheetState::Editing);
}

#[test]
fn headless_a_sheet_from_an_empty_draft_is_the_same_as_a_new_sheet() {
    assert_eq!(ComposeSheet::with_draft(ComposeDraft::new()), ComposeSheet::new());
}

#[test]
fn headless_a_sheet_from_a_draft_keeps_its_limits() {
    let mut draft = ComposeDraft::new();
    for i in 0..MAX_IMAGES {
        draft.add_image(&format!("img{i}.png")).expect("under the limit");
    }
    let mut sheet = ComposeSheet::with_draft(draft);
    assert_eq!(sheet.add_image("extra.png"), Err(ComposeError::Full(MAX_IMAGES)));
}

#[test]
fn headless_a_blank_sheet_from_a_draft_reports_cancelled_when_sent() {
    let mut sheet = ComposeSheet::with_draft(ComposeDraft::new());
    assert!(sheet.present());
    assert_eq!(sheet.finish(true), Some(ComposeResult::Cancelled), "a blank post is not sent");
    assert_eq!(sheet.result(), Some(ComposeResult::Cancelled));
}
