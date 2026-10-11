//! App icon, drawn without dependencies.
use springboard::gfx::{Image, Surface};
use springboard::shell::icons;
use bifrost_app::kit::ui::{BLACK, SYSTEM_RED, WHITE};
const RED: u32 = SYSTEM_RED;

/// White calendar page with a red header and a large date, drawn without
/// dependencies. The shell applies its normal squircle mask.
pub fn icon(size: usize) -> Image {
    let paper = WHITE;
    let ink = BLACK;
    let mut img = Image {
        w: size,
        h: size,
        px: vec![paper; size * size],
    };
    let f = size as f32;
    let mut s = Surface {
        w: size,
        h: size,
        stride: size,
        px: &mut img.px,
        y0: 0,
    };
    s.fill_rrect(0.0, 0.0, f, f * 0.27, 0.0, RED);
    // A procedural 28 avoids loading a font while home icons are assembled;
    // curves are flattened to short round-capped strokes so they stay smooth.
    let thick = f * 0.05;
    let mut stroke = |pts: &[(f32, f32)]| {
        for w in pts.windows(2) {
            icons::line(&mut s, (f * w[0].0, f * w[0].1), (f * w[1].0, f * w[1].1), thick, ink);
        }
    };
    let arc = |cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32| -> Vec<(f32, f32)> {
        (0..=48)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / 48.0).to_radians();
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    };
    let mut two = arc(0.29, 0.50, 0.105, 0.10, -165.0, 35.0);
    two.push((0.185, 0.81));
    two.push((0.395, 0.81));
    stroke(&two);
    stroke(&arc(0.69, 0.495, 0.095, 0.09, 0.0, 360.0));
    stroke(&arc(0.69, 0.71, 0.11, 0.10, 0.0, 360.0));
    for i in 0..3 {
        s.fill_rrect(
            f * (0.29 + 0.16 * i as f32),
            f * 0.11,
            f * 0.08,
            f * 0.04,
            f * 0.01,
            paper,
        );
    }
    img
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_antialiased_not_blocky() {
        let img = icon(256);
        let mut partial = 0;
        for p in &img.px[256 * 100..256 * 215] {
            if *p != BLACK && *p != WHITE {
                partial += 1;
            }
        }
        assert!(partial > 400, "expected smooth edges, got {partial} blended pixels");
    }
}
