//! Parity sdk-rest-12: headless tests of the audio service client that apps share (`springboard::svc::audio`,
//! re-exported to apps as `bifrost_app::audio`). A fake service listens on a Unix socket in a temporary directory;
//! no audio device is opened and no sound is played.

use bifrost_proto::svc::audio::{Cmd, Evt, Repeat, Track};
use bifrost_proto::svc::{read_frame, write_frame};
use springboard::svc::audio::{idle, socket_path, Client};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

const SERVICE_VERSION: &str = "0.9.1";

/// Binds the fake service at the socket the client connects to under `root`.
fn service(root: &Path) -> UnixListener {
    let sock = socket_path(root);
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    UnixListener::bind(sock).unwrap()
}

fn next_cmd(server: &mut UnixStream) -> Cmd {
    Cmd::decode(&read_frame(server).unwrap()).unwrap()
}

/// Accepts the client and checks its opening handshake: a ping, then a subscription.
fn accept_handshake(listener: &UnixListener) -> UnixStream {
    let (mut server, _) = listener.accept().unwrap();
    assert_eq!(next_cmd(&mut server), Cmd::Ping, "the client pings before it subscribes");
    assert_eq!(next_cmd(&mut server), Cmd::Subscribe);
    server
}

/// A client armed under `root` and connected to a fake service, with the service's end of the connection.
struct Session {
    listener: UnixListener,
    client: Client,
    server: UnixStream,
}

fn session(root: &Path) -> Session {
    let listener = service(root);
    let mut client = Client::for_root(root);
    client.arm();
    client.tick(1.0);
    let server = accept_handshake(&listener);
    Session { listener, client, server }
}

#[test]
fn a_lazy_client_does_not_connect_until_it_is_armed() {
    let dir = tempfile::tempdir().unwrap();
    let listener = service(dir.path());
    let mut client = Client::for_root(dir.path());
    assert!(!client.armed(), "a client made for a root waits for arm, so bifrostd does not start the service");
    client.tick(0.0);
    assert!(!client.connected());
    listener.set_nonblocking(true).unwrap();
    assert!(listener.accept().is_err(), "nothing connected while the client is unarmed");
    client.arm();
    client.tick(1.0);
    assert!(client.connected());
    assert_eq!(client.connects(), 1);
}

#[test]
fn a_pong_sets_the_service_version_and_a_state_frame_sets_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    write_frame(&mut s.server, &Evt::Pong { id: "is.olibuijr.audio".into(), version: SERVICE_VERSION.into() }.encode()).unwrap();
    let mut state = idle();
    state.playing = true;
    state.volume = 0.5;
    state.queue_len = 3;
    write_frame(&mut s.server, &Evt::State(state).encode()).unwrap();
    assert!(s.client.pump(), "a state frame changes what the client shows");
    s.client.tick(1.0);
    assert_eq!(s.client.version(), Some(SERVICE_VERSION));
    assert!(s.client.snapshot().playing);
    assert_eq!(s.client.snapshot().volume, 0.5);
    assert_eq!(s.client.snapshot().queue_len, 3);
}

#[test]
fn transport_commands_reach_the_service_in_the_order_they_were_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    assert!(s.client.play());
    assert!(s.client.pause());
    assert!(s.client.next());
    assert!(s.client.seek_ms(4500.0));
    assert_eq!(next_cmd(&mut s.server), Cmd::Play);
    assert_eq!(next_cmd(&mut s.server), Cmd::Pause);
    assert_eq!(next_cmd(&mut s.server), Cmd::Next);
    assert_eq!(next_cmd(&mut s.server), Cmd::SeekMs(4500.0));
}

#[test]
fn volume_is_clamped_to_the_unit_range_before_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    assert!(s.client.set_volume(3.0));
    assert!(s.client.set_volume(-1.0));
    assert!(s.client.set_volume(0.25));
    assert_eq!(next_cmd(&mut s.server), Cmd::SetVolume(1.0));
    assert_eq!(next_cmd(&mut s.server), Cmd::SetVolume(0.0));
    assert_eq!(next_cmd(&mut s.server), Cmd::SetVolume(0.25));
}

#[test]
fn a_queue_is_sent_with_its_tracks_start_shuffle_and_seed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let tracks = vec![Track { id: 1, path: "a.mp3".into(), url: None }, Track { id: 2, path: "b.mp3".into(), url: None }];
    assert!(s.client.play_queue(tracks.clone(), 1, true, 7));
    assert!(s.client.set_repeat(Repeat::All));
    assert_eq!(next_cmd(&mut s.server), Cmd::PlayQueue { tracks, start: 1, shuffle: true, seed: 7 });
    assert_eq!(next_cmd(&mut s.server), Cmd::SetRepeat(Repeat::All));
}

#[test]
fn commands_are_refused_without_a_connection_and_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::new(socket_path(dir.path()));
    assert!(!client.connected());
    assert!(!client.play(), "no connection, so nothing is sent");
    assert!(!client.set_volume(0.5));
    assert!(!client.record_stop());
}

#[test]
fn a_closed_service_drops_the_connection_and_clears_what_was_shown() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    write_frame(&mut s.server, &Evt::Pong { id: "is.olibuijr.audio".into(), version: SERVICE_VERSION.into() }.encode()).unwrap();
    let mut state = idle();
    state.playing = true;
    write_frame(&mut s.server, &Evt::State(state).encode()).unwrap();
    s.client.pump();
    s.client.tick(1.0);
    assert!(s.client.snapshot().playing);
    drop(s.server);
    assert!(s.client.pump(), "the end of the stream is a change");
    assert!(!s.client.connected());
    assert_eq!(s.client.version(), None);
    assert!(!s.client.snapshot().playing, "a dead service shows nothing as playing");
}

#[test]
fn the_client_reconnects_and_counts_the_new_connection_when_the_service_returns() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    // the connection held for a while: a service that goes away and comes back is found again at once
    s.client.tick(6000.0);
    drop(s.server);
    s.client.pump();
    assert!(!s.client.connected());
    s.client.tick(6002.0);
    assert!(s.client.connected(), "the socket still exists, so the client reconnects on its next tick");
    assert_eq!(s.client.connects(), 2);
    let _server = accept_handshake(&s.listener);
}

#[test]
fn a_service_that_closes_the_connection_at_once_is_not_reconnected_in_a_loop() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    // closed before it held: the next attempt waits a second, not the next tick
    drop(s.server);
    s.client.pump();
    assert!(!s.client.connected());
    for at in [2.0, 500.0, 1000.0] {
        s.client.tick(at);
        assert!(!s.client.connected(), "no reconnect at {at} ms");
    }
    assert_eq!(s.client.connects(), 1);
    s.client.tick(1001.0);
    assert!(s.client.connected(), "after the wait it tries again");
    assert_eq!(s.client.connects(), 2);
}
