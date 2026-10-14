//! Parity sdk-rest-16: headless tests for intent schema domains and visual presentation, and for the system audio service
//! client. The audio tests talk over an in-memory socket pair; no service, device or socket path on disk is used.

use bifrost_app::intent_presentation::{Field, Presentation, PresentationError, SchemaDomain, MAX_SUBTITLE, MAX_TITLE};
use bifrost_app::system_audio::{self, AudioClient, AudioError, Command, Evt, Repeat, Snapshot, Source, DEFAULT_SOCKET, SERVICE_ID};
use bifrost_proto::svc;
use bifrost_proto::svc::audio::{Cmd, Track};
use std::collections::BTreeSet;
use std::os::unix::net::UnixStream;

// ---------------------------------------------------------------------------------------------------------------------
// intent_presentation: schema domains
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn every_schema_domain_has_a_distinct_lowercase_name_that_parses_back() {
    let names: BTreeSet<&str> = SchemaDomain::ALL.iter().map(|domain| domain.name()).collect();
    assert_eq!(names.len(), SchemaDomain::ALL.len());
    for domain in SchemaDomain::ALL {
        assert_eq!(domain.name(), domain.name().to_lowercase());
        assert_eq!(SchemaDomain::parse(domain.name()), Some(domain));
    }
}

#[test]
fn a_domain_name_is_matched_exactly() {
    assert_eq!(SchemaDomain::parse("notes"), Some(SchemaDomain::Notes));
    assert_eq!(SchemaDomain::parse("Notes"), None);
    assert_eq!(SchemaDomain::parse("note"), None);
}

// ---------------------------------------------------------------------------------------------------------------------
// intent_presentation: titles, subtitles, symbols
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_title_is_trimmed_and_required() {
    let presentation = Presentation::new("  Open note  ").unwrap();
    assert_eq!(presentation.title(), "Open note");
    assert_eq!(Presentation::new("   "), Err(PresentationError::Empty(Field::Title)));
}

#[test]
fn a_title_of_exactly_the_limit_is_accepted_and_one_more_is_refused() {
    assert!(Presentation::new(&"a".repeat(MAX_TITLE)).is_ok());
    assert_eq!(
        Presentation::new(&"a".repeat(MAX_TITLE + 1)),
        Err(PresentationError::TooLong { field: Field::Title, length: MAX_TITLE + 1 })
    );
}

#[test]
fn a_title_with_a_line_break_is_refused() {
    assert_eq!(Presentation::new("Open\nnote"), Err(PresentationError::ControlCharacter(Field::Title)));
}

#[test]
fn a_subtitle_is_optional_and_limited_to_its_own_length() {
    let plain = Presentation::new("Groceries").unwrap();
    assert_eq!(plain.subtitle(), None);
    let with = plain.with_subtitle(" Shared list ").unwrap();
    assert_eq!(with.subtitle(), Some("Shared list"));
    let long = "b".repeat(MAX_SUBTITLE + 1);
    assert_eq!(
        Presentation::new("Groceries").unwrap().with_subtitle(&long),
        Err(PresentationError::TooLong { field: Field::Subtitle, length: MAX_SUBTITLE + 1 })
    );
    assert_eq!(Presentation::new("Groceries").unwrap().with_subtitle("  "), Err(PresentationError::Empty(Field::Subtitle)));
}

#[test]
fn a_symbol_is_lowercase_letters_digits_and_single_dots() {
    let ok = Presentation::new("Notes").unwrap().with_symbol("note.text").unwrap();
    assert_eq!(ok.symbol(), Some("note.text"));
    for bad in ["", "Note", ".note", "note.", "note..text", "note text", "note-text"] {
        assert_eq!(Presentation::new("Notes").unwrap().with_symbol(bad), Err(PresentationError::InvalidSymbol), "{bad:?}");
    }
    let too_long = "a".repeat(65);
    assert_eq!(Presentation::new("Notes").unwrap().with_symbol(&too_long), Err(PresentationError::InvalidSymbol));
}

#[test]
fn a_presentation_records_its_schema_domain() {
    assert_eq!(Presentation::new("Notes").unwrap().domain(), None);
    let filed = Presentation::new("Notes").unwrap().in_domain(SchemaDomain::Notes);
    assert_eq!(filed.domain(), Some(SchemaDomain::Notes));
    assert_eq!(filed.title(), "Notes");
}

// ---------------------------------------------------------------------------------------------------------------------
// system_audio: socket choice and checked commands
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_service_socket_is_the_override_when_set_and_the_default_otherwise() {
    assert_eq!(system_audio::socket_path(None).to_str(), Some(DEFAULT_SOCKET));
    assert_eq!(system_audio::socket_path(Some("")).to_str(), Some(DEFAULT_SOCKET));
    assert_eq!(system_audio::socket_path(Some("/run/test/audio.sock")).to_str(), Some("/run/test/audio.sock"));
    assert_eq!(SERVICE_ID, "is.olibuijr.audio");
}

#[test]
fn a_volume_is_checked_to_lie_between_zero_and_one() {
    assert!(Command::volume(0.0).is_ok());
    assert!(Command::volume(1.0).is_ok());
    assert_eq!(Command::volume(1.01), Err(AudioError::InvalidVolume));
    assert_eq!(Command::volume(-0.1), Err(AudioError::InvalidVolume));
    assert_eq!(Command::volume(f32::NAN), Err(AudioError::InvalidVolume));
}

#[test]
fn a_seek_position_is_finite_and_not_negative() {
    assert!(Command::seek(0.0).is_ok());
    assert!(Command::seek(1500.0).is_ok());
    assert_eq!(Command::seek(-1.0), Err(AudioError::InvalidPosition));
    assert_eq!(Command::seek(f64::NAN), Err(AudioError::InvalidPosition));
    assert_eq!(Command::seek(f64::INFINITY), Err(AudioError::InvalidPosition));
}

#[test]
fn a_file_source_is_an_absolute_path() {
    assert_eq!(Source::file("notes.mp3"), Err(AudioError::RelativePath));
    assert_eq!(Source::file(""), Err(AudioError::EmptyPath));
    assert_eq!(Source::file("/music/a.mp3"), Ok(Source::File("/music/a.mp3".into())));
}

#[test]
fn a_stream_source_is_an_http_or_https_url_without_spaces() {
    assert_eq!(Source::stream("ftp://radio.example/live"), Err(AudioError::InvalidStreamUrl));
    assert_eq!(Source::stream("https://"), Err(AudioError::InvalidStreamUrl));
    assert_eq!(Source::stream("https://radio.example/a b"), Err(AudioError::InvalidStreamUrl));
    assert_eq!(
        Source::stream("https://radio.example/live"),
        Ok(Source::Stream("https://radio.example/live".to_string()))
    );
}

#[test]
fn a_queue_needs_a_source_and_a_start_inside_it() {
    assert_eq!(Command::play_queue(&[], 0, false, 1), Err(AudioError::EmptyQueue));
    let two = [Source::file("/music/a.mp3").unwrap(), Source::file("/music/b.mp3").unwrap()];
    assert_eq!(Command::play_queue(&two, 2, false, 1), Err(AudioError::StartOutOfRange { start: 2, len: 2 }));
    assert!(Command::play_queue(&two, 1, true, 7).is_ok());
}

#[test]
fn a_recording_goes_to_an_absolute_wav_path() {
    assert_eq!(Command::record_start("memo.wav"), Err(AudioError::RelativePath));
    assert_eq!(Command::record_start("/var/lib/memo.m4a"), Err(AudioError::NotWavPath));
    assert_eq!(Command::record_start(""), Err(AudioError::EmptyPath));
    assert!(Command::record_start("/var/lib/memo.wav").is_ok());
}

// ---------------------------------------------------------------------------------------------------------------------
// system_audio: framing over a socket pair
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_ping_sends_one_frame_and_reads_the_service_identity() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let service = std::thread::spawn(move || {
        let frame = svc::read_frame(&mut service_end).unwrap();
        assert_eq!(Cmd::decode(&frame), Ok(Cmd::Ping));
        let reply = Evt::Pong { id: SERVICE_ID.into(), version: "0.9.1".into() };
        svc::write_frame(&mut service_end, &reply.encode()).unwrap();
    });
    let mut client = AudioClient::new(client_end);
    assert_eq!(client.ping(), Ok((SERVICE_ID.to_string(), "0.9.1".to_string())));
    service.join().unwrap();
}

#[test]
fn a_ping_answered_with_a_state_push_is_an_unexpected_reply() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let service = std::thread::spawn(move || {
        let _ = svc::read_frame(&mut service_end).unwrap();
        let reply = Evt::State(idle_snapshot());
        svc::write_frame(&mut service_end, &reply.encode()).unwrap();
    });
    let mut client = AudioClient::new(client_end);
    assert_eq!(client.ping(), Err(AudioError::UnexpectedReply));
    service.join().unwrap();
}

#[test]
fn a_command_is_written_without_waiting_for_a_reply() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let mut client = AudioClient::new(client_end);
    client.send(&Command::pause()).unwrap();
    client.send(&Command::volume(0.25).unwrap()).unwrap();
    let first = svc::read_frame(&mut service_end).unwrap();
    let second = svc::read_frame(&mut service_end).unwrap();
    assert_eq!(Cmd::decode(&first), Ok(Cmd::Pause));
    assert_eq!(Cmd::decode(&second), Ok(Cmd::SetVolume(0.25)));
}

#[test]
fn a_queue_is_sent_with_file_and_stream_tracks_numbered_from_one() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let mut client = AudioClient::new(client_end);
    let sources = [Source::file("/music/a.mp3").unwrap(), Source::stream("https://radio.example/live").unwrap()];
    client.send(&Command::play_queue(&sources, 1, false, 42).unwrap()).unwrap();

    let frame = svc::read_frame(&mut service_end).unwrap();
    match Cmd::decode(&frame) {
        Ok(Cmd::PlayQueue { tracks, start, shuffle, seed }) => {
            assert_eq!(start, 1);
            assert!(!shuffle);
            assert_eq!(seed, 42);
            assert_eq!(
                tracks,
                vec![
                    Track { id: 1, path: "/music/a.mp3".into(), url: None },
                    Track { id: 2, path: String::new(), url: Some("https://radio.example/live".into()) },
                ]
            );
        }
        other => panic!("expected a play queue command, got {other:?}"),
    }
}

#[test]
fn subscribing_returns_the_state_the_service_reports() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let service = std::thread::spawn(move || {
        let frame = svc::read_frame(&mut service_end).unwrap();
        assert_eq!(Cmd::decode(&frame), Ok(Cmd::Subscribe));
        let reply = Evt::State(Snapshot { volume: 0.5, repeat: Repeat::All, queue_len: 3, ..idle_snapshot() });
        svc::write_frame(&mut service_end, &reply.encode()).unwrap();
    });
    let mut client = AudioClient::new(client_end);
    let state = client.subscribe().unwrap();
    assert_eq!(state.volume, 0.5);
    assert_eq!(state.repeat, Repeat::All);
    assert_eq!(state.queue_len, 3);
    service.join().unwrap();
}

#[test]
fn a_reply_with_an_unknown_tag_is_a_protocol_error() {
    let (client_end, mut service_end) = UnixStream::pair().unwrap();
    let service = std::thread::spawn(move || {
        let _ = svc::read_frame(&mut service_end).unwrap();
        svc::write_frame(&mut service_end, &[0xff, 0xff, 0xff, 0x7f]).unwrap();
    });
    let mut client = AudioClient::new(client_end);
    assert_eq!(client.ping(), Err(AudioError::Protocol));
    service.join().unwrap();
}

fn idle_snapshot() -> Snapshot {
    Snapshot {
        track: None,
        index: None,
        playing: false,
        position_ms: 0.0,
        duration_ms: None,
        shuffle: false,
        repeat: Repeat::Off,
        queue_len: 0,
        volume: 1.0,
        muted: false,
        error: None,
        live: None,
    }
}
