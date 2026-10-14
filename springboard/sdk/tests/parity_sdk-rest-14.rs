//! Parity sdk-rest-14: headless tests of the app-facing client for background services. Each test serves one connection
//! on a Unix socket in a temporary folder; no test starts a real service, the supervisor or the shell.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use bifrost_app::service_client::{socket_path, ServiceClient, ServiceError, ServiceId, ServiceInfo};
use bifrost_proto::svc::{self, common};

const AUDIO: &str = "is.olibuijr.audio";

/// A folder shaped like the shell's data folder, with an empty `run/`, and a service socket bound under it.
fn service_root(id: &str) -> (tempfile::TempDir, PathBuf, UnixListener) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::create_dir_all(root.join("run")).unwrap();
    let listener = UnixListener::bind(socket_path(&root, &ServiceId::new(id).unwrap())).unwrap();
    (dir, root, listener)
}

/// Serves the first connection by letting `answer` read and reply on it.
fn serve_once(listener: UnixListener, answer: impl FnOnce(&mut UnixStream) + Send + 'static) {
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            answer(&mut stream);
        }
    });
}

fn connect(root: &Path, id: &str, timeout: Duration) -> Result<ServiceClient, ServiceError> {
    ServiceClient::connect(root, &ServiceId::new(id).unwrap(), timeout)
}

#[test]
fn a_service_id_accepts_the_names_the_supervisor_accepts() {
    assert_eq!(ServiceId::new(AUDIO).unwrap().as_str(), AUDIO);
    assert!(ServiceId::new("is.olibuijr.audio-2_x").is_ok());
}

#[test]
fn a_service_id_refuses_paths_spaces_and_empty_text() {
    for bad in ["", "a/b", "../etc", "is audio", "is.olibuijr.audio\n", "é"] {
        assert_eq!(ServiceId::new(bad), Err(ServiceError::InvalidId(bad.to_string())), "{bad:?} must not name a socket");
    }
}

#[test]
fn the_socket_of_a_service_is_named_after_it_under_run() {
    let id = ServiceId::new(AUDIO).unwrap();
    assert_eq!(socket_path(Path::new("/var/lib/springboard"), &id), PathBuf::from("/var/lib/springboard/run/is.olibuijr.audio.sock"));
}

#[test]
fn connecting_to_a_service_without_a_socket_reports_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    assert_eq!(connect(dir.path(), AUDIO, Duration::from_millis(100)).err(), Some(ServiceError::NotInstalled));
}

#[test]
fn a_socket_that_nothing_accepts_on_reports_not_listening() {
    let (_dir, root, listener) = service_root(AUDIO);
    drop(listener);
    assert_eq!(connect(&root, AUDIO, Duration::from_millis(100)).err(), Some(ServiceError::NotListening));
}

#[test]
fn a_ping_returns_the_id_and_version_the_service_answers_with() {
    let (_dir, root, listener) = service_root(AUDIO);
    serve_once(listener, |s| {
        let request = svc::read_frame(s).unwrap();
        assert_eq!(request, common::ping(), "the client sends the common ping frame");
        svc::write_frame(s, &common::pong(AUDIO, "2.1.0")).unwrap();
    });
    let mut client = connect(&root, AUDIO, Duration::from_secs(5)).unwrap();
    assert_eq!(client.ping(), Ok(ServiceInfo { id: AUDIO.into(), version: "2.1.0".into() }));
}

#[test]
fn a_ping_refuses_an_answer_from_a_different_service() {
    let (_dir, root, listener) = service_root(AUDIO);
    serve_once(listener, |s| {
        svc::read_frame(s).unwrap();
        svc::write_frame(s, &common::pong("is.other", "1.0.0")).unwrap();
    });
    let mut client = connect(&root, AUDIO, Duration::from_secs(5)).unwrap();
    assert_eq!(
        client.ping(),
        Err(ServiceError::WrongService { expected: AUDIO.into(), found: "is.other".into() })
    );
}

#[test]
fn a_call_returns_the_reply_frame_as_the_service_sent_it() {
    let (_dir, root, listener) = service_root(AUDIO);
    serve_once(listener, |s| {
        let mut request = svc::read_frame(s).unwrap();
        request.reverse();
        svc::write_frame(s, &request).unwrap();
    });
    let mut client = connect(&root, AUDIO, Duration::from_secs(5)).unwrap();
    assert_eq!(client.call(&[1, 2, 3]), Ok(vec![3, 2, 1]));
}

#[test]
fn a_request_over_the_frame_limit_is_refused_before_anything_is_sent() {
    // the listener stays open, so the connection waits in the backlog and only the size check can refuse the call
    let (_dir, root, _listener) = service_root(AUDIO);
    let mut client = connect(&root, AUDIO, Duration::from_secs(5)).unwrap();
    let too_big = vec![0u8; svc::MAX_FRAME + 1];
    assert_eq!(client.call(&too_big), Err(ServiceError::TooLarge { len: svc::MAX_FRAME + 1, max: svc::MAX_FRAME }));
}

#[test]
fn a_service_that_closes_during_a_call_reports_disconnected() {
    let (_dir, root, listener) = service_root(AUDIO);
    serve_once(listener, |s| {
        svc::read_frame(s).unwrap();
    });
    let mut client = connect(&root, AUDIO, Duration::from_secs(5)).unwrap();
    assert_eq!(client.ping(), Err(ServiceError::Disconnected));
}

#[test]
fn a_service_that_does_not_answer_in_time_reports_a_timeout() {
    let (_dir, root, listener) = service_root(AUDIO);
    serve_once(listener, |s| {
        svc::read_frame(s).unwrap();
        thread::sleep(Duration::from_secs(3));
    });
    let mut client = connect(&root, AUDIO, Duration::from_millis(100)).unwrap();
    assert_eq!(client.ping(), Err(ServiceError::Timeout));
}

#[test]
fn an_error_names_the_limit_it_went_over() {
    let text = ServiceError::TooLarge { len: 300_000, max: svc::MAX_FRAME }.to_string();
    assert!(text.contains("262144"), "the message gives the frame limit in bytes: {text}");
    assert_eq!(ServiceError::NotInstalled.to_string(), "the service has no socket");
}
