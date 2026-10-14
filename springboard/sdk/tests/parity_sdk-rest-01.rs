//! Parity sdk-rest-01: headless tests for the decompression session, the hardware decode query and the log claim
//! check. No test opens a decoder library, probes a render node or reads the system journal: the decoder is a
//! `VideoDecoder` double that records what the session sends it, and the probe result is plain data.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bifrost_app::log::{registry_from_manifests, SHELL_SUBSYSTEM};
use bifrost_app::log_ownership::{check_claim, owner_of, ClaimError};
use bifrost_app::video_decode::{Codec, HardwareSupport, Preference};
use bifrost_app::video_decompression::{
    can_accept, codec_for_sample_entry, is_decodable, support_from_probe, DecodedFrame, DecodedPixels, DecompressionSession, FormatDescription,
    SessionError,
};
use springboard::gfx::Image;
use springboard::media::hwdec::vaapi::Support;
use springboard::media::{Codec as MediaCodec, Frame, Pixels, VideoDecoder};

const SPS: [u8; 4] = [0x67, 0x42, 0x00, 0x1e];
const PPS: [u8; 4] = [0x68, 0xce, 0x38, 0x80];
/// Annex B parameter sets, as the decoder must receive them.
const PARAMETER_SETS: [u8; 16] = [0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e, 0, 0, 0, 1, 0x68, 0xce, 0x38, 0x80];
/// One IDR NAL unit with a four-byte length prefix, as a container stores it.
const IDR_SAMPLE: [u8; 7] = [0, 0, 0, 3, 0x65, 0x88, 0x84];
/// The same NAL unit with a start code.
const IDR_ANNEXB: [u8; 7] = [0, 0, 0, 1, 0x65, 0x88, 0x84];

/// An avcC record (version 1, four-byte lengths, one SPS and one PPS).
fn avc_c() -> Vec<u8> {
    let mut record = vec![1, 0x42, 0x00, 0x1e, 0xff, 0xe1, 0x00, 0x04];
    record.extend_from_slice(&SPS);
    record.push(1);
    record.extend_from_slice(&[0x00, 0x04]);
    record.extend_from_slice(&PPS);
    record
}

/// An hvcC header (version 1, no arrays).
fn hvc_c() -> Vec<u8> {
    let mut record = vec![0u8; 23];
    record[0] = 1;
    record
}

#[derive(Default)]
struct Trace {
    configured: Option<(MediaCodec, Vec<u8>, u32, u32)>,
    pushed: Vec<(Vec<u8>, i64)>,
    flushes: usize,
    zero_copy: bool,
}

/// A decoder double: every push yields one picture with the sample's presentation time.
struct Fake {
    trace: Arc<Mutex<Trace>>,
    queued: VecDeque<Frame>,
    refuse_samples: bool,
}

fn gray(pts_us: i64) -> Frame {
    Frame { width: 2, height: 2, pts_us, pixels: Pixels::Nv12 { y: vec![16; 4], uv: vec![128; 2], stride: 2 } }
}

impl VideoDecoder for Fake {
    fn configure(&mut self, codec: MediaCodec, extradata: &[u8], width: u32, height: u32) -> io::Result<()> {
        self.trace.lock().unwrap().configured = Some((codec, extradata.to_vec(), width, height));
        Ok(())
    }

    fn push(&mut self, nal_units: &[u8], pts_us: i64) -> io::Result<()> {
        if self.refuse_samples {
            return Err(io::Error::other("decoder rejected the sample"));
        }
        self.trace.lock().unwrap().pushed.push((nal_units.to_vec(), pts_us));
        self.queued.push_back(gray(pts_us));
        Ok(())
    }

    fn pull(&mut self) -> io::Result<Option<Frame>> {
        Ok(self.queued.pop_front())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.queued.clear();
        self.trace.lock().unwrap().flushes += 1;
        Ok(())
    }

    fn set_zero_copy(&mut self, on: bool) -> bool {
        self.trace.lock().unwrap().zero_copy = on;
        on
    }
}

type Pictures = Rc<RefCell<Vec<DecodedFrame>>>;

fn collector() -> (Pictures, Box<dyn FnMut(DecodedFrame)>) {
    let pictures: Pictures = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&pictures);
    (pictures, Box::new(move |f: DecodedFrame| sink.borrow_mut().push(f)))
}

fn open(codec: Codec, extradata: Vec<u8>, refuse_samples: bool) -> (DecompressionSession, Arc<Mutex<Trace>>, Pictures) {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let (pictures, sink) = collector();
    let format = FormatDescription::new(codec, 640, 360, extradata).unwrap();
    let decoder = Box::new(Fake { trace: Arc::clone(&trace), queued: VecDeque::new(), refuse_samples });
    let session = DecompressionSession::with_decoder(decoder, format, sink).unwrap();
    (session, trace, pictures)
}

// ---- format description ------------------------------------------------------------------------------------------

#[test]
fn a_picture_side_must_be_between_one_and_the_maximum() {
    assert!(matches!(FormatDescription::new(Codec::H264, 0, 720, avc_c()), Err(SessionError::InvalidFormat(_))));
    assert!(matches!(FormatDescription::new(Codec::H264, 1280, 8193, avc_c()), Err(SessionError::InvalidFormat(_))));
    let format = FormatDescription::new(Codec::H264, 8192, 1, avc_c()).unwrap();
    assert_eq!((format.width(), format.height()), (8192, 1));
}

#[test]
fn a_parameter_record_needs_version_one_and_its_fixed_header() {
    let short = vec![1, 0x42, 0x00];
    assert!(matches!(FormatDescription::new(Codec::H264, 640, 360, short), Err(SessionError::InvalidFormat(_))));
    let mut wrong_version = avc_c();
    wrong_version[0] = 2;
    assert!(matches!(FormatDescription::new(Codec::H264, 640, 360, wrong_version), Err(SessionError::InvalidFormat(_))));
    assert!(matches!(FormatDescription::new(Codec::Hevc, 640, 360, avc_c()), Err(SessionError::InvalidFormat(_))), "an hvcC header is 23 bytes");
    assert!(FormatDescription::new(Codec::Hevc, 640, 360, hvc_c()).is_ok());
}

#[test]
fn parameter_sets_already_in_annex_b_form_are_accepted_as_they_are() {
    let format = FormatDescription::new(Codec::H264, 640, 360, PARAMETER_SETS.to_vec()).unwrap();
    assert_eq!(format.extradata(), PARAMETER_SETS.as_slice());
    assert_eq!(format.codec(), Codec::H264);
}

#[test]
fn only_h264_and_hevc_are_decodable_in_this_build() {
    assert!(is_decodable(Codec::H264) && is_decodable(Codec::Hevc));
    assert!(!is_decodable(Codec::Vp9) && !is_decodable(Codec::Av1));
    assert_eq!(FormatDescription::new(Codec::Vp9, 640, 360, avc_c()), Err(SessionError::UnsupportedCodec(Codec::Vp9)));
    assert_eq!(FormatDescription::new(Codec::Av1, 640, 360, avc_c()), Err(SessionError::UnsupportedCodec(Codec::Av1)));
}

#[test]
fn a_sample_entry_code_names_its_codec_and_unknown_codes_name_none() {
    assert_eq!(codec_for_sample_entry(b"avc1"), Some(Codec::H264));
    assert_eq!(codec_for_sample_entry(b"avc3"), Some(Codec::H264));
    assert_eq!(codec_for_sample_entry(b"hvc1"), Some(Codec::Hevc));
    assert_eq!(codec_for_sample_entry(b"hev1"), Some(Codec::Hevc));
    assert_eq!(codec_for_sample_entry(b"vp09"), Some(Codec::Vp9), "named, though not decodable here");
    assert_eq!(codec_for_sample_entry(b"mp4a"), None, "audio has no video codec");
}

// ---- hardware decode query ---------------------------------------------------------------------------------------

#[test]
fn a_render_node_that_cannot_be_probed_supports_no_codec() {
    assert_eq!(support_from_probe(None), HardwareSupport::default());
    assert!(!HardwareSupport::default().supports(Codec::H264));
}

#[test]
fn the_probe_answers_per_codec() {
    let probe = Support { node: "/dev/dri/renderD128".into(), vendor: "Test".into(), h264: true, hevc: false, vp9: true, av1: false };
    let support = support_from_probe(Some(&probe));
    assert!(support.supports(Codec::H264));
    assert!(!support.supports(Codec::Hevc));
    assert!(support.supports(Codec::Vp9));
    assert!(!support.supports(Codec::Av1));
}

#[test]
fn a_format_the_hardware_lacks_is_refused_when_hardware_is_required() {
    let format = FormatDescription::new(Codec::Hevc, 1920, 1080, hvc_c()).unwrap();
    let h264_only = HardwareSupport { h264: true, ..HardwareSupport::default() };
    assert_eq!(can_accept(&format, Preference::RequireHardware, h264_only), Err(SessionError::NoHardwarePath(Codec::Hevc)));
}

#[test]
fn a_format_the_hardware_lacks_falls_back_to_software_when_automatic() {
    let format = FormatDescription::new(Codec::Hevc, 1920, 1080, hvc_c()).unwrap();
    assert_eq!(can_accept(&format, Preference::Automatic, HardwareSupport::default()), Ok(()));
}

#[test]
fn a_format_the_hardware_decodes_is_accepted_when_hardware_is_required() {
    let format = FormatDescription::new(Codec::Hevc, 1920, 1080, hvc_c()).unwrap();
    let hevc = HardwareSupport { hevc: true, ..HardwareSupport::default() };
    assert_eq!(can_accept(&format, Preference::RequireHardware, hevc), Ok(()));
}

// ---- decompression session ---------------------------------------------------------------------------------------

#[test]
fn the_decoder_is_configured_with_the_codec_size_and_parameter_record() {
    let (_session, trace, _pictures) = open(Codec::H264, avc_c(), false);
    assert_eq!(trace.lock().unwrap().configured, Some((MediaCodec::H264, avc_c(), 640, 360)));
}

#[test]
fn the_first_sample_carries_the_parameter_sets_and_later_samples_do_not() {
    let (mut session, trace, _pictures) = open(Codec::H264, avc_c(), false);
    session.decode_frame(&IDR_SAMPLE, 0).unwrap();
    session.decode_frame(&IDR_SAMPLE, 33_333).unwrap();
    let trace = trace.lock().unwrap();
    let first = [PARAMETER_SETS.as_slice(), IDR_ANNEXB.as_slice()].concat();
    assert_eq!(trace.pushed[0], (first, 0));
    assert_eq!(trace.pushed[1], (IDR_ANNEXB.to_vec(), 33_333));
}

#[test]
fn every_decoded_picture_reaches_the_output_handler_in_submission_order() {
    let (mut session, _trace, pictures) = open(Codec::H264, avc_c(), false);
    session.decode_frame(&IDR_SAMPLE, 0).unwrap();
    session.decode_frame(&IDR_SAMPLE, 33_333).unwrap();
    let pictures = pictures.borrow();
    assert_eq!(pictures.len(), 2);
    assert_eq!((pictures[0].presentation_us, pictures[1].presentation_us), (0, 33_333));
    assert_eq!((pictures[0].width, pictures[0].height), (2, 2));
    assert!(matches!(&pictures[0].pixels, DecodedPixels::Nv12 { luma, stride: 2, .. } if luma.len() == 4));
}

#[test]
fn a_flush_drops_the_pictures_in_flight_and_the_next_sample_carries_the_parameter_sets_again() {
    let (mut session, trace, _pictures) = open(Codec::H264, avc_c(), false);
    session.decode_frame(&IDR_SAMPLE, 0).unwrap();
    session.flush().unwrap();
    session.decode_frame(&IDR_SAMPLE, 1_000).unwrap();
    let trace = trace.lock().unwrap();
    assert_eq!(trace.flushes, 1);
    assert_eq!(trace.pushed[1].0, [PARAMETER_SETS.as_slice(), IDR_ANNEXB.as_slice()].concat());
}

#[test]
fn a_refused_sample_is_reported_with_the_decoder_message_and_no_picture_is_output() {
    let (mut session, trace, pictures) = open(Codec::H264, avc_c(), true);
    let err = session.decode_frame(&IDR_SAMPLE, 0).unwrap_err();
    assert_eq!(err, SessionError::Decoder("decoder rejected the sample".into()));
    assert!(pictures.borrow().is_empty());
    assert!(trace.lock().unwrap().pushed.is_empty());
}

#[test]
fn an_empty_sample_is_refused_before_the_decoder_sees_it() {
    let (mut session, trace, _pictures) = open(Codec::H264, avc_c(), false);
    assert!(matches!(session.decode_frame(&[], 0), Err(SessionError::InvalidSample(_))));
    assert!(trace.lock().unwrap().pushed.is_empty());
}

#[test]
fn zero_copy_is_requested_from_the_decoder_and_its_answer_is_returned() {
    let (mut session, trace, _pictures) = open(Codec::H264, avc_c(), false);
    assert!(session.set_zero_copy(true));
    assert!(trace.lock().unwrap().zero_copy);
}

#[test]
fn a_decodable_format_opens_a_session_over_the_decoder_it_is_given() {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let (_pictures, sink) = collector();
    let format = FormatDescription::new(Codec::H264, 640, 360, avc_c()).unwrap();
    let decoder = Box::new(Fake { trace, queued: VecDeque::new(), refuse_samples: false });
    let session = DecompressionSession::with_decoder(decoder, format, sink);
    assert!(session.is_ok());
}

#[test]
fn a_decoded_picture_keeps_its_rgba_pixels_and_its_presentation_time() {
    let frame = Frame { width: 2, height: 1, pts_us: 42, pixels: Pixels::Rgba(Image::new(2, 1)) };
    let picture = DecodedFrame::from(frame);
    assert_eq!((picture.width, picture.height, picture.presentation_us), (2, 1, 42));
    assert!(matches!(&picture.pixels, DecodedPixels::Rgba(img) if img.w == 2 && img.h == 1));
}

// ---- subsystem ownership on read ---------------------------------------------------------------------------------

const NOTES_MANIFEST: &str = "id = \"is.o.notes.desktop\"\n[[log.subsystem]]\nid = \"is.o.notes\"\ncategories = [\"sync\"]\n";

fn store_exe(app: &str) -> String {
    format!("/var/lib/springboard/apps/{app}/1.0.0/bin")
}

#[test]
fn the_app_that_declared_a_subsystem_may_log_as_it() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    assert_eq!(check_claim(&registry, Some(&store_exe("is.o.notes.desktop")), "is.o.notes"), Ok(()));
}

#[test]
fn another_app_is_refused_a_subsystem_it_did_not_declare() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    let refused = check_claim(&registry, Some(&store_exe("is.o.evil")), "is.o.notes");
    assert_eq!(refused, Err(ClaimError::OwnedByAnother { subsystem: "is.o.notes".into(), owner: "is.o.notes.desktop".into() }));
    assert_eq!(refused.unwrap_err().to_string(), "subsystem is.o.notes belongs to is.o.notes.desktop");
}

#[test]
fn a_system_binary_may_log_as_any_valid_subsystem_including_the_shell() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    assert_eq!(check_claim(&registry, Some("/usr/bin/springboard"), "is.o.notes"), Ok(()));
    assert_eq!(check_claim(&registry, None, SHELL_SUBSYSTEM), Ok(()));
}

#[test]
fn an_undeclared_subsystem_is_open_to_store_apps_and_the_shell_subsystem_is_not() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    assert_eq!(check_claim(&registry, Some(&store_exe("is.o.evil")), "is.o.free"), Ok(()));
    assert_eq!(check_claim(&registry, Some(&store_exe("is.o.evil")), SHELL_SUBSYSTEM), Err(ClaimError::Reserved(SHELL_SUBSYSTEM.into())));
}

#[test]
fn a_subsystem_id_with_spaces_is_invalid() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    assert_eq!(check_claim(&registry, None, "is o notes"), Err(ClaimError::Invalid("is o notes".into())));
}

#[test]
fn the_owner_of_a_subsystem_is_the_app_that_declared_it_and_only_that() {
    let registry = registry_from_manifests(&[NOTES_MANIFEST]);
    assert_eq!(owner_of(&registry, "is.o.notes"), Some("is.o.notes.desktop"));
    assert_eq!(owner_of(&registry, "is.o.free"), None);
}
