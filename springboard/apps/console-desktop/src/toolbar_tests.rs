//! View > Hide Toolbar, tested on the window's state with the test context's kit `Ui`: the toolbar's controls go away with
//! it, the search field gives up its focus, and showing the toolbar again brings the controls back.

use crate::console::Console;
use crate::services::test_ctx;

#[test]
fn hiding_the_toolbar_removes_its_controls() {
    let ui = test_ctx().ui;
    let mut c = Console::new(900.0, 600.0, Vec::new());
    assert!(!c.controls(&ui).is_empty(), "the toolbar shows its items by default");
    c.toggle_toolbar();
    assert!(!c.show_toolbar);
    assert!(c.controls(&ui).is_empty(), "a hidden toolbar offers no controls");
}

#[test]
fn hiding_the_toolbar_gives_up_the_search_focus() {
    let mut c = Console::new(900.0, 600.0, Vec::new());
    c.focus_search();
    assert!(c.search_focused);
    c.toggle_toolbar();
    assert!(!c.search_focused, "the search field is not shown, so it cannot keep the keyboard");
}

#[test]
fn showing_the_toolbar_again_brings_its_controls_back() {
    let ui = test_ctx().ui;
    let mut c = Console::new(900.0, 600.0, Vec::new());
    c.toggle_toolbar();
    c.toggle_toolbar();
    assert!(c.show_toolbar);
    assert!(!c.controls(&ui).is_empty());
}
