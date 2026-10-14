//! Parity sdk-rest-12: headless tests for the drawable setup model, the sprite batching model and the video
//! decoder choice. No test opens a device, a socket or a decoder library, and no test draws a frame.

use bifrost_ui::drawable_setup::{DevicePreference, DrawableSetup, SetupError, FORMAT_ALPHA, FORMAT_OPAQUE};
use bifrost_ui::sprite_batch::{QuadError, Run, SpriteBatch, MAX_QUADS, VERTICES_PER_QUAD};
use bifrost_ui::video_decoder_plan::{choose, DecodePath, DecoderRequest, HardwareSupport, VideoCodec};

const LIMIT: u32 = 16384;
const UNIT_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];

fn setup(width: u32, height: u32) -> DrawableSetup {
    DrawableSetup { width, height, alpha: false, device: DevicePreference::Default }
}

// ---- drawable setup ----

#[test]
fn an_opaque_drawable_uses_the_presenter_opaque_code() {
    assert_eq!(FORMAT_OPAQUE, 0x3432_4258, "XB24, the code the presenter allocates opaque buffers with");
    assert_eq!(setup(4, 4).validate(LIMIT).unwrap().format, 0x3432_4258);
}

#[test]
fn an_alpha_drawable_uses_the_presenter_alpha_code() {
    assert_eq!(FORMAT_ALPHA, 0x3432_4241, "AB24");
    let d = DrawableSetup { alpha: true, ..setup(4, 4) }.validate(LIMIT).unwrap();
    assert_eq!(d.format, 0x3432_4241);
}

#[test]
fn a_zero_side_is_refused_with_both_sides_named() {
    assert_eq!(setup(0, 720).validate(LIMIT), Err(SetupError::ZeroSize { width: 0, height: 720 }));
    assert_eq!(setup(1280, 0).validate(LIMIT), Err(SetupError::ZeroSize { width: 1280, height: 0 }));
}

#[test]
fn the_longer_side_is_checked_against_the_device_limit() {
    assert!(setup(LIMIT, 1).validate(LIMIT).is_ok(), "a side equal to the limit is accepted");
    assert_eq!(setup(1, LIMIT + 1).validate(LIMIT), Err(SetupError::TooLarge { side: LIMIT + 1, max: LIMIT }));
}

#[test]
fn a_render_node_must_be_a_numbered_dri_node() {
    let ok = DrawableSetup { device: DevicePreference::RenderNode("/dev/dri/renderD128".into()), ..setup(2, 2) };
    let d = ok.validate(LIMIT).unwrap();
    assert_eq!(d.device, DevicePreference::RenderNode("/dev/dri/renderD128".into()));
    for bad in ["/dev/dri/card0", "/dev/dri/renderD", "/dev/dri/renderDx", "/dev/null"] {
        let req = DrawableSetup { device: DevicePreference::RenderNode(bad.into()), ..setup(2, 2) };
        assert_eq!(req.validate(LIMIT), Err(SetupError::BadRenderNode(bad.into())), "{bad}");
    }
}

#[test]
fn the_buffer_size_is_four_bytes_per_pixel_without_padding() {
    let d = setup(320, 200).validate(LIMIT).unwrap();
    assert_eq!(d.row_bytes(), 1280);
    assert_eq!(d.byte_len(), 256_000);
}

#[test]
fn the_default_device_is_the_shell_device() {
    assert_eq!(DevicePreference::default(), DevicePreference::Default);
}

#[test]
fn a_setup_error_names_the_limit_in_its_message() {
    let text = SetupError::TooLarge { side: 20000, max: 16384 }.to_string();
    assert!(text.contains("20000") && text.contains("16384"), "{text}");
}

// ---- sprite batch ----

#[test]
fn a_quad_is_two_triangles_in_corner_order() {
    let mut b = SpriteBatch::new();
    b.push_quad([10.0, 20.0, 30.0, 40.0], UNIT_UV, WHITE, None).unwrap();
    let v = b.vertices();
    assert_eq!(v.len(), VERTICES_PER_QUAD);
    assert_eq!(v[0].pos, [10.0, 20.0], "top left");
    assert_eq!(v[1].pos, [40.0, 20.0], "top right");
    assert_eq!(v[2].pos, [10.0, 60.0], "bottom left");
    assert_eq!(v[3].pos, v[1].pos, "the second triangle starts at the top right");
    assert_eq!(v[4].pos, [40.0, 60.0], "bottom right");
    assert_eq!(v[5].pos, v[2].pos, "the second triangle ends at the bottom left");
}

#[test]
fn the_texture_coordinates_follow_the_corners() {
    let mut b = SpriteBatch::new();
    b.push_quad([0.0, 0.0, 1.0, 1.0], [0.25, 0.5, 0.75, 1.0], WHITE, Some(3)).unwrap();
    assert_eq!(b.vertices()[0].uv, [0.25, 0.5]);
    assert_eq!(b.vertices()[1].uv, [0.75, 0.5]);
    assert_eq!(b.vertices()[2].uv, [0.25, 1.0]);
    assert_eq!(b.vertices()[4].uv, [0.75, 1.0]);
}

#[test]
fn an_empty_area_is_refused_and_adds_nothing() {
    let mut b = SpriteBatch::new();
    assert_eq!(b.push_quad([0.0, 0.0, 0.0, 5.0], UNIT_UV, WHITE, None), Err(QuadError::EmptyArea));
    assert_eq!(b.push_quad([0.0, 0.0, 5.0, -1.0], UNIT_UV, WHITE, None), Err(QuadError::EmptyArea));
    assert_eq!(b.quads(), 0);
    assert!(b.runs().is_empty());
}

#[test]
fn solid_quads_join_the_textured_run_they_surround() {
    let mut b = SpriteBatch::new();
    b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, None).unwrap();
    b.push_quad([1.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, Some(7)).unwrap();
    b.push_quad([2.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, None).unwrap();
    assert_eq!(b.runs(), &[Run { texture: Some(7), start: 0, count: 18 }]);
}

#[test]
fn a_change_of_texture_starts_a_new_run() {
    let mut b = SpriteBatch::new();
    b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, Some(1)).unwrap();
    b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, Some(1)).unwrap();
    b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, Some(2)).unwrap();
    assert_eq!(
        b.runs(),
        &[Run { texture: Some(1), start: 0, count: 12 }, Run { texture: Some(2), start: 12, count: 6 }]
    );
}

#[test]
fn a_batch_is_full_after_max_quads_and_refuses_more_until_cleared() {
    let mut b = SpriteBatch::new();
    for i in 0..MAX_QUADS {
        b.push_quad([i as f32, 0.0, 1.0, 1.0], UNIT_UV, WHITE, None).unwrap();
    }
    assert!(b.is_full());
    assert_eq!(b.quads(), 4096);
    assert_eq!(b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, None), Err(QuadError::Full));
    b.clear();
    assert_eq!(b.quads(), 0);
    assert!(b.push_quad([0.0, 0.0, 1.0, 1.0], UNIT_UV, WHITE, None).is_ok());
}

#[test]
fn a_vertex_is_eight_floats_of_thirty_two_bytes() {
    assert_eq!(std::mem::size_of::<bifrost_ui::sprite_batch::Vertex>(), 32);
    assert_eq!(VERTICES_PER_QUAD, 6);
}

// ---- video decoder choice ----

fn request(codec: VideoCodec) -> DecoderRequest {
    DecoderRequest { codec, support: HardwareSupport { h264: true, hevc: false }, software_forced: false, library_matches: true }
}

#[test]
fn sample_entry_codes_name_the_two_codecs() {
    assert_eq!(VideoCodec::from_sample_entry("avc1"), Some(VideoCodec::H264));
    assert_eq!(VideoCodec::from_sample_entry("avc3"), Some(VideoCodec::H264));
    assert_eq!(VideoCodec::from_sample_entry("hvc1"), Some(VideoCodec::Hevc));
    assert_eq!(VideoCodec::from_sample_entry("hev1"), Some(VideoCodec::Hevc));
    assert_eq!(VideoCodec::from_sample_entry("mp4a"), None, "an audio entry is not a video codec");
}

#[test]
fn a_supported_codec_decodes_in_hardware() {
    assert_eq!(choose(request(VideoCodec::H264)), DecodePath::Hardware);
}

#[test]
fn a_codec_the_gpu_lacks_decodes_in_software() {
    assert_eq!(choose(request(VideoCodec::Hevc)), DecodePath::Software);
}

#[test]
fn the_software_switch_wins_over_hardware_support() {
    let forced = DecoderRequest { software_forced: true, ..request(VideoCodec::H264) };
    assert_eq!(choose(forced), DecodePath::Software);
}

#[test]
fn a_mismatched_library_is_never_used_for_hardware() {
    let other = DecoderRequest { library_matches: false, ..request(VideoCodec::H264) };
    assert_eq!(choose(other), DecodePath::Software);
}

#[test]
fn no_hardware_support_at_all_means_software_for_every_codec() {
    let none = DecoderRequest { support: HardwareSupport::default(), ..request(VideoCodec::H264) };
    assert_eq!(choose(none), DecodePath::Software);
}

#[test]
fn a_failed_hardware_stream_falls_back_once_to_software() {
    assert_eq!(DecodePath::Hardware.fallback(), Some(DecodePath::Software));
    assert_eq!(DecodePath::Software.fallback(), None, "software is the last resort");
}
