//! PDF page geometry as a pure model: the display boxes of a page (media, crop, bleed, trim, art), the page rotation,
//! the transform that maps page space onto a display box (rotated, origin at the box corner), aspect-fit sizing for
//! page thumbnails, and the page size of a page made from an image.
//!
//! Boxes fall back along a chain when a page does not set them: the crop box follows the media box, and the bleed,
//! trim and art boxes follow the crop box. Setting a box therefore moves every box that still falls back to it.
//! Rotation is stored as one of 0, 90, 180 or 270 degrees, clockwise, the way the page is displayed. The renderer
//! applies the returned [`AffineTransform`] to its own drawing calls; nothing here draws.

use crate::affine::AffineTransform;
use crate::Rect;

/// The display boxes a page can report or set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayBox {
    /// The physical medium. Always set on a page.
    Media,
    /// The visible region. Falls back to the media box.
    Crop,
    /// The production clip region. Falls back to the crop box.
    Bleed,
    /// The intended size of the finished page. Falls back to the crop box.
    Trim,
    /// The meaningful content and its white space. Falls back to the crop box.
    Art,
}

impl DisplayBox {
    fn index(self) -> usize {
        match self {
            DisplayBox::Media => 0,
            DisplayBox::Crop => 1,
            DisplayBox::Bleed => 2,
            DisplayBox::Trim => 3,
            DisplayBox::Art => 4,
        }
    }

    /// The box this one falls back to when a page does not set it.
    fn fallback(self) -> Option<DisplayBox> {
        match self {
            DisplayBox::Media => None,
            DisplayBox::Crop => Some(DisplayBox::Media),
            DisplayBox::Bleed | DisplayBox::Trim | DisplayBox::Art => Some(DisplayBox::Crop),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageError {
    /// A box, image or target size is zero, negative, or not finite.
    BadSize,
    /// The rotation is not a multiple of 90 degrees.
    BadRotation,
}

fn valid_rect(r: Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite() && r.w > 0.0 && r.h > 0.0
}

/// The boxes of one page. Unset boxes resolve through their fallback chain at read time.
#[derive(Clone, Debug, PartialEq)]
pub struct PageBoxes {
    slots: [Option<Rect>; 5],
}

impl PageBoxes {
    /// A page whose media box is `media`. Every other box falls back to it.
    pub fn new(media: Rect) -> Result<PageBoxes, PageError> {
        if !valid_rect(media) {
            return Err(PageError::BadSize);
        }
        let mut slots = [None; 5];
        slots[DisplayBox::Media.index()] = Some(media);
        Ok(PageBoxes { slots })
    }

    /// The effective bounds of `which`: its own rectangle, or the first one its fallback chain sets.
    pub fn bounds(&self, which: DisplayBox) -> Rect {
        let mut current = which;
        loop {
            if let Some(r) = self.slots[current.index()] {
                return r;
            }
            // The media box is always set, so the chain ends there.
            current = current.fallback().expect("the media box is always set");
        }
    }

    /// Sets `which` to `rect`. The rectangle must have a positive, finite size.
    pub fn set_bounds(&mut self, rect: Rect, which: DisplayBox) -> Result<(), PageError> {
        if !valid_rect(rect) {
            return Err(PageError::BadSize);
        }
        self.slots[which.index()] = Some(rect);
        Ok(())
    }
}

/// Normalises a rotation in degrees to 0, 90, 180 or 270. Negative and larger multiples wrap around, so `-90`
/// becomes `270` and `450` becomes `90`. Other angles are refused.
pub fn normalize_rotation(degrees: i32) -> Result<u16, PageError> {
    let d = degrees.rem_euclid(360);
    if d % 90 != 0 {
        return Err(PageError::BadRotation);
    }
    Ok(d as u16)
}

/// One page: its boxes and its rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPage {
    boxes: PageBoxes,
    rotation: u16,
}

impl PdfPage {
    /// A page with the given media box and no rotation.
    pub fn new(media: Rect) -> Result<PdfPage, PageError> {
        Ok(PdfPage { boxes: PageBoxes::new(media)?, rotation: 0 })
    }

    /// A page made from an image of `width` by `height` points: its media box is that size, at the origin.
    pub fn from_image_size(width: f32, height: f32) -> Result<PdfPage, PageError> {
        PdfPage::new(Rect { x: 0.0, y: 0.0, w: width, h: height })
    }

    pub fn boxes(&self) -> &PageBoxes {
        &self.boxes
    }

    pub fn boxes_mut(&mut self) -> &mut PageBoxes {
        &mut self.boxes
    }

    /// The rotation in degrees: 0, 90, 180 or 270.
    pub fn rotation(&self) -> u16 {
        self.rotation
    }

    /// Sets the rotation. Any multiple of 90 is accepted and stored normalised.
    pub fn set_rotation(&mut self, degrees: i32) -> Result<(), PageError> {
        self.rotation = normalize_rotation(degrees)?;
        Ok(())
    }

    /// The size of `which` as displayed: width and height swap for a quarter turn.
    pub fn displayed_size(&self, which: DisplayBox) -> (f32, f32) {
        let r = self.boxes.bounds(which);
        if self.rotation == 90 || self.rotation == 270 {
            (r.h, r.w)
        } else {
            (r.w, r.h)
        }
    }

    /// The transform from page space to a display of `which` whose origin is at the bottom left of the displayed
    /// box. The result maps the box onto `(0, 0)` to `displayed_size(which)`. Rotation is clockwise.
    pub fn transform_for(&self, which: DisplayBox) -> AffineTransform {
        let r = self.boxes.bounds(which);
        let (w, h) = (r.w, r.h);
        let to_origin = AffineTransform::translate(-r.x, -r.y);
        let turn = match self.rotation {
            90 => AffineTransform { a: 0.0, b: -1.0, c: 1.0, d: 0.0, tx: 0.0, ty: w },
            180 => AffineTransform { a: -1.0, b: 0.0, c: 0.0, d: -1.0, tx: w, ty: h },
            270 => AffineTransform { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: h, ty: 0.0 },
            _ => AffineTransform::IDENTITY,
        };
        to_origin.then(turn)
    }

    /// The scale that fits the displayed box inside `max` (width, height), keeping its aspect ratio. Pages are scaled
    /// up or down to fit, as thumbnails are.
    pub fn fit_scale(&self, which: DisplayBox, max: (f32, f32)) -> Result<f32, PageError> {
        if !(max.0.is_finite() && max.1.is_finite() && max.0 > 0.0 && max.1 > 0.0) {
            return Err(PageError::BadSize);
        }
        let (w, h) = self.displayed_size(which);
        Ok((max.0 / w).min(max.1 / h))
    }

    /// The size of a thumbnail of `which` that fits inside `max`, keeping its aspect ratio.
    pub fn thumbnail_size(&self, which: DisplayBox, max: (f32, f32)) -> Result<(f32, f32), PageError> {
        let s = self.fit_scale(which, max)?;
        let (w, h) = self.displayed_size(which);
        Ok((w * s, h * s))
    }

    /// The transform that draws `which` centred inside `target` (width, height), scaled to fit. It maps page space
    /// onto a rectangle at the origin of the target, so the caller adds its own placement.
    pub fn fit_transform(&self, which: DisplayBox, target: (f32, f32)) -> Result<AffineTransform, PageError> {
        let s = self.fit_scale(which, target)?;
        let (w, h) = self.displayed_size(which);
        let ox = (target.0 - w * s) / 2.0;
        let oy = (target.1 - h * s) / 2.0;
        Ok(self.transform_for(which).then(AffineTransform::scale(s, s)).then(AffineTransform::translate(ox, oy)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    fn letter() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 612.0, h: 792.0 }
    }

    #[test]
    fn unset_boxes_fall_back_to_the_media_box() {
        let boxes = PageBoxes::new(letter()).unwrap();
        assert_eq!(boxes.bounds(DisplayBox::Crop), letter());
        assert_eq!(boxes.bounds(DisplayBox::Bleed), letter());
        assert_eq!(boxes.bounds(DisplayBox::Art), letter());
    }

    #[test]
    fn setting_the_crop_box_moves_the_boxes_that_fall_back_to_it() {
        let mut boxes = PageBoxes::new(letter()).unwrap();
        let crop = Rect { x: 10.0, y: 20.0, w: 400.0, h: 500.0 };
        boxes.set_bounds(crop, DisplayBox::Crop).unwrap();
        assert_eq!(boxes.bounds(DisplayBox::Trim), crop);
        assert_eq!(boxes.bounds(DisplayBox::Media), letter());
        // A box set on its own is not moved by the crop box.
        let trim = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        boxes.set_bounds(trim, DisplayBox::Trim).unwrap();
        boxes.set_bounds(Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 }, DisplayBox::Crop).unwrap();
        assert_eq!(boxes.bounds(DisplayBox::Trim), trim);
    }

    #[test]
    fn boxes_reject_empty_and_non_finite_rectangles() {
        let mut boxes = PageBoxes::new(letter()).unwrap();
        assert_eq!(boxes.set_bounds(Rect { x: 0.0, y: 0.0, w: 0.0, h: 10.0 }, DisplayBox::Crop), Err(PageError::BadSize));
        assert_eq!(boxes.set_bounds(Rect { x: 0.0, y: f32::NAN, w: 10.0, h: 10.0 }, DisplayBox::Art), Err(PageError::BadSize));
        assert_eq!(PageBoxes::new(Rect { x: 0.0, y: 0.0, w: -3.0, h: 10.0 }), Err(PageError::BadSize));
        assert_eq!(boxes.bounds(DisplayBox::Crop), letter());
    }

    #[test]
    fn rotation_wraps_to_a_quarter_turn_and_rejects_other_angles() {
        assert_eq!(normalize_rotation(450), Ok(90));
        assert_eq!(normalize_rotation(-90), Ok(270));
        assert_eq!(normalize_rotation(360), Ok(0));
        assert_eq!(normalize_rotation(45), Err(PageError::BadRotation));
        let mut page = PdfPage::new(letter()).unwrap();
        page.set_rotation(-180).unwrap();
        assert_eq!(page.rotation(), 180);
    }

    #[test]
    fn a_quarter_turn_swaps_the_displayed_width_and_height() {
        let mut page = PdfPage::from_image_size(200.0, 100.0).unwrap();
        assert_eq!(page.displayed_size(DisplayBox::Media), (200.0, 100.0));
        page.set_rotation(90).unwrap();
        assert_eq!(page.displayed_size(DisplayBox::Media), (100.0, 200.0));
    }

    #[test]
    fn transform_maps_the_box_onto_its_displayed_size_at_every_rotation() {
        let mut page = PdfPage::new(Rect { x: 5.0, y: 7.0, w: 4.0, h: 10.0 }).unwrap();
        for deg in [0, 90, 180, 270] {
            page.set_rotation(deg).unwrap();
            let (dw, dh) = page.displayed_size(DisplayBox::Media);
            let local = Rect { x: 5.0, y: 7.0, w: 4.0, h: 10.0 };
            let mapped = page.transform_for(DisplayBox::Media).apply_rect(local);
            assert!(near((mapped.x, mapped.y), (0.0, 0.0)), "rotation {deg}: origin {mapped:?}");
            assert!(near((mapped.w, mapped.h), (dw, dh)), "rotation {deg}: size {mapped:?}");
        }
    }

    #[test]
    fn a_quarter_turn_is_clockwise() {
        // A 4 by 10 box turned 90 degrees clockwise: its top left corner ends at the top right of a 10 by 4 display.
        let mut page = PdfPage::new(Rect { x: 0.0, y: 0.0, w: 4.0, h: 10.0 }).unwrap();
        page.set_rotation(90).unwrap();
        let t = page.transform_for(DisplayBox::Media);
        assert!(near(t.apply(0.0, 10.0), (10.0, 4.0)));
        assert!(near(t.apply(0.0, 0.0), (0.0, 4.0)));
        assert!(near(t.apply(4.0, 0.0), (0.0, 0.0)));
    }

    #[test]
    fn crop_box_origin_is_moved_to_zero() {
        let mut page = PdfPage::new(letter()).unwrap();
        page.boxes_mut().set_bounds(Rect { x: 100.0, y: 200.0, w: 300.0, h: 400.0 }, DisplayBox::Crop).unwrap();
        let t = page.transform_for(DisplayBox::Crop);
        assert!(near(t.apply(100.0, 200.0), (0.0, 0.0)));
        assert!(near(t.apply(400.0, 600.0), (300.0, 400.0)));
    }

    #[test]
    fn thumbnail_keeps_the_aspect_ratio_and_respects_rotation() {
        let mut page = PdfPage::from_image_size(200.0, 100.0).unwrap();
        let (w, h) = page.thumbnail_size(DisplayBox::Media, (100.0, 100.0)).unwrap();
        assert!(near((w, h), (100.0, 50.0)));
        page.set_rotation(90).unwrap();
        let (w, h) = page.thumbnail_size(DisplayBox::Media, (100.0, 100.0)).unwrap();
        assert!(near((w, h), (50.0, 100.0)));
        assert_eq!(page.thumbnail_size(DisplayBox::Media, (0.0, 100.0)), Err(PageError::BadSize));
    }

    #[test]
    fn fit_transform_centres_the_page_in_the_target() {
        let page = PdfPage::new(letter()).unwrap();
        let t: AffineTransform = page.fit_transform(DisplayBox::Media, (300.0, 300.0)).unwrap();
        let s = 300.0 / 792.0;
        let placed = t.apply_rect(Rect { x: 0.0, y: 0.0, w: 612.0, h: 792.0 });
        assert!((placed.h - 300.0).abs() < 1e-2, "height {}", placed.h);
        assert!((placed.w - 612.0 * s).abs() < 1e-2, "width {}", placed.w);
        assert!((placed.x - (300.0 - 612.0 * s) / 2.0).abs() < 1e-2, "x {}", placed.x);
        assert!(placed.y.abs() < 1e-2);
    }

    #[test]
    fn a_page_made_from_an_image_has_the_image_size() {
        let page = PdfPage::from_image_size(640.0, 480.0).unwrap();
        assert_eq!(page.boxes().bounds(DisplayBox::Media), Rect { x: 0.0, y: 0.0, w: 640.0, h: 480.0 });
        assert_eq!(PdfPage::from_image_size(0.0, 480.0), Err(PageError::BadSize));
    }
}
