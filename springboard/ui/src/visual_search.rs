//! Visual search model: a captured scene (a screenshot, a photo, or a frame of the camera stream), the object
//! selection on a scene, the scene labels an app can match on, and the merge of the matches the apps return.
//!
//! Pure model. The shell captures the pixels and asks the apps through `bifrost_app::visual_search`; the shell
//! draws the merged matches. Pixels are 8-bit RGBA, row-major, without row padding. See docs/parity/visual-intelligence.md.

/// Largest side of a scene in pixels. Bigger scenes are refused before any pixel is copied.
pub const MAX_SIDE: u32 = 8192;
/// How many matches the shell shows for one search.
pub const MAX_MATCHES: usize = 20;

/// Where the scene came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    /// A screenshot, or an object selected in one.
    Screenshot,
    /// A still photo from the camera.
    Photo,
    /// One frame of the camera stream.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// A side is zero or above `MAX_SIDE`, or the selection holds no pixel.
    Size,
    /// The pixel buffer does not hold exactly `width * height * 4` bytes.
    PixelCount { expected: usize, got: usize },
}

/// A rectangle in scene pixels, origin at the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What an app receives for a search: the scene's pixels and the labels that classify it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub kind: SceneKind,
    pub width: u32,
    pub height: u32,
    /// RGBA8, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// Classification labels, lowercase, trimmed, without duplicates. Empty when the shell has no classifier.
    pub labels: Vec<String>,
}

impl Scene {
    /// A scene of `width` x `height` RGBA pixels. Refuses a zero or oversized side and a pixel buffer of the wrong length.
    pub fn new(kind: SceneKind, width: u32, height: u32, pixels: Vec<u8>) -> Result<Scene, SceneError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(SceneError::Size);
        }
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return Err(SceneError::PixelCount { expected, got: pixels.len() });
        }
        Ok(Scene { kind, width, height, pixels, labels: Vec::new() })
    }

    /// Sets the labels. Each label is trimmed and lowercased; empty and repeated labels are dropped, order is kept.
    pub fn with_labels(mut self, labels: &[&str]) -> Scene {
        let mut out: Vec<String> = Vec::new();
        for l in labels {
            let t = l.trim().to_lowercase();
            if !t.is_empty() && !out.contains(&t) {
                out.push(t);
            }
        }
        self.labels = out;
        self
    }

    /// The RGBA bytes of the pixel at (`x`, `y`), or `None` outside the scene.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]])
    }

    /// The object selection: a new scene with only the pixels of `rect`, clamped to this scene. Kind and labels are kept.
    /// `Err(Size)` when the clamped rectangle is empty, for example a selection that lies outside the scene.
    pub fn crop(&self, rect: PixelRect) -> Result<Scene, SceneError> {
        let x0 = rect.x.min(self.width);
        let y0 = rect.y.min(self.height);
        let x1 = rect.x.saturating_add(rect.w).min(self.width);
        let y1 = rect.y.saturating_add(rect.h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return Err(SceneError::Size);
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
        let row = self.width as usize * 4;
        for y in y0..y1 {
            let start = y as usize * row + x0 as usize * 4;
            pixels.extend_from_slice(&self.pixels[start..start + w as usize * 4]);
        }
        Ok(Scene { kind: self.kind, width: w, height: h, pixels, labels: self.labels.clone() })
    }
}

/// An app's answer for one scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The app that answered. The shell sets it from the provider, so an app cannot answer for another app.
    pub app: String,
    /// The intent the shell runs when the match is opened.
    pub intent: String,
    pub title: String,
    pub subtitle: String,
    /// 0 to 100. Higher is a better match.
    pub score: u32,
}

/// Score in 0 to 100 for an app's keywords against the scene labels: the share of keywords that equal a label.
/// Zero when there are no keywords.
pub fn label_score(scene: &Scene, keywords: &[&str]) -> u32 {
    let wanted: Vec<String> = keywords.iter().map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect();
    if wanted.is_empty() {
        return 0;
    }
    let hits = wanted.iter().filter(|k| scene.labels.contains(k)).count();
    (hits * 100 / wanted.len()) as u32
}

/// Merges the matches of several apps: drops matches without a title, keeps the best copy of a repeated
/// (app, intent, title), sorts by score (then app and title) and keeps at most `limit`.
pub fn merge(matches: Vec<Match>, limit: usize) -> Vec<Match> {
    let mut kept: Vec<Match> = Vec::new();
    for m in matches {
        if m.title.trim().is_empty() {
            continue;
        }
        match kept.iter_mut().find(|k| k.app == m.app && k.intent == m.intent && k.title == m.title) {
            Some(k) => {
                if m.score > k.score {
                    *k = m;
                }
            }
            None => kept.push(m),
        }
    }
    kept.sort_by(|a, b| b.score.cmp(&a.score).then(a.app.cmp(&b.app)).then(a.title.cmp(&b.title)));
    kept.truncate(limit);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(w: u32, h: u32) -> Scene {
        // pixel (x, y) has red = x, green = y, blue = 0, alpha = 255, so a crop can be checked by value
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        Scene::new(SceneKind::Screenshot, w, h, px).unwrap()
    }

    fn m(app: &str, intent: &str, title: &str, score: u32) -> Match {
        Match { app: app.into(), intent: intent.into(), title: title.into(), subtitle: String::new(), score }
    }

    #[test]
    fn new_checks_size_and_pixel_count() {
        assert_eq!(Scene::new(SceneKind::Photo, 0, 4, vec![]), Err(SceneError::Size));
        assert_eq!(Scene::new(SceneKind::Photo, MAX_SIDE + 1, 1, vec![]), Err(SceneError::Size));
        assert_eq!(
            Scene::new(SceneKind::Photo, 2, 2, vec![0; 15]),
            Err(SceneError::PixelCount { expected: 16, got: 15 })
        );
        assert!(Scene::new(SceneKind::Stream, 2, 2, vec![0; 16]).is_ok());
    }

    #[test]
    fn labels_are_normalised_and_deduplicated() {
        let s = scene(1, 1).with_labels(&["  Dog ", "dog", "", "Beach"]);
        assert_eq!(s.labels, vec!["dog".to_string(), "beach".to_string()]);
    }

    #[test]
    fn crop_keeps_the_selected_pixels_in_order() {
        let s = scene(4, 3);
        let c = s.crop(PixelRect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.pixel(0, 0), Some([1, 1, 0, 255]));
        assert_eq!(c.pixel(1, 1), Some([2, 2, 0, 255]));
        assert_eq!(c.pixel(2, 0), None);
    }

    #[test]
    fn crop_clamps_to_the_scene_and_refuses_an_empty_selection() {
        let s = scene(4, 3);
        let c = s.crop(PixelRect { x: 3, y: 2, w: 10, h: 10 }).unwrap();
        assert_eq!((c.width, c.height), (1, 1));
        assert_eq!(c.pixel(0, 0), Some([3, 2, 0, 255]));
        assert_eq!(s.crop(PixelRect { x: 4, y: 0, w: 2, h: 2 }), Err(SceneError::Size));
        assert_eq!(s.crop(PixelRect { x: 0, y: 0, w: 0, h: 2 }), Err(SceneError::Size));
    }

    #[test]
    fn label_score_is_the_share_of_keywords_found() {
        let s = scene(1, 1).with_labels(&["dog", "beach"]);
        assert_eq!(label_score(&s, &["Dog", "beach", "cat", "car"]), 50);
        assert_eq!(label_score(&s, &["dog"]), 100);
        assert_eq!(label_score(&s, &[]), 0);
        assert_eq!(label_score(&scene(1, 1), &["dog"]), 0);
    }

    #[test]
    fn merge_sorts_by_score_and_limits() {
        let out = merge(vec![m("b", "x", "Low", 10), m("a", "x", "High", 90), m("c", "x", "Mid", 50)], 2);
        assert_eq!(out.iter().map(|k| k.title.as_str()).collect::<Vec<_>>(), vec!["High", "Mid"]);
    }

    #[test]
    fn merge_drops_untitled_and_keeps_the_best_duplicate() {
        let out = merge(
            vec![m("a", "open", "Tent", 40), m("a", "open", "Tent", 70), m("a", "open", "  ", 99)],
            MAX_MATCHES,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].score, 70);
    }

    #[test]
    fn merge_breaks_ties_by_app_then_title() {
        let out = merge(vec![m("z", "i", "A", 50), m("a", "i", "B", 50), m("a", "i", "A", 50)], MAX_MATCHES);
        let order: Vec<(String, String)> = out.iter().map(|k| (k.app.clone(), k.title.clone())).collect();
        assert_eq!(
            order,
            vec![("a".into(), "A".into()), ("a".into(), "B".into()), ("z".into(), "A".into())]
        );
    }
}
