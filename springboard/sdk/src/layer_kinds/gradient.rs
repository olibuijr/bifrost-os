//! Gradient content: colours along a ramp, drawn axial, radial or conic between a start and an end point given in unit
//! coordinates of the content (0,0 is the top left, 1,1 the bottom right). A radial gradient is an ellipse when the
//! content is not square, as the unit space is stretched with it.
use bifrost_render::gfx::{self, Image};
pub use bifrost_ui::gradient::GradientType;
use bifrost_ui::gradient::{Gradient, Stop};

#[derive(Clone, Debug, PartialEq)]
pub struct GradientContent {
    pub kind: GradientType,
    /// Straight RGBA colours along the ramp.
    pub colors: Vec<[u8; 4]>,
    /// Stop positions 0 to 1, one per colour. `None`, or a list of another length, spaces the colours evenly.
    pub locations: Option<Vec<f32>>,
    pub start: (f32, f32),
    pub end: (f32, f32),
}

impl Default for GradientContent {
    /// Axial, top to bottom, no colours.
    fn default() -> Self {
        GradientContent { kind: GradientType::Axial, colors: Vec::new(), locations: None, start: (0.5, 0.0), end: (0.5, 1.0) }
    }
}

impl GradientContent {
    pub fn new(kind: GradientType, colors: Vec<[u8; 4]>) -> Self {
        GradientContent { kind, colors, ..Default::default() }
    }

    /// The ramp these colours and locations make.
    pub fn ramp(&self) -> Gradient {
        let n = self.colors.len();
        let even = |i: usize| if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
        let stops = self
            .colors
            .iter()
            .enumerate()
            .map(|(i, &rgba)| {
                let at = match &self.locations {
                    Some(l) if l.len() == n => l[i],
                    _ => even(i),
                };
                Stop { at, rgba }
            })
            .collect();
        Gradient::new(stops)
    }

    /// The straight RGBA colour at unit position `(u, v)` of the content.
    pub fn color_at(&self, u: f32, v: f32) -> [u8; 4] {
        self.ramp().color_at(Gradient::layer_t(self.kind, self.start, self.end, u, v))
    }

    /// Rasterises into a `w` x `h` image.
    pub fn render(&self, w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        if w == 0 || h == 0 || self.colors.is_empty() {
            return img;
        }
        let ramp = self.ramp();
        // The ramp is sampled through a 256 entry table of premultiplied colours.
        let lut: Vec<u32> = (0..256)
            .map(|i| {
                let c = ramp.color_at(i as f32 / 255.0);
                gfx::rgba(c[0], c[1], c[2], c[3])
            })
            .collect();
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let t = Gradient::layer_t(self.kind, self.start, self.end, u, v);
                img.px[y * w + x] = lut[(t * 255.0 + 0.5).clamp(0.0, 255.0) as usize];
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn an_axial_gradient_runs_from_top_to_bottom_by_default() {
        let g = GradientContent::new(GradientType::Axial, vec![BLACK, WHITE]);
        let img = g.render(4, 16);
        assert_eq!(img.px[0] & 0xff, 8, "the top row is close to the first colour");
        assert!((img.px[15 * 4] & 0xff) > 240, "the bottom row is close to the last colour");
        assert_eq!(img.px[8], img.px[8 + 3], "a row is one colour for a vertical axis");
    }

    #[test]
    fn locations_move_the_stops() {
        let mut g = GradientContent::new(GradientType::Axial, vec![BLACK, WHITE]);
        g.locations = Some(vec![0.5, 1.0]);
        assert_eq!(g.color_at(0.5, 0.25), BLACK);
        assert_eq!(g.color_at(0.5, 0.75)[0], 128);
        g.locations = Some(vec![0.0]);
        assert_eq!(g.color_at(0.5, 0.5)[0], 128, "a list of the wrong length spaces the colours evenly");
    }

    #[test]
    fn a_radial_gradient_grows_with_distance_from_the_start() {
        let mut g = GradientContent::new(GradientType::Radial, vec![WHITE, BLACK]);
        g.start = (0.5, 0.5);
        g.end = (1.0, 0.5);
        assert_eq!(g.color_at(0.5, 0.5), WHITE);
        assert_eq!(g.color_at(1.0, 0.5), BLACK);
        assert_eq!(g.color_at(0.0, 0.0), BLACK, "past the end point the last colour holds");
    }

    #[test]
    fn a_conic_gradient_turns_clockwise_from_the_end_direction() {
        let mut g = GradientContent::new(GradientType::Conic, vec![BLACK, WHITE]);
        g.start = (0.5, 0.5);
        g.end = (1.0, 0.5);
        // Just past the start direction the ramp is near 0, a quarter turn clockwise (down) it is a quarter of the way.
        assert_eq!(g.color_at(1.0, 0.5), BLACK);
        let q = g.color_at(0.5, 1.0)[0];
        assert!((q as i32 - 64).abs() <= 2);
        let half = g.color_at(0.0, 0.5)[0];
        assert!((half as i32 - 128).abs() <= 2);
    }

    #[test]
    fn translucent_colours_are_premultiplied() {
        let g = GradientContent::new(GradientType::Axial, vec![[255, 0, 0, 128], [255, 0, 0, 128]]);
        let img = g.render(2, 2);
        assert_eq!(img.px[0], gfx::rgba(255, 0, 0, 128));
    }

    #[test]
    fn no_colours_render_nothing() {
        assert!(GradientContent::default().render(3, 3).px.iter().all(|&p| p == 0));
    }
}
