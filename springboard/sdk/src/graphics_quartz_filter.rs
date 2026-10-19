//! Filters for written PDF files (`QuartzFilter`, `QuartzFilterManager`): a colour transform applied to every fill and
//! stroke colour and every raster image of a page, a lightness shift, and a limit on image size. The built-in filters
//! are the ones the print dialog's PDF menu offers (Black & White, Gray Tone, Sepia Tone, Blue Tone, Lightness
//! Decrease, Lightness Increase, Reduce File Size). A filter is applied when a `graphics_pdf::PdfWriter` writes its file
//! (`PdfWriter::finish_filtered`); it does not rewrite a PDF that was read from elsewhere.
//!
//! ```ignore
//! let manager = QuartzFilterManager::new();
//! let filter = manager.find("Sepia Tone").unwrap();
//! let bytes = writer.finish_filtered(filter);
//! ```

/// A colour and image filter for PDF output.
#[derive(Clone, Debug, PartialEq)]
pub struct QuartzFilter {
    pub name: String,
    /// Row-major 3x3 matrix applied to (r, g, b) in 0..=1; `None` leaves colours alone.
    pub matrix: Option<[[f64; 3]; 3]>,
    /// Added to every channel after the matrix (negative darkens), clamped to 0..=1.
    pub lightness: f64,
    /// Images larger than this on their long side are scaled down to it; `None` keeps them.
    pub max_image_side: Option<usize>,
}

const LUMA: [f64; 3] = [0.299, 0.587, 0.114];

fn clamp01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

impl QuartzFilter {
    /// A filter that changes nothing.
    pub fn identity(name: &str) -> QuartzFilter {
        QuartzFilter { name: name.to_string(), matrix: None, lightness: 0.0, max_image_side: None }
    }

    pub fn black_and_white() -> QuartzFilter {
        QuartzFilter { matrix: Some([LUMA; 3]), ..QuartzFilter::identity("Black & White") }
    }

    pub fn gray_tone() -> QuartzFilter {
        let row = [LUMA[0] * 0.8, LUMA[1] * 0.8, LUMA[2] * 0.8];
        QuartzFilter { matrix: Some([row; 3]), lightness: 0.1, ..QuartzFilter::identity("Gray Tone") }
    }

    pub fn sepia_tone() -> QuartzFilter {
        let m = [[0.393, 0.769, 0.189], [0.349, 0.686, 0.168], [0.272, 0.534, 0.131]];
        QuartzFilter { matrix: Some(m), ..QuartzFilter::identity("Sepia Tone") }
    }

    pub fn blue_tone() -> QuartzFilter {
        let tint = [0.75, 0.9, 1.0];
        let m = [0, 1, 2].map(|row| LUMA.map(|l| l * tint[row]));
        QuartzFilter { matrix: Some(m), ..QuartzFilter::identity("Blue Tone") }
    }

    pub fn lightness_decrease() -> QuartzFilter {
        QuartzFilter { lightness: -0.2, ..QuartzFilter::identity("Lightness Decrease") }
    }

    pub fn lightness_increase() -> QuartzFilter {
        QuartzFilter { lightness: 0.2, ..QuartzFilter::identity("Lightness Increase") }
    }

    pub fn reduce_file_size() -> QuartzFilter {
        QuartzFilter { max_image_side: Some(512), ..QuartzFilter::identity("Reduce File Size") }
    }

    /// One colour (channels in 0..=1) through the filter.
    pub fn map_color(&self, rgb: [f64; 3]) -> [f64; 3] {
        let src = rgb.map(clamp01);
        let mixed = match &self.matrix {
            Some(m) => m.map(|row| row[0] * src[0] + row[1] * src[1] + row[2] * src[2]),
            None => src,
        };
        mixed.map(|v| clamp01(v + self.lightness))
    }

    fn changes_colour(&self) -> bool {
        self.matrix.is_some() || self.lightness != 0.0
    }

    /// A page content stream with every `r g b rg` and `r g b RG` colour operator filtered. Everything else is
    /// copied; runs of white space inside a line collapse to one space.
    pub fn filter_content(&self, content: &str) -> String {
        if !self.changes_colour() {
            return content.to_string();
        }
        let mut out = String::with_capacity(content.len());
        for line in content.lines() {
            let mut tokens: Vec<String> = Vec::new();
            for tok in line.split_whitespace() {
                if (tok == "rg" || tok == "RG") && tokens.len() >= 3 {
                    let n = tokens.len();
                    let parsed: Vec<Option<f64>> = tokens[n - 3..].iter().map(|t| t.parse::<f64>().ok()).collect();
                    if let [Some(r), Some(g), Some(b)] = parsed[..] {
                        let [r, g, b] = self.map_color([r, g, b]);
                        tokens.truncate(n - 3);
                        tokens.push(format!("{r:.4}"));
                        tokens.push(format!("{g:.4}"));
                        tokens.push(format!("{b:.4}"));
                    }
                }
                tokens.push(tok.to_string());
            }
            out.push_str(&tokens.join(" "));
            out.push('\n');
        }
        out
    }

    /// A raster image (three bytes per pixel, rows top to bottom) through the filter: scaled down by a box filter to
    /// the size limit, then colour-mapped. Returns the pixels and the new width and height; a buffer whose length does
    /// not match its size comes back unchanged.
    pub fn filter_image(&self, rgb: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
        if w == 0 || h == 0 || rgb.len() != w * h * 3 {
            return (rgb.to_vec(), w, h);
        }
        let (mut px, mut nw, mut nh) = (rgb.to_vec(), w, h);
        if let Some(max) = self.max_image_side.filter(|m| *m > 0) {
            let long = w.max(h);
            if long > max {
                nw = ((w * max) / long).max(1);
                nh = ((h * max) / long).max(1);
                px = box_scale(rgb, w, h, nw, nh);
            }
        }
        if self.changes_colour() {
            for p in px.chunks_exact_mut(3) {
                let [r, g, b] = self.map_color([p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0]);
                p[0] = (r * 255.0).round() as u8;
                p[1] = (g * 255.0).round() as u8;
                p[2] = (b * 255.0).round() as u8;
            }
        }
        (px, nw, nh)
    }
}

/// Scales RGB pixels to `nw` x `nh` by averaging the source pixels each output pixel covers.
fn box_scale(rgb: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(nw * nh * 3);
    for oy in 0..nh {
        let y0 = oy * h / nh;
        let y1 = ((oy + 1) * h / nh).max(y0 + 1).min(h);
        for ox in 0..nw {
            let x0 = ox * w / nw;
            let x1 = ((ox + 1) * w / nw).max(x0 + 1).min(w);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += rgb[(y * w + x) * 3 + c] as u32;
                    }
                    count += 1;
                }
            }
            for s in sum {
                out.push(((s + count / 2) / count.max(1)) as u8);
            }
        }
    }
    out
}

/// The filters the system offers and the ones an app has added (`QuartzFilterManager`).
#[derive(Clone, Debug, PartialEq)]
pub struct QuartzFilterManager {
    filters: Vec<QuartzFilter>,
    builtin: usize,
}

impl Default for QuartzFilterManager {
    fn default() -> Self {
        Self::new()
    }
}

impl QuartzFilterManager {
    /// A manager with the built-in filters.
    pub fn new() -> Self {
        let filters = vec![
            QuartzFilter::black_and_white(),
            QuartzFilter::blue_tone(),
            QuartzFilter::gray_tone(),
            QuartzFilter::lightness_decrease(),
            QuartzFilter::lightness_increase(),
            QuartzFilter::reduce_file_size(),
            QuartzFilter::sepia_tone(),
        ];
        let builtin = filters.len();
        QuartzFilterManager { filters, builtin }
    }

    /// Every filter, the built-in ones first.
    pub fn filters(&self) -> &[QuartzFilter] {
        &self.filters
    }

    /// The filter with this name (case sensitive).
    pub fn find(&self, name: &str) -> Option<&QuartzFilter> {
        self.filters.iter().find(|f| f.name == name)
    }

    /// Adds a filter, or replaces an app-added one of the same name. A built-in name is refused (false).
    pub fn add(&mut self, filter: QuartzFilter) -> bool {
        match self.filters.iter().position(|f| f.name == filter.name) {
            Some(i) if i < self.builtin => false,
            Some(i) => {
                self.filters[i] = filter;
                true
            }
            None => {
                self.filters.push(filter);
                true
            }
        }
    }

    /// Removes an app-added filter; built-in filters stay (false).
    pub fn remove(&mut self, name: &str) -> bool {
        match self.filters.iter().position(|f| f.name == name) {
            Some(i) if i >= self.builtin => {
                self.filters.remove(i);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn black_and_white_maps_every_colour_to_its_luma() {
        let f = QuartzFilter::black_and_white();
        let [r, g, b] = f.map_color([1.0, 0.0, 0.0]);
        assert!(near(r, 0.299) && r == g && g == b);
        assert!(f.map_color([1.0, 1.0, 1.0]).iter().all(|v| near(*v, 1.0)));
        assert_eq!(f.map_color([0.0, 0.0, 0.0]), [0.0; 3]);
    }

    #[test]
    fn lightness_shifts_and_clamps() {
        let up = QuartzFilter::lightness_increase().map_color([0.5, 0.9, 0.0]);
        assert!(near(up[0], 0.7) && near(up[1], 1.0) && near(up[2], 0.2), "{up:?}");
        let down = QuartzFilter::lightness_decrease().map_color([0.1, 0.5, 1.0]);
        assert!(near(down[0], 0.0) && near(down[1], 0.3) && near(down[2], 0.8), "{down:?}");
    }

    #[test]
    fn content_colour_operators_are_filtered_and_the_rest_is_kept() {
        let src = "1.0000 0.0000 0.0000 rg 0 0 10 10 re f\n0.0000 0.0000 1.0000 RG 2 w 0 0 m 5 5 l S\nBT /F0 12 Tf 1 2 Td <4869> Tj ET\n";
        let out = QuartzFilter::black_and_white().filter_content(src);
        assert!(out.contains("0.2990 0.2990 0.2990 rg 0 0 10 10 re f"), "{out}");
        assert!(out.contains("0.1140 0.1140 0.1140 RG 2 w 0 0 m 5 5 l S"), "{out}");
        assert!(out.contains("BT /F0 12 Tf 1 2 Td <4869> Tj ET"), "{out}");
        assert_eq!(QuartzFilter::identity("none").filter_content(src), src);
    }

    #[test]
    fn images_scale_down_to_the_limit_and_keep_their_aspect() {
        let f = QuartzFilter { max_image_side: Some(2), ..QuartzFilter::identity("small") };
        let rgb: Vec<u8> = (0..8 * 4 * 3).map(|i| (i % 251) as u8).collect();
        let (px, w, h) = f.filter_image(&rgb, 8, 4);
        assert_eq!((w, h), (2, 1));
        assert_eq!(px.len(), 6);
        let (same, sw, sh) = f.filter_image(&rgb[..6], 2, 1);
        assert_eq!((sw, sh, same.len()), (2, 1, 6));
    }

    #[test]
    fn scaling_averages_the_pixels_it_covers() {
        let f = QuartzFilter { max_image_side: Some(1), ..QuartzFilter::identity("one pixel") };
        // 2x2: black, white / white, black average to mid grey
        let rgb = vec![0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0];
        let (px, w, h) = f.filter_image(&rgb, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert!((127..=128).contains(&px[0]) && px[0] == px[1] && px[1] == px[2], "{px:?}");
    }

    #[test]
    fn a_wrong_sized_image_is_returned_unchanged() {
        let f = QuartzFilter::black_and_white();
        let (px, w, h) = f.filter_image(&[1, 2, 3, 4], 2, 2);
        assert_eq!((px, w, h), (vec![1, 2, 3, 4], 2, 2));
    }

    #[test]
    fn image_colours_follow_the_matrix() {
        // white keeps its red and green (the rows sum above 1) and dims blue: 0.272 + 0.534 + 0.131 = 0.937
        let (px, _, _) = QuartzFilter::sepia_tone().filter_image(&[255, 255, 255], 1, 1);
        assert_eq!(px, vec![255, 255, 239]);
    }

    #[test]
    fn the_manager_lists_built_ins_and_guards_them() {
        let mut m = QuartzFilterManager::new();
        assert_eq!(m.filters().len(), 7);
        assert!(m.find("Sepia Tone").is_some() && m.find("Nope").is_none());
        assert!(!m.add(QuartzFilter::identity("Sepia Tone")), "built-ins are not replaced");
        assert!(m.add(QuartzFilter { lightness: 0.5, ..QuartzFilter::identity("Mine") }));
        assert!(m.add(QuartzFilter { lightness: 0.6, ..QuartzFilter::identity("Mine") }));
        assert_eq!(m.find("Mine").unwrap().lightness, 0.6);
        assert_eq!(m.filters().len(), 8);
        assert!(!m.remove("Black & White"));
        assert!(m.remove("Mine") && !m.remove("Mine"));
        assert_eq!(m.filters().len(), 7);
    }
}
