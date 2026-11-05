use crate::shell::ui::Weight;

#[test]
fn the_bar_lays_out_from_cached_widths_the_same_way_twice() {
    let sh = crate::shell::tests::desktop_shell();
    let first = sh.menu_buttons();
    assert_eq!(first, sh.menu_buttons(), "a frame that lays the bar out again gets the same menus");
    assert_eq!(sh.measure_chrome("Control", 13.0, Weight::Semibold), sh.ui.measure("Control", 13.0, Weight::Semibold));
    assert_eq!(sh.fit_chrome("a very long network name indeed", 13.0, 40.0), sh.ui.fit("a very long network name indeed", 13.0, 40.0));
}
