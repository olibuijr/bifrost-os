//! Parity sdk-complete-40: headless tests for the text layout inputs (the legacy kern table, OpenType feature
//! settings and script detection) and the frame interpolator's stream state. No test draws a frame to a display.

use bifrost_render::frame_interp::{FrameError, FrameInterpolator};
use bifrost_render::kern::Kern;
use bifrost_render::shape_ot::{detect_script, tag, Features};

const HORIZONTAL: u16 = 0x0001;

/// A `kern` table with one format 0 horizontal subtable holding `pairs` (left glyph, right glyph, value in units).
fn kern_table(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
    let length = 6 + 8 + 6 * pairs.len();
    let mut out: Vec<u8> = vec![0, 0, 0, 1];
    out.extend([0, 0]);
    out.extend((length as u16).to_be_bytes());
    out.extend(HORIZONTAL.to_be_bytes());
    out.extend((pairs.len() as u16).to_be_bytes());
    out.extend([0; 6]);
    for (l, r, v) in pairs {
        out.extend(l.to_be_bytes());
        out.extend(r.to_be_bytes());
        out.extend(v.to_be_bytes());
    }
    out
}

#[test]
fn a_kern_table_tightens_only_the_pairs_it_lists() {
    let kern = Kern::parse(&kern_table(&[(36, 37, -60)])).expect("a version 0 table");
    assert_eq!(kern.pair(36, 37), -60);
    assert_eq!(kern.pair(37, 36), 0, "the pair is ordered: left then right");
}

#[test]
fn a_run_of_glyphs_gets_one_adjustment_per_gap_scaled_to_pixels() {
    let kern = Kern::parse(&kern_table(&[(1, 2, -500)])).expect("a version 0 table");
    let adjustments = kern.run_adjustments(&[1, 2, 3], 20.0, 1000.0);
    assert_eq!(adjustments.len(), 3);
    assert!((adjustments[0] + 10.0).abs() < 1e-6, "-500 units at 20 px on a 1000 unit em is -10 px");
    assert_eq!(adjustments[1], 0.0);
    assert_eq!(adjustments[2], 0.0, "the last glyph has no following gap");
}

#[test]
fn a_kern_table_of_another_version_is_refused() {
    let mut table = kern_table(&[(1, 2, -5)]);
    table[1] = 1;
    assert!(Kern::parse(&table).is_none());
}

#[test]
fn feature_settings_parse_on_off_and_values() {
    let f = Features::parse("liga=0, ss01, -kern, smcp=2");
    assert_eq!(f.value(tag("liga")), Some(0));
    assert_eq!(f.value(tag("ss01")), Some(1), "a bare tag is on");
    assert_eq!(f.value(tag("kern")), Some(0), "a minus sign turns the feature off");
    assert_eq!(f.value(tag("smcp")), Some(2), "a number selects an alternate");
    assert_eq!(f.value(tag("onum")), None, "a feature not listed keeps its default");
}

#[test]
fn a_feature_tag_is_padded_with_spaces_to_four_bytes() {
    assert_eq!(tag("ss1"), *b"ss1 ");
    assert_eq!(tag("ss01"), *b"ss01");
    assert_eq!(tag("longertag"), *b"long");
}

#[test]
fn setting_a_feature_twice_keeps_the_last_value() {
    let mut f = Features::new();
    f.on("liga").off("liga");
    assert_eq!(f.value(tag("liga")), Some(0));
    assert_eq!(f.items().len(), 1);
}

#[test]
fn the_script_follows_the_first_letter_and_defaults_to_latin() {
    assert_eq!(detect_script("Hello"), *b"latn");
    assert_eq!(detect_script("123 !?"), *b"latn");
    assert_eq!(detect_script("\u{644}\u{645}\u{631}\u{62D}\u{628}\u{627}"), *b"arab");
    assert_eq!(detect_script("\u{3053}\u{3093}\u{306B}"), *b"kana");
    assert_eq!(detect_script("\u{3C0}"), *b"grek");
}

#[test]
fn the_interpolator_needs_two_frames_before_it_builds_one() {
    let mut interp = FrameInterpolator::new(2, 2).expect("a 2 by 2 stream");
    assert_eq!(interp.between(0.5, None), Ok(None));
    interp.push(vec![[0.0; 3]; 4]).unwrap();
    assert_eq!(interp.between(0.5, None), Ok(None), "one frame is not a pair");
    interp.push(vec![[1.0; 3]; 4]).unwrap();
    let mid = interp.between(0.5, None).unwrap().expect("two frames make a pair");
    assert!(mid.iter().all(|p| (p[0] - 0.5).abs() < 1e-6));
}

#[test]
fn each_new_frame_moves_the_pair_forward() {
    let mut interp = FrameInterpolator::new(1, 1).expect("a 1 by 1 stream");
    interp.push(vec![[0.0; 3]]).unwrap();
    interp.push(vec![[1.0; 3]]).unwrap();
    interp.push(vec![[0.5; 3]]).unwrap();
    let t0 = interp.between(0.0, None).unwrap().expect("a pair");
    assert_eq!(t0, vec![[1.0; 3]], "t = 0 is the frame before the newest one");
    let t1 = interp.between(1.0, None).unwrap().expect("a pair");
    assert_eq!(t1, vec![[0.5; 3]], "t = 1 is the newest frame");
}

#[test]
fn a_reset_at_a_scene_cut_starts_a_new_pair() {
    let mut interp = FrameInterpolator::new(1, 1).expect("a 1 by 1 stream");
    interp.push(vec![[0.0; 3]]).unwrap();
    interp.push(vec![[1.0; 3]]).unwrap();
    interp.reset();
    assert_eq!(interp.between(0.5, None), Ok(None));
    interp.push(vec![[0.2; 3]]).unwrap();
    assert_eq!(interp.between(0.5, None), Ok(None), "one frame after the cut is not a pair");
}

#[test]
fn the_interpolator_refuses_a_zero_size_and_a_frame_of_the_wrong_length() {
    assert_eq!(FrameInterpolator::new(0, 4).err(), Some(FrameError::ZeroSize));
    let mut interp = FrameInterpolator::new(2, 2).expect("a 2 by 2 stream");
    assert_eq!(interp.push(vec![[0.0; 3]; 3]), Err(FrameError::LengthMismatch));
    interp.push(vec![[0.0; 3]; 4]).unwrap();
    interp.push(vec![[1.0; 3]; 4]).unwrap();
    assert_eq!(interp.between(1.5, None), Err(FrameError::TimeOutOfRange));
}
