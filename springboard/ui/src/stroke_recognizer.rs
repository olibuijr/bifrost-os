//! A stroke recogniser for handwriting and gestures, after the $1 unistroke method: a stroke is resampled to a fixed
//! number of points, rotated so its first point lies on the zero angle, scaled to a unit square and moved to the origin,
//! then compared with templates by average point distance with a golden-section search over rotation. Pure std;
//! `Recognizer::recognize` returns the best template name and a score from 0 to 1. Scribble and PencilKit-style
//! handwriting recognition feed it the points of a finished stroke.

use std::f32::consts::PI;

pub const POINTS: usize = 64;
const SQUARE: f32 = 250.0;
const ANGLE_RANGE: f32 = 45.0 * PI / 180.0;
const ANGLE_PRECISION: f32 = 2.0 * PI / 180.0;
const PHI: f32 = 0.618_034;

type Pt = (f32, f32);

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn path_length(p: &[Pt]) -> f32 {
    p.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// Resamples to `n` evenly spaced points along the path.
pub fn resample(points: &[Pt], n: usize) -> Vec<Pt> {
    if points.len() < 2 || n < 2 {
        return points.to_vec();
    }
    let interval = path_length(points) / (n - 1) as f32;
    if interval <= 0.0 {
        return vec![points[0]; n];
    }
    let mut pts = points.to_vec();
    let mut out = vec![pts[0]];
    let mut acc = 0.0;
    let mut i = 1;
    while i < pts.len() {
        let d = dist(pts[i - 1], pts[i]);
        if acc + d >= interval && d > 0.0 {
            let t = (interval - acc) / d;
            let q = (pts[i - 1].0 + t * (pts[i].0 - pts[i - 1].0), pts[i - 1].1 + t * (pts[i].1 - pts[i - 1].1));
            out.push(q);
            pts.insert(i, q);
            acc = 0.0;
        } else {
            acc += d;
        }
        i += 1;
    }
    while out.len() < n {
        out.push(*pts.last().unwrap_or(&points[0]));
    }
    out.truncate(n);
    out
}

fn centroid(p: &[Pt]) -> Pt {
    let n = p.len().max(1) as f32;
    (p.iter().map(|q| q.0).sum::<f32>() / n, p.iter().map(|q| q.1).sum::<f32>() / n)
}

fn rotate_by(p: &[Pt], angle: f32) -> Vec<Pt> {
    let c = centroid(p);
    let (s, co) = angle.sin_cos();
    p.iter().map(|q| ((q.0 - c.0) * co - (q.1 - c.1) * s + c.0, (q.0 - c.0) * s + (q.1 - c.1) * co + c.1)).collect()
}

fn scale_and_translate(p: &[Pt]) -> Vec<Pt> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for q in p {
        x0 = x0.min(q.0);
        y0 = y0.min(q.1);
        x1 = x1.max(q.0);
        y1 = y1.max(q.1);
    }
    let (w, h) = ((x1 - x0).max(1e-3), (y1 - y0).max(1e-3));
    let scaled: Vec<Pt> = p.iter().map(|q| ((q.0 - x0) * SQUARE / w, (q.1 - y0) * SQUARE / h)).collect();
    let c = centroid(&scaled);
    scaled.iter().map(|q| (q.0 - c.0, q.1 - c.1)).collect()
}

/// The normalised form of a stroke, ready to compare.
pub fn normalize(points: &[Pt]) -> Vec<Pt> {
    let r = resample(points, POINTS);
    if r.is_empty() {
        return r;
    }
    let c = centroid(&r);
    let angle = (c.1 - r[0].1).atan2(c.0 - r[0].0);
    scale_and_translate(&rotate_by(&r, -angle))
}

fn path_distance(a: &[Pt], b: &[Pt]) -> f32 {
    a.iter().zip(b).map(|(p, q)| dist(*p, *q)).sum::<f32>() / a.len().max(1) as f32
}

fn distance_at_best_angle(p: &[Pt], t: &[Pt]) -> f32 {
    let (mut a, mut b) = (-ANGLE_RANGE, ANGLE_RANGE);
    let mut x1 = PHI * a + (1.0 - PHI) * b;
    let mut f1 = path_distance(&rotate_by(p, x1), t);
    let mut x2 = (1.0 - PHI) * a + PHI * b;
    let mut f2 = path_distance(&rotate_by(p, x2), t);
    while (b - a).abs() > ANGLE_PRECISION {
        if f1 < f2 {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = PHI * a + (1.0 - PHI) * b;
            f1 = path_distance(&rotate_by(p, x1), t);
        } else {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = (1.0 - PHI) * a + PHI * b;
            f2 = path_distance(&rotate_by(p, x2), t);
        }
    }
    f1.min(f2)
}

#[derive(Clone, Debug, Default)]
pub struct Recognizer {
    templates: Vec<(String, Vec<Pt>)>,
}

impl Recognizer {
    /// Adds a template stroke under `name` (several per name are fine).
    pub fn add(&mut self, name: &str, points: &[Pt]) {
        if points.len() >= 2 {
            self.templates.push((name.to_string(), normalize(points)));
        }
    }

    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// The best match and its score (1.0 is identical, 0.0 is as far as a stroke can be). None without templates or
    /// for a stroke with fewer than two points.
    pub fn recognize(&self, points: &[Pt]) -> Option<(String, f32)> {
        if points.len() < 2 || self.templates.is_empty() {
            return None;
        }
        let p = normalize(points);
        let half_diagonal = 0.5 * (2.0 * SQUARE * SQUARE).sqrt();
        self.templates
            .iter()
            .map(|(n, t)| (n, distance_at_best_angle(&p, t)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(n, d)| (n.clone(), (1.0 - d / half_diagonal).clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize) -> Vec<Pt> {
        (0..n).map(|i| (i as f32 * 3.0, i as f32 * 1.0)).collect()
    }

    fn circle(r: f32, cx: f32, cy: f32) -> Vec<Pt> {
        (0..=40).map(|i| {
            let a = i as f32 / 40.0 * 2.0 * PI;
            (cx + r * a.cos(), cy + r * a.sin())
        }).collect()
    }

    fn vee() -> Vec<Pt> {
        let mut v: Vec<Pt> = (0..20).map(|i| (i as f32 * 5.0, i as f32 * 10.0)).collect();
        v.extend((0..20).map(|i| (100.0 + i as f32 * 5.0, 190.0 - i as f32 * 10.0)));
        v
    }

    #[test]
    fn resample_gives_even_points() {
        let r = resample(&line(10), POINTS);
        assert_eq!(r.len(), POINTS);
        let gaps: Vec<f32> = r.windows(2).map(|w| dist(w[0], w[1])).collect();
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        assert!(gaps.iter().all(|g| (g - mean).abs() < mean * 0.05), "{gaps:?}");
        assert_eq!(resample(&[(1.0, 1.0)], 8), vec![(1.0, 1.0)]);
        assert_eq!(resample(&[(1.0, 1.0), (1.0, 1.0)], 4).len(), 4);
    }

    #[test]
    fn same_shape_at_other_scale_position_and_rotation_matches() {
        let mut r = Recognizer::default();
        r.add("circle", &circle(50.0, 0.0, 0.0));
        r.add("vee", &vee());
        r.add("line", &line(30));
        assert_eq!(r.len(), 3);
        let (name, score) = r.recognize(&circle(20.0, 300.0, -40.0)).unwrap();
        assert_eq!(name, "circle");
        assert!(score > 0.85, "{score}");
        let scaled_vee: Vec<Pt> = vee().iter().map(|p| (p.0 * 0.3 + 40.0, p.1 * 0.3 + 10.0)).collect();
        assert_eq!(r.recognize(&scaled_vee).unwrap().0, "vee");
        assert_eq!(r.recognize(&line(12)).unwrap().0, "line");
    }

    #[test]
    fn empty_cases() {
        let r = Recognizer::default();
        assert_eq!(r.recognize(&line(5)), None);
        let mut r = Recognizer::default();
        r.add("x", &[(0.0, 0.0)]);
        assert_eq!(r.len(), 0);
        r.add("line", &line(5));
        assert_eq!(r.recognize(&[(0.0, 0.0)]), None);
    }
}
