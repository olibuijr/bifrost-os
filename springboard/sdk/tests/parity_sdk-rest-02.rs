//! Parity sdk-rest-02: headless tests for the video frame hand-off and the child process launch. No test opens a
//! display, a device or a network connection. The process tests start only `/bin/sh`, which prints a fixed text.

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use bifrost_app::child_process::{ProcessSpec, SpawnError, Termination};
use bifrost_app::video_decode::Codec;
use bifrost_app::video_frame::{FrameError, FrameStream, VideoFormat};
use bifrost_render::pixel_format::FourCc;
use bifrost_app::pixel_buffer::PixelBuffer;

fn nv12() -> FourCc {
    FourCc::new(*b"NV12")
}

fn bgra() -> FourCc {
    FourCc::new(*b"AR24")
}

fn nv12_format(width: usize, height: usize) -> VideoFormat {
    VideoFormat::new(Codec::H264, nv12(), width, height).expect("a 4:2:0 format of this size")
}

fn picture(code: FourCc, width: usize, height: usize) -> PixelBuffer {
    PixelBuffer::new(code, width, height).expect("a buffer of this format and size")
}

// Video formats: the codec, the size and the pixel format a stream promises.

#[test]
fn a_video_format_needs_a_picture_with_both_dimensions() {
    assert_eq!(VideoFormat::new(Codec::Av1, nv12(), 0, 2), Err(FrameError::EmptySize));
    assert_eq!(VideoFormat::new(Codec::Av1, nv12(), 4, 0), Err(FrameError::EmptySize));
}

#[test]
fn a_video_format_needs_a_pixel_format_the_system_knows() {
    let unknown = FourCc::new(*b"ZZZZ");
    assert_eq!(VideoFormat::new(Codec::Vp9, unknown, 4, 2), Err(FrameError::UnknownPixelFormat));
}

#[test]
fn a_video_format_keeps_its_codec_size_and_pixel_format() {
    let format = VideoFormat::new(Codec::Hevc, bgra(), 640, 360).expect("a valid format");
    assert_eq!(format.codec(), Codec::Hevc);
    assert_eq!((format.width(), format.height()), (640, 360));
    assert_eq!(format.code(), bgra());
}

// Frames: a picture is accepted only when it matches the format it was negotiated for.

#[test]
fn a_picture_with_another_pixel_format_does_not_match_the_format() {
    let format = nv12_format(4, 2);
    assert!(format.matches(&picture(nv12(), 4, 2)));
    assert!(!format.matches(&picture(bgra(), 4, 2)), "the same size in another pixel format is another format");
}

#[test]
fn a_picture_with_another_size_does_not_match_the_format() {
    let format = nv12_format(4, 2);
    assert!(!format.matches(&picture(nv12(), 4, 4)));
    assert!(!format.matches(&picture(nv12(), 8, 2)));
}

#[test]
fn a_stream_refuses_a_picture_that_does_not_match_its_format() {
    let mut stream = FrameStream::new(nv12_format(4, 2));
    assert_eq!(stream.push(1_000, picture(bgra(), 4, 2)), Err(FrameError::FormatMismatch));
    assert_eq!(stream.pending(), 0, "a refused picture is not queued");
}

// Presentation order: each frame must come after the one before it.

#[test]
fn a_stream_refuses_a_frame_that_is_not_later_than_the_previous_frame() {
    let mut stream = FrameStream::new(nv12_format(4, 2));
    stream.push(1_000, picture(nv12(), 4, 2)).expect("the first frame");
    assert_eq!(stream.push(1_000, picture(nv12(), 4, 2)), Err(FrameError::NotInPresentationOrder));
    assert_eq!(stream.push(999, picture(nv12(), 4, 2)), Err(FrameError::NotInPresentationOrder));
    assert_eq!(stream.pending(), 1);
}

#[test]
fn a_refused_frame_does_not_move_the_stream_clock_forward() {
    let mut stream = FrameStream::new(nv12_format(4, 2));
    stream.push(5_000, picture(bgra(), 4, 2)).expect_err("a mismatched picture is refused");
    stream.push(3_000, picture(nv12(), 4, 2)).expect("the refused frame set no time, so 3000 is still in order");
    assert_eq!(stream.pending(), 1);
}

#[test]
fn a_stream_hands_frames_out_oldest_first_and_keeps_their_times() {
    let mut stream = FrameStream::new(nv12_format(4, 2));
    stream.push(1_000, picture(nv12(), 4, 2)).expect("first");
    stream.push(2_000, picture(nv12(), 4, 2)).expect("second");
    assert_eq!(stream.pending(), 2);

    let first = stream.next_frame().expect("the first frame waits");
    assert_eq!(first.presentation_us(), 1_000);
    assert_eq!((first.picture().width(), first.picture().height()), (4, 2));

    let second = stream.next_frame().expect("the second frame waits");
    assert_eq!(second.presentation_us(), 2_000);
    assert!(stream.next_frame().is_none(), "no frame is left after the second");
}

#[test]
fn a_frame_gives_its_picture_to_the_caller_unchanged() {
    let mut stream = FrameStream::new(nv12_format(4, 2));
    stream.push(10, picture(nv12(), 4, 2)).expect("a frame");
    let frame = stream.next_frame().expect("the frame");
    let buffer = frame.into_picture();
    assert_eq!(buffer.code(), nv12());
    assert_eq!(buffer.width(), 4);
}

// Child processes: a launch is checked before anything starts.

#[test]
fn a_relative_or_empty_program_is_refused() {
    assert_eq!(ProcessSpec::new("sh").map(|_| ()), Err(SpawnError::NotAbsolute));
    assert_eq!(ProcessSpec::new("").map(|_| ()), Err(SpawnError::NotAbsolute));
}

#[test]
fn a_nul_byte_in_an_argument_is_refused() {
    let spec = ProcessSpec::new("/bin/sh").expect("an absolute path");
    assert_eq!(spec.argument("a\0b").map(|_| ()), Err(SpawnError::NulByte));
}

#[test]
fn an_environment_name_cannot_be_empty_or_hold_an_equals_sign() {
    let spec = ProcessSpec::new("/bin/sh").expect("an absolute path");
    assert_eq!(spec.clone().env("", "1").map(|_| ()), Err(SpawnError::InvalidEnvName));
    assert_eq!(spec.env("A=B", "1").map(|_| ()), Err(SpawnError::InvalidEnvName));
}

#[test]
fn a_later_environment_value_replaces_an_earlier_one() {
    let spec = ProcessSpec::new("/bin/sh")
        .expect("an absolute path")
        .argument("-c")
        .expect("an argument")
        .argument("printf %s \"$BIFROST_SDK_REST_02\"")
        .expect("an argument")
        .env("BIFROST_SDK_REST_02", "first")
        .expect("a variable")
        .env("BIFROST_SDK_REST_02", "second")
        .expect("a variable");
    let finished = spec.run().expect("the shell starts");
    assert_eq!(finished.stdout, b"second".to_vec());
    assert_eq!(spec.arguments().len(), 2);
}

#[test]
fn a_missing_program_reports_that_it_was_not_found() {
    let spec = ProcessSpec::new("/nonexistent/bifrost-sdk-rest-02").expect("an absolute path");
    assert_eq!(spec.run(), Err(SpawnError::Launch(std::io::ErrorKind::NotFound)));
}

#[test]
fn a_finished_child_reports_its_exit_code_and_both_outputs() {
    let finished = ProcessSpec::new("/bin/sh")
        .expect("an absolute path")
        .argument("-c")
        .expect("an argument")
        .argument("printf out; printf err >&2; exit 3")
        .expect("an argument")
        .run()
        .expect("the shell starts");
    assert_eq!(finished.termination, Termination::Exited(3));
    assert!(!finished.termination.succeeded());
    assert_eq!(finished.stdout, b"out".to_vec());
    assert_eq!(finished.stderr, b"err".to_vec());
}

#[test]
fn a_child_that_exits_with_zero_succeeds() {
    let status = ExitStatus::from_raw(0);
    assert_eq!(Termination::from_status(status), Some(Termination::Exited(0)));
    assert!(Termination::Exited(0).succeeded());
}

#[test]
fn a_child_stopped_by_a_signal_reports_the_signal_not_an_exit_code() {
    let status = ExitStatus::from_raw(9);
    assert_eq!(Termination::from_status(status), Some(Termination::Signaled(9)));
    assert!(!Termination::Signaled(9).succeeded());
}

#[test]
fn an_exit_code_is_read_from_the_high_byte_of_the_wait_status() {
    let status = ExitStatus::from_raw(3 << 8);
    assert_eq!(Termination::from_status(status), Some(Termination::Exited(3)));
}
