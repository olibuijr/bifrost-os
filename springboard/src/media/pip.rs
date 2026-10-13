//! Picture in picture geometry: how big a shrunken movie picture is, which corner it rests in, and where a dragged
//! picture snaps when the finger lifts. Pure arithmetic on rectangles in one coordinate space (points or pixels, the
//! caller's choice). Showing the picture above other windows is the shell's job; this only says where it goes.

/// `x, y, width, height` of a rectangle.
pub type Rect = (f32, f32, f32, f32);

/// The picture's width as a share of the viewport width when it is shrunk.
pub const WIDTH_FRACTION: f32 = 0.3;

/// The aspect ratio used when the movie's size is not known (16:9).
const FALLBACK_ASPECT: f32 = 9.0 / 16.0;

/// One of the four rests a shrunken picture snaps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The size of the shrunken picture for a movie of `movie` pixels in a viewport of `viewport`. The width is the
/// viewport's `WIDTH_FRACTION`, raised to `min_width` and capped at the viewport width; the height follows the movie's
/// aspect, and a picture too tall for the viewport is narrowed to fit.
pub fn size(movie: (u32, u32), viewport: (f32, f32), min_width: f32) -> (f32, f32) {
    let (vw, vh) = viewport;
    let aspect = if movie.0 == 0 || movie.1 == 0 { FALLBACK_ASPECT } else { movie.1 as f32 / movie.0 as f32 };
    let mut width = (vw * WIDTH_FRACTION).max(min_width).min(vw).max(0.0);
    if width * aspect > vh {
        width = (vh / aspect).max(0.0);
    }
    (width, width * aspect)
}

/// The rectangle of a `(width, height)` picture resting in `corner`, `margin` in from the viewport edges.
pub fn place(corner: Corner, (w, h): (f32, f32), viewport: (f32, f32), margin: f32) -> Rect {
    let (vw, vh) = viewport;
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => margin,
        Corner::TopRight | Corner::BottomRight => vw - margin - w,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => margin,
        Corner::BottomLeft | Corner::BottomRight => vh - margin - h,
    };
    (clamp_axis(x, w, vw), clamp_axis(y, h, vh), w, h)
}

/// The corner a picture snaps to when released with its centre at `centre`: the quadrant the centre is in.
pub fn nearest_corner(centre: (f32, f32), viewport: (f32, f32)) -> Corner {
    let left = centre.0 < viewport.0 / 2.0;
    let top = centre.1 < viewport.1 / 2.0;
    match (left, top) {
        (true, true) => Corner::TopLeft,
        (false, true) => Corner::TopRight,
        (true, false) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    }
}

/// The rectangle while it is dragged: `start` moved by `(dx, dy)`, kept inside the viewport.
pub fn drag(start: Rect, (dx, dy): (f32, f32), viewport: (f32, f32)) -> Rect {
    let (x, y, w, h) = start;
    (clamp_axis(x + dx, w, viewport.0), clamp_axis(y + dy, h, viewport.1), w, h)
}

/// The centre of a rectangle.
pub fn centre(r: Rect) -> (f32, f32) {
    (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
}

/// Keeps a span of `size` starting at `pos` inside `[0, total]`. A span larger than the viewport starts at 0.
fn clamp_axis(pos: f32, size: f32, total: f32) -> f32 {
    pos.min(total - size).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_keeps_the_movie_aspect_and_the_width_floor() {
        let (w, h) = size((1920, 1080), (1000.0, 700.0), 200.0);
        assert!((w - 300.0).abs() < 1e-3);
        assert!((h - 168.75).abs() < 1e-3);
        // a narrow viewport still gets the minimum width
        assert!((size((1920, 1080), (400.0, 700.0), 200.0).0 - 200.0).abs() < 1e-3);
    }

    #[test]
    fn a_tall_movie_is_narrowed_to_fit_the_height() {
        let (w, h) = size((1080, 1920), (1000.0, 400.0), 100.0);
        assert!((h - 400.0).abs() < 1e-3);
        assert!((w - 225.0).abs() < 1e-3);
    }

    #[test]
    fn unknown_movie_size_uses_sixteen_by_nine() {
        let (w, h) = size((0, 0), (1000.0, 700.0), 100.0);
        assert!((h / w - 9.0 / 16.0).abs() < 1e-5);
    }

    #[test]
    fn corners_sit_inside_the_margin() {
        let v = (1000.0, 600.0);
        assert_eq!(place(Corner::TopLeft, (300.0, 150.0), v, 20.0), (20.0, 20.0, 300.0, 150.0));
        assert_eq!(place(Corner::BottomRight, (300.0, 150.0), v, 20.0), (680.0, 430.0, 300.0, 150.0));
        assert_eq!(place(Corner::TopRight, (300.0, 150.0), v, 20.0), (680.0, 20.0, 300.0, 150.0));
        assert_eq!(place(Corner::BottomLeft, (300.0, 150.0), v, 20.0), (20.0, 430.0, 300.0, 150.0));
    }

    #[test]
    fn release_snaps_to_the_nearest_quadrant() {
        let v = (1000.0, 600.0);
        assert_eq!(nearest_corner((100.0, 100.0), v), Corner::TopLeft);
        assert_eq!(nearest_corner((900.0, 100.0), v), Corner::TopRight);
        assert_eq!(nearest_corner((100.0, 500.0), v), Corner::BottomLeft);
        assert_eq!(nearest_corner((600.0, 400.0), v), Corner::BottomRight);
    }

    #[test]
    fn a_drag_stays_inside_the_viewport() {
        let v = (1000.0, 600.0);
        let start = (20.0, 20.0, 300.0, 150.0);
        assert_eq!(drag(start, (-500.0, -500.0), v), (0.0, 0.0, 300.0, 150.0));
        assert_eq!(drag(start, (2000.0, 2000.0), v), (700.0, 450.0, 300.0, 150.0));
        assert_eq!(drag(start, (30.0, 10.0), v), (50.0, 30.0, 300.0, 150.0));
    }

    #[test]
    fn centre_is_the_middle_of_the_rectangle() {
        assert_eq!(centre((10.0, 20.0, 100.0, 50.0)), (60.0, 45.0));
    }
}
