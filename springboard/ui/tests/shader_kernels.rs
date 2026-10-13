//! Golden-pixel tests for the shader library and the image kernel path. No GPU adapter is used: the CPU kernels are
//! the reference, and a GPU back end would be compared against them where an adapter exists.
use bifrost_render::gfx::Image;
use bifrost_ui::image_kernels::{convolve, histogram, source_over, Kernel, KernelKind};
use bifrost_ui::shader_library::{LibraryError, ShaderLibrary};
use bifrost_ui::shaders::{
    ci_color_matrix, ci_filter, ci_filter_names, is_builtin_kernel, library, make_compute_pipeline_state, CiKernel,
    FunctionConstantValues, KernelError,
};

fn px(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

fn solid(w: usize, h: usize, p: u32) -> Image {
    Image { w, h, px: vec![p; w * h] }
}

fn ch(p: u32, i: u32) -> i32 {
    (p >> (8 * i) & 255) as i32
}

fn near(a: u32, b: u32, tol: i32) -> bool {
    (0..4).all(|i| (ch(a, i) - ch(b, i)).abs() <= tol)
}

const WHITE: u32 = 0xffff_ffff;

#[test]
fn library_looks_functions_up_by_name_and_errors_on_a_missing_one() {
    let lib = library();
    assert!(lib.make_function("ci_gaussian_blur").is_ok());
    assert_eq!(lib.make_function("nope"), Err(LibraryError::NotFound("nope".into())));
    assert!(is_builtin_kernel("ci_sepia_tone"));
    assert!(!is_builtin_kernel("quad_vertex"));
    assert_eq!(lib.check_render_pipeline("quad_vertex", "quad_fragment"), Ok(()));
}

#[test]
fn make_library_from_source_feeds_pipeline_creation() {
    let lib = ShaderLibrary::from_source("kernel void ci_sepia_tone(texture2d<float> t)").unwrap();
    assert!(make_compute_pipeline_state(&lib, "ci_sepia_tone", &FunctionConstantValues::new()).is_ok());
    let err = make_compute_pipeline_state(&lib, "ci_vignette", &FunctionConstantValues::new()).unwrap_err();
    assert_eq!(err, KernelError::Library(LibraryError::NotFound("ci_vignette".into())));
}

#[test]
fn function_constants_specialise_a_kernel() {
    let lib = library();
    let none = make_compute_pipeline_state(&lib, "ci_sepia_tone", &FunctionConstantValues::new()).unwrap();
    let mut c = FunctionConstantValues::new();
    c.set("intensity", 0.0);
    let off = make_compute_pipeline_state(&lib, "ci_sepia_tone", &c).unwrap();
    assert_eq!(none.kernel(), &Kernel::Sepia { intensity: 1.0 });
    let img = solid(2, 2, px(10, 200, 30, 255));
    assert_eq!(off.run(&img), img, "intensity 0 leaves the image alone");
    assert_ne!(none.run(&img), img);
}

#[test]
fn a_function_that_needs_data_is_not_a_constants_only_pipeline() {
    let err = make_compute_pipeline_state(&library(), "mps_image_convolution", &FunctionConstantValues::new());
    assert_eq!(err, Err(KernelError::NeedsData("mps_image_convolution".into())));
    let err = make_compute_pipeline_state(&library(), "quad_vertex", &FunctionConstantValues::new());
    assert!(matches!(err, Err(KernelError::Library(LibraryError::WrongStage { .. }))));
}

#[test]
fn sepia_of_white_is_warm() {
    let out = Kernel::Sepia { intensity: 1.0 }.run(&solid(1, 1, WHITE));
    assert!(near(out.px[0], px(255, 255, 239, 255), 1), "{:08x}", out.px[0]);
}

#[test]
fn color_controls_zero_saturation_is_grey_and_defaults_are_identity() {
    let img = solid(1, 1, px(255, 0, 0, 255));
    let grey = Kernel::ColorControls { saturation: 0.0, brightness: 0.0, contrast: 1.0 }.run(&img).px[0];
    assert!(ch(grey, 0) == ch(grey, 1) && ch(grey, 1) == ch(grey, 2), "{grey:08x}");
    assert!(near(grey, px(76, 76, 76, 255), 1));
    let same = Kernel::ColorControls { saturation: 1.0, brightness: 0.0, contrast: 1.0 }.run(&img);
    assert!(near(same.px[0], img.px[0], 1));
}

#[test]
fn color_matrix_swaps_channels_and_keeps_alpha() {
    let f = ci_color_matrix([0.0, 0.0, 1.0, 0.0], [0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0; 4]);
    let out = f.run(&solid(1, 1, px(255, 0, 0, 255)));
    assert!(near(out.px[0], px(0, 0, 255, 255), 1));
}

#[test]
fn vignette_darkens_corners_but_not_the_centre() {
    let out = Kernel::Vignette { intensity: 1.0, radius: 1.0 }.run(&solid(9, 9, WHITE));
    assert!(ch(out.px[4 * 9 + 4], 0) > 250, "centre stays bright");
    assert!(ch(out.px[0], 0) < 40, "corner is dark");
    assert_eq!(ch(out.px[0], 3), 255, "alpha is untouched");
    let none = Kernel::Vignette { intensity: 0.0, radius: 1.0 }.run(&solid(4, 4, WHITE));
    assert!(none.px.iter().all(|&p| p == WHITE));
}

#[test]
fn gaussian_blur_keeps_flat_images_and_spreads_an_impulse() {
    let flat = solid(5, 5, px(100, 100, 100, 255));
    let out = Kernel::GaussianBlur { radius: 1.5 }.run(&flat);
    assert!(out.px.iter().all(|&p| near(p, flat.px[0], 1)));
    let mut dot = solid(7, 7, px(0, 0, 0, 255));
    dot.px[3 * 7 + 3] = WHITE;
    let out = Kernel::GaussianBlur { radius: 1.0 }.run(&dot);
    assert!(ch(out.px[3 * 7 + 3], 0) < 255 && ch(out.px[3 * 7 + 3], 0) > ch(out.px[3 * 7 + 4], 0));
    assert!(ch(out.px[3 * 7 + 4], 0) > 0, "neighbour picks up light");
    assert_eq!(out.px[3 * 7 + 2], out.px[3 * 7 + 4], "symmetric");
}

#[test]
fn affine_translation_moves_pixels_and_leaves_transparent_edges() {
    let a = px(255, 0, 0, 255);
    let b = px(0, 255, 0, 255);
    let c = px(0, 0, 255, 255);
    let img = Image { w: 3, h: 1, px: vec![a, b, c] };
    let out = Kernel::AffineTransform { m: [1.0, 0.0, 0.0, 1.0, 1.0, 0.0] }.run(&img);
    assert_eq!(out.px, vec![0, a, b]);
    let singular = Kernel::AffineTransform { m: [0.0; 6] }.run(&img);
    assert!(singular.px.iter().all(|&p| p == 0));
}

#[test]
fn lanczos_changes_size_and_keeps_flat_colour() {
    let img = solid(4, 4, px(50, 100, 150, 255));
    let half = Kernel::LanczosScale { scale: 0.5, aspect_ratio: 1.0 }.run(&img);
    assert_eq!((half.w, half.h), (2, 2));
    assert!(half.px.iter().all(|&p| near(p, img.px[0], 1)));
    let wide = Kernel::LanczosScale { scale: 2.0, aspect_ratio: 1.0 }.run(&img);
    assert_eq!((wide.w, wide.h), (8, 8));
    assert!(wide.px.iter().all(|&p| near(p, img.px[0], 1)));
    let aspect = Kernel::LanczosScale { scale: 1.0, aspect_ratio: 2.0 }.run(&img);
    assert_eq!((aspect.w, aspect.h), (8, 4));
    let same = Kernel::LanczosScale { scale: 1.0, aspect_ratio: 1.0 }.run(&img);
    assert!(same.px.iter().all(|&p| near(p, img.px[0], 1)));
}

#[test]
fn source_over_follows_premultiplied_blending() {
    let bg = solid(1, 1, px(0, 0, 255, 255));
    let opaque = solid(1, 1, px(255, 0, 0, 255));
    assert_eq!(source_over(&opaque, &bg).unwrap().px, opaque.px);
    let clear = solid(1, 1, 0);
    assert_eq!(source_over(&clear, &bg).unwrap().px, bg.px);
    let half = solid(1, 1, px(128, 0, 0, 128));
    let out = source_over(&half, &bg).unwrap().px[0];
    assert!(near(out, px(128, 0, 127, 255), 1), "{out:08x}");
    assert!(source_over(&opaque, &solid(2, 1, 0)).is_none());
}

#[test]
fn convolution_identity_shift_and_bad_shapes() {
    let a = px(10, 10, 10, 255);
    let b = px(20, 20, 20, 255);
    let c = px(30, 30, 30, 255);
    let img = Image { w: 3, h: 1, px: vec![a, b, c] };
    let ident = convolve(&img, 3, 3, &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0], 0.0).unwrap();
    assert_eq!(ident.px, img.px);
    let shift = convolve(&img, 3, 1, &[0.0, 0.0, 1.0], 0.0).unwrap();
    assert_eq!(shift.px, vec![b, c, c], "correlation reads the right neighbour, edges clamp");
    assert!(convolve(&img, 2, 2, &[1.0; 4], 0.0).is_none());
    assert!(convolve(&img, 3, 3, &[1.0; 4], 0.0).is_none());
    let boxed = convolve(&solid(4, 4, px(90, 90, 90, 255)), 3, 3, &[1.0 / 9.0; 9], 0.0).unwrap();
    assert!(boxed.px.iter().all(|&p| near(p, px(90, 90, 90, 255), 1)));
}

#[test]
fn histogram_counts_each_channel() {
    let img = Image { w: 3, h: 1, px: vec![px(1, 2, 3, 255), px(1, 5, 3, 255), px(7, 2, 3, 0)] };
    let h = histogram(&img);
    assert_eq!(h[0][1], 2);
    assert_eq!(h[0][7], 1);
    assert_eq!(h[1][2], 2);
    assert_eq!(h[1][5], 1);
    assert_eq!(h[2][3], 3);
    assert_eq!(h[3][255], 2);
    assert_eq!(h[3][0], 1);
    assert_eq!(h.iter().map(|c| c.iter().sum::<u32>()).collect::<Vec<_>>(), vec![3, 3, 3, 3]);
}

#[test]
fn ci_filters_are_routed_through_the_library() {
    let f = ci_filter("CIGaussianBlur", &[("inputRadius", 2.5), ("inputBogus", 9.0)]).unwrap();
    assert_eq!(f.function_name(), "ci_gaussian_blur");
    assert_eq!(f.kernel(), &Kernel::GaussianBlur { radius: 2.5 });
    let v = ci_filter("CIVignette", &[("inputIntensity", 1.0)]).unwrap();
    assert_eq!(v.kernel(), &Kernel::Vignette { intensity: 1.0, radius: 1.0 });
    assert_eq!(ci_filter("CINoSuchFilter", &[]), Err(KernelError::Unsupported("CINoSuchFilter".into())));
    for name in ["CIGaussianBlur", "CIColorControls", "CIColorMatrix", "CISepiaTone", "CIVignette", "CIAffineTransform", "CISourceOverCompositing", "CILanczosScaleTransform"] {
        assert!(ci_filter_names().contains(&name), "{name}");
    }
}

#[test]
fn ci_kernel_classes_check_the_function_kind() {
    let lib = library();
    let color = CiKernel::new("ci_sepia_tone", &lib, KernelKind::Color).unwrap();
    assert_eq!(color.apply(&solid(1, 1, WHITE)).px.len(), 1);
    assert!(CiKernel::new("ci_affine_transform", &lib, KernelKind::Warp).is_ok());
    assert!(CiKernel::new("ci_gaussian_blur", &lib, KernelKind::General).is_ok());
    assert_eq!(
        CiKernel::new("ci_affine_transform", &lib, KernelKind::Color),
        Err(KernelError::KindMismatch { name: "ci_affine_transform".into(), wanted: KernelKind::Color, found: KernelKind::Warp })
    );
    assert!(matches!(CiKernel::new("missing", &lib, KernelKind::General), Err(KernelError::Library(LibraryError::NotFound(_)))));
}
