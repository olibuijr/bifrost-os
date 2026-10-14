//! Parity sdk-rest-07 (video playback audio): headless tests for the video playback facade. No test plays sound,
//! opens a display, or needs a media file: the open tests use a path that does not exist.

use std::path::Path;

use bifrost_app::video_playback::{audio_output, open, VideoAudio, VideoError};

#[test]
fn a_silent_output_accepts_the_frames_a_player_writes() {
    let mut out = audio_output(VideoAudio::Silent);
    assert!(out.write(&[0i16; 8]).is_ok(), "a muted video still advances its clock");
}

#[test]
fn opening_a_missing_video_on_the_speaker_route_is_not_found() {
    let missing = Path::new("/nonexistent/bifrost-sdk-rest-07/clip.mp4");
    assert_eq!(open(missing, VideoAudio::Speaker).err(), Some(VideoError::NotFound));
}

#[test]
fn opening_a_missing_video_on_the_silent_route_is_not_found() {
    let missing = Path::new("/nonexistent/bifrost-sdk-rest-07/clip.mp4");
    assert_eq!(open(missing, VideoAudio::Silent).err(), Some(VideoError::NotFound));
}

#[test]
fn the_speaker_and_silent_routes_are_different_choices() {
    assert_ne!(VideoAudio::Speaker, VideoAudio::Silent);
}

#[test]
fn a_video_error_says_what_went_wrong() {
    assert_eq!(VideoError::NotFound.to_string(), "the video file does not exist");
    assert_eq!(VideoError::Unreadable("bad header".into()).to_string(), "the video could not be read: bad header");
}
