//! Parity sdk-complete-07: headless tests for the guest stream request order (set parameters, prepare, start, stop,
//! release). No test touches the VIRTIO transport or the host audio daemon.

use bifrost_core::guest_audio::{Direction, GuestStream, Request, RequestError, SampleFormat, StreamConfig, StreamState};

fn stereo_s16() -> StreamConfig {
    StreamConfig::new(Direction::Output, SampleFormat::S16, 48_000, 2, 4_800, 192_000).unwrap()
}

#[test]
fn a_new_stream_is_idle_and_has_no_parameters() {
    let stream = GuestStream::new();
    assert_eq!(stream.state(), StreamState::Idle);
    assert_eq!(stream.config(), None);
}

#[test]
fn the_stream_follows_set_prepare_start_stop_and_release_in_order() {
    let mut stream = GuestStream::new();
    assert_eq!(stream.request(Request::SetParams(stereo_s16())), Ok(StreamState::ParamsSet));
    assert_eq!(stream.request(Request::Prepare), Ok(StreamState::Prepared));
    assert_eq!(stream.request(Request::Start), Ok(StreamState::Running));
    assert_eq!(stream.request(Request::Stop), Ok(StreamState::Prepared));
    assert_eq!(stream.request(Request::Release), Ok(StreamState::Idle));
    assert_eq!(stream.config(), None, "release forgets the parameters");
}

#[test]
fn a_stream_cannot_start_before_it_is_prepared() {
    let mut stream = GuestStream::new();
    stream.request(Request::SetParams(stereo_s16())).unwrap();
    assert_eq!(
        stream.request(Request::Start),
        Err(RequestError { state: StreamState::ParamsSet, request: Request::Start })
    );
    assert_eq!(stream.state(), StreamState::ParamsSet, "a refused request changes nothing");
}

#[test]
fn a_stream_cannot_be_prepared_without_parameters() {
    let mut stream = GuestStream::new();
    assert_eq!(
        stream.request(Request::Prepare),
        Err(RequestError { state: StreamState::Idle, request: Request::Prepare })
    );
}

#[test]
fn parameters_cannot_change_while_the_stream_runs() {
    let mut stream = GuestStream::new();
    for request in [Request::SetParams(stereo_s16()), Request::Prepare, Request::Start] {
        stream.request(request).unwrap();
    }
    let mono = StreamConfig::new(Direction::Output, SampleFormat::S16, 48_000, 1, 2_400, 96_000).unwrap();
    assert!(stream.request(Request::SetParams(mono)).is_err());
    assert_eq!(stream.config(), Some(&stereo_s16()));
}

#[test]
fn a_stopped_stream_can_start_again_and_release_works_from_any_state() {
    let mut stream = GuestStream::new();
    for request in [Request::SetParams(stereo_s16()), Request::Prepare, Request::Start, Request::Stop, Request::Start] {
        stream.request(request).unwrap();
    }
    assert_eq!(stream.state(), StreamState::Running);
    assert_eq!(stream.request(Request::Release), Ok(StreamState::Idle));
}
