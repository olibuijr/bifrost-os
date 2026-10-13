//! Where the Color Meter's pixels come from. The app reads the display through `ScreenSource` and never talks to the
//! compositor. `ShellSource` asks the shell's `screen` service for the display size, the pointer and a square of
//! pixels around the pointer. It keeps at most one request of each kind in flight: the app sends what [`ScreenSource::wants`]
//! returns, reports each request id with [`ScreenSource::track`], and passes every reply to [`ScreenSource::answer`].
//! `read` serves pixels from the last square that came back, so the readout never waits for the shell.

use crate::color::Rgb8;
use crate::loupe;
use bifrost_app::screen_sample;
use bifrost_core::json::Json;

/// A rectangle of display pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// A request the source needs the shell to answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Display,
    Pointer,
    /// The codes of this rectangle.
    Loupe(PixelRect),
}

pub trait ScreenSource {
    /// The display size in pixels, or `None` when no display can be read.
    fn display_size(&self) -> Option<(u32, u32)>;

    /// The pointer position in display pixels, or `None` when the source does not know it.
    fn pointer(&self) -> Option<(u32, u32)>;

    /// The codes of every pixel of `rect`, row by row, or `None` when the source cannot read it.
    fn read(&self, rect: PixelRect) -> Option<Vec<Rgb8>>;

    /// The requests to send now, for a loupe of `cells` pixels across centred on `center`. Kinds already in flight are
    /// not asked again. Sources that read nothing ask for nothing.
    fn wants(&mut self, _center: (u32, u32), _cells: u32) -> Vec<Ask> {
        Vec::new()
    }

    /// Records that the request `request` was sent for `ask`.
    fn track(&mut self, _request: u32, _ask: Ask) {}

    /// Takes the reply to `request` when it was one of ours. True when it was, and a frame is then needed.
    fn answer(&mut self, _request: u32, _result: &Result<Json, String>) -> bool {
        false
    }
}

/// Pixels this many cells wider than the loupe are requested, so a pointer that moves a pixel or two still reads from the
/// square that came back.
const MARGIN: u32 = 4;
/// The largest loupe side asked of the shell, in cells (the shell's own limit is 64 display pixels).
const MAX_CELLS: u32 = 60;

/// The production source: the shell's `screen` service. Until its first replies arrive it reads nothing.
#[derive(Default)]
pub struct ShellSource {
    display: Option<(u32, u32)>,
    pointer: Option<(u32, u32)>,
    /// The last square of pixels that came back, with its rectangle.
    loupe: Option<(PixelRect, Vec<Rgb8>)>,
    /// Requests sent and not yet answered, with what each one asked for.
    in_flight: Vec<(u32, Ask)>,
}

/// Whether two asks are the same kind, whatever their rectangle.
fn same_kind(a: Ask, b: Ask) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

impl ShellSource {
    fn busy(&self, ask: Ask) -> bool {
        self.in_flight.iter().any(|(_, a)| same_kind(*a, ask))
    }
}

impl ScreenSource for ShellSource {
    fn display_size(&self) -> Option<(u32, u32)> {
        self.display
    }

    fn pointer(&self) -> Option<(u32, u32)> {
        self.pointer
    }

    fn read(&self, rect: PixelRect) -> Option<Vec<Rgb8>> {
        let (outer, px) = self.loupe.as_ref()?;
        let x0 = rect.x.checked_sub(outer.x)?;
        let y0 = rect.y.checked_sub(outer.y)?;
        if x0.checked_add(rect.w)? > outer.w || y0.checked_add(rect.h)? > outer.h {
            return None;
        }
        let mut out = Vec::with_capacity((rect.w * rect.h) as usize);
        for row in y0..y0 + rect.h {
            let start = (row * outer.w + x0) as usize;
            out.extend_from_slice(&px[start..start + rect.w as usize]);
        }
        Some(out)
    }

    fn wants(&mut self, center: (u32, u32), cells: u32) -> Vec<Ask> {
        let mut out = Vec::new();
        if self.display.is_none() && !self.busy(Ask::Display) {
            out.push(Ask::Display);
        }
        if !self.busy(Ask::Pointer) {
            out.push(Ask::Pointer);
        }
        if let Some((w, h)) = self.display {
            if !self.busy(Ask::Loupe(PixelRect { x: 0, y: 0, w: 0, h: 0 })) {
                let side = (cells + MARGIN).min(MAX_CELLS);
                let rect = loupe::sample_rect(center.0, center.1, side, w, h);
                out.push(Ask::Loupe(rect));
            }
        }
        out
    }

    fn track(&mut self, request: u32, ask: Ask) {
        self.in_flight.push((request, ask));
    }

    fn answer(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        let Some(i) = self.in_flight.iter().position(|(r, _)| *r == request) else { return false };
        let (_, ask) = self.in_flight.remove(i);
        match ask {
            Ask::Display => self.display = result.as_ref().ok().and_then(screen_sample::decode_size),
            Ask::Pointer => self.pointer = result.as_ref().ok().and_then(screen_sample::decode_point),
            Ask::Loupe(rect) => {
                self.loupe = result
                    .as_ref()
                    .ok()
                    .and_then(screen_sample::decode)
                    .filter(|(w, h, _)| *w == rect.w && *h == rect.h)
                    .map(|(_, _, bytes)| (rect, bytes.chunks_exact(3).map(|c| Rgb8::new(c[0], c[1], c[2])).collect()));
            }
        }
        true
    }
}

/// An in-memory display for tests.
#[cfg(test)]
pub struct Fixture {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<Rgb8>,
    pub at: Option<(u32, u32)>,
}

#[cfg(test)]
impl Fixture {
    /// `None` when the pixel count does not match the size.
    pub fn new(w: u32, h: u32, pixels: Vec<Rgb8>) -> Option<Fixture> {
        (pixels.len() == (w as usize) * (h as usize)).then_some(Fixture { w, h, pixels, at: None })
    }
}

#[cfg(test)]
impl ScreenSource for Fixture {
    fn display_size(&self) -> Option<(u32, u32)> {
        Some((self.w, self.h))
    }

    fn pointer(&self) -> Option<(u32, u32)> {
        self.at
    }

    fn read(&self, rect: PixelRect) -> Option<Vec<Rgb8>> {
        if rect.x.checked_add(rect.w)? > self.w || rect.y.checked_add(rect.h)? > self.h {
            return None;
        }
        let mut out = Vec::with_capacity((rect.w * rect.h) as usize);
        for row in rect.y..rect.y + rect.h {
            let start = (row * self.w + rect.x) as usize;
            out.extend_from_slice(&self.pixels[start..start + rect.w as usize]);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_production_source_reads_nothing_before_the_shell_answers() {
        let s = ShellSource::default();
        assert_eq!(s.display_size(), None);
        assert_eq!(s.pointer(), None);
        assert_eq!(s.read(PixelRect { x: 0, y: 0, w: 1, h: 1 }), None);
    }

    #[test]
    fn wants_asks_for_the_display_and_pointer_first_then_a_loupe_square() {
        let mut s = ShellSource::default();
        let first = s.wants((50, 40), 11);
        assert!(first.contains(&Ask::Display) && first.contains(&Ask::Pointer));
        assert!(!first.iter().any(|a| matches!(a, Ask::Loupe(_))));
        s.track(1, Ask::Display);
        s.track(2, Ask::Pointer);
        // nothing in flight of these kinds is asked again
        assert!(s.wants((50, 40), 11).is_empty());
        assert!(s.answer(1, &Ok(size(100, 80))));
        let next = s.wants((50, 40), 11);
        assert_eq!(next, vec![Ask::Loupe(PixelRect { x: 43, y: 33, w: 15, h: 15 })]);
    }

    #[test]
    fn only_our_requests_are_taken() {
        let mut s = ShellSource::default();
        s.track(7, Ask::Pointer);
        assert!(!s.answer(8, &Ok(Json::Null)));
        assert!(s.answer(7, &Err("the Screen permission is not granted".into())));
        assert_eq!(s.pointer(), None);
    }

    #[test]
    fn a_loupe_reply_serves_the_rectangles_inside_it() {
        let mut s = ShellSource::default();
        let rect = PixelRect { x: 0, y: 0, w: 2, h: 2 };
        s.track(5, Ask::Loupe(rect));
        let rgb: Vec<Json> = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].iter().map(|b| Json::Num(*b as f64)).collect();
        let reply = Json::Obj(vec![("w".into(), Json::Num(2.0)), ("h".into(), Json::Num(2.0)), ("rgb".into(), Json::Arr(rgb))]);
        assert!(s.answer(5, &Ok(reply)));
        assert_eq!(s.read(PixelRect { x: 1, y: 0, w: 1, h: 2 }), Some(vec![Rgb8::new(4, 5, 6), Rgb8::new(10, 11, 12)]));
        assert_eq!(s.read(PixelRect { x: 1, y: 1, w: 2, h: 1 }), None);
    }

    #[test]
    fn a_loupe_reply_of_the_wrong_size_is_dropped() {
        let mut s = ShellSource::default();
        s.track(5, Ask::Loupe(PixelRect { x: 0, y: 0, w: 2, h: 2 }));
        let reply = Json::Obj(vec![("w".into(), Json::Num(1.0)), ("h".into(), Json::Num(1.0)), ("rgb".into(), Json::Arr(vec![Json::Num(0.0); 3]))]);
        s.answer(5, &Ok(reply));
        assert_eq!(s.read(PixelRect { x: 0, y: 0, w: 1, h: 1 }), None);
    }

    fn size(w: u32, h: u32) -> Json {
        Json::Obj(vec![("w".into(), Json::Num(w as f64)), ("h".into(), Json::Num(h as f64))])
    }

    #[test]
    fn a_fixture_reads_rows_in_order_and_refuses_outside_rects() {
        let px: Vec<Rgb8> = (0..6u8).map(|i| Rgb8::new(i, i, i)).collect();
        let f = Fixture::new(3, 2, px).unwrap();
        let got = f.read(PixelRect { x: 1, y: 0, w: 2, h: 2 }).unwrap();
        assert_eq!(got, vec![Rgb8::new(1, 1, 1), Rgb8::new(2, 2, 2), Rgb8::new(4, 4, 4), Rgb8::new(5, 5, 5)]);
        assert!(f.read(PixelRect { x: 2, y: 0, w: 2, h: 1 }).is_none());
        assert!(f.read(PixelRect { x: 0, y: 1, w: 1, h: 2 }).is_none());
    }

    #[test]
    fn a_fixture_refuses_a_wrong_pixel_count() {
        assert!(Fixture::new(2, 2, vec![Rgb8::new(0, 0, 0); 3]).is_none());
    }
}
