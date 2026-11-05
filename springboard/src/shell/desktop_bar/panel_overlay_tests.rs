#[test]
fn a_panel_overlay_covers_the_panel_and_its_shadow() {
    assert_eq!(super::panel_overlay_rect((100.,50.,200.,300.),10.),(90,40,220,320));
    assert_eq!(super::panel_overlay_rect((5.,5.,20.,20.),10.),(0,0,35,35),"clipped at the top-left corner");
}

/// E2E NC2: the column opens under the menu bar, and its overlay surface (shadow margin included) starts there too,
/// so the bar's status items and clock stay visible and take their clicks.
#[cfg(feature="desktop-wayland")]
#[test]
fn a_panel_overlay_starts_under_the_menu_bar() {
    let mut sh=crate::shell::tests::desktop_shell();
    sh.anim=None;
    // the fixture starts on the lock screen, where the column does not open
    sh.screen=crate::shell::Screen::Home;
    sh.open_desk_notification_center(1.0);
    let bar=sh.desktop_bar_height();
    let overlays=sh.desk_panel_overlays();
    let column=overlays.iter().find(|o|o.0==5).expect("the open column is an overlay surface");
    assert!(column.2>=bar,"the surface starts at y {} under the {bar} px bar",column.2);
    assert!(column.4>0,"and has height");
    let top=sh.center_desk_rect().expect("the column is open").1;
    assert!(top>=bar as f32,"the column itself is under the bar");
}
