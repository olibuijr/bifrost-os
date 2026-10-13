//! Window occlusion for the GPU presenter: a window whose whole drawable lies inside the opaque part of a
//! window above it cannot be seen, so the presenter does not draw it. Pure geometry; the caller decides
//! which windows are opaque (no vibrancy, no transform in flight) and which carry materials.

/// One window in stack order, bottom first, in screen px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// The whole drawable: the content plus its shadow margins (x, y, w, h).
    pub rect: [i32; 4],
    /// The part of the content that is opaque, inset by the corner radius (x, y, w, h); None when the window
    /// can not hide anything behind it.
    pub cover: Option<[i32; 4]>,
    /// The window has a material: its backdrop reads the windows below it, so it is always drawn.
    pub glass: bool,
    /// The window's transform is the identity: an animating window is neither hidden nor a cover.
    pub still: bool,
}

/// For each layer (same order as `layers`), whether it is hidden: a still, glass-free layer whose drawable lies
/// inside the cover of a still layer above it. Glass between the two blocks the hiding, because the glass blurs
/// what is below it and the hidden window's pixels would show in that blur.
pub fn hidden(layers: &[Layer]) -> Vec<bool> {
    (0..layers.len()).map(|i| layers[i].still && !layers[i].glass && covered_from_above(layers, i)).collect()
}

fn covered_from_above(layers: &[Layer], i: usize) -> bool {
    for up in layers.iter().skip(i + 1) {
        if up.still && up.cover.is_some_and(|c| inside(layers[i].rect, c)) {
            return true;
        }
        if up.glass {
            return false;
        }
    }
    false
}

/// True when rectangle `inner` (x, y, w, h) lies inside `outer`.
pub fn inside(inner: [i32; 4], outer: [i32; 4]) -> bool {
    inner[0] >= outer[0]
        && inner[1] >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}

/// The opaque rectangle of a window content rect (x, y, w, h) with corner radius `radius` px: inset by the
/// radius on every side, so the rounded corners are never counted as opaque. None when nothing is left.
pub fn opaque_inset(content: [i32; 4], radius: i32) -> Option<[i32; 4]> {
    let r = radius.max(0);
    let (w, h) = (content[2] - 2 * r, content[3] - 2 * r);
    (w > 0 && h > 0).then_some([content[0] + r, content[1] + r, w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(rect: [i32; 4], cover: Option<[i32; 4]>) -> Layer {
        Layer { rect, cover, glass: false, still: true }
    }

    #[test]
    fn a_window_inside_an_opaque_window_above_is_hidden() {
        let below = layer([100, 100, 200, 150], None);
        let above = layer([0, 0, 800, 600], Some([0, 0, 800, 600]));
        assert_eq!(hidden(&[below, above]), vec![true, false]);
    }

    #[test]
    fn a_window_that_sticks_out_is_not_hidden() {
        let below = layer([100, 100, 200, 150], None);
        let above = layer([0, 0, 250, 600], Some([0, 0, 250, 600]));
        assert_eq!(hidden(&[below, above]), vec![false, false]);
    }

    #[test]
    fn the_shadow_margin_counts_as_part_of_the_drawable() {
        // the drawable of the lower window reaches past the cover by its shadow, so it stays visible
        let below = layer([100, 100, 200, 150], None);
        let below_with_shadow = Layer { rect: [90, 90, 220, 170], ..below };
        let above = layer([0, 0, 800, 600], Some([100, 100, 200, 150]));
        assert_eq!(hidden(&[below_with_shadow, above]), vec![false, false]);
        assert_eq!(hidden(&[below, above]), vec![true, false]);
    }

    #[test]
    fn a_translucent_window_above_hides_nothing() {
        let below = layer([100, 100, 200, 150], None);
        let above = layer([0, 0, 800, 600], None);
        assert_eq!(hidden(&[below, above]), vec![false, false]);
    }

    #[test]
    fn a_hidden_window_can_still_be_a_cover_for_the_one_below_it() {
        let top = layer([0, 0, 800, 600], Some([0, 0, 800, 600]));
        let mid = layer([100, 100, 200, 150], Some([100, 100, 200, 150]));
        let bottom = layer([120, 120, 50, 50], None);
        assert_eq!(hidden(&[bottom, mid, top]), vec![true, true, false]);
    }

    #[test]
    fn glass_is_always_drawn_and_blocks_the_hiding_below_it() {
        let below = layer([100, 100, 200, 150], None);
        let glass = Layer { glass: true, ..layer([300, 300, 100, 100], None) };
        let above = layer([0, 0, 800, 600], Some([0, 0, 800, 600]));
        // glass between the two: the lower window stays drawn, and so does the glass itself
        assert_eq!(hidden(&[below, glass, above]), vec![false, false, false]);
        // glass above the cover: the lower window is hidden, the glass is drawn
        assert_eq!(hidden(&[below, above, glass]), vec![true, false, false]);
    }

    #[test]
    fn an_animating_window_is_neither_hidden_nor_a_cover() {
        let below = layer([100, 100, 200, 150], None);
        let moving_above = Layer { still: false, ..layer([0, 0, 800, 600], Some([0, 0, 800, 600])) };
        assert_eq!(hidden(&[below, moving_above]), vec![false, false]);
        let moving_below = Layer { still: false, ..below };
        let above = layer([0, 0, 800, 600], Some([0, 0, 800, 600]));
        assert_eq!(hidden(&[moving_below, above]), vec![false, false]);
    }

    #[test]
    fn the_rounded_corners_are_not_opaque() {
        assert_eq!(opaque_inset([10, 20, 300, 200], 12), Some([22, 32, 276, 176]));
        assert_eq!(opaque_inset([10, 20, 300, 200], 0), Some([10, 20, 300, 200]));
        assert_eq!(opaque_inset([0, 0, 20, 20], 10), None);
        // a window behind the rounded corner of the one above is kept
        let below = layer([12, 12, 50, 50], None);
        let above = layer([10, 10, 300, 200], opaque_inset([10, 10, 300, 200], 12));
        assert_eq!(hidden(&[below, above]), vec![false, false]);
    }
}
