//! An image object: a named image with one or more representations, a point size, a template
//! flag, and resizable cap insets.
//!
//! Each representation is either a bitmap with pixel dimensions or a vector representation that
//! scales to any size (`NSPDFImageRep` and similar). [`ImageObject::best_rep`] picks the representation to draw for a
//! target size and display scale consistently: a vector representation wins, otherwise the smallest bitmap
//! that is still at least as large as the target in pixels, falling back to the largest bitmap.
//!
//! [`ImageObject::resizable_slices`] gives the nine-slice layout of a resizable image (`resizingMode = .stretch` with
//! `capInsets`): the corners keep their size and the edges and centre stretch.
//!
//! ```ignore
//! let mut icon = ImageObject::from_encoded(&png_bytes).unwrap();
//! icon.set_template(true);
//! let rep = icon.best_rep((16.0, 16.0), 2.0); // the 32 px bitmap on a Retina-class display
//! ```
//!
//! Decoding goes through [`crate::graphics_imageio::pixel_size`], which reads only the header. Pixels are not loaded
//! here; apps decode with `graphics_imageio::decode` when they draw.

use crate::graphics_imageio;
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_render::gfx::Image;

/// A rectangle as (x, y, width, height), y growing downwards.
pub type Rect = (f32, f32, f32, f32);

/// Cap insets of a resizable image, in points (top, left, bottom, right).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapInsets {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

/// One piece of a nine-slice layout: where it comes from in the image and where it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub src: Rect,
    pub dst: Rect,
}

/// What a representation holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RepKind {
    /// A pixel grid of `pixel_width` by `pixel_height`.
    Bitmap { pixel_width: u32, pixel_height: u32 },
    /// Resolution-independent content (a vector page, for example); it is drawn at any size.
    Vector,
}

/// One representation of an image: its kind and its size in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageRep {
    pub kind: RepKind,
    /// Size in points.
    pub size: (f32, f32),
}

impl ImageRep {
    /// A bitmap of `pixel_width` by `pixel_height` pixels shown at `size` points. `None` for an empty pixel grid, a
    /// non-positive or non-finite size.
    pub fn bitmap(pixel_width: u32, pixel_height: u32, size: (f32, f32)) -> Option<ImageRep> {
        if pixel_width == 0 || pixel_height == 0 || !valid_size(size) {
            return None;
        }
        Some(ImageRep { kind: RepKind::Bitmap { pixel_width, pixel_height }, size })
    }

/// A bitmap shown at 72 points per inch, so one pixel is one point by default.
    pub fn bitmap_at_72_dpi(pixel_width: u32, pixel_height: u32) -> Option<ImageRep> {
        ImageRep::bitmap(pixel_width, pixel_height, (pixel_width as f32, pixel_height as f32))
    }

    /// A vector representation of `size` points. `None` for a non-positive or non-finite size.
    pub fn vector(size: (f32, f32)) -> Option<ImageRep> {
        if !valid_size(size) {
            return None;
        }
        Some(ImageRep { kind: RepKind::Vector, size })
    }

    /// Whether the representation scales to any size.
    pub fn is_vector(&self) -> bool {
        matches!(self.kind, RepKind::Vector)
    }

    /// Pixels per point along x, or `None` for a vector representation.
    pub fn scale(&self) -> Option<f32> {
        match self.kind {
            RepKind::Bitmap { pixel_width, .. } => Some(pixel_width as f32 / self.size.0),
            RepKind::Vector => None,
        }
    }

    /// Pixel area for a bitmap, 0 for a vector representation (used to rank bitmaps).
    fn pixel_area(&self) -> u64 {
        match self.kind {
            RepKind::Bitmap { pixel_width, pixel_height } => pixel_width as u64 * pixel_height as u64,
            RepKind::Vector => 0,
        }
    }

    /// Whether a bitmap has at least `w` by `h` pixels. Vector representations always do.
    fn covers_pixels(&self, w: f32, h: f32) -> bool {
        match self.kind {
            RepKind::Bitmap { pixel_width, pixel_height } => pixel_width as f32 >= w && pixel_height as f32 >= h,
            RepKind::Vector => true,
        }
    }
}

/// An image object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageObject {
    name: Option<String>,
    size: Option<(f32, f32)>,
    reps: Vec<ImageRep>,
    template: bool,
    cap_insets: CapInsets,
}

impl ImageObject {
    /// An image with no representations yet.
    pub fn new() -> Self {
        ImageObject::default()
    }

/// An empty image with a name; named images are looked up by that name.
    pub fn named(name: impl Into<String>) -> Self {
        ImageObject { name: Some(name.into()), ..ImageObject::default() }
    }

    /// An image with one bitmap representation read from encoded bytes (PNG, JPEG, and the other formats the codec
    /// reads). The representation is 72 dpi. `None` when the bytes are not a supported image.
    pub fn from_encoded(bytes: &[u8]) -> Option<ImageObject> {
        let (w, h) = graphics_imageio::pixel_size(bytes)?;
        let rep = ImageRep::bitmap_at_72_dpi(u32::try_from(w).ok()?, u32::try_from(h).ok()?)?;
        let mut image = ImageObject::new();
        image.add_rep(rep);
        Some(image)
    }

    /// The name the image was registered under, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The image's size in points: the size set explicitly, else the size of the first representation.
    pub fn size(&self) -> Option<(f32, f32)> {
        self.size.or_else(|| self.reps.first().map(|r| r.size))
    }

    /// Sets the size in points. Returns `false` and keeps the old size for a non-positive or non-finite size.
    pub fn set_size(&mut self, size: (f32, f32)) -> bool {
        if !valid_size(size) {
            return false;
        }
        self.size = Some(size);
        true
    }

    /// Adds a representation. The representations keep the order they were added in.
    pub fn add_rep(&mut self, rep: ImageRep) {
        self.reps.push(rep);
    }

    /// The representations, in the order they were added.
    pub fn reps(&self) -> &[ImageRep] {
        &self.reps
    }

/// The representation to draw at `target` points on a display with `scale` pixels per point (the resolver picks
    /// the best representation for the drawing context). A vector representation is preferred. Otherwise the
    /// smallest bitmap with at least `target * scale` pixels in each direction is chosen, or the largest bitmap when
    /// none is big enough. Ties keep the earlier representation. `None` when there are no representations.
    pub fn best_rep(&self, target: (f32, f32), scale: f32) -> Option<&ImageRep> {
        if let Some(vector) = self.reps.iter().find(|r| r.is_vector()) {
            return Some(vector);
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let need = (target.0.max(0.0) * scale, target.1.max(0.0) * scale);
        let covering = self.reps.iter().filter(|r| r.covers_pixels(need.0, need.1));
        let best_covering = covering.fold(None::<&ImageRep>, |best, r| match best {
            Some(b) if b.pixel_area() <= r.pixel_area() => Some(b),
            _ => Some(r),
        });
        best_covering.or_else(|| {
            self.reps.iter().fold(None::<&ImageRep>, |best, r| match best {
                Some(b) if b.pixel_area() >= r.pixel_area() => Some(b),
                _ => Some(r),
            })
        })
    }

    /// Whether the image is a template: drawn as a single-colour mask that takes the tint of its context
    /// (`isTemplate`).
    pub fn is_template(&self) -> bool {
        self.template
    }

    /// Marks the image as a template or not.
    pub fn set_template(&mut self, template: bool) {
        self.template = template;
    }

    /// The cap insets of a resizable image.
    pub fn cap_insets(&self) -> CapInsets {
        self.cap_insets
    }

    /// Sets the cap insets. Returns `false` and keeps the old insets when a value is negative or not finite.
    pub fn set_cap_insets(&mut self, insets: CapInsets) -> bool {
        let values = [insets.top, insets.left, insets.bottom, insets.right];
        if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return false;
        }
        self.cap_insets = insets;
        true
    }

    /// The nine-slice layout that draws the image at `target` points with its cap insets kept fixed. Slices run
    /// row by row from the top left: corners keep their size, edges and the centre stretch. `None` when the image has
    /// no size, the insets do not fit inside the image, or `target` is smaller than the insets, in which case the
    /// caller draws the whole image scaled.
    pub fn resizable_slices(&self, target: (f32, f32)) -> Option<[Slice; 9]> {
        let (sw, sh) = self.size()?;
        let c = self.cap_insets;
        if c.left + c.right > sw || c.top + c.bottom > sh {
            return None;
        }
        let (tw, th) = target;
        if !(tw.is_finite() && th.is_finite()) || tw < c.left + c.right || th < c.top + c.bottom {
            return None;
        }
        let src_x = [0.0, c.left, sw - c.right];
        let src_w = [c.left, sw - c.left - c.right, c.right];
        let dst_x = [0.0, c.left, tw - c.right];
        let dst_w = [c.left, tw - c.left - c.right, c.right];
        let src_y = [0.0, c.top, sh - c.bottom];
        let src_h = [c.top, sh - c.top - c.bottom, c.bottom];
        let dst_y = [0.0, c.top, th - c.bottom];
        let dst_h = [c.top, th - c.top - c.bottom, c.bottom];
        let mut out = [Slice { src: (0.0, 0.0, 0.0, 0.0), dst: (0.0, 0.0, 0.0, 0.0) }; 9];
        for row in 0..3 {
            for col in 0..3 {
                out[row * 3 + col] = Slice {
                    src: (src_x[col], src_y[row], src_w[col], src_h[row]),
                    dst: (dst_x[col], dst_y[row], dst_w[col], dst_h[row]),
                };
            }
        }
        Some(out)
    }
}

impl ImageObject {
    /// Draws `pixels` (the decoded pixels of this image's chosen representation, see [`best_rep`](Self::best_rep))
    /// into `dst` (device pixels) on a canvas, the way the image object draws itself in a view:
    ///
    /// - a template image is a coverage mask filled with `tint` (pass the context's colour, `LABEL.now()` for
    ///   ordinary content or the accent for a selected control), so it follows Light, Dark and the accent;
    /// - any other image draws as itself at `alpha` (0..1); `tint` is ignored;
    /// - with cap insets and a `dst` at least as large as them, nine slices keep the corners at their size and
    ///   stretch the edges and centre ([`resizable_slices`](Self::resizable_slices)); `scale` is device pixels per
    ///   point, and the insets are read in the image's own pixels per point.
    ///
    /// Without a point size the image is drawn whole. Nothing is drawn for an empty pixel grid or an empty `dst`.
    pub fn draw(&self, c: &mut dyn Canvas, pixels: &Image, dst: RectF, scale: f32, tint: u32, alpha: f32) {
        if pixels.w == 0 || pixels.h == 0 || !(dst.w > 0.0 && dst.h > 0.0) {
            return;
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let has_caps = self.cap_insets != CapInsets::default();
        let slices = if has_caps { self.resizable_slices((dst.w / scale, dst.h / scale)) } else { None };
        let Some(slices) = slices else {
            self.draw_piece(c, pixels, dst, tint, alpha);
            return;
        };
        // image pixels per point, from the pixel grid over the point size
        let (sw, sh) = self.size().unwrap_or((pixels.w as f32, pixels.h as f32));
        let (kx, ky) = (pixels.w as f32 / sw, pixels.h as f32 / sh);
        for s in slices {
            let (sx, sy, sw, sh) = s.src;
            let (x0, y0) = ((sx * kx).round() as usize, (sy * ky).round() as usize);
            let (x1, y1) = (((sx + sw) * kx).round() as usize, ((sy + sh) * ky).round() as usize);
            let (x1, y1) = (x1.min(pixels.w), y1.min(pixels.h));
            let (dx, dy, dw, dh) = s.dst;
            if x1 <= x0 || y1 <= y0 || dw <= 0.0 || dh <= 0.0 {
                continue;
            }
            let piece = pixels.crop(x0, y0, x1 - x0, y1 - y0);
            let to = RectF::new(dst.x + dx * scale, dst.y + dy * scale, dw * scale, dh * scale);
            self.draw_piece(c, &piece, to, tint, alpha);
        }
    }

    fn draw_piece(&self, c: &mut dyn Canvas, img: &Image, dst: RectF, tint: u32, alpha: f32) {
        if self.template {
            // a tinted draw takes its coverage from the mask, so the opacity scales the mask
            let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
            let faded;
            let mask = if a >= 255 {
                img
            } else {
                faded = Image { w: img.w, h: img.h, px: img.px.iter().map(|&p| bifrost_render::gfx::scale(p, a)).collect() };
                &faded
            };
            c.image_tinted(ImageRef { id: crate::widgets::image_id(mask), image: mask }, dst, tint);
        } else {
            let r = ImageRef { id: crate::widgets::image_id(img), image: img };
            c.image(r, dst, alpha.clamp(0.0, 1.0));
        }
    }
}

/// The named images an app registers. Lookup by name is served here. A name is registered once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageCatalog {
    images: Vec<ImageObject>,
}

impl ImageCatalog {
    /// An empty catalogue.
    pub fn new() -> Self {
        ImageCatalog::default()
    }

    /// Registers `image` under its name. Returns `false` and changes nothing when the image has no name or the name
    /// is already registered.
    pub fn register(&mut self, image: ImageObject) -> bool {
        let name = match image.name() {
            Some(name) => name.to_string(),
            None => return false,
        };
        if self.named(&name).is_some() {
            return false;
        }
        self.images.push(image);
        true
    }

    /// The image registered under `name`.
    pub fn named(&self, name: &str) -> Option<&ImageObject> {
        self.images.iter().find(|image| image.name() == Some(name))
    }

    /// Removes the image registered under `name`. Returns whether one was registered.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.images.len();
        self.images.retain(|image| image.name() != Some(name));
        self.images.len() != before
    }

    /// The registered names, in registration order.
    pub fn names(&self) -> Vec<&str> {
        self.images.iter().filter_map(ImageObject::name).collect()
    }
}

/// The pixels of a template image drawn in `tint` (`isTemplate` images are masks that take the tint of their
/// context). Each output pixel keeps the tint's colour bits and takes the source alpha scaled by the tint's alpha.
/// Pixels are `0xAARRGGBB`, with alpha in the top byte.
pub fn tint_template(pixels: &[u32], tint: u32) -> Vec<u32> {
    let tint_alpha = tint >> 24;
    let colour = tint & 0x00ff_ffff;
    pixels
        .iter()
        .map(|&pixel| {
            let alpha = (pixel >> 24) * tint_alpha / 255;
            (alpha << 24) | colour
        })
        .collect()
}

fn valid_size(size: (f32, f32)) -> bool {
    size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(w: u32, h: u32) -> ImageRep {
        ImageRep::bitmap_at_72_dpi(w, h).unwrap()
    }

    #[test]
    fn bitmap_rep_refuses_empty_pixels_and_bad_sizes() {
        assert!(ImageRep::bitmap(0, 4, (1.0, 1.0)).is_none());
        assert!(ImageRep::bitmap(4, 4, (0.0, 1.0)).is_none());
        assert!(ImageRep::bitmap(4, 4, (f32::NAN, 1.0)).is_none());
        assert!(ImageRep::vector((-1.0, 2.0)).is_none());
    }

    #[test]
    fn scale_is_pixels_per_point_for_bitmaps_only() {
        let rep = ImageRep::bitmap(32, 32, (16.0, 16.0)).unwrap();
        assert_eq!(rep.scale(), Some(2.0));
        assert_eq!(ImageRep::vector((16.0, 16.0)).unwrap().scale(), None);
    }

    #[test]
    fn size_comes_from_the_first_rep_until_set() {
        let mut image = ImageObject::new();
        assert!(image.size().is_none());
        image.add_rep(bitmap(10, 20));
        image.add_rep(bitmap(40, 40));
        assert_eq!(image.size(), Some((10.0, 20.0)));
        assert!(image.set_size((5.0, 5.0)));
        assert_eq!(image.size(), Some((5.0, 5.0)));
        assert!(!image.set_size((0.0, 5.0)));
        assert_eq!(image.size(), Some((5.0, 5.0)));
    }

    #[test]
    fn named_image_keeps_its_name() {
        assert_eq!(ImageObject::named("Logo").name(), Some("Logo"));
        assert_eq!(ImageObject::new().name(), None);
    }

    #[test]
    fn best_rep_picks_smallest_covering_bitmap() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(16, 16));
        image.add_rep(bitmap(32, 32));
        image.add_rep(bitmap(64, 64));
        // 16 points at 2x needs 32 pixels: the 32 px bitmap is the smallest that covers it.
        assert_eq!(image.best_rep((16.0, 16.0), 2.0).unwrap().size, (32.0, 32.0));
        // 16 points at 1x: the 16 px bitmap.
        assert_eq!(image.best_rep((16.0, 16.0), 1.0).unwrap().size, (16.0, 16.0));
    }

    #[test]
    fn best_rep_falls_back_to_the_largest_bitmap() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(16, 16));
        image.add_rep(bitmap(32, 48));
        // Nothing covers 100 points: the largest by area (32 by 48) is chosen.
        assert_eq!(image.best_rep((100.0, 100.0), 1.0).unwrap().size, (32.0, 48.0));
    }

    #[test]
    fn best_rep_prefers_a_vector_rep() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(256, 256));
        image.add_rep(ImageRep::vector((256.0, 256.0)).unwrap());
        assert!(image.best_rep((16.0, 16.0), 2.0).unwrap().is_vector());
    }

    #[test]
    fn best_rep_is_none_without_reps() {
        assert!(ImageObject::new().best_rep((16.0, 16.0), 1.0).is_none());
    }

    #[test]
    fn non_positive_scale_is_treated_as_one() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(16, 16));
        image.add_rep(bitmap(32, 32));
        assert_eq!(image.best_rep((16.0, 16.0), 0.0).unwrap().size, (16.0, 16.0));
        assert_eq!(image.best_rep((16.0, 16.0), f32::NAN).unwrap().size, (16.0, 16.0));
    }

    #[test]
    fn template_flag_round_trips() {
        let mut image = ImageObject::new();
        assert!(!image.is_template());
        image.set_template(true);
        assert!(image.is_template());
    }

    #[test]
    fn cap_insets_refuse_negative_values() {
        let mut image = ImageObject::new();
        let good = CapInsets { top: 2.0, left: 3.0, bottom: 2.0, right: 3.0 };
        assert!(image.set_cap_insets(good));
        assert_eq!(image.cap_insets(), good);
        assert!(!image.set_cap_insets(CapInsets { top: -1.0, ..good }));
        assert_eq!(image.cap_insets(), good);
    }

    #[test]
    fn nine_slice_corners_keep_size_and_edges_stretch() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(20, 10));
        image.set_cap_insets(CapInsets { top: 2.0, left: 3.0, bottom: 2.0, right: 4.0 });
        let slices = image.resizable_slices((50.0, 30.0)).unwrap();
        // Top left corner: 3 by 2 in both source and destination.
        assert_eq!(slices[0].src, (0.0, 0.0, 3.0, 2.0));
        assert_eq!(slices[0].dst, (0.0, 0.0, 3.0, 2.0));
        // Bottom right corner: 4 by 2, anchored to the target's bottom right.
        assert_eq!(slices[8].src, (16.0, 8.0, 4.0, 2.0));
        assert_eq!(slices[8].dst, (46.0, 28.0, 4.0, 2.0));
        // Centre stretches to fill the space between the caps.
        assert_eq!(slices[4].src, (3.0, 2.0, 13.0, 6.0));
        assert_eq!(slices[4].dst, (3.0, 2.0, 43.0, 26.0));
    }

    #[test]
    fn nine_slice_destination_covers_the_target_exactly() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(40, 24));
        image.set_cap_insets(CapInsets { top: 5.0, left: 6.0, bottom: 7.0, right: 8.0 });
        let target = (120.0, 90.0);
        let slices = image.resizable_slices(target).unwrap();
        let area: f32 = slices.iter().map(|s| s.dst.2 * s.dst.3).sum();
        assert!((area - target.0 * target.1).abs() < 1e-3);
        let src_area: f32 = slices.iter().map(|s| s.src.2 * s.src.3).sum();
        assert!((src_area - 40.0 * 24.0).abs() < 1e-3);
    }

    #[test]
    fn nine_slice_is_none_when_the_target_is_smaller_than_the_caps() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(20, 10));
        image.set_cap_insets(CapInsets { top: 2.0, left: 3.0, bottom: 2.0, right: 4.0 });
        assert!(image.resizable_slices((6.0, 30.0)).is_none());
        assert!(image.resizable_slices((30.0, 3.0)).is_none());
    }

    #[test]
    fn nine_slice_is_none_when_the_caps_do_not_fit_the_image() {
        let mut image = ImageObject::new();
        image.add_rep(bitmap(5, 5));
        image.set_cap_insets(CapInsets { top: 0.0, left: 3.0, bottom: 0.0, right: 3.0 });
        assert!(image.resizable_slices((20.0, 20.0)).is_none());
        assert!(ImageObject::new().resizable_slices((20.0, 20.0)).is_none());
    }

    #[test]
    fn from_encoded_reads_the_size_from_a_png_header() {
        let rgba = vec![255u8; 3 * 2 * 4];
        let png = graphics_imageio::encode(bifrost_render::cg::imageio::Format::Png, 3, 2, &rgba).unwrap();
        let image = ImageObject::from_encoded(&png).unwrap();
        assert_eq!(image.reps().len(), 1);
        assert_eq!(image.reps()[0].kind, RepKind::Bitmap { pixel_width: 3, pixel_height: 2 });
        assert_eq!(image.size(), Some((3.0, 2.0)));
    }

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    fn draw_into(obj: &ImageObject, img: &Image, w: usize, h: usize, tint: u32, alpha: f32) -> Vec<u32> {
        let mut px = vec![0u32; w * h];
        {
            let mut s = bifrost_render::gfx::Surface { w, h, stride: w, px: &mut px, y0: 0 };
            obj.draw(&mut s, img, RectF::new(0.0, 0.0, w as f32, h as f32), 1.0, tint, alpha);
        }
        px
    }

    const RED: u32 = 0xff00_00ff;
    const BLUE: u32 = 0xffff_0000;
    const WHITE: u32 = 0xffff_ffff;

    #[test]
    fn a_template_draws_as_a_mask_in_the_tint_and_other_images_as_themselves() {
        let mut obj = ImageObject::new();
        obj.add_rep(bitmap(2, 2));
        obj.set_template(true);
        let out = draw_into(&obj, &solid(2, 2, WHITE), 4, 4, RED, 1.0);
        assert_eq!(out[5], RED, "the white mask takes the tint");
        obj.set_template(false);
        let out = draw_into(&obj, &solid(2, 2, BLUE), 4, 4, RED, 1.0);
        assert_eq!(out[5], BLUE, "a plain image ignores the tint");
        assert!(draw_into(&obj, &Image::new(0, 0), 4, 4, RED, 1.0).iter().all(|&p| p == 0));
    }

    #[test]
    fn a_faded_template_covers_less() {
        let mut obj = ImageObject::new();
        obj.add_rep(bitmap(2, 2));
        obj.set_template(true);
        let out = draw_into(&obj, &solid(2, 2, WHITE), 2, 2, RED, 0.5);
        let a = out[0] >> 24;
        assert!(a > 100 && a < 160, "about half coverage, got {a}");
    }

    #[test]
    fn cap_insets_keep_the_corners_and_stretch_the_centre() {
        // a 3 x 3 image: red corners, blue everywhere else
        let mut img = solid(3, 3, BLUE);
        for i in [0, 2, 6, 8] {
            img.px[i] = RED;
        }
        let mut obj = ImageObject::new();
        obj.add_rep(bitmap(3, 3));
        assert!(obj.set_cap_insets(CapInsets { top: 1.0, left: 1.0, bottom: 1.0, right: 1.0 }));
        let out = draw_into(&obj, &img, 9, 9, WHITE, 1.0);
        assert_eq!((out[0], out[8], out[80]), (RED, RED, RED), "corners stay one pixel");
        assert_eq!(out[1], BLUE, "the top edge stretches, the corner does not");
        assert_eq!(out[4 * 9 + 4], BLUE);
    }

    #[test]
    fn from_encoded_refuses_bytes_that_are_not_an_image() {
        assert!(ImageObject::from_encoded(b"not an image at all").is_none());
    }
}
