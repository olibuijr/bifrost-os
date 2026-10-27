//! Screenshot as an installed BifrOSt app: pick what to capture (the screen, a window or a selection), an optional
//! timer, and press Capture. The picture is taken by the shell's screenshot service and saved to Photos, as the
//! screenshot toolbar does; the status line names the file once it is there. Hosted through the SDK in a full-size window.

use bifrost_app::controls::sheet;
use bifrost_app::controls::{Node, Rect as KRect, Role};
use bifrost_app::kit::ui::{
    CAPSULE_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE3_PT, SPACE_L,
    SPACE_M, SPACE_XL, SPACE_XS, WHITE,
};
use bifrost_app::kit::{Damage, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{screenshot, widgets};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use bifrost_screenshot::session::{TARGETS, TIMERS};
use bifrost_screenshot::{Session, Step, Target, Timer};

/// The widest the option column grows, in points; a wider window centres it.
const COLUMN_MAX_PT: f32 = 420.0;
/// Height of a section label row and of the Capture button, in points.
const LABEL_H_PT: f32 = 16.0;
const BUTTON_H_PT: f32 = 28.0;
const BUTTON_W_PT: f32 = 180.0;
/// The status line's row.
const STATUS_H_PT: f32 = 16.0;
/// How often the countdown wakes the app, in milliseconds.
const WAKE_MS: f64 = 250.0;

/// Accessibility ids: the Capture segments, the Timer segments and the button.
const NODE_TARGET: u64 = 10;
const NODE_TIMER: u64 = 20;
const NODE_BUTTON: u64 = 30;
/// Menu item ids.
const MENU_SCREEN: u32 = 0x6001;
const MENU_WINDOW: u32 = 0x6002;
const MENU_SELECTION: u32 = 0x6003;

fn target_label(target: Target) -> &'static str {
    match target {
        Target::Screen => "Screen",
        Target::Window => "Window",
        Target::Selection => "Selection",
    }
}

fn timer_label(timer: Timer) -> &'static str {
    match timer {
        Timer::Off => "Off",
        Timer::Seconds5 => "5 seconds",
        Timer::Seconds10 => "10 seconds",
    }
}

fn pick_of(target: Target) -> screenshot::Pick {
    match target {
        Target::Screen => screenshot::Pick::Screen,
        Target::Window => screenshot::Pick::Window,
        Target::Selection => screenshot::Pick::Selection,
    }
}

/// What a press started on, so a release on the same control acts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Target(usize),
    Timer(usize),
    Button,
}

/// The window's controls, in points from the window's top-left.
struct Layout {
    column_x: f32,
    column_w: f32,
    capture_label_y: f32,
    capture: KRect,
    timer_label_y: f32,
    timer: KRect,
    button: KRect,
    status_y: f32,
}

/// The column derives from the window size: centred across, and centred down the area under the toolbar.
fn layout(w_px: f32, h_px: f32, ui: &Ui) -> Layout {
    let p = ui.pt(1.0);
    let (w, h) = (w_px / p, h_px / p);
    let column_w = (w - 2.0 * SPACE_XL).clamp(0.0, COLUMN_MAX_PT);
    let column_x = (w - column_w) / 2.0;
    let seg_h = sheet::SEGMENT_H_PT;
    let content_h = 2.0 * (LABEL_H_PT + SPACE_XS + seg_h) + SPACE_L + SPACE_XL + BUTTON_H_PT + SPACE_M + STATUS_H_PT;
    let avail = (h - UNIFIED_BAR_PT).max(0.0);
    let mut y = UNIFIED_BAR_PT + ((avail - content_h) / 2.0).max(SPACE_XL.min(avail));
    let capture_label_y = y + LABEL_H_PT / 2.0;
    y += LABEL_H_PT + SPACE_XS;
    let capture = KRect { x: column_x, y, w: column_w, h: seg_h };
    y += seg_h + SPACE_L;
    let timer_label_y = y + LABEL_H_PT / 2.0;
    y += LABEL_H_PT + SPACE_XS;
    let timer = KRect { x: column_x, y, w: column_w, h: seg_h };
    y += seg_h + SPACE_XL;
    let button_w = column_w.min(BUTTON_W_PT);
    let button = KRect { x: column_x + (column_w - button_w) / 2.0, y, w: button_w, h: BUTTON_H_PT };
    let status_y = y + BUTTON_H_PT + SPACE_M + STATUS_H_PT / 2.0;
    Layout { column_x, column_w, capture_label_y, capture, timer_label_y, timer, button, status_y }
}

struct ScreenshotApp {
    session: Session,
    /// The screenshot request the shell has not answered yet.
    request: Option<u32>,
    press: Option<Hit>,
    labels: TextCache,
    /// The window size in pixels.
    size: (f32, f32),
}

impl ScreenshotApp {
    fn layout(&self, ui: &Ui) -> Layout {
        layout(self.size.0, self.size.1, ui)
    }

    fn hit_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        let p = ui.pt(1.0);
        let (x, y) = (x / p, y / p);
        let lay = self.layout(ui);
        if lay.button.contains(x, y) {
            return Some(Hit::Button);
        }
        if let Some(i) = sheet::segment_hit(lay.capture, TARGETS.len(), x, y) {
            return Some(Hit::Target(i));
        }
        sheet::segment_hit(lay.timer, TIMERS.len(), x, y).map(Hit::Timer)
    }

    /// Runs the Capture button: starts the countdown, cancels it, or asks the shell for the picture.
    fn capture(&mut self, ctx: &Ctx) -> bool {
        match self.session.press() {
            Step::Take => {
                self.take(ctx);
                true
            }
            Step::Redraw => true,
            Step::Idle => false,
        }
    }

    fn take(&mut self, ctx: &Ctx) {
        let pick = pick_of(self.session.options.target);
        self.request = Some(screenshot::take(ctx, pick));
    }

    fn activate(&mut self, ctx: &Ctx, hit: Hit) -> bool {
        match hit {
            Hit::Button => self.capture(ctx),
            // the options are fixed while the timer runs
            Hit::Target(_) | Hit::Timer(_) if self.session.counting() => false,
            Hit::Target(i) => {
                self.session.options.set_target(TARGETS[i]);
                true
            }
            Hit::Timer(i) => {
                self.session.options.set_timer(TIMERS[i]);
                true
            }
        }
    }

    /// Draws `text` with its left edge at `x` and its centre line at `mid`, in pixels.
    fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32) {
        let img = self.labels.get(ui, text, pt, weight, color);
        s.blit(img, x.round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }

    fn text_centred(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, cx: f32, mid: f32) {
        let img = self.labels.get(ui, text, pt, weight, color);
        s.blit(img, (cx - img.w as f32 / 2.0).round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }
}

impl App for ScreenshotApp {
    /// A single-window utility: the red light quits it (`applicationShouldTerminateAfterLastWindowClosed:`).
    fn terminates_after_last_window_closed(&self) -> bool {
        true
    }
    fn id(&self) -> &str {
        "is.olibuijr.screenshot.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => match kind {
                TouchKind::Down => {
                    self.press = self.hit_at(&ctx.ui, x, y);
                    true
                }
                TouchKind::Move => false,
                TouchKind::Up | TouchKind::Cancel => {
                    let Some(hit) = self.press.take() else { return false };
                    let released_on = matches!(kind, TouchKind::Up) && self.hit_at(&ctx.ui, x, y) == Some(hit);
                    if released_on {
                        self.activate(ctx, hit);
                    }
                    // the pressed look ends either way
                    true
                }
            },
            Input::Key(KeyInput::Code(13)) => self.capture(ctx),
            // Escape cancels a running timer
            Input::Key(KeyInput::Code(1 | 27)) if self.session.counting() => self.capture(ctx),
            _ => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        // the clock is in milliseconds
        match self.session.tick(t / 1000.0) {
            Step::Take => {
                self.take(ctx);
                true
            }
            Step::Redraw => true,
            Step::Idle => false,
        }
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.session.counting().then_some(t + WAKE_MS)
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.request != Some(request) {
            return false;
        }
        self.request = None;
        self.session.started(screenshot::started_from_reply(result));
        true
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        let Some(path) = screenshot::captured_path(service, event, body) else { return false };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        self.session.captured(&name);
        true
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let free = !self.session.busy();
        vec![AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(MENU_SCREEN, "Capture Entire Screen").enabled(free),
                AppMenuItem::new(MENU_WINDOW, "Capture Window\u{2026}").enabled(free),
                AppMenuItem::new(MENU_SELECTION, "Capture Selection\u{2026}").enabled(free),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let target = match id {
            MENU_SCREEN => Target::Screen,
            MENU_WINDOW => Target::Window,
            MENU_SELECTION => Target::Selection,
            _ => return false,
        };
        // a menu command takes the picture at once; a running timer is left to finish
        if self.session.counting() || self.session.busy() {
            return false;
        }
        self.session.options.set_target(target);
        self.capture(ctx);
        true
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let lay = self.layout(&ctx.ui);
        let enabled = !self.session.counting();
        let mut nodes = Vec::new();
        let options = &self.session.options;
        for (i, rect) in sheet::segments(lay.capture, TARGETS.len()).into_iter().enumerate() {
            nodes.push(Node {
                id: NODE_TARGET + i as u64,
                role: Role::Button,
                label: target_label(TARGETS[i]).into(),
                value: if options.target == TARGETS[i] { "selected".into() } else { String::new() },
                bounds: rect,
                enabled,
            });
        }
        for (i, rect) in sheet::segments(lay.timer, TIMERS.len()).into_iter().enumerate() {
            nodes.push(Node {
                id: NODE_TIMER + i as u64,
                role: Role::Button,
                label: format!("Timer {}", timer_label(TIMERS[i])),
                value: if options.timer == TIMERS[i] { "selected".into() } else { String::new() },
                bounds: rect,
                enabled,
            });
        }
        nodes.push(Node {
            id: NODE_BUTTON,
            role: Role::Button,
            label: if self.session.counting() { "Cancel".into() } else { "Capture".into() },
            value: String::new(),
            bounds: lay.button,
            enabled: !self.session.busy(),
        });
        nodes
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        self.size = (s.w as f32, s.h as f32);
        let p = ui.pt(1.0);
        let lay = self.layout(ui);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());

        // toolbar band with the window title
        let width = s.w as f32 / p;
        widgets::toolbar_band(s, ui, KRect { x: 0.0, y: 0.0, w: width, h: UNIFIED_BAR_PT });
        self.text_centred(s, ui, "Screenshot", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), s.w as f32 / 2.0, ui.pt(UNIFIED_BAR_PT / 2.0));

        let counting = self.session.counting();
        let left = ui.pt(lay.column_x);
        self.text(s, ui, "Capture", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), left, ui.pt(lay.capture_label_y));
        let labels: Vec<&str> = TARGETS.iter().map(|t| target_label(*t)).collect();
        let selected = TARGETS.iter().position(|t| *t == self.session.options.target).unwrap_or(0);
        widgets::segmented(s, ui, lay.capture, &labels, selected, !counting);

        self.text(s, ui, "Timer", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), left, ui.pt(lay.timer_label_y));
        let labels: Vec<&str> = TIMERS.iter().map(|t| timer_label(*t)).collect();
        let selected = TIMERS.iter().position(|t| *t == self.session.options.timer).unwrap_or(0);
        widgets::segmented(s, ui, lay.timer, &labels, selected, !counting);

        // the Capture button: filled with the accent; Cancel while the timer runs; dim while the shell has not answered
        let busy = self.session.busy();
        let pressed = self.press == Some(Hit::Button);
        let b = lay.button;
        let (x, y, w, h) = (ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h));
        let (fill, ink) = if busy {
            (CAPSULE_BG.now(), LABEL_3.now())
        } else if counting {
            (CAPSULE_BG.now(), LABEL.now())
        } else if pressed {
            (LABEL_2.now(), WHITE)
        } else {
            (LINK, WHITE)
        };
        s.fill_rrect(x, y, w, h, h / 2.0, fill);
        let caption = if counting { "Cancel" } else { "Capture" };
        self.text_centred(s, ui, caption, MAC_BODY_PT, Weight::Bold, ink, x + w / 2.0, y + h / 2.0);

        // the status line, kept inside the column
        let status = ui.fit(self.session.status(), MAC_FOOTNOTE_PT, ui.pt(lay.column_w));
        self.text_centred(s, ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), s.w as f32 / 2.0, ui.pt(lay.status_y));

        let mut damage = Damage::new(s.w, s.h);
        damage.all();
        damage
    }
}

fn main() {
    bifrost_app::run(ScreenshotApp { session: Session::new(), request: None, press: None, labels: TextCache::default(), size: (0.0, 0.0) });
}
