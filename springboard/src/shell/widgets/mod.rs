//! The Today View widgets, each rendered into its own tile image (bifrost
//! 15 look: 22 pt corners, small 155 x 155 pt, medium 329 x 155 pt) whenever
//! the data behind it changes, so the page itself only blits.

pub mod battery;
pub mod calendar;
pub mod clock;
pub mod deildu;
pub mod photos;
pub mod terminal;
pub mod weather;
pub mod wifi;

use crate::gfx::{rgba, rrect_mask, Image, Surface};

/// Side of a small widget and width of a medium one, in points.
pub const SMALL_PT: f32 = 155.0;
pub const MEDIUM_PT: f32 = 329.0;
pub const RADIUS_PT: f32 = 22.0;
/// Inset of a widget's content from its edge.
pub const PAD_PT: f32 = 16.0;

/// Every widget the Today View can show, in the default order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Clock,
    Battery,
    Wifi,
    Deildu,
    Calendar,
    Terminal,
    Photos,
    Weather,
    /// Smart Stack: swipe vertically through the small widgets
    Stack,
}

impl Kind {
    pub const ALL: [Kind; 9] = [Kind::Clock, Kind::Battery, Kind::Wifi, Kind::Deildu, Kind::Calendar, Kind::Terminal, Kind::Stack, Kind::Photos, Kind::Weather];

    /// The key the preferences file stores this widget under.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Clock => "clock",
            Kind::Battery => "battery",
            Kind::Wifi => "wifi",
            Kind::Deildu => "deildu",
            Kind::Calendar => "calendar",
            Kind::Terminal => "terminal",
            Kind::Photos => "photos",
            Kind::Weather => "weather",
            Kind::Stack => "stack",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Medium widgets span the column; small ones sit two to a row.
    pub fn medium(self) -> bool {
        matches!(self, Kind::Clock | Kind::Deildu | Kind::Terminal)
    }

    /// Tile size in px at `s` px per point.
    pub fn size(self, s: f32) -> (usize, usize) {
        let w = if self.medium() { MEDIUM_PT } else { SMALL_PT };
        ((w * s).round() as usize, (SMALL_PT * s).round() as usize)
    }
}

/// The dark frosted tile every widget draws on.
pub fn background() -> u32 {
    rgba(30, 30, 32, 236)
}

/// A blank tile of `w` x `h` px with rounded corners.
pub fn tile(s: f32, w: usize, h: usize) -> Image {
    let mut img = Image { w, h, px: vec![background(); w * h] };
    img.apply_mask(&rrect_mask(w, h, RADIUS_PT * s));
    img
}

/// Draws into an image as if it were a frame.
pub fn surface(img: &mut Image) -> Surface<'_> {
    Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }
}

/// Blits `text` onto `img` at (x, y).
pub fn put(img: &mut Image, text: &Image, x: f32, y: f32) {
    surface(img).blit(text, x.round() as i32, y.round() as i32, 255, 255);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_their_keys_and_know_their_size() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_key(k.key()), Some(k));
        }
        assert_eq!(Kind::from_key("nope"), None);
        assert_eq!(Kind::Clock.size(2.0), (658, 310));
        assert_eq!(Kind::Battery.size(2.0), (310, 310));
        assert!(Kind::Deildu.medium() && !Kind::Photos.medium());
    }

    #[test]
    fn tile_is_rounded_and_opaque_enough() {
        let t = tile(1.5625, 100, 60);
        assert_eq!(t.px[0] >> 24, 0, "corner is clear");
        assert!(t.px[30 * 100 + 50] >> 24 > 200);
        let mut img = tile(1.0, 40, 40);
        put(&mut img, &Image { w: 2, h: 2, px: vec![0xffff_ffff; 4] }, 10.0, 10.0);
        assert_eq!(img.px[10 * 40 + 10], 0xffff_ffff);
    }
}
