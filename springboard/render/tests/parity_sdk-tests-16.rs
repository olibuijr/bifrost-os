//! Parity sdk-tests-16: headless tests for rendering intents with black point compensation, and for the buffer pool.
//! Every test runs on plain numbers and in-memory buffers. No display, device or clock is involved, and no colour
//! profile file is read.

use bifrost_render::buffer_pool::{Config, Pool, PoolError};
use bifrost_render::colorspace::{Space, Transform};
use bifrost_render::rendering_intent::{compensate_black, BlackPoints, Intent, IntentTransform};

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

// ---------------------------------------------------------------------------------------------------------------------
// Rendering intents (`rendering_intent`)
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_four_named_intents_are_listed_in_the_order_a_settings_page_shows() {
    assert_eq!(Intent::ALL.map(|i| i.name()), ["Perceptual", "Relative Colorimetric", "Saturation", "Absolute Colorimetric"]);
    assert_eq!(Intent::Default.name(), "Default");
}

#[test]
fn only_absolute_colorimetric_keeps_the_source_white_as_it_is() {
    assert!(Intent::Default.adapts_white());
    assert!(Intent::Perceptual.adapts_white());
    assert!(Intent::RelativeColorimetric.adapts_white());
    assert!(Intent::Saturation.adapts_white());
    assert!(!Intent::AbsoluteColorimetric.adapts_white());
}

#[test]
fn a_transform_reports_the_intent_it_was_built_with() {
    let t = IntentTransform::new(Space::Srgb, Space::DisplayP3, Intent::Saturation);
    assert_eq!(t.intent(), Intent::Saturation);
}

#[test]
fn relative_colorimetric_between_the_same_space_leaves_colours_unchanged() {
    let t = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric);
    for c in [[0.2, 0.5, 0.8], [0.9, 0.1, 0.4], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
        assert!(close(t.apply(c), c, 1e-9), "{c:?} became {:?}", t.apply(c));
    }
}

#[test]
fn relative_colorimetric_matches_the_plain_transform_between_two_spaces() {
    let plain = Transform::new(Space::Srgb, Space::DisplayP3);
    let relative = IntentTransform::new(Space::Srgb, Space::DisplayP3, Intent::RelativeColorimetric);
    for c in [[0.2, 0.5, 0.8], [0.9, 0.1, 0.4], [1.0, 1.0, 1.0]] {
        assert!(close(relative.apply(c), plain.apply(c), 1e-9), "{c:?}");
    }
}

#[test]
fn relative_colorimetric_keeps_white_white_between_white_points() {
    let t = IntentTransform::new(Space::Srgb, Space::RommRgb, Intent::RelativeColorimetric);
    assert!(close(t.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-9));
}

#[test]
fn absolute_colorimetric_does_not_map_the_source_white_to_a_neutral_white() {
    // The D65 white of the source is not the D50 white of the destination, so its channels differ.
    let t = IntentTransform::new(Space::Srgb, Space::RommRgb, Intent::AbsoluteColorimetric);
    let out = t.apply([1.0, 1.0, 1.0]);
    let spread = out.iter().cloned().fold(f64::MIN, f64::max) - out.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread > 0.01, "absolute white should not be neutral: {out:?}");
}

#[test]
fn perceptual_brings_a_wide_gamut_red_into_range_and_keeps_the_channel_order() {
    let t = IntentTransform::new(Space::DisplayP3, Space::Srgb, Intent::Perceptual);
    let out = t.apply([1.0, 0.0, 0.0]);
    assert!(out.iter().all(|c| (0.0..=1.0).contains(c)), "{out:?}");
    assert!(out[0] > out[1] && out[0] > out[2], "red stays the strongest channel: {out:?}");
    assert!(out[1] < out[2], "green stays the weakest of the other two: {out:?}");
}

#[test]
fn perceptual_leaves_colours_that_are_already_in_gamut_as_relative_colorimetric_does() {
    let perceptual = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::Perceptual);
    let relative = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric);
    for c in [[0.25, 0.5, 0.75], [0.0, 1.0, 0.5]] {
        assert!(close(perceptual.apply(c), relative.apply(c), 1e-9), "{c:?}");
    }
}

#[test]
fn saturation_clips_out_of_gamut_channels_to_the_edge_of_the_destination() {
    let t = IntentTransform::new(Space::DisplayP3, Space::Srgb, Intent::Saturation);
    assert_eq!(t.apply_u8([255, 0, 0]), [255, 0, 0]);
    let out = t.apply([1.0, 0.0, 0.0]);
    assert!(out[1].abs() < 1e-12 && out[2].abs() < 1e-12, "{out:?}");
}

#[test]
fn perceptual_and_saturation_keep_every_grid_colour_inside_the_destination_range() {
    let steps = [0.0, 0.5, 1.0];
    for intent in [Intent::Perceptual, Intent::Saturation] {
        let t = IntentTransform::new(Space::DisplayP3, Space::Srgb, intent);
        for r in steps {
            for g in steps {
                for b in steps {
                    let out = t.apply([r, g, b]);
                    assert!(out.iter().all(|c| (0.0..=1.0).contains(c)), "{intent:?} {:?} gave {out:?}", [r, g, b]);
                }
            }
        }
    }
}

#[test]
fn compensate_black_maps_the_source_black_to_the_destination_black_and_keeps_white() {
    let (src, dst) = (0.02, 0.05);
    assert!((compensate_black(src, src, dst) - dst).abs() < 1e-12);
    assert!((compensate_black(1.0, src, dst) - 1.0).abs() < 1e-12);
    assert!(compensate_black(0.0, 0.0, 0.0).abs() < 1e-12);
}

#[test]
fn compensate_black_with_a_full_source_black_maps_every_value_to_the_destination_black() {
    assert_eq!(compensate_black(0.3, 1.0, 0.1), 0.1);
    assert_eq!(compensate_black(0.9, 1.5, 0.1), 0.1);
}

#[test]
fn compensate_black_is_strictly_increasing_over_the_whole_range() {
    let mut last = f64::MIN;
    for step in 0..=100 {
        let v = compensate_black(step as f64 / 100.0, 0.02, 0.05);
        assert!(v > last, "step {step}");
        last = v;
    }
}

#[test]
fn black_point_compensation_lifts_black_to_the_destination_black_and_keeps_white() {
    let black = BlackPoints { source: 0.0, destination: 0.02 };
    let t = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric).with_black_points(black);
    let lifted = (Space::Srgb.from_linear(0.02) * 255.0).round() as u8;
    assert_eq!(t.apply_u8([0, 0, 0]), [lifted, lifted, lifted]);
    assert_eq!(t.apply_u8([255, 255, 255]), [255, 255, 255]);
}

#[test]
fn zero_black_points_leave_every_8_bit_colour_unchanged() {
    let t = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric)
        .with_black_points(BlackPoints { source: 0.0, destination: 0.0 });
    for c in [[0u8, 1, 128], [200, 17, 255]] {
        assert_eq!(t.apply_u8(c), c);
    }
}

#[test]
fn black_points_compare_by_both_values() {
    let a = BlackPoints { source: 0.0, destination: 0.02 };
    assert_eq!(a, BlackPoints { source: 0.0, destination: 0.02 });
    assert_ne!(a, BlackPoints { source: 0.0, destination: 0.0 });
}

// ---------------------------------------------------------------------------------------------------------------------
// Buffer pool (`buffer_pool`)
// ---------------------------------------------------------------------------------------------------------------------

fn config(min: usize, max_age: Option<u64>, threshold: Option<usize>) -> Config {
    Config { min_count: min, max_age, threshold }
}

#[test]
fn a_new_pool_holds_no_buffers() {
    let pool: Pool<u32> = Pool::new(config(0, None, None));
    assert_eq!((pool.live(), pool.free_count()), (0, 0));
}

#[test]
fn acquiring_from_an_empty_pool_makes_one_buffer_and_counts_it_as_live() {
    let mut pool: Pool<u32> = Pool::new(config(0, None, None));
    let mut made = 0u32;
    let buf = pool.acquire(|| {
        made += 1;
        made
    });
    assert_eq!(buf, Ok(1));
    assert_eq!((pool.live(), pool.free_count(), made), (1, 0, 1));
}

#[test]
fn a_released_buffer_is_handed_out_again_before_a_new_one_is_made() {
    let mut pool: Pool<&str> = Pool::new(config(0, None, None));
    let a = pool.acquire(|| "first").unwrap();
    pool.release(a, 0);
    assert_eq!(pool.free_count(), 1);
    assert_eq!(pool.acquire(|| "second"), Ok("first"));
    assert_eq!(pool.live(), 1);
}

#[test]
fn the_most_recently_released_buffer_is_handed_out_first() {
    let mut pool: Pool<&str> = Pool::new(config(0, None, None));
    let a = pool.acquire(|| "a").unwrap();
    let b = pool.acquire(|| "b").unwrap();
    pool.release(a, 1);
    pool.release(b, 2);
    assert_eq!(pool.acquire(|| "new"), Ok("b"));
    assert_eq!(pool.acquire(|| "new"), Ok("a"));
}

#[test]
fn a_threshold_refuses_a_buffer_past_the_limit_until_one_is_released() {
    let mut pool: Pool<u32> = Pool::new(config(0, None, Some(2)));
    let mut next = 0u32;
    let mut make = || {
        next += 1;
        next
    };
    let a = pool.acquire(&mut make).unwrap();
    let _b = pool.acquire(&mut make).unwrap();
    assert_eq!(pool.acquire(&mut make), Err(PoolError::WouldExceedThreshold));
    pool.release(a, 1);
    assert_eq!(pool.acquire(&mut make), Ok(a));
    assert_eq!(pool.live(), 2);
}

#[test]
fn a_free_buffer_is_kept_at_its_maximum_age_and_dropped_one_tick_later() {
    let mut pool: Pool<u32> = Pool::new(config(0, Some(10), None));
    let x = pool.acquire(|| 7).unwrap();
    pool.release(x, 0);
    assert_eq!(pool.age(10), 0, "exactly at the limit it stays");
    assert_eq!(pool.age(11), 1, "one tick past the limit it goes");
    assert_eq!((pool.live(), pool.free_count()), (0, 0));
}

#[test]
fn ageing_never_drops_a_pool_below_its_minimum_count() {
    let mut pool: Pool<u32> = Pool::new(config(1, Some(10), None));
    assert_eq!(pool.prewarm(0, || 7), 1);
    assert_eq!(pool.age(100), 0);
    assert_eq!((pool.live(), pool.free_count()), (1, 1));
}

#[test]
fn ageing_drops_the_oldest_free_buffer_first_and_keeps_the_newer_one() {
    let mut pool: Pool<&str> = Pool::new(config(0, Some(10), None));
    let a = pool.acquire(|| "a").unwrap();
    let b = pool.acquire(|| "b").unwrap();
    pool.release(a, 0);
    pool.release(b, 5);
    assert_eq!(pool.age(12), 1, "a has been idle 12 ticks, b only 7");
    assert_eq!(pool.live(), 1);
    assert_eq!(pool.acquire(|| "new"), Ok("b"));
}

#[test]
fn without_a_maximum_age_no_free_buffer_is_ever_dropped_by_ageing() {
    let mut pool: Pool<u32> = Pool::new(config(0, None, None));
    let x = pool.acquire(|| 1).unwrap();
    pool.release(x, 0);
    assert_eq!(pool.age(u64::MAX), 0);
    assert_eq!(pool.free_count(), 1);
}

#[test]
fn prewarming_creates_the_minimum_and_stops_at_the_threshold() {
    let mut pool: Pool<u32> = Pool::new(config(2, None, None));
    assert_eq!(pool.prewarm(0, || 0), 2);
    assert_eq!((pool.live(), pool.free_count()), (2, 2));
    assert_eq!(pool.prewarm(0, || 0), 0, "already at the minimum");

    let mut capped: Pool<u32> = Pool::new(config(3, None, Some(2)));
    assert_eq!(capped.prewarm(0, || 0), 2);
    assert_eq!(capped.live(), 2);
}

#[test]
fn flushing_drops_every_free_buffer_and_keeps_the_buffers_in_use() {
    let mut pool: Pool<u32> = Pool::new(config(0, None, None));
    let mut next = 0u32;
    let mut make = || {
        next += 1;
        next
    };
    let in_use = pool.acquire(&mut make).unwrap();
    let b = pool.acquire(&mut make).unwrap();
    let c = pool.acquire(&mut make).unwrap();
    pool.release(b, 0);
    pool.release(c, 0);
    assert_eq!(pool.live(), 3);
    assert_eq!(pool.flush(), 2);
    assert_eq!((pool.live(), pool.free_count()), (1, 0));
    pool.release(in_use, 1);
    assert_eq!(pool.free_count(), 1);
}
