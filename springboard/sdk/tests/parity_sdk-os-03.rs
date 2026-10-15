//! Parity sdk-os-03: headless tests for the download staging, sample buffer and crash reason models. No test touches a
//! network, a device, a display or a compositor; the staging tests write to a temporary directory.

use bifrost_app::background_download::{stage, DownloadError, DownloadSpec};
use bifrost_app::crash_report::{CrashReason, ProcessCrash};
use bifrost_app::sample_buffer::{sort_by_presentation, SampleBuffer, SampleError, SampleTiming};
use springboard::media::demux::Sample;

/// SHA-256 of the three bytes `abc`.
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn abc_spec(name: &str) -> DownloadSpec {
    DownloadSpec::new(name, 3, ABC_SHA256).expect("a valid spec")
}

#[test]
fn a_spec_stores_an_upper_case_digest_in_lower_case() {
    let spec = DownloadSpec::new("springboard", 3, &ABC_SHA256.to_ascii_uppercase()).unwrap();
    assert_eq!(spec.sha256(), ABC_SHA256);
    assert_eq!(spec.name(), "springboard");
    assert_eq!(spec.expected_size(), 3);
}

#[test]
fn a_digest_that_is_not_sixty_four_hex_digits_is_refused() {
    assert_eq!(DownloadSpec::new("x", 1, "abc"), Err(DownloadError::InvalidDigest));
    assert_eq!(DownloadSpec::new("x", 1, &"z".repeat(64)), Err(DownloadError::InvalidDigest));
}

#[test]
fn a_name_that_could_leave_the_download_directory_is_refused() {
    for name in ["", ".", "..", "../x", "a/b", "a\\b", "a\0b"] {
        assert_eq!(DownloadSpec::new(name, 3, ABC_SHA256), Err(DownloadError::InvalidName(name.into())), "name {name:?}");
    }
}

#[test]
fn the_partial_name_adds_a_part_suffix() {
    assert_eq!(abc_spec("springboard").partial_name(), "springboard.part");
}

#[test]
fn verify_accepts_a_payload_with_the_expected_size_and_digest() {
    assert_eq!(abc_spec("x").verify(b"abc"), Ok(()));
}

#[test]
fn verify_reports_a_payload_of_the_wrong_size() {
    assert_eq!(abc_spec("x").verify(b"ab"), Err(DownloadError::SizeMismatch { expected: 3, actual: 2 }));
}

#[test]
fn verify_reports_a_payload_with_the_wrong_digest() {
    let err = abc_spec("x").verify(b"abd").unwrap_err();
    assert!(matches!(&err, DownloadError::DigestMismatch { expected, .. } if expected == ABC_SHA256), "got {err:?}");
}

#[test]
fn fraction_is_the_share_received_and_is_clamped_to_one() {
    let spec = DownloadSpec::new("x", 4, ABC_SHA256).unwrap();
    assert_eq!(spec.fraction(0), 0.0);
    assert_eq!(spec.fraction(1), 0.25);
    assert_eq!(spec.fraction(9), 1.0);
    assert_eq!(DownloadSpec::new("x", 0, ABC_SHA256).unwrap().fraction(0), 1.0, "an empty payload is complete at once");
}

#[test]
fn stage_writes_a_verified_payload_under_its_final_name_and_leaves_no_part_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = stage(dir.path(), &abc_spec("payload.bin"), b"abc").unwrap();
    assert_eq!(path, dir.path().join("payload.bin"));
    assert_eq!(std::fs::read(&path).unwrap(), b"abc");
    assert!(!dir.path().join("payload.bin.part").exists());
}

#[test]
fn stage_never_writes_a_payload_that_fails_the_check() {
    let dir = tempfile::tempdir().unwrap();
    let spec = DownloadSpec::new("payload.bin", 3, &"0".repeat(64)).unwrap();
    assert!(matches!(stage(dir.path(), &spec, b"abc"), Err(DownloadError::DigestMismatch { .. })));
    assert!(!dir.path().join("payload.bin").exists());
    assert!(!dir.path().join("payload.bin.part").exists());
}

#[test]
fn stage_replaces_an_earlier_staged_payload_with_the_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let spec = DownloadSpec::new("payload.bin", 3, ABC_SHA256).unwrap();
    std::fs::write(dir.path().join("payload.bin"), b"old").unwrap();
    stage(dir.path(), &spec, b"abc").unwrap();
    assert_eq!(std::fs::read(dir.path().join("payload.bin")).unwrap(), b"abc");
}

fn demuxed(track: u32, pts_us: i64, duration_us: i64, keyframe: bool, data: Vec<u8>) -> Sample {
    Sample { track, pts_us, dts_us: pts_us - 5_000, duration_us, keyframe, data }
}

#[test]
fn a_demuxed_sample_keeps_its_track_timing_sync_flag_and_bytes() {
    let buf = SampleBuffer::from_demuxed(&demuxed(2, 40_000, 33_000, true, vec![1, 2, 3])).unwrap();
    assert_eq!(buf.track(), 2);
    assert_eq!(buf.timing(), SampleTiming { presentation_us: 40_000, decode_us: 35_000, duration_us: 33_000 });
    assert!(buf.is_sync());
    assert_eq!(buf.data(), &[1, 2, 3]);
    assert_eq!(buf.data_len(), 3);
    assert_eq!(buf.presentation_end_us(), 73_000);
}

#[test]
fn a_sample_without_bytes_is_refused() {
    let timing = SampleTiming { presentation_us: 0, decode_us: 0, duration_us: 1 };
    assert_eq!(SampleBuffer::new(1, timing, true, Vec::new()), Err(SampleError::EmptyData));
    assert_eq!(SampleBuffer::from_demuxed(&demuxed(1, 0, 1, true, Vec::new())), Err(SampleError::EmptyData));
}

#[test]
fn a_negative_duration_is_refused() {
    let timing = SampleTiming { presentation_us: 0, decode_us: 0, duration_us: -1 };
    assert_eq!(SampleBuffer::new(1, timing, false, vec![0]), Err(SampleError::NegativeDuration(-1)));
}

#[test]
fn the_presentation_end_saturates_instead_of_overflowing() {
    let timing = SampleTiming { presentation_us: i64::MAX, decode_us: 0, duration_us: 5 };
    let buf = SampleBuffer::new(1, timing, false, vec![0]).unwrap();
    assert_eq!(buf.presentation_end_us(), i64::MAX);
}

#[test]
fn a_batch_sorts_by_presentation_time_and_keeps_equal_times_in_order() {
    let mut batch = vec![
        SampleBuffer::from_demuxed(&demuxed(3, 30_000, 1, false, vec![0])).unwrap(),
        SampleBuffer::from_demuxed(&demuxed(1, 10_000, 1, true, vec![0])).unwrap(),
        SampleBuffer::from_demuxed(&demuxed(2, 10_000, 1, false, vec![0])).unwrap(),
    ];
    sort_by_presentation(&mut batch);
    let tracks: Vec<u32> = batch.iter().map(|s| s.track()).collect();
    assert_eq!(tracks, vec![1, 2, 3]);
}

#[test]
fn a_crash_reason_keeps_its_text_through_the_process_record() {
    let crash = ProcessCrash::new("SIGSEGV at 0x0", Vec::new());
    assert_eq!(crash.reason.to_string(), "SIGSEGV at 0x0");
    assert_eq!(crash.reason, CrashReason { description: "SIGSEGV at 0x0".into() });
}

#[test]
fn an_empty_crash_reason_displays_as_nothing() {
    assert_eq!(CrashReason::default().to_string(), "");
}
