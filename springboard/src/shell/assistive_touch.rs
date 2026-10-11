//! AssistiveTouch: a movable floating Home button and six system shortcuts.
//! The shell executes the returned actions and gates this overlay while locked.

use super::glyphs;
use super::ui::{Ui, Weight, WHITE};
use super::Event;
use crate::gfx::{rgba, Surface};

const BUTTON_PT: f32 = 56.0;
const DRAG_PT: f32 = 8.0;
const IDLE_MS: f64 = 3000.0;
type BoxPx = (f32, f32, f32, f32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Home,
    AppSwitcher,
    ControlCenter,
    NotificationCenter,
    Lock,
    Screenshot,
    /// Opens the Device page; the menu stays open.
    Device,
    /// Device page: toggles Rotation Lock.
    Rotate,
    VolumeUp,
    VolumeDown,
    /// Back to the top-level menu from the Device page.
    Back,
}

impl Action {
    /// Top-level page; App Switcher is the single Custom slot.
    const ALL: [Self; 6] = [
        Self::NotificationCenter,
        Self::ControlCenter,
        Self::Home,
        Self::Device,
        Self::Screenshot,
        Self::AppSwitcher,
    ];
    const DEVICE: [Self; 6] = [
        Self::Lock,
        Self::Rotate,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::Screenshot,
        Self::Back,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::AppSwitcher => "Custom",
            Self::Device => "Device",
            Self::Rotate => "Rotate Screen",
            Self::VolumeUp => "Volume Up",
            Self::VolumeDown => "Volume Down",
            Self::Back => "Back",
            Self::ControlCenter => "Control Center",
            Self::NotificationCenter => "Notification Center",
            Self::Lock => "Lock Screen",
            Self::Screenshot => "Screenshot",
        }
    }
}

/// `consumed` owns the entire finger sequence, including outside dismissal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub consumed: bool,
    pub changed: bool,
    pub action: Option<Action>,
}

#[derive(Clone, Copy, Debug)]
enum Touch {
    /// Keep consuming a finger whose coordinate space changed on resize.
    Cancelled,
    Button {
        down: (f32, f32),
        origin: (f32, f32),
        dragged: bool,
    },
    Menu {
        action: Option<Action>,
        down: (f32, f32),
        moved: bool,
    },
}

pub struct AssistiveTouch {
    enabled: bool,
    open: bool,
    device_page: bool,
    touch: Option<Touch>,
    position: (f32, f32),
    viewport: (i32, i32, f32),
    last_active: Option<f64>,
    dimmed: bool,
}

fn inside((x, y, w, h): BoxPx, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl AssistiveTouch {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        let mut out = Self {
            enabled: false,
            open: false,
            device_page: false,
            touch: None,
            position: (w as f32, h as f32 * 0.65),
            viewport: (w, h, ui.s),
            last_active: None,
            dimmed: false,
        };
        out.clamp(ui, w, h);
        out
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        self.dismiss();
        self.dimmed = false;
        self.last_active = None;
        true
    }

    pub fn shown(&self) -> bool {
        self.enabled && self.open
    }

    /// Call when locking or turning the screen off; no finger leaks on return.
    pub fn dismiss(&mut self) -> bool {
        let changed = self.open || self.touch.is_some();
        self.open = false;
        self.device_page = false;
        self.touch = None;
        changed
    }

    fn size(ui: &Ui, w: i32, h: i32) -> f32 {
        ui.pt(BUTTON_PT).min(w.max(1) as f32).min(h.max(1) as f32)
    }

    fn bounds(ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let d = Self::size(ui, w, h);
        let mx = ui.pt(6.0).min(((w as f32 - d) / 2.0).max(0.0));
        let my = ui.pt(28.0).min(((h as f32 - d) / 2.0).max(0.0));
        (
            mx,
            my,
            (w as f32 - d - mx).max(mx),
            (h as f32 - d - my).max(my),
        )
    }

    fn clamp(&mut self, ui: &Ui, w: i32, h: i32) {
        let (x0, y0, x1, y1) = Self::bounds(ui, w, h);
        self.position.0 = self.position.0.clamp(x0, x1);
        self.position.1 = self.position.1.clamp(y0, y1);
    }

    /// Preserve the docked edge and relative height across orientation changes.
    pub fn resize(&mut self, ui: &Ui, w: i32, h: i32) -> bool {
        if self.viewport == (w, h, ui.s) {
            return false;
        }
        let (ow, oh, os) = self.viewport;
        let old_d = (BUTTON_PT * os).min(ow.max(1) as f32).min(oh.max(1) as f32);
        let right = self.position.0 + old_d / 2.0 >= ow as f32 / 2.0;
        let fraction = (self.position.1 + old_d / 2.0) / oh.max(1) as f32;
        let d = Self::size(ui, w, h);
        self.position = (
            if right { w as f32 } else { 0.0 },
            fraction * h as f32 - d / 2.0,
        );
        self.viewport = (w, h, ui.s);
        // A gesture started in the old coordinate space cannot trigger actions.
        if self.touch.is_some() {
            self.touch = Some(Touch::Cancelled);
        }
        self.clamp(ui, w, h);
        self.enabled
    }

    pub fn button_rect(&self, ui: &Ui, w: i32, h: i32) -> BoxPx {
        let d = Self::size(ui, w, h);
        (self.position.0, self.position.1, d, d)
    }

    fn menu_rect(ui: &Ui, w: i32, h: i32) -> BoxPx {
        let mw = ui.pt(324.0).min((w as f32 - ui.pt(16.0)).max(1.0));
        let mh = ui.pt(274.0).min((h as f32 - ui.pt(32.0)).max(1.0));
        ((w as f32 - mw) / 2.0, (h as f32 - mh) / 2.0, mw, mh)
    }

    pub fn action_rect(ui: &Ui, w: i32, h: i32, action: Action) -> BoxPx {
        let (x, y, mw, mh) = Self::menu_rect(ui, w, h);
        let index = Action::ALL
            .iter()
            .chain(Action::DEVICE.iter())
            .position(|&a| a == action)
            .unwrap()
            % 6;
        let index = if Action::ALL.contains(&action) { Action::ALL.iter().position(|&a| a == action).unwrap() } else { index };
        let padding = ui.pt(12.0).min(mw / 12.0).min(mh / 12.0);
        let cw = (mw - 2.0 * padding) / 3.0;
        let ch = (mh - 2.0 * padding) / 2.0;
        (
            x + padding + (index % 3) as f32 * cw,
            y + padding + (index / 3) as f32 * ch,
            cw,
            ch,
        )
    }

    fn page(&self) -> [Action; 6] {
        if self.device_page { Action::DEVICE } else { Action::ALL }
    }

    fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Action> {
        self.page()
            .into_iter()
            .find(|&a| inside(Self::action_rect(ui, w, h, a), x, y))
    }

    pub fn event(&mut self, event: Event, t: f64, ui: &Ui, w: i32, h: i32) -> Response {
        let resized = self.resize(ui, w, h);
        if !self.enabled {
            return Response::default();
        }
        let mut response = Response {
            changed: resized,
            ..Response::default()
        };
        let threshold = ui.pt(DRAG_PT);
        match event {
            Event::Down(x, y) => {
                if self.open {
                    self.touch = Some(Touch::Menu {
                        action: self.hit(ui, w, h, x, y),
                        down: (x, y),
                        moved: false,
                    });
                } else if inside(self.button_rect(ui, w, h), x, y) {
                    self.touch = Some(Touch::Button {
                        down: (x, y),
                        origin: self.position,
                        dragged: false,
                    });
                } else {
                    return response;
                }
                response.consumed = true;
                response.changed = true;
                self.last_active = Some(t);
                self.dimmed = false;
            }
            Event::Move(x, y) => {
                let Some(mut touch) = self.touch else {
                    return response;
                };
                response.consumed = true;
                match &mut touch {
                    Touch::Cancelled => {}
                    Touch::Button {
                        down,
                        origin,
                        dragged,
                    } => {
                        let delta = (x - down.0, y - down.1);
                        *dragged |= delta.0.hypot(delta.1) > threshold;
                        if *dragged {
                            self.position = (origin.0 + delta.0, origin.1 + delta.1);
                            self.clamp(ui, w, h);
                            response.changed = true;
                        }
                    }
                    Touch::Menu { down, moved, .. } => {
                        *moved |= (x - down.0).hypot(y - down.1) > threshold;
                        response.changed = true;
                    }
                }
                self.touch = Some(touch);
                self.last_active = Some(t);
            }
            Event::Up(x, y) => {
                let Some(touch) = self.touch.take() else {
                    return response;
                };
                response.consumed = true;
                response.changed = true;
                self.last_active = Some(t);
                match touch {
                    Touch::Cancelled => {}
                    Touch::Button {
                        down,
                        origin,
                        dragged,
                    } => {
                        let delta = (x - down.0, y - down.1);
                        if dragged || delta.0.hypot(delta.1) > threshold {
                            self.position = (origin.0 + delta.0, origin.1 + delta.1);
                            self.clamp(ui, w, h);
                            let (left, _, right, _) = Self::bounds(ui, w, h);
                            self.position.0 =
                                if self.position.0 + Self::size(ui, w, h) / 2.0 < w as f32 / 2.0 {
                                    left
                                } else {
                                    right
                                };
                        } else {
                            self.open = true;
                        }
                    }
                    Touch::Menu {
                        action,
                        down,
                        moved,
                    } => {
                        let tap = !moved && (x - down.0).hypot(y - down.1) <= threshold;
                        if tap {
                            if let Some(action) = action {
                                if self.hit(ui, w, h, x, y) == Some(action) {
                                    match action {
                                        Action::Device => self.device_page = true,
                                        Action::Back => self.device_page = false,
                                        // Volume and rotation repeat without closing.
                                        Action::VolumeUp | Action::VolumeDown | Action::Rotate => {
                                            response.action = Some(action)
                                        }
                                        _ => {
                                            response.action = Some(action);
                                            self.open = false;
                                            self.device_page = false;
                                        }
                                    }
                                }
                            } else if !inside(Self::menu_rect(ui, w, h), down.0, down.1)
                                && !inside(Self::menu_rect(ui, w, h), x, y)
                            {
                                self.open = false;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        response
    }

    /// One wake per interaction; fading never schedules a continuous animation.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        (self.enabled && !self.open && self.touch.is_none() && !self.dimmed)
            .then(|| (self.last_active.unwrap_or(t) + IDLE_MS).max(t))
    }

    pub fn tick(&mut self, t: f64) -> bool {
        if self.enabled && self.last_active.is_none() {
            self.last_active = Some(t);
        }
        if self.enabled
            && !self.open
            && self.touch.is_none()
            && !self.dimmed
            && t >= self.last_active.unwrap_or(t) + IDLE_MS
        {
            self.dimmed = true;
            true
        } else {
            false
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        self.resize(ui, w, h);
        if !self.enabled {
            return;
        }
        self.last_active.get_or_insert(t);
        if self.open {
            s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 45));
            let (x, y, mw, mh) = Self::menu_rect(ui, w, h);
            s.fill_rrect(
                x + ui.pt(2.0),
                y + ui.pt(5.0),
                mw,
                mh,
                ui.pt(26.0),
                rgba(0, 0, 0, 80),
            );
            s.fill_rrect(x, y, mw, mh, ui.pt(26.0), rgba(30, 30, 32, 247));
            s.stroke_rrect(
                x,
                y,
                mw,
                mh,
                ui.pt(26.0),
                ui.pt(0.7),
                rgba(255, 255, 255, 35),
            );
            let pressed = match self.touch {
                Some(Touch::Menu {
                    action,
                    moved: false,
                    ..
                }) => action,
                _ => None,
            };
            for action in self.page() {
                let (ax, ay, aw, ah) = Self::action_rect(ui, w, h, action);
                if pressed == Some(action) {
                    s.fill_rrect(
                        ax + ui.pt(2.0),
                        ay + ui.pt(2.0),
                        aw - ui.pt(4.0),
                        ah - ui.pt(4.0),
                        ui.pt(14.0),
                        rgba(255, 255, 255, 30),
                    );
                }
                let icon = ui.pt(34.0).min(aw * 0.45).min(ah * 0.4);
                draw_icon(s, action, ax + aw / 2.0, ay + ah * 0.34, icon, WHITE);
                let lines = ui.wrap(action.label(), 12.0, Weight::Medium, aw - ui.pt(6.0));
                for (line, text) in lines.iter().enumerate() {
                    let image = ui.text(text, 12.0, Weight::Medium, WHITE);
                    s.blit(
                        &image,
                        (ax + (aw - image.w as f32) / 2.0) as i32,
                        (ay + ah * 0.61 + line as f32 * ui.pt(16.0) * ui.text_scale) as i32,
                        255,
                        255,
                    );
                }
            }
        } else {
            let (x, y, d, _) = self.button_rect(ui, w, h);
            let alpha = if self.dimmed { 115 } else { 240 };
            s.fill_rrect(x, y, d, d, d * 0.28, rgba(34, 34, 36, alpha));
            s.stroke_rrect(
                x,
                y,
                d,
                d,
                d * 0.28,
                ui.pt(0.7),
                rgba(255, 255, 255, alpha / 3),
            );
            s.fill_rrect(
                x + d * 0.14,
                y + d * 0.14,
                d * 0.72,
                d * 0.72,
                d * 0.36,
                rgba(110, 110, 115, alpha),
            );
            s.fill_rrect(
                x + d * 0.22,
                y + d * 0.22,
                d * 0.56,
                d * 0.56,
                d * 0.28,
                rgba(198, 198, 200, alpha),
            );
            s.fill_rrect(
                x + d * 0.29,
                y + d * 0.29,
                d * 0.42,
                d * 0.42,
                d * 0.21,
                rgba(255, 255, 255, alpha),
            );
        }
    }
}

fn draw_icon(s: &mut Surface, action: Action, cx: f32, cy: f32, size: f32, color: u32) {
    let x = cx - size / 2.0;
    let y = cy - size / 2.0;
    let line = size * 0.075;
    let paint_line = |s: &mut Surface, a, b| {
        glyphs::paint(
            s,
            (x - line, y - line, size + 2.0 * line, size + 2.0 * line),
            color,
            glyphs::segment(a, b, line),
        )
    };
    match action {
        Action::Home => s.stroke_rrect(
            x + size * 0.08,
            y + size * 0.08,
            size * 0.84,
            size * 0.84,
            size / 2.0,
            line,
            color,
        ),
        Action::AppSwitcher => {
            s.stroke_rrect(
                x,
                y + size * 0.12,
                size * 0.7,
                size * 0.76,
                line,
                line,
                color,
            );
            s.fill_rrect(
                x + size * 0.3,
                y,
                size * 0.7,
                size,
                line,
                rgba(30, 30, 32, 255),
            );
            s.stroke_rrect(x + size * 0.3, y, size * 0.7, size, line, line, color);
        }
        Action::ControlCenter => {
            for (row, knob) in [0.65, 0.3, 0.6].into_iter().enumerate() {
                let yy = y + size * (0.16 + row as f32 * 0.34);
                paint_line(s, (x, yy), (x + size, yy));
                s.fill_rrect(
                    x + size * knob - line * 1.2,
                    yy - line * 1.5,
                    line * 2.4,
                    line * 3.0,
                    line,
                    color,
                );
            }
        }
        Action::NotificationCenter => {
            for row in 0..3 {
                let yy = y + size * (0.1 + row as f32 * 0.31);
                s.fill_rrect(x, yy, size * 0.13, size * 0.13, line / 2.0, color);
                paint_line(
                    s,
                    (x + size * 0.3, yy + size * 0.065),
                    (x + size, yy + size * 0.065),
                );
            }
        }
        Action::Lock => {
            s.stroke_rrect(
                x + size * 0.24,
                y,
                size * 0.52,
                size * 0.66,
                size * 0.26,
                line,
                color,
            );
            s.fill_rrect(
                x + size * 0.1,
                y + size * 0.4,
                size * 0.8,
                size * 0.6,
                line,
                color,
            );
            s.fill_rrect(
                cx - line / 2.0,
                y + size * 0.56,
                line,
                size * 0.23,
                line / 2.0,
                rgba(30, 30, 32, 255),
            );
        }
        Action::Device => {
            s.stroke_rrect(x + size * 0.2, y, size * 0.6, size, line * 1.5, line, color);
            s.fill_rrect(cx - size * 0.1, y + size * 0.82, size * 0.2, line, line / 2.0, color);
        }
        Action::Rotate => {
            glyphs::paint(s, (x, y, size, size), color, glyphs::ring((cx, cy), size * 0.4, line));
            s.fill_rrect(cx - size * 0.1, cy - size * 0.2, size * 0.2, size * 0.4, line, color);
        }
        Action::VolumeUp | Action::VolumeDown => {
            s.fill_rrect(x, y + size * 0.34, size * 0.3, size * 0.32, line, color);
            s.fill_rrect(x + size * 0.2, y + size * 0.16, size * 0.24, size * 0.68, line, color);
            if action == Action::VolumeUp {
                glyphs::paint(s, (x, y, size, size), color, glyphs::ring((cx + size * 0.2, cy), size * 0.3, line));
            }
        }
        Action::Back => {
            paint_line(s, (x + size * 0.65, y + size * 0.1), (x + size * 0.25, cy));
            paint_line(s, (x + size * 0.25, cy), (x + size * 0.65, y + size * 0.9));
        }
        Action::Screenshot => {
            s.stroke_rrect(x, y + size * 0.18, size, size * 0.7, line, line, color);
            s.fill_rrect(
                x + size * 0.3,
                y + size * 0.04,
                size * 0.4,
                size * 0.16,
                line / 2.0,
                color,
            );
            glyphs::paint(
                s,
                (x, y, size, size),
                color,
                glyphs::ring((cx, cy + size * 0.03), size * 0.22, line),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay() -> (Ui, AssistiveTouch) {
        let ui = crate::shell::tests::ui();
        let mut overlay = AssistiveTouch::new(&ui, 768, 1024);
        overlay.set_enabled(true);
        (ui, overlay)
    }

    fn open(overlay: &mut AssistiveTouch, ui: &Ui) {
        let (x, y, d, _) = overlay.button_rect(ui, 768, 1024);
        assert!(
            overlay
                .event(Event::Down(x + d / 2.0, y + d / 2.0), 10.0, ui, 768, 1024)
                .consumed
        );
        assert!(
            overlay
                .event(Event::Up(x + d / 2.0, y + d / 2.0), 30.0, ui, 768, 1024)
                .consumed
        );
        assert!(overlay.shown());
    }

    #[test]
    fn tap_opens_and_all_six_actions_have_distinct_targets() {
        let (ui, mut overlay) = overlay();
        for action in Action::ALL.into_iter().filter(|&a| a != Action::Device) {
            open(&mut overlay, &ui);
            let (x, y, w, h) = AssistiveTouch::action_rect(&ui, 768, 1024, action);
            let (x, y) = (x + w / 2.0, y + h / 2.0);
            assert!(
                overlay
                    .event(Event::Down(x, y), 50.0, &ui, 768, 1024)
                    .consumed
            );
            let result = overlay.event(Event::Up(x, y), 70.0, &ui, 768, 1024);
            assert_eq!(result.action, Some(action));
            assert!(result.consumed && result.changed);
            assert!(!overlay.shown());
        }
    }

    #[test]
    fn drag_captures_finger_clamps_and_never_opens_menu() {
        let (ui, mut overlay) = overlay();
        let (x, y, _, _) = overlay.button_rect(&ui, 768, 1024);
        overlay.event(Event::Down(x + 20.0, y + 20.0), 0.0, &ui, 768, 1024);
        assert!(
            overlay
                .event(Event::Move(-500.0, -1000.0), 10.0, &ui, 768, 1024)
                .consumed
        );
        assert!(
            overlay
                .event(Event::Up(-500.0, -1000.0), 20.0, &ui, 768, 1024)
                .consumed
        );
        let (x, y, d, _) = overlay.button_rect(&ui, 768, 1024);
        assert!(x >= 0.0 && y >= 0.0 && x + d <= 768.0 && y + d <= 1024.0);
        assert!(!overlay.shown());
        // Crossing the threshold and returning to the start is still a drag.
        overlay.event(Event::Down(x + 20.0, y + 20.0), 30.0, &ui, 768, 1024);
        overlay.event(Event::Move(x + 80.0, y + 20.0), 40.0, &ui, 768, 1024);
        overlay.event(Event::Up(x + 20.0, y + 20.0), 50.0, &ui, 768, 1024);
        assert!(!overlay.shown());
    }

    #[test]
    fn outside_dismissal_consumes_up_and_menu_drags_do_not_activate() {
        let (ui, mut overlay) = overlay();
        open(&mut overlay, &ui);
        assert!(
            overlay
                .event(Event::Down(5.0, 5.0), 50.0, &ui, 768, 1024)
                .consumed
        );
        let result = overlay.event(Event::Up(5.0, 5.0), 70.0, &ui, 768, 1024);
        assert!(result.consumed && result.action.is_none() && !overlay.shown());
        open(&mut overlay, &ui);
        let (x, y, w, h) = AssistiveTouch::action_rect(&ui, 768, 1024, Action::Home);
        let (x, y) = (x + w / 2.0, y + h / 2.0);
        overlay.event(Event::Down(x, y), 100.0, &ui, 768, 1024);
        overlay.event(Event::Move(x + 50.0, y), 120.0, &ui, 768, 1024);
        assert_eq!(
            overlay.event(Event::Up(x, y), 150.0, &ui, 768, 1024).action,
            None
        );
        assert!(overlay.shown());
    }

    #[test]
    fn disabled_ignores_input_and_resize_keeps_button_on_screen() {
        let (mut ui, mut overlay) = overlay();
        open(&mut overlay, &ui);
        assert!(overlay.set_enabled(false));
        assert!(!overlay.shown());
        assert_eq!(
            overlay.event(Event::Down(5.0, 5.0), 50.0, &ui, 768, 1024),
            Response::default()
        );
        assert!(overlay.set_enabled(true));
        ui.s = 0.5;
        assert!(overlay.resize(&ui, 160, 90));
        let (x, y, d, _) = overlay.button_rect(&ui, 160, 90);
        assert!(x >= 0.0 && y >= 0.0 && x + d <= 160.0 && y + d <= 90.0);
        assert!(x > 80.0, "right edge survives resize");
        for (w, h) in [(1, 1), (20, 100), (100, 20), (20, 20)] {
            overlay.resize(&ui, w, h);
            let (x, y, d, _) = overlay.button_rect(&ui, w, h);
            assert!(x >= 0.0 && y >= 0.0 && x + d <= w as f32 && y + d <= h as f32);
        }
    }

    #[test]
    fn idle_fades_once_and_only_schedules_when_idle() {
        let (ui, mut overlay) = overlay();
        assert!(!overlay.tick(0.0));
        assert_eq!(overlay.next_wake(10.0), Some(IDLE_MS));
        assert!(!overlay.tick(IDLE_MS - 1.0));
        assert!(overlay.tick(IDLE_MS));
        assert!(!overlay.tick(IDLE_MS + 1.0));
        assert_eq!(overlay.next_wake(IDLE_MS), None);
        open(&mut overlay, &ui);
        assert_eq!(overlay.next_wake(IDLE_MS), None);
    }

    #[test]
    fn enabling_later_starts_a_fresh_idle_timer() {
        let (_, mut overlay) = overlay();
        overlay.set_enabled(false);
        overlay.set_enabled(true);
        assert_eq!(overlay.next_wake(10000.0), Some(13000.0));
        assert!(!overlay.tick(10000.0));
        assert!(!overlay.tick(12999.0));
        assert!(overlay.tick(13000.0));
    }

    #[test]
    fn resizing_cancels_action_but_still_consumes_the_release() {
        let (mut ui, mut overlay) = overlay();
        open(&mut overlay, &ui);
        let (x, y, w, h) = AssistiveTouch::action_rect(&ui, 768, 1024, Action::Home);
        let (x, y) = (x + w / 2.0, y + h / 2.0);
        overlay.event(Event::Down(x, y), 100.0, &ui, 768, 1024);
        ui.s = 0.75;
        overlay.resize(&ui, 1024, 768);
        let result = overlay.event(Event::Up(x, y), 200.0, &ui, 1024, 768);
        assert!(result.consumed);
        assert_eq!(result.action, None);
    }

    #[test]
    fn menu_dims_the_underlying_screen_without_replacing_it() {
        let (ui, mut overlay) = overlay();
        open(&mut overlay, &ui);
        let mut pixels = vec![rgba(80, 100, 120, 255); 768 * 1024];
        overlay.draw(
            &mut Surface {
                w: 768,
                h: 1024,
                stride: 768,
                px: &mut pixels,
                y0: 0,
            },
            &ui,
            768,
            1024,
            100.0,
        );
        assert_eq!(pixels[0] >> 24, 255, "the underlying frame remains opaque");
        assert!(
            (1..80).contains(&(pixels[0] & 255)),
            "the backdrop retains the screen's color"
        );
    }
}
