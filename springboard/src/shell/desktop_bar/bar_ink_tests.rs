use super::wallpaper_is_light;
use crate::gfx::{pack, View};

fn view(w: usize, h: usize, f: impl Fn(usize) -> u32) -> Vec<u32> {
    (0..w * h).map(|i| f(i / w)).collect()
}

#[test]
fn white_and_light_grey_wallpapers_get_dark_bar_text() {
    let px = view(64, 40, |_| pack(255, 255, 255, 255));
    assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    let px = view(64, 40, |_| pack(200, 200, 200, 255));
    assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
}

#[test]
fn dark_and_mid_grey_wallpapers_keep_white_bar_text() {
    let px = view(64, 40, |_| pack(0, 0, 0, 255));
    assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    let px = view(64, 40, |_| pack(128, 128, 128, 255));
    assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
}

#[test]
fn only_the_band_under_the_bar_counts() {
    // a light sky over a dark landscape: the bar sits on the sky
    let px = view(64, 40, |y| if y < 8 { pack(235, 225, 210, 255) } else { pack(10, 10, 10, 255) });
    assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    let px = view(64, 40, |y| if y < 8 { pack(10, 10, 10, 255) } else { pack(250, 250, 250, 255) });
    assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
}

#[test]
fn green_weighs_more_than_blue() {
    // perceived lightness, not a plain channel average: pure green is light, pure blue is dark
    let px = view(16, 8, |_| pack(0, 255, 0, 255));
    assert!(wallpaper_is_light(View { w: 16, h: 8, px: &px }, 8));
    let px = view(16, 8, |_| pack(0, 0, 255, 255));
    assert!(!wallpaper_is_light(View { w: 16, h: 8, px: &px }, 8));
}

#[test]
fn an_empty_wallpaper_is_not_light() {
    assert!(!wallpaper_is_light(View { w: 0, h: 0, px: &[] }, 8));
}
