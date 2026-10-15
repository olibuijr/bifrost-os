//! Parity sdk-os-02: headless model tests for the system-owned capabilities that apps see through the SDK.
//! Each model checks its inputs and computes its results. No test opens a device, a file, an audio stream or a display.

use bifrost_app::biquad::{Biquad, Coefficients, Design, FilterError};
use bifrost_app::drawable_pool::{DrawableId, DrawablePool, PoolError};
use bifrost_app::extent_map::{Extent, ExtentError, ExtentKind, ExtentMap};
use bifrost_app::gpu_device::{GpuInventory, RenderNode, FIRST_RENDER_MINOR};
use bifrost_app::gpu_pass::{blur_plan, ColorAttachment, FixedPipeline, LoadAction, PassError, RenderPass, StoreAction};
use bifrost_app::pixel_buffer::{Layout, PixelError, PixelFormat};

// Device inventory: the render nodes of this machine, in the order the default device is chosen.

#[test]
fn only_render_node_names_become_devices_in_minor_order() {
    let names = ["card0", "renderD129", "renderD128", "renderD12", "renderDx", "renderD128", "by-path"];
    let inventory = GpuInventory::from_entry_names(names);
    let minors: Vec<u32> = inventory.devices().iter().map(|n| n.minor).collect();
    assert_eq!(minors, vec![128, 129], "card nodes, short numbers and duplicates are not devices");
}

#[test]
fn the_default_device_is_the_lowest_render_node() {
    let inventory = GpuInventory::from_entry_names(["renderD129", "renderD128"]);
    let default = inventory.default_device().expect("two render nodes");
    assert_eq!(default.path(), "/dev/dri/renderD128");
    assert_eq!(FIRST_RENDER_MINOR, 128);
}

#[test]
fn a_machine_without_a_render_node_has_no_default_device() {
    let inventory = GpuInventory::from_entry_names(["card0", "controlD64"]);
    assert!(inventory.devices().is_empty());
    assert_eq!(inventory.default_device(), None);
    assert_eq!(RenderNode::from_entry_name("renderD127"), None, "below the first render minor");
}

// Drawables: a fixed pool of frame buffers, taken, presented and returned.

#[test]
fn a_pool_holds_two_or_three_drawables() {
    assert_eq!(DrawablePool::new(1, 640, 480).unwrap_err(), PoolError::CountOutOfRange(1));
    assert_eq!(DrawablePool::new(4, 640, 480).unwrap_err(), PoolError::CountOutOfRange(4));
    assert_eq!(DrawablePool::new(3, 640, 480).unwrap().count(), 3);
}

#[test]
fn a_pool_rejects_an_empty_drawable_size() {
    assert_eq!(DrawablePool::new(2, 0, 480).unwrap_err(), PoolError::EmptySize);
    assert_eq!(DrawablePool::new(2, 640, 0).unwrap_err(), PoolError::EmptySize);
}

#[test]
fn next_drawable_is_none_when_every_drawable_is_busy() {
    let mut pool = DrawablePool::new(2, 640, 480).unwrap();
    assert_eq!(pool.next_drawable(), Some(DrawableId(0)));
    assert_eq!(pool.next_drawable(), Some(DrawableId(1)));
    assert_eq!(pool.next_drawable(), None, "the caller skips the frame");
    assert_eq!(pool.free_count(), 0);
}

#[test]
fn a_presented_drawable_returns_to_the_pool_after_release() {
    let mut pool = DrawablePool::new(2, 640, 480).unwrap();
    let first = pool.next_drawable().unwrap();
    pool.present(first).unwrap();
    let second = pool.next_drawable().unwrap();
    assert_eq!(pool.next_drawable(), None, "the presented drawable is still with the display");
    pool.release(first).unwrap();
    assert_eq!(pool.next_drawable(), Some(first));
    assert_ne!(first, second);
}

#[test]
fn only_a_drawing_drawable_can_be_presented_or_abandoned() {
    let mut pool = DrawablePool::new(2, 640, 480).unwrap();
    assert_eq!(pool.present(DrawableId(0)), Err(PoolError::WrongState(DrawableId(0))));
    let id = pool.next_drawable().unwrap();
    pool.present(id).unwrap();
    assert_eq!(pool.present(id), Err(PoolError::WrongState(id)), "a queued drawable cannot be queued twice");
    assert_eq!(pool.release(DrawableId(1)), Err(PoolError::WrongState(DrawableId(1))));
}

#[test]
fn an_abandoned_frame_frees_its_drawable_without_presenting() {
    let mut pool = DrawablePool::new(2, 640, 480).unwrap();
    let id = pool.next_drawable().unwrap();
    pool.abandon(id).unwrap();
    assert_eq!(pool.free_count(), 2);
}

#[test]
fn the_drawable_size_changes_only_while_the_pool_is_idle() {
    let mut pool = DrawablePool::new(2, 640, 480).unwrap();
    let id = pool.next_drawable().unwrap();
    assert_eq!(pool.resize(800, 600), Err(PoolError::InUse));
    pool.abandon(id).unwrap();
    pool.resize(800, 600).unwrap();
    assert_eq!(pool.size(), (800, 600));
    assert_eq!(pool.resize(0, 600), Err(PoolError::EmptySize));
}

// Render passes: validated descriptions over a fixed set of compositor pipelines.

fn attachment() -> ColorAttachment {
    ColorAttachment { load: LoadAction::Clear, store: StoreAction::Store }
}

#[test]
fn the_fixed_pipelines_are_quad_glass_and_shadow() {
    assert_eq!(FixedPipeline::ALL.len(), 3);
    assert_eq!(FixedPipeline::from_name("glass"), Some(FixedPipeline::Glass));
    assert_eq!(FixedPipeline::from_name("custom"), None, "applications cannot add pipelines");
    assert_eq!(FixedPipeline::Shadow.name(), "shadow");
}

#[test]
fn a_pass_needs_one_to_eight_colour_attachments() {
    let none = RenderPass::new(FixedPipeline::Quad, 640, 480, vec![]);
    assert_eq!(none.unwrap_err(), PassError::NoAttachments);
    let nine = RenderPass::new(FixedPipeline::Quad, 640, 480, vec![attachment(); 9]);
    assert_eq!(nine.unwrap_err(), PassError::TooManyAttachments(9));
    let eight = RenderPass::new(FixedPipeline::Quad, 640, 480, vec![attachment(); 8]).unwrap();
    assert_eq!(eight.colors.len(), 8);
}

#[test]
fn a_pass_rejects_an_empty_target() {
    let result = RenderPass::new(FixedPipeline::Glass, 0, 480, vec![attachment()]);
    assert_eq!(result.unwrap_err(), PassError::EmptySize);
}

#[test]
fn a_valid_pass_keeps_its_pipeline_size_and_attachments() {
    let pass = RenderPass::new(FixedPipeline::Glass, 1280, 720, vec![attachment(), attachment()]).unwrap();
    assert_eq!(pass.pipeline, FixedPipeline::Glass);
    assert_eq!((pass.width, pass.height), (1280, 720));
    assert_eq!(pass.colors[0].load, LoadAction::Clear);
}

// Reduced-resolution material work: the blur divisor and the working size, scaled back up linearly.

#[test]
fn small_blur_radii_keep_full_resolution() {
    for radius in 0..=3 {
        assert_eq!(blur_plan(1000, 800, radius).divisor, 1, "radius {radius}");
    }
}

#[test]
fn medium_and_large_blur_radii_work_at_half_and_quarter_size() {
    assert_eq!(blur_plan(1000, 800, 4).divisor, 2);
    assert_eq!(blur_plan(1000, 800, 7).divisor, 2);
    assert_eq!(blur_plan(1000, 800, 8).divisor, 4);
    assert_eq!(blur_plan(1000, 800, 100).divisor, 4);
}

#[test]
fn the_working_size_is_the_divided_size_rounded_up() {
    let plan = blur_plan(1000, 800, 8);
    assert_eq!((plan.width, plan.height), (250, 200));
    let odd = blur_plan(1001, 801, 8);
    assert_eq!((odd.width, odd.height), (251, 201), "no column or row is lost");
}

#[test]
fn a_target_too_small_for_the_divisor_is_blurred_at_full_size() {
    let plan = blur_plan(7, 100, 8);
    assert_eq!(plan.divisor, 1);
    assert_eq!((plan.width, plan.height), (7, 100));
}

// Pixel buffers: format, plane layout and addressing for frames and images.

#[test]
fn a_bgra_buffer_is_four_bytes_per_pixel_with_no_padding() {
    let layout = Layout::new(PixelFormat::Bgra8888, 3, 2, 1).unwrap();
    assert_eq!(layout.planes.len(), 1);
    assert_eq!(layout.planes[0].bytes_per_row, 12);
    assert_eq!(layout.total_bytes, 24);
}

#[test]
fn rows_are_padded_to_the_requested_alignment() {
    let layout = Layout::new(PixelFormat::Bgra8888, 3, 2, 64).unwrap();
    assert_eq!(layout.planes[0].bytes_per_row, 64, "12 bytes round up to one 64-byte row");
    assert_eq!(layout.total_bytes, 128);
}

#[test]
fn an_nv12_frame_has_a_full_luma_plane_and_a_half_height_chroma_plane() {
    let layout = Layout::new(PixelFormat::Nv12, 1920, 1080, 1).unwrap();
    assert_eq!(layout.planes.len(), 2);
    assert_eq!(layout.planes[0].height, 1080);
    assert_eq!(layout.planes[1].height, 540);
    assert_eq!(layout.planes[1].offset, 1920 * 1080);
    assert_eq!(layout.total_bytes, 1920 * 1080 * 3 / 2, "4:2:0 is one and a half bytes per pixel");
}

#[test]
fn nv12_needs_even_dimensions_and_a_power_of_two_alignment() {
    assert_eq!(Layout::new(PixelFormat::Nv12, 1921, 1080, 1).unwrap_err(), PixelError::OddSize);
    assert_eq!(Layout::new(PixelFormat::Nv12, 1920, 1081, 1).unwrap_err(), PixelError::OddSize);
    assert_eq!(Layout::new(PixelFormat::Bgra8888, 4, 4, 3).unwrap_err(), PixelError::BadAlignment(3));
    assert_eq!(Layout::new(PixelFormat::Bgra8888, 0, 4, 1).unwrap_err(), PixelError::EmptySize);
}

#[test]
fn an_offset_is_found_inside_a_plane_and_refused_outside_it() {
    let layout = Layout::new(PixelFormat::Nv12, 1920, 1080, 1).unwrap();
    assert_eq!(layout.offset_of(0, 5, 3), Some(3 * 1920 + 5), "luma is one byte per pixel");
    assert_eq!(layout.offset_of(1, 2, 3), Some(1920 * 1080 + 3 * 1920 + 2 * 2), "a chroma pair is two bytes");
    assert_eq!(layout.offset_of(0, 1920, 0), None, "x is past the luma width");
    assert_eq!(layout.offset_of(1, 960, 0), None, "x is past the chroma width of 960 pairs");
    assert_eq!(layout.offset_of(1, 0, 540), None, "y is past the chroma height");
    assert_eq!(layout.offset_of(2, 0, 0), None, "there are only two planes");
}

// Biquad filters: second-order sections with the standard audio cookbook coefficients.

#[test]
fn a_filter_rejects_a_bad_rate_frequency_or_q() {
    assert_eq!(Coefficients::design(Design::LowPass, 0.0, 1000.0, 0.7).unwrap_err(), FilterError::BadSampleRate);
    assert_eq!(Coefficients::design(Design::LowPass, 48000.0, 24000.0, 0.7).unwrap_err(), FilterError::FrequencyOutOfRange);
    assert_eq!(Coefficients::design(Design::LowPass, 48000.0, 0.0, 0.7).unwrap_err(), FilterError::FrequencyOutOfRange);
    assert_eq!(Coefficients::design(Design::HighPass, 48000.0, 1000.0, 0.0).unwrap_err(), FilterError::BadQ);
}

#[test]
fn a_low_pass_passes_direct_current_at_unity_gain() {
    let c = Coefficients::design(Design::LowPass, 48000.0, 1000.0, 0.7071).unwrap();
    assert!(c.magnitude_db(0.0, 48000.0).abs() < 1e-9);
}

#[test]
fn a_butterworth_low_pass_is_minus_three_decibels_at_its_cutoff() {
    let c = Coefficients::design(Design::LowPass, 48000.0, 1000.0, std::f64::consts::FRAC_1_SQRT_2).unwrap();
    let db = c.magnitude_db(1000.0, 48000.0);
    assert!((db + 3.0103).abs() < 0.01, "got {db}");
}

#[test]
fn a_high_pass_removes_direct_current() {
    let c = Coefficients::design(Design::HighPass, 48000.0, 1000.0, 0.7071).unwrap();
    assert!(c.magnitude_db(0.0, 48000.0) < -200.0);
}

#[test]
fn a_peak_filter_gives_its_full_gain_at_its_centre_frequency() {
    let c = Coefficients::design(Design::Peak { gain_db: 6.0 }, 48000.0, 1000.0, 1.0).unwrap();
    assert!((c.magnitude_db(1000.0, 48000.0) - 6.0).abs() < 1e-6);
}

#[test]
fn a_peak_filter_at_zero_decibels_leaves_every_frequency_alone() {
    let c = Coefficients::design(Design::Peak { gain_db: 0.0 }, 48000.0, 1000.0, 2.0).unwrap();
    assert!(c.magnitude_db(123.0, 48000.0).abs() < 1e-9);
    assert!(c.magnitude_db(9000.0, 48000.0).abs() < 1e-9);
}

#[test]
fn a_low_pass_step_settles_at_one() {
    let c = Coefficients::design(Design::LowPass, 48000.0, 1000.0, 0.7071).unwrap();
    let mut filter = Biquad::new(c);
    let mut block = vec![1.0; 2000];
    filter.process_block(&mut block);
    assert!((block[1999] - 1.0).abs() < 1e-6);
    assert!(block[0] < 0.5, "the first sample has not yet passed the filter");
}

#[test]
fn a_block_gives_the_same_output_as_sample_by_sample() {
    let c = Coefficients::design(Design::Peak { gain_db: -4.0 }, 44100.0, 500.0, 1.5).unwrap();
    let input = [0.5, -0.25, 0.75, 0.0, -1.0, 0.3];
    let mut one = Biquad::new(c);
    let stepped: Vec<f64> = input.iter().map(|&x| one.process(x)).collect();
    let mut two = Biquad::new(c);
    let mut block = input.to_vec();
    two.process_block(&mut block);
    assert_eq!(stepped, block);
}

#[test]
fn reset_clears_the_filter_history() {
    let c = Coefficients::design(Design::LowPass, 48000.0, 1000.0, 0.7071).unwrap();
    let mut filter = Biquad::new(c);
    filter.process(1.0);
    filter.process(1.0);
    filter.reset();
    assert_eq!(filter.process(0.0), 0.0);
}

// Extent maps: which byte ranges of a file are stored where on the volume.

fn data(file_offset: u64, length: u64, disk: u64) -> Extent {
    Extent { file_offset, length, kind: ExtentKind::Data, disk_offset: Some(disk) }
}

#[test]
fn contiguous_extents_merge_in_file_and_on_disk() {
    let map = ExtentMap::new(vec![data(0, 4096, 1000), data(4096, 4096, 5096)]).unwrap();
    assert_eq!(map.extents().len(), 1);
    assert_eq!(map.extents()[0].length, 8192);
    assert_eq!(map.end(), 8192);
}

#[test]
fn extents_stay_apart_when_the_disk_is_not_contiguous() {
    let map = ExtentMap::new(vec![data(0, 4096, 1000), data(4096, 4096, 9000)]).unwrap();
    assert_eq!(map.extents().len(), 2);
}

#[test]
fn an_extent_must_have_a_length_and_a_disk_address_that_fits_its_kind() {
    assert_eq!(ExtentMap::new(vec![data(0, 0, 1000)]).unwrap_err(), ExtentError::ZeroLength);
    let hole_on_disk = Extent { file_offset: 0, length: 10, kind: ExtentKind::Hole, disk_offset: Some(1) };
    assert_eq!(ExtentMap::new(vec![hole_on_disk]).unwrap_err(), ExtentError::BadDiskAddress);
    let data_without_disk = Extent { file_offset: 0, length: 10, kind: ExtentKind::Data, disk_offset: None };
    assert_eq!(ExtentMap::new(vec![data_without_disk]).unwrap_err(), ExtentError::BadDiskAddress);
}

#[test]
fn overlapping_extents_are_refused_at_the_first_shared_byte() {
    let result = ExtentMap::new(vec![data(0, 100, 0), data(50, 100, 500)]);
    assert_eq!(result.unwrap_err(), ExtentError::Overlap(50));
}

#[test]
fn input_in_any_order_is_sorted_before_it_is_checked() {
    let map = ExtentMap::new(vec![data(4096, 4096, 9000), data(0, 4096, 1000)]).unwrap();
    assert_eq!(map.extents()[0].file_offset, 0);
    assert_eq!(map.locate(5000).unwrap().disk_offset, Some(9000 + 904));
}

#[test]
fn a_file_offset_maps_to_its_disk_offset_inside_an_extent() {
    let map = ExtentMap::new(vec![data(0, 4096, 1000), data(4096, 4096, 5096)]).unwrap();
    let location = map.locate(5000).expect("inside the file");
    assert_eq!(location.kind, ExtentKind::Data);
    assert_eq!(location.disk_offset, Some(6000));
    assert_eq!(map.locate(8192), None, "the end of the file is not inside it");
}

#[test]
fn a_hole_has_no_disk_address() {
    let hole = Extent { file_offset: 0, length: 4096, kind: ExtentKind::Hole, disk_offset: None };
    let map = ExtentMap::new(vec![hole, data(4096, 4096, 1000)]).unwrap();
    let location = map.locate(100).unwrap();
    assert_eq!(location.kind, ExtentKind::Hole);
    assert_eq!(location.disk_offset, None);
}
