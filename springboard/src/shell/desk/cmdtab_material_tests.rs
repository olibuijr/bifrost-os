use super::*;

/// The Cmd+Tab capsule is the Search material (a translucent tint over the presenter's blur), not the menus'
/// nearly opaque flat plate: the wallpaper must show through it.
#[test]
fn the_cmd_tab_capsule_is_translucent_glass_not_a_flat_plate() {
    let mut sh = super::super::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::Files, 1.0);
    sh.switcher_cycle(false);
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut buf = vec![0u32; w * h];
    sh.draw_cmdtab(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
    let l = sh.cmdtab_layout(w as f32, h as f32).expect("the strip is open");
    let alpha = buf[(l.y + l.h / 2.0) as usize * w + (l.x + l.pad / 2.0) as usize] >> 24;
    assert!(alpha > 100 && alpha < 200, "the capsule fill is translucent (alpha {alpha}), not the menus' 240");
}
