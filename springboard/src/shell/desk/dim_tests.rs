use super::{dim_rect, BACKGROUND_DIM};

/// A window that is not in front recedes by darkening its colour, never by becoming see-through: the alpha of every
/// pixel (clear where an app leaves a material region for the window server, a faint tint, opaque) is untouched.
#[test]
fn dimming_a_background_window_never_changes_how_opaque_a_pixel_is() {
    let mut px = vec![0x0000_0000u32, 0x2010_2030, 0x80ff_ffff, 0xff80_80ff];
    let before: Vec<u32> = px.iter().map(|p| p >> 24).collect();
    dim_rect(&mut px, 4, 0, 0, 4, 1, BACKGROUND_DIM);
    let after: Vec<u32> = px.iter().map(|p| p >> 24).collect();
    assert_eq!(before, after);
    assert_eq!(after, vec![0x00, 0x20, 0x80, 0xff]);
}

#[test]
fn dims_only_the_rectangle_and_keeps_alpha() {
    let mut px = vec![0xff80_80ffu32; 16];
    dim_rect(&mut px, 4, 1, 1, 3, 3, 51); // 20 %
    assert_eq!(px[0], 0xff80_80ff, "outside is untouched");
    assert_eq!(px[1 * 4 + 1], 0xff66_66cc, "inside is 80 % of the channels, alpha stays");
    assert_eq!(px[2 * 4 + 2], 0xff66_66cc);
    assert_eq!(px[3 * 4 + 3], 0xff80_80ff);
}

#[test]
fn a_rectangle_past_the_buffer_is_clipped_not_a_panic() {
    let mut px = vec![0xffff_ffffu32; 8];
    dim_rect(&mut px, 4, 0, 0, 9, 9, 255);
    assert!(px.iter().all(|&p| p == 0xff00_0000));
}
