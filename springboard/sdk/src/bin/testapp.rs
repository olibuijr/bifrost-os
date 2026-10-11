//! Test helper for the host tests: paints a blue frame and a white box per touch.
use bifrost_app::kit::{pack, Damage, Surface};
use bifrost_app::{App, Ctx, Input, TouchKind};

#[derive(Default)]
struct TestApp {
    boxes: Vec<(i32, i32)>,
    drawn: usize,
    started: bool,
}

impl App for TestApp {
    fn id(&self) -> &str {
        "is.olibuijr.testapp"
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Down, x, .. } if x < 0.0 => std::process::exit(3),
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                self.boxes.push((x as i32, y as i32));
                ctx.haptic();
                true
            }
            _ => false,
        }
    }

    fn suspend(&mut self, ctx: &Ctx) {
        let _ = std::fs::write(ctx.data_dir.join("suspended"), self.boxes.len().to_string());
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let mut d = Damage::new(s.w, s.h);
        if !self.started {
            self.started = true;
            for p in s.px.iter_mut() {
                *p = pack(0, 0, 255, 255);
            }
            d.all();
        }
        for &(x, y) in &self.boxes[self.drawn..] {
            for yy in y.max(0)..(y + 4).min(s.h as i32) {
                for xx in x.max(0)..(x + 4).min(s.w as i32) {
                    s.px[yy as usize * s.stride + xx as usize] = pack(255, 255, 255, 255);
                }
            }
            d.add(x, y, 4, 4);
        }
        self.drawn = self.boxes.len();
        let _ = ctx;
        d
    }
}

fn main() {
    bifrost_app::run(TestApp::default());
}
