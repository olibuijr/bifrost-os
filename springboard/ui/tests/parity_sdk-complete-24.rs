//! Parity sdk-complete-24: headless tests for the video track model (`bifrost_ui::video_info`): validation of the
//! frame size, duration, frame rate and codec, the resolution label, the aspect ratio and the duration text.

use bifrost_ui::video_info::{VideoInfo, VideoInfoError};

#[test]
fn a_full_hd_track_reports_its_label_aspect_and_duration_text() {
    let v = VideoInfo::new(1920, 1080, 5400.0, 24.0, "  h264 ").expect("a valid track");
    assert_eq!(v.codec(), "h264", "the codec name is trimmed");
    assert_eq!(v.resolution_label(), "1080p");
    assert_eq!(v.duration_text(), "1:30:00");
    assert!((v.aspect_ratio() - 16.0 / 9.0).abs() < 1e-9);
    assert!(v.is_hd());
    assert_eq!((v.width(), v.height(), v.frame_rate()), (1920, 1080, 24.0));
}

#[test]
fn a_track_of_2160_lines_or_more_is_labelled_4k() {
    let v = VideoInfo::new(3840, 2160, 60.0, 25.0, "hevc").expect("a valid track");
    assert_eq!(v.resolution_label(), "4K");
    assert_eq!(VideoInfo::new(4096, 2304, 60.0, 25.0, "hevc").expect("valid").resolution_label(), "4K");
}

#[test]
fn a_frame_height_below_2160_keeps_its_line_count_as_the_label() {
    assert_eq!(VideoInfo::new(2560, 1440, 60.0, 30.0, "vp9").expect("valid").resolution_label(), "1440p");
    assert_eq!(VideoInfo::new(640, 480, 60.0, 30.0, "mpeg4").expect("valid").resolution_label(), "480p");
}

#[test]
fn only_frames_of_720_lines_or_more_are_hd() {
    assert!(VideoInfo::new(1280, 720, 1.0, 24.0, "h264").expect("valid").is_hd());
    assert!(!VideoInfo::new(1024, 719, 1.0, 24.0, "h264").expect("valid").is_hd());
}

#[test]
fn a_zero_frame_dimension_is_refused() {
    assert_eq!(VideoInfo::new(0, 1080, 1.0, 24.0, "h264"), Err(VideoInfoError::BadFrameSize));
    assert_eq!(VideoInfo::new(1920, 0, 1.0, 24.0, "h264"), Err(VideoInfoError::BadFrameSize));
}

#[test]
fn a_negative_or_non_finite_duration_is_refused_and_zero_is_allowed() {
    assert_eq!(VideoInfo::new(1920, 1080, -1.0, 24.0, "h264"), Err(VideoInfoError::BadDuration));
    assert_eq!(VideoInfo::new(1920, 1080, f64::NAN, 24.0, "h264"), Err(VideoInfoError::BadDuration));
    assert_eq!(VideoInfo::new(1920, 1080, f64::INFINITY, 24.0, "h264"), Err(VideoInfoError::BadDuration));
    assert!(VideoInfo::new(1920, 1080, 0.0, 24.0, "h264").is_ok());
}

#[test]
fn a_zero_negative_or_non_finite_frame_rate_is_refused() {
    assert_eq!(VideoInfo::new(1920, 1080, 1.0, 0.0, "h264"), Err(VideoInfoError::BadFrameRate));
    assert_eq!(VideoInfo::new(1920, 1080, 1.0, -24.0, "h264"), Err(VideoInfoError::BadFrameRate));
    assert_eq!(VideoInfo::new(1920, 1080, 1.0, f64::NAN, "h264"), Err(VideoInfoError::BadFrameRate));
}

#[test]
fn a_blank_codec_name_is_refused() {
    assert_eq!(VideoInfo::new(1920, 1080, 1.0, 24.0, "   "), Err(VideoInfoError::EmptyCodec));
}

#[test]
fn the_duration_text_drops_fractions_and_omits_the_hour_under_one_hour() {
    assert_eq!(VideoInfo::new(640, 480, 61.9, 30.0, "h264").expect("valid").duration_text(), "1:01");
    assert_eq!(VideoInfo::new(640, 480, 0.0, 30.0, "h264").expect("valid").duration_text(), "0:00");
    assert_eq!(VideoInfo::new(640, 480, 3599.0, 30.0, "h264").expect("valid").duration_text(), "59:59");
    assert_eq!(VideoInfo::new(640, 480, 3600.0, 30.0, "h264").expect("valid").duration_text(), "1:00:00");
}
