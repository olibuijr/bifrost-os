//! the terminal core's compact tab strip and centered panel geometry in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeaderHit {
    Tab(usize),
    Close(usize),
    New,
    Connections,
    Processes,
    Menu,
}
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub tab_w: f32,
    pub plus: f32,
    pub ssh: f32,
    pub sys: f32,
    pub app: f32,
    pub more: f32,
    pub scale: f32,
    count: usize,
}
impl Header {
    pub fn new(width: f32, scale: f32, count: usize) -> Self {
        let reserve = 276.0 * scale;
        let available = (width - reserve - 96.0 * scale).max(0.0);
        let tab_w = if count == 0 {
            0.0
        } else {
            (available / count as f32).min(180.0 * scale)
        };
        let plus = tab_w * count as f32;
        Self {
            tab_w,
            plus,
            ssh: plus + 44.0 * scale,
            sys: width - reserve,
            app: width - 156.0 * scale,
            more: width - 44.0 * scale,
            scale,
            count,
        }
    }
    pub fn close_width(&self) -> f32 {
        (32.0 * self.scale).min(self.tab_w / 2.0)
    }
    pub fn hit(&self, x: f32, width: f32) -> Option<HeaderHit> {
        if x < 0.0 || x >= width {
            return None;
        }
        if x >= self.more {
            return Some(HeaderHit::Menu);
        }
        if x >= self.sys && x < self.app {
            return Some(HeaderHit::Processes);
        }
        if x >= self.plus && x < self.ssh {
            return Some(HeaderHit::New);
        }
        if x >= self.ssh && x < self.ssh + 52.0 * self.scale {
            return Some(HeaderHit::Connections);
        }
        if self.tab_w > 0.0 && x < self.plus {
            let i = (x / self.tab_w) as usize;
            if i < self.count {
                return Some(if x >= (i + 1) as f32 * self.tab_w - self.close_width() {
                    HeaderHit::Close(i)
                } else {
                    HeaderHit::Tab(i)
                });
            }
        }
        None
    }
}
/// A panel leaves terminal content visible around every edge. Form height is
/// bounded by the keyboard, so its footer and all fields remain reachable.
pub fn panel(
    width: f32,
    height: f32,
    scale: f32,
    bottom: f32,
    form: bool,
    menu: bool,
) -> (f32, f32, f32, f32) {
    let w = if menu {
        (480.0 * scale).min(width * 0.9)
    } else {
        width * 0.9
    };
    let top = (height * 0.05).max(76.0 * scale);
    let available = (bottom - top - 16.0 * scale).max(80.0 * scale);
    let h = if form {
        available.min(420.0 * scale)
    } else if menu {
        available.min(700.0 * scale)
    } else {
        available
    };
    ((width - w) / 2.0, top, w, h)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_tabs_and_controls_have_distinct_targets() {
        for count in [1, 2, 5, 12] {
            let g = Header::new(1200.0, 1200.0 / 768.0, count);
            for i in 0..count {
                assert_eq!(
                    g.hit(i as f32 * g.tab_w + g.tab_w * 0.2, 1200.0),
                    Some(HeaderHit::Tab(i))
                );
                assert_eq!(
                    g.hit((i + 1) as f32 * g.tab_w - g.close_width() / 2.0, 1200.0),
                    Some(HeaderHit::Close(i))
                );
            }
            assert_eq!(g.hit(g.plus + 10.0, 1200.0), Some(HeaderHit::New));
            assert_eq!(g.hit(g.ssh + 10.0, 1200.0), Some(HeaderHit::Connections));
            assert_eq!(g.hit(g.sys + 10.0, 1200.0), Some(HeaderHit::Processes));
            assert_eq!(g.hit(g.more + 10.0, 1200.0), Some(HeaderHit::Menu));
            assert!(g.ssh + 52.0 * g.scale <= g.sys + 0.1);
        }
    }
    #[test]
    fn meter_badges_have_room_for_tag_and_value() {
        let g = Header::new(1200.0, 1.0, 3);
        // SYS badge 114 pt and APP badge 106 pt must fit inside their slots.
        assert!(g.sys + 114.0 * g.scale <= g.app);
        assert!(g.app + 106.0 * g.scale <= g.more);
    }
    #[test]
    fn panels_keep_margins_and_clear_the_keyboard() {
        let (x, y, w, h) = panel(1200.0, 1920.0, 1.5625, 1360.0, true, false);
        assert!(x > 0.0 && x + w < 1200.0 && y > 60.0 && y + h < 1360.0);
        assert!(h >= 350.0 * 1.5625);
    }
}
