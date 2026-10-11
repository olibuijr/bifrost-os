//! The Screen Time bar chart: 24 hourly bars for a day or 7 daily bars for
//! a week, drawn with the same helpers as the battery graph.

use super::super::icons::BLUE;
use super::super::ui::{Ui, Weight, SECONDARY, SEPARATOR, WHITE};
use crate::gfx::{pack, Surface};

const AXIS: u32 = pack(72, 72, 74, 255);

/// "1h 5m", "42m", "<1m" or "0m" for a number of seconds.
pub fn duration(secs: u32) -> String {
    match secs {
        0 => "0m".into(),
        1..=59 => "<1m".into(),
        _ => {
            let (h, m) = (secs / 3600, secs % 3600 / 60);
            if h == 0 {
                format!("{m}m")
            } else if m == 0 {
                format!("{h}h")
            } else {
                format!("{h}h {m}m")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// small caption above the figure, e.g. "TODAY"
    pub caption: String,
    /// the big figure, e.g. "2h 5m"
    pub figure: String,
    /// bar values in seconds, oldest first
    pub bars: Vec<u32>,
    /// a label under bar `i` for every `(i, text)`
    pub labels: Vec<(usize, String)>,
    /// the bar drawn in the accent colour (the current hour or today)
    pub current: Option<usize>,
}

/// A round axis maximum at or above `v`: 1, 2, 5 times a power of ten of
/// the unit (minutes), never under 5 minutes.
pub fn axis_max(v: u32) -> u32 {
    let mins = (v as f32 / 60.0).ceil().max(5.0) as u32;
    let mut unit = 1;
    loop {
        for m in [1, 2, 5] {
            if m * unit >= mins {
                return m * unit * 60;
            }
        }
        unit *= 10;
    }
}

impl Chart {
    /// Draws into `(x, y, w, h)` px: caption and figure on top, bars with a
    /// right-hand axis label, hour or day labels underneath.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) {
        let cap = ui.text(&self.caption, 13.0, Weight::Regular, SECONDARY);
        s.blit(&cap, x as i32, y as i32, 255, 255);
        let fig = ui.text(&self.figure, 28.0, Weight::Semibold, WHITE);
        s.blit(&fig, x as i32, (y + ui.pt(18.0)) as i32, 255, 255);
        let label_w = ui.pt(44.0);
        let gx = x;
        let gy = y + ui.pt(18.0) + fig.h as f32 + ui.pt(8.0);
        let gw = w - label_w;
        let gh = (y + h - ui.pt(24.0) - gy).max(ui.pt(20.0));
        let top = axis_max(self.bars.iter().copied().max().unwrap_or(0));
        for (frac, text) in [(0.0, duration(top)), (0.5, duration(top / 2)), (1.0, "0".to_string())] {
            let ly = gy + gh * frac;
            s.fill_rrect(gx, ly - 0.5, gw, 1.0, 0.0, if frac == 1.0 { SEPARATOR } else { AXIS });
            let img = ui.text(&text, 12.0, Weight::Regular, SECONDARY);
            s.blit(&img, (gx + gw + ui.pt(8.0)) as i32, (ly - img.h as f32 / 2.0) as i32, 255, 255);
        }
        let n = self.bars.len().max(1);
        let bw = gw / n as f32;
        for (i, &v) in self.bars.iter().enumerate() {
            if v == 0 {
                continue;
            }
            let bh = (gh * v.min(top) as f32 / top as f32).max(ui.pt(2.0));
            let color = if self.current == Some(i) { BLUE } else { pack(10, 100, 200, 255) };
            s.fill_rrect(gx + i as f32 * bw + bw * 0.15, gy + gh - bh, bw * 0.7, bh, 0.0, color);
        }
        for (i, text) in &self.labels {
            let img = ui.text(text, 12.0, Weight::Regular, SECONDARY);
            let cx = gx + (*i as f32 + 0.5) * bw;
            s.blit(&img, (cx - img.w as f32 / 2.0) as i32, (gy + gh + ui.pt(6.0)) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_mobile() {
        assert_eq!(duration(0), "0m");
        assert_eq!(duration(30), "<1m");
        assert_eq!(duration(60 * 42), "42m");
        assert_eq!(duration(3600), "1h");
        assert_eq!(duration(3600 + 5 * 60 + 7), "1h 5m");
    }

    #[test]
    fn axis_max_is_a_round_number() {
        assert_eq!(axis_max(0), 300);
        assert_eq!(axis_max(7 * 60), 600);
        assert_eq!(axis_max(11 * 60), 1200);
        assert_eq!(axis_max(3 * 3600), 12_000);
    }

    #[test]
    fn chart_draws_bars_in_the_accent_colour() {
        let ui = crate::shell::tests::ui();
        let mut bars = vec![0; 24];
        bars[9] = 1800;
        bars[10] = 600;
        let c = Chart { caption: "TODAY".into(), figure: "40m".into(), bars, labels: vec![(0, "12 AM".into()), (12, "12 PM".into())], current: Some(10) };
        let mut buf = vec![0u32; 1000 * 400];
        c.draw(&mut Surface { w: 1000, h: 400, stride: 1000, px: &mut buf, y0: 0 }, &ui, (20.0, 10.0, 960.0, 380.0));
        assert!(buf.contains(&BLUE), "current bar");
        assert!(buf.contains(&pack(10, 100, 200, 255)), "other bars");
        let empty = Chart { bars: vec![0; 24], current: None, ..c };
        let mut buf = vec![0u32; 1000 * 400];
        empty.draw(&mut Surface { w: 1000, h: 400, stride: 1000, px: &mut buf, y0: 0 }, &ui, (20.0, 10.0, 960.0, 380.0));
        assert!(!buf.contains(&BLUE));
    }
}
