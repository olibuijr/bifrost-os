//! Parity sdk-complete-32: headless tests for loudness normalization (`springboard::media::volume`). Pure arithmetic:
//! no audio device, no clock and no file is read.

use springboard::media::volume::{
    normalization_gain_db, normalized_linear, Volume, NORMALIZATION_MAX_BOOST_DB, NORMALIZATION_TARGET_DB,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_track_at_the_target_needs_no_gain() {
    assert!(close(normalization_gain_db(-16.0, -16.0, 6.0), 0.0));
}

#[test]
fn a_loud_track_is_turned_down_by_the_full_difference() {
    assert!(close(normalization_gain_db(-10.0, -16.0, 6.0), -6.0));
    assert!(close(normalization_gain_db(-4.0, -16.0, 6.0), -12.0), "the cut is not limited");
}

#[test]
fn a_quiet_track_is_raised_but_never_by_more_than_the_boost_limit() {
    assert!(close(normalization_gain_db(-19.0, -16.0, 6.0), 3.0));
    assert!(close(normalization_gain_db(-30.0, -16.0, 6.0), 6.0), "14 dB needed, 6 dB allowed");
}

#[test]
fn a_silent_or_unmeasured_track_is_left_alone() {
    assert!(close(normalization_gain_db(f32::NEG_INFINITY, -16.0, 6.0), 0.0));
    assert!(close(normalization_gain_db(f32::NAN, -16.0, 6.0), 0.0));
}

#[test]
fn a_negative_boost_limit_is_read_as_no_boost_and_still_allows_cuts() {
    assert!(close(normalization_gain_db(-30.0, -16.0, -3.0), 0.0), "a quiet track gets no boost");
    assert!(close(normalization_gain_db(-10.0, -16.0, -3.0), -6.0));
}

#[test]
fn the_published_defaults_aim_at_minus_sixteen_with_six_db_of_boost() {
    assert!(close(NORMALIZATION_TARGET_DB, -16.0));
    assert!(close(NORMALIZATION_MAX_BOOST_DB, 6.0));
}

#[test]
fn a_gain_of_minus_six_db_halves_the_full_volume_multiplier() {
    let full = Volume::new(1.0);
    assert!(close(normalized_linear(full, 0.0), 1.0));
    assert!(close(normalized_linear(full, -6.0), 0.501_187), "about half amplitude");
}

#[test]
fn muted_stays_silent_whatever_the_normalization_gain() {
    assert_eq!(normalized_linear(Volume::new(0.0), 6.0), 0.0);
}

#[test]
fn the_volume_and_the_normalization_multiply() {
    let half = Volume::at_step(8);
    let combined = normalized_linear(half, -6.0);
    assert!(close(combined, half.linear() * 0.501_187));
}

mod loop_regions {
    use springboard::media::loop_region::{Head, LoopMode, LoopRegion};

    #[test]
    fn a_loop_region_needs_at_least_one_frame() {
        assert!(LoopRegion::new(10, 10, LoopMode::Forward).is_none());
        assert_eq!(LoopRegion::new(4, 10, LoopMode::PingPong).map(|r| r.end()), Some(10));
    }

    #[test]
    fn a_forward_loop_on_the_sdk_path_wraps_inside_the_region() {
        let r = LoopRegion::new(2, 6, LoopMode::Forward).unwrap();
        let head = Head::Playing { position: 4, backward: false };
        assert_eq!(r.advance(head, 3, 100), Head::Playing { position: 3, backward: false });
    }

    #[test]
    fn a_one_shot_stops_at_the_sample_end() {
        let r = LoopRegion::new(2, 6, LoopMode::Off).unwrap();
        let head = r.advance(r.head_at_start(), 9, 8);
        assert_eq!(head, Head::Done);
    }

    #[test]
    fn a_ping_pong_loop_reverses_at_the_region_end() {
        let r = LoopRegion::new(0, 4, LoopMode::PingPong).unwrap();
        let head = r.advance(Head::Playing { position: 3, backward: false }, 1, 100);
        assert_eq!(head, Head::Playing { position: 3, backward: true });
    }
}

mod engine_registry {
    use springboard::media::engine::{AssetRegistry, Engine, EngineError, UpdateMode};
    use springboard::media::loop_region::LoopMode;

    #[test]
    fn an_asset_is_registered_once_and_listed_in_name_order() {
        let mut registry = AssetRegistry::new();
        registry.register("snare", "drums/snare.wav", LoopMode::Off).unwrap();
        registry.register("bass", "bass/loop.wav", LoopMode::Forward).unwrap();
        assert_eq!(registry.names().collect::<Vec<_>>(), vec!["bass", "snare"]);
        assert_eq!(registry.get("bass").map(|a| a.loop_mode), Some(LoopMode::Forward));
        assert_eq!(
            registry.register("snare", "other.wav", LoopMode::Off),
            Err(EngineError::DuplicateAsset("snare".into()))
        );
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn an_asset_needs_a_name_and_a_path() {
        let mut registry = AssetRegistry::new();
        assert_eq!(registry.register("  ", "a.wav", LoopMode::Off), Err(EngineError::EmptyName));
        assert_eq!(registry.register("a", "", LoopMode::Off), Err(EngineError::EmptyPath));
        assert!(registry.is_empty());
    }

    #[test]
    fn unregistering_returns_the_asset_and_an_unknown_name_is_refused() {
        let mut registry = AssetRegistry::new();
        registry.register("click", "ui/click.wav", LoopMode::Off).unwrap();
        let removed = registry.unregister("click").unwrap();
        assert_eq!(removed.path, "ui/click.wav");
        assert_eq!(registry.unregister("click"), Err(EngineError::UnknownAsset("click".into())));
    }

    #[test]
    fn the_update_mode_is_automatic_until_an_app_changes_it() {
        let mut engine = Engine::new();
        assert_eq!(engine.update_mode(), UpdateMode::Automatic);
        engine.set_update_mode(UpdateMode::Manual).unwrap();
        assert_eq!(engine.update_mode(), UpdateMode::Manual);
    }

    #[test]
    fn the_update_mode_is_locked_while_the_engine_runs() {
        let mut engine = Engine::new();
        engine.start().unwrap();
        assert_eq!(engine.set_update_mode(UpdateMode::Manual), Err(EngineError::ModeLockedWhileRunning));
        assert_eq!(engine.update_mode(), UpdateMode::Automatic);
        engine.stop().unwrap();
        assert!(engine.set_update_mode(UpdateMode::Manual).is_ok());
    }

    #[test]
    fn start_and_stop_refuse_a_second_call_in_the_same_state() {
        let mut engine = Engine::new();
        assert_eq!(engine.stop(), Err(EngineError::NotRunning));
        engine.start().unwrap();
        assert_eq!(engine.start(), Err(EngineError::AlreadyRunning));
        assert!(engine.is_running());
    }

    #[test]
    fn playing_needs_a_running_engine_and_a_registered_name() {
        let mut engine = Engine::new();
        engine.assets_mut().register("hit", "fx/hit.wav", LoopMode::Off).unwrap();
        assert_eq!(engine.play("hit").map(|_| ()), Err(EngineError::NotRunning));
        engine.start().unwrap();
        assert_eq!(engine.play("hit").map(|a| a.path.clone()), Ok("fx/hit.wav".to_string()));
        assert_eq!(engine.play("miss").map(|_| ()), Err(EngineError::UnknownAsset("miss".into())));
    }

    #[test]
    fn every_engine_error_has_a_readable_message() {
        assert_eq!(EngineError::NotRunning.to_string(), "the engine is not running");
        assert_eq!(
            EngineError::DuplicateAsset("hit".into()).to_string(),
            "an asset named hit is already registered"
        );
    }
}
