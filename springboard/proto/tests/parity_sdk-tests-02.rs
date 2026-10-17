//! Parity sdk-tests-02: headless tests of the playback command set on the audio service wire
//! (`bifrost_proto::svc::audio`). Each transport command and the state event round trip through their bytes; no
//! audio device, socket or service process is involved.

use bifrost_proto::svc::audio::{Cmd, Evt, Repeat, Snapshot, Track};
use bifrost_proto::svc::Enc;
use bifrost_proto::DecodeError;

fn round_trip(cmd: &Cmd) -> Result<Cmd, DecodeError> {
    Cmd::decode(&cmd.encode())
}

#[test]
fn headless_each_transport_command_round_trips_through_its_wire_form() {
    let commands = vec![
        Cmd::Ping,
        Cmd::Subscribe,
        Cmd::Play,
        Cmd::Pause,
        Cmd::Toggle,
        Cmd::Next,
        Cmd::Prev,
        Cmd::JumpTo(3),
        Cmd::SeekMs(1234.5),
        Cmd::SetShuffle { on: true, seed: 9 },
        Cmd::SetRepeat(Repeat::One),
        Cmd::SetVolume(0.25),
        Cmd::SetRate(0.5),
        Cmd::SetMuted(true),
        Cmd::Stop,
    ];
    for cmd in commands {
        assert_eq!(round_trip(&cmd), Ok(cmd.clone()), "{cmd:?} changed on the wire");
    }
}

#[test]
fn headless_each_transport_command_has_its_own_bytes() {
    let commands = [Cmd::Play, Cmd::Pause, Cmd::Toggle, Cmd::Next, Cmd::Prev, Cmd::Stop];
    let encoded: Vec<Vec<u8>> = commands.iter().map(Cmd::encode).collect();
    for (i, a) in encoded.iter().enumerate() {
        for b in &encoded[i + 1..] {
            assert_ne!(a, b, "two transport commands share a wire form");
        }
    }
}

#[test]
fn headless_a_play_queue_keeps_its_order_urls_and_start_index() {
    let queue = Cmd::PlayQueue {
        tracks: vec![
            Track { id: 7, path: "/music/a.mp3".into(), url: None },
            Track { id: 9, path: String::new(), url: Some("https://radio.example/live".into()) },
            Track { id: 11, path: "/music/c.flac".into(), url: None },
        ],
        start: 2,
        shuffle: false,
        seed: 0,
    };
    assert_eq!(round_trip(&queue), Ok(queue.clone()));
    if let Ok(Cmd::PlayQueue { tracks, start, .. }) = round_trip(&queue) {
        let ids: Vec<u64> = tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![7, 9, 11]);
        assert_eq!(start, 2);
    } else {
        panic!("a play queue must decode as a play queue");
    }
}

#[test]
fn headless_a_seek_keeps_its_fractional_milliseconds() {
    assert_eq!(round_trip(&Cmd::SeekMs(0.5)), Ok(Cmd::SeekMs(0.5)));
    assert_eq!(round_trip(&Cmd::SeekMs(-0.25)), Ok(Cmd::SeekMs(-0.25)));
}

#[test]
fn headless_an_unknown_command_tag_is_refused_with_its_number() {
    let bytes = Enc::tag(999).done();
    assert_eq!(Cmd::decode(&bytes), Err(DecodeError::UnknownTag(999)));
}

#[test]
fn headless_a_repeat_value_outside_the_three_modes_is_refused() {
    let bytes = Enc::tag(12).u8(7).done();
    assert_eq!(Cmd::decode(&bytes), Err(DecodeError::BadValue));
    let all = Enc::tag(12).u8(1).done();
    assert_eq!(Cmd::decode(&all), Ok(Cmd::SetRepeat(Repeat::All)));
}

#[test]
fn headless_the_state_event_carries_the_playback_fields() {
    let state = Evt::State(Snapshot {
        track: Some(Track { id: 7, path: "/music/a.mp3".into(), url: None }),
        index: Some(0),
        playing: true,
        position_ms: 42.5,
        duration_ms: Some(180_000.0),
        shuffle: true,
        repeat: Repeat::All,
        queue_len: 3,
        volume: 0.5,
        muted: false,
        error: None,
        live: None,
        rate: 2.0,
    });
    assert_eq!(Evt::decode(&state.encode()), Ok(state.clone()));
}
