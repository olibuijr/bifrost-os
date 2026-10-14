//! Photo looks by name (mono, noir, tonal, chrome, fade, instant, process, transfer) and a false
//! colour map. Each look is a short chain of `colorfx` operations. The looks approximate the
//! reference photo effects; their tone curves are not the reference curves. Colours are
//! unpremultiplied RGB with each channel in 0..=1.
use crate::colorfx::{self, Rgb};

/// A photo look. `ALL` lists them in the order of the reference names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoLook {
    Chrome,
    Fade,
    Instant,
    Mono,
    Noir,
    Process,
    Tonal,
    Transfer,
}

impl PhotoLook {
    /// Every look, in a fixed order.
    pub const ALL: [PhotoLook; 8] = [
        PhotoLook::Chrome,
        PhotoLook::Fade,
        PhotoLook::Instant,
        PhotoLook::Mono,
        PhotoLook::Noir,
        PhotoLook::Process,
        PhotoLook::Tonal,
        PhotoLook::Transfer,
    ];

    /// The lower-case name of the look, the one `from_name` accepts.
    pub fn name(self) -> &'static str {
        match self {
            PhotoLook::Chrome => "chrome",
            PhotoLook::Fade => "fade",
            PhotoLook::Instant => "instant",
            PhotoLook::Mono => "mono",
            PhotoLook::Noir => "noir",
            PhotoLook::Process => "process",
            PhotoLook::Tonal => "tonal",
            PhotoLook::Transfer => "transfer",
        }
    }

    /// The look called `name`, or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<PhotoLook> {
        PhotoLook::ALL.into_iter().find(|&look| look.name() == name)
    }
}

/// Rec. 601 luma as a grey colour.
fn mono(c: Rgb) -> Rgb {
    let l = c.luma();
    Rgb::new(l, l, l)
}

/// Raises the black point to `floor` and keeps white at 1.
fn lift(c: Rgb, floor: f32) -> Rgb {
    let f = floor.clamp(0.0, 1.0);
    Rgb::new(f + (1.0 - f) * c.r, f + (1.0 - f) * c.g, f + (1.0 - f) * c.b).clamped()
}

/// Applies a photo look to one colour.
pub fn apply(look: PhotoLook, c: Rgb) -> Rgb {
    match look {
        PhotoLook::Mono => mono(c),
        PhotoLook::Noir => colorfx::contrast(mono(c), 1.5),
        PhotoLook::Tonal => colorfx::sepia(mono(c), 0.4),
        PhotoLook::Chrome => colorfx::contrast(colorfx::saturation(c, 1.15), 1.1),
        PhotoLook::Fade => lift(colorfx::contrast(c, 0.85), 0.06),
        PhotoLook::Instant => lift(colorfx::sepia(c, 0.3), 0.08),
        PhotoLook::Process => colorfx::contrast(Rgb::new(c.r * 1.1, c.g, c.b * 0.8).clamped(), 1.2),
        PhotoLook::Transfer => colorfx::contrast(colorfx::saturation(c, 0.7), 1.25),
    }
}

/// False colour: each colour becomes the mix of `dark` and `light` at its luma, so black maps to
/// `dark` and white to `light`.
pub fn false_colour(c: Rgb, dark: Rgb, light: Rgb) -> Rgb {
    dark.mix(light, c.luma().clamp(0.0, 1.0)).clamped()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn names_round_trip_through_from_name() {
        for look in PhotoLook::ALL {
            assert_eq!(PhotoLook::from_name(look.name()), Some(look));
        }
        assert_eq!(PhotoLook::from_name("sepia"), None);
        assert_eq!(PhotoLook::from_name(""), None);
    }

    #[test]
    fn mono_and_noir_give_equal_channels() {
        let c = Rgb::new(0.9, 0.3, 0.1);
        for look in [PhotoLook::Mono, PhotoLook::Noir] {
            let out = apply(look, c);
            assert!(close(out.r, out.g) && close(out.g, out.b), "{:?}", look);
        }
        assert!(close(apply(PhotoLook::Mono, c).r, c.luma()));
    }

    #[test]
    fn every_look_stays_in_range_over_a_sweep_of_colours() {
        for look in PhotoLook::ALL {
            for i in 0..=8 {
                for j in 0..=8 {
                    let c = Rgb::new(i as f32 / 8.0, j as f32 / 8.0, ((i + j) % 9) as f32 / 8.0);
                    let out = apply(look, c);
                    for v in [out.r, out.g, out.b] {
                        assert!((0.0..=1.0).contains(&v), "{:?} gave {}", look, v);
                    }
                }
            }
        }
    }

    #[test]
    fn fade_lifts_black_and_keeps_white_below_one() {
        let black = apply(PhotoLook::Fade, Rgb::new(0.0, 0.0, 0.0));
        assert!(black.r > 0.0 && close(black.r, black.g));
        let white = apply(PhotoLook::Fade, Rgb::new(1.0, 1.0, 1.0));
        assert!(white.r <= 1.0 && white.r > 0.9);
    }

    #[test]
    fn false_colour_maps_luma_between_the_two_colours() {
        let dark = Rgb::new(0.0, 0.0, 0.5);
        let light = Rgb::new(1.0, 1.0, 0.0);
        let black = false_colour(Rgb::new(0.0, 0.0, 0.0), dark, light);
        assert!(close(black.b, 0.5) && close(black.r, 0.0));
        let white = false_colour(Rgb::new(1.0, 1.0, 1.0), dark, light);
        assert!(close(white.r, 1.0) && close(white.b, 0.0));
    }
}
