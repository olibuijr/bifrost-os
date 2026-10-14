//! Parity tests for the wire types of the now-playing playback snapshot and the shared payload types (task
//! sdk-tests-01b). They are headless host tests: pure encoding and naming, with no socket, device or clock.

// ---- now-playing: playback state snapshot (bifrost_proto::svc::audio) ----

use bifrost_proto::svc::audio::{Evt, Live, Repeat, Snapshot, Track};

fn snapshot() -> Snapshot {
    Snapshot {
        track: Some(Track { id: 7, path: "/music/a.flac".into(), url: None }),
        index: Some(2),
        playing: true,
        position_ms: 1250.0,
        duration_ms: Some(5000.0),
        shuffle: true,
        repeat: Repeat::All,
        queue_len: 3,
        volume: 0.5,
        muted: false,
        error: None,
        live: None,
    }
}

#[test]
fn headless_playback_snapshot_round_trips_through_the_audio_protocol() {
    let state = Evt::State(snapshot());
    let decoded = Evt::decode(&state.encode()).expect("the snapshot decodes");
    assert_eq!(decoded, state, "every snapshot field survives the wire");
}

#[test]
fn headless_playback_snapshot_carries_the_service_state_without_artwork_or_app_metadata() {
    let mut s = snapshot();
    s.live = Some(Live { phase: 1, title: Some("Radio".into()), error: None });
    let decoded = Evt::decode(&Evt::State(s.clone()).encode()).expect("a live snapshot decodes");
    match decoded {
        Evt::State(back) => {
            assert_eq!(back.live, s.live, "the live phase and title are carried");
            assert_eq!(back.track.as_ref().map(|t| t.path.as_str()), Some("/music/a.flac"), "the track path is the only track text");
            assert_eq!(back.repeat, Repeat::All);
        }
        other => panic!("expected a state event, got {other:?}"),
    }
}

// ---- uniform type identifiers: the type of a shared payload (bifrost_proto::rich::Payload) ----

use bifrost_proto::rich::Payload;

#[test]
fn headless_payload_types_text_and_urls_have_fixed_type_identifiers() {
    assert_eq!(Payload::Text("hi".into()).uti(), "public.utf8-plain-text");
    assert_eq!(Payload::Url("https://example.com".into()).uti(), "public.url");
}

#[test]
fn headless_payload_types_a_file_takes_the_type_of_its_extension() {
    let png = Payload::File { grant: [0; 16], name: "photo.png".into() };
    assert_eq!(png.uti(), "public.png");
    let unknown = Payload::File { grant: [0; 16], name: "notes.unknownext".into() };
    assert_eq!(unknown.uti(), "public.data", "an unknown extension is generic data");
}
