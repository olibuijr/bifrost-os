use super::*;

#[test]
fn cached_shadow_matches_cpu_layers_across_focus_clipping_and_alpha() {
    let ui = super::super::tests::ui();
    let (width, height) = (500usize, 380usize);
    for focused in [false, true] {
        for background_color in [0xff34_5678, 0] {
            let y0 = 12usize;
            let (x, y, w, h, rad) = (-8.0, 8.0, 280.0, 180.0, ui.pt(17.0));
            let background = vec![background_color; width * (height - y0)];
            let mut expected = background.clone();
            let mut actual = background;
            let mut expected_surface = Surface { w: width, h: height, stride: width, px: &mut expected, y0 };
            let mut actual_surface = Surface { w: width, h: height, stride: width, px: &mut actual, y0 };
            draw_window_shadow_layers(&mut expected_surface, &ui, x, y, w, h, rad, focused);
            cached_window_shadow(&mut actual_surface, &ui, x, y, w, h, rad, focused);
            let max_delta = expected.iter().zip(&actual).flat_map(|(a, b)| (0..4).map(move |shift| {
                ((((a >> (shift * 8)) & 0xff) as i16) - (((b >> (shift * 8)) & 0xff) as i16)).abs()
            })).max().unwrap_or(0);
            assert!(max_delta <= 2, "focus={focused}, background={background_color:#010x}: channel delta {max_delta}");
        }
    }
}
