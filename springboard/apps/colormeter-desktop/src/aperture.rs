//! The meter's aperture: the odd square of pixels averaged around the pointer (1 to 31 across), the average of
//! those pixels clamped at the screen edge, and the Update Continuously switch that decides when to sample.
#![allow(dead_code)] // the aperture slider, the loupe outline and the View menu wire this in next

/// The widest aperture, in pixels.
pub const MAX_SIZE: u32 = 31;

/// An aperture size: always odd, so the pointer pixel sits in the centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aperture(u32);

impl Aperture {
    /// The size nearest to `size` that is odd and between 1 and `MAX_SIZE`.
    pub fn new(size: u32) -> Aperture {
        let s = size.clamp(1, MAX_SIZE);
        Aperture(if s % 2 == 0 { s - 1 } else { s })
    }

    pub fn size(self) -> u32 {
        self.0
    }

    /// The slider value 0.0 to 1.0 for this size, and back.
    pub fn slider(self) -> f32 {
        (self.0 - 1) as f32 / (MAX_SIZE - 1) as f32
    }

    pub fn from_slider(v: f32) -> Aperture {
        Aperture::new(1 + (v.clamp(0.0, 1.0) * (MAX_SIZE - 1) as f32).round() as u32)
    }

    /// The square around `(cx, cy)` as `(x0, y0, x1, y1)` (end exclusive), clipped to a `w` by `h` screen.
    pub fn bounds(self, cx: i32, cy: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
        let r = (self.0 / 2) as i32;
        ((cx - r).max(0), (cy - r).max(0), (cx + r + 1).min(w), (cy + r + 1).min(h))
    }
}

/// The average colour `(r, g, b)` of the aperture on a `w` by `h` screen of `0xAARRGGBB` pixels. None when the square
/// holds no pixel (the pointer is off the screen).
pub fn average(px: &[u32], w: usize, h: usize, cx: i32, cy: i32, aperture: Aperture) -> Option<(u8, u8, u8)> {
    let (x0, y0, x1, y1) = aperture.bounds(cx, cy, w as i32, h as i32);
    if x0 >= x1 || y0 >= y1 || px.len() < w * h {
        return None;
    }
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = px[y as usize * w + x as usize];
            r += u64::from(p >> 16 & 0xff);
            g += u64::from(p >> 8 & 0xff);
            b += u64::from(p & 0xff);
            n += 1;
        }
    }
    Some(((r / n) as u8, (g / n) as u8, (b / n) as u8))
}

/// When the meter takes a new sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampling {
    pub continuous: bool,
    last: Option<(i32, i32)>,
}

impl Sampling {
    pub fn new(continuous: bool) -> Sampling {
        Sampling { continuous, last: None }
    }

    /// Whether to sample now: always when continuous, else only when the pointer has moved since the last sample.
    pub fn should_sample(&mut self, pointer: (i32, i32)) -> bool {
        let moved = self.last != Some(pointer);
        if self.continuous || moved {
            self.last = Some(pointer);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_odd_and_clamped() {
        assert_eq!(Aperture::new(0).size(), 1);
        assert_eq!(Aperture::new(4).size(), 3);
        assert_eq!(Aperture::new(5).size(), 5);
        assert_eq!(Aperture::new(99).size(), 31);
        assert_eq!(Aperture::from_slider(0.0).size(), 1);
        assert_eq!(Aperture::from_slider(1.0).size(), 31);
        assert_eq!(Aperture::new(31).slider(), 1.0);
    }

    #[test]
    fn bounds_clip_at_the_edges() {
        let a = Aperture::new(5);
        assert_eq!(a.bounds(10, 10, 100, 100), (8, 8, 13, 13));
        assert_eq!(a.bounds(0, 0, 100, 100), (0, 0, 3, 3));
        assert_eq!(a.bounds(99, 99, 100, 100), (97, 97, 100, 100));
        let (x0, _, x1, _) = a.bounds(500, 5, 100, 100);
        assert!(x0 >= x1);
    }

    #[test]
    fn average_of_a_square() {
        // 4x1 screen: black, white, white, black
        let px = [0xff00_0000, 0xffff_ffff, 0xffff_ffff, 0xff00_0000];
        assert_eq!(average(&px, 4, 1, 1, 0, Aperture::new(1)), Some((255, 255, 255)));
        assert_eq!(average(&px, 4, 1, 1, 0, Aperture::new(3)), Some((170, 170, 170)));
        assert_eq!(average(&px, 4, 1, 50, 0, Aperture::new(3)), None);
    }

    #[test]
    fn sampling_modes() {
        let mut s = Sampling::new(false);
        assert!(s.should_sample((1, 1)));
        assert!(!s.should_sample((1, 1)));
        assert!(s.should_sample((2, 1)));
        let mut c = Sampling::new(true);
        assert!(c.should_sample((1, 1)));
        assert!(c.should_sample((1, 1)));
    }
}
