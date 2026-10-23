//! The desktop Control Center's module detail views (W5-10). As on the reference, a click on a module's label (not its
//! icon disc, which keeps toggling) replaces the modules with that module's detail: Wi-Fi lists the networks with the
//! radio's switch, Bluetooth the paired devices (or says there is no controller), Focus the modes, Display the
//! brightness slider (with a backlight or a DDC/CI monitor), Dark Mode and Night Shift, Sound the volume and the
//! outputs, and Now Playing the artwork with the transport. Each list ends with the module's Settings page. A click
//! beside the detail goes back to the modules.

use super::*;
use crate::bare::bluetooth::{Cmd as BtCmd, State as BtState};
use crate::shell::night_shift::NightShift;

/// The title row of a detail, with the switch at its right.
pub(super) const DT_HEADER_PT: f32 = 44.0;
/// A slider row (Display brightness, Sound volume).
pub(super) const DT_SLIDER_PT: f32 = 40.0;
/// A list row.
pub(super) const DT_ROW_PT: f32 = 28.0;
/// A section heading ("Other Networks", "Output").
pub(super) const DT_HEADING_PT: f32 = 24.0;
/// The "… Settings…" row under the list, with its separator.
pub(super) const DT_FOOTER_PT: f32 = 34.0;
/// The largest side of the Now Playing artwork.
pub(super) const DT_ART_PT: f32 = 180.0;
/// The title and artist under the artwork.
pub(super) const DT_TEXT_PT: f32 = 44.0;
/// The transport row of the Now Playing detail.
pub(super) const DT_TRANSPORT_PT: f32 = 56.0;
/// The most list rows a detail shows (the rest are in Settings).
pub(super) const DT_MAX_ROWS: usize = 10;

/// The module whose detail replaces the modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Wifi,
    Bluetooth,
    Focus,
    Display,
    Sound,
    NowPlaying,
}

impl Detail {
    /// The detail a desktop module opens.
    pub fn of(tile: Tile) -> Option<Detail> {
        Some(match tile {
            Tile::Wifi => Detail::Wifi,
            Tile::Bluetooth => Detail::Bluetooth,
            Tile::Focus => Detail::Focus,
            Tile::Brightness => Detail::Display,
            Tile::Volume => Detail::Sound,
            Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext => Detail::NowPlaying,
            _ => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Detail::Wifi => "Wi-Fi",
            Detail::Bluetooth => "Bluetooth",
            Detail::Focus => "Focus",
            Detail::Display => "Display",
            Detail::Sound => "Sound",
            Detail::NowPlaying => "Now Playing",
        }
    }

    /// The last row's label and the Settings page it opens (`Settings::open_page` names).
    pub fn settings(self) -> Option<(&'static str, &'static str)> {
        match self {
            Detail::Wifi => Some(("Wi-Fi Settings\u{2026}", "Wi-Fi")),
            Detail::Bluetooth => Some(("Bluetooth Settings\u{2026}", "Bluetooth")),
            Detail::Focus => Some(("Focus Settings\u{2026}", "Focus")),
            Detail::Display => Some(("Displays Settings\u{2026}", "Display & Brightness")),
            Detail::Sound => Some(("Sound Settings\u{2026}", "Sound")),
            Detail::NowPlaying => None,
        }
    }
}

/// What a click on a detail row does.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    None,
    Wifi(WifiAction),
    Bluetooth(BtCmd),
    /// switch this Focus mode on by hand, or Focus off (None)
    Focus(Option<String>),
    /// flip the appearance between Light and Dark
    DarkMode,
    /// switch Night Shift on (true) or off by hand
    NightShift(bool),
    Route(crate::media::route::Route),
    /// a network that needs a password: its Settings page asks for it
    Settings(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStyle {
    /// a section title, not clickable
    Heading,
    /// a choice, checked while `on`
    Check,
    /// a setting with a switch at the right
    Toggle,
    /// a quiet line that explains an empty list
    Note,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub title: String,
    pub style: RowStyle,
    pub on: bool,
    pub action: RowAction,
}

impl Row {
    fn heading(title: &str) -> Row {
        Row { title: title.to_string(), style: RowStyle::Heading, on: false, action: RowAction::None }
    }

    fn note(title: &str) -> Row {
        Row { title: title.to_string(), style: RowStyle::Note, on: false, action: RowAction::None }
    }

    fn check(title: String, on: bool, action: RowAction) -> Row {
        Row { title, style: RowStyle::Check, on, action }
    }

    fn height(&self, ui: &Ui) -> f32 {
        ui.pt(if self.style == RowStyle::Heading { DT_HEADING_PT } else { DT_ROW_PT })
    }
}

type R4 = (f32, f32, f32, f32);

/// Where everything of a detail sits, in px. One layout serves the drawing and the hit tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub panel: R4,
    pub header: R4,
    pub switch: Option<R4>,
    pub slider: Option<(Tile, R4)>,
    pub art: Option<R4>,
    pub text: Option<R4>,
    pub transport: Option<R4>,
    pub rows: Vec<(Row, R4)>,
    pub footer: Option<(&'static str, &'static str, R4)>,
}

/// A slider row's track: (left, right, centre y) in px.
pub(super) fn detail_track(ui: &Ui, (x, y, w, h): R4) -> (f32, f32, f32) {
    (x + ui.pt(DK_PAD_PT + 22.0), x + w - ui.pt(DK_PAD_PT + 22.0), y + h / 2.0)
}

/// Whether a press at (x, y) is on a slider's track: its band, a little past both ends.
pub(super) fn on_track(ui: &Ui, rect: R4, x: f32, y: f32) -> bool {
    let (l, r, cy) = detail_track(ui, rect);
    (y - cy).abs() <= ui.pt(12.0) && x >= l - ui.pt(10.0) && x <= r + ui.pt(10.0)
}

/// A separator line across the panel: `(x, width, colour)` at `y`.
fn hairline(s: &mut Surface, ui: &Ui, (x, w, color): (f32, f32, u32), y: f32) {
    s.fill_rrect(x, y, w, 1.0f32.max(ui.s * 0.5), 0.0, color);
}

impl Shell {
    /// The list rows of a detail, from the live state.
    pub(super) fn desk_detail_rows(&self, d: Detail) -> Vec<Row> {
        let mut rows = vec![];
        match d {
            Detail::Wifi => {
                let Some(n) = self.info.desktop_network.as_ref().filter(|n| n.wifi_available && n.wifi_on) else { return rows };
                if let Some(ssid) = &n.ssid {
                    rows.push(Row::heading("Known Network"));
                    rows.push(Row::check(ssid.clone(), true, RowAction::None));
                }
                let others: Vec<&super::super::Network> = n.networks.iter().filter(|net| !net.ssid.is_empty() && n.ssid.as_deref() != Some(net.ssid.as_str())).take(DT_MAX_ROWS).collect();
                if !others.is_empty() {
                    rows.push(Row::heading("Other Networks"));
                }
                for net in others {
                    let action = if net.saved {
                        RowAction::Wifi(WifiAction::Select(net.ssid.clone()))
                    } else if !net.secure {
                        RowAction::Wifi(WifiAction::Join { ssid: net.ssid.clone(), psk: None })
                    } else {
                        RowAction::Settings("Wi-Fi")
                    };
                    rows.push(Row::check(net.ssid.clone(), false, action));
                }
                if rows.is_empty() {
                    rows.push(Row::note("No networks found"));
                }
            }
            Detail::Bluetooth => {
                let bt = &self.info.bt;
                if bt.state == BtState::Unavailable {
                    rows.push(Row::note("No Bluetooth controller was found"));
                } else if bt.on() {
                    rows.push(Row::heading("Devices"));
                    let mut any = false;
                    for dev in bt.devices.iter().filter(|dev| dev.paired || dev.connected).take(DT_MAX_ROWS) {
                        any = true;
                        let busy = bt.busy.as_deref() == Some(dev.addr.as_str());
                        let action = if busy {
                            RowAction::None
                        } else if dev.connected {
                            RowAction::Bluetooth(BtCmd::Disconnect(dev.addr.clone()))
                        } else {
                            RowAction::Bluetooth(BtCmd::Connect(dev.addr.clone()))
                        };
                        let title = if busy { format!("{}\u{2026}", dev.label()) } else { dev.label().to_string() };
                        rows.push(Row::check(title, dev.connected, action));
                    }
                    if !any {
                        rows.push(Row::note("No devices"));
                    }
                }
            }
            Detail::Focus => {
                let active = self.notify.active_mode().map(|m| m.id);
                for m in &self.notify.modes.list {
                    let on = active.as_deref() == Some(m.id.as_str());
                    rows.push(Row::check(m.name.clone(), on, RowAction::Focus(if on { None } else { Some(m.id.clone()) })));
                }
            }
            Detail::Display => {
                rows.push(Row { title: "Dark Mode".into(), style: RowStyle::Toggle, on: !bifrost_render::appearance::target_is_light(), action: RowAction::DarkMode });
                let night = NightShift::load(&self.prefs).manual;
                rows.push(Row { title: "Night Shift".into(), style: RowStyle::Toggle, on: night, action: RowAction::NightShift(!night) });
            }
            Detail::Sound => {
                rows.push(Row::heading("Output"));
                for dest in self.route_menu_below((0.0, 0.0, 0.0, 0.0)).rows {
                    let action = route_of_id(&dest.id).map_or(RowAction::None, RowAction::Route);
                    rows.push(Row::check(dest.name, dest.active, action));
                }
            }
            Detail::NowPlaying => {}
        }
        rows
    }

    /// The left edge, the top and the width of the panel's column, in px.
    pub(super) fn desk_column(&self) -> (f32, f32, f32) {
        let ui = &self.ui;
        let full = ui.pt(2.0 * DK_COL_PT + DK_COL_GAP_PT);
        let x0 = self.w as f32 - ui.pt(DK_RIGHT_PT) - full;
        (x0, self.desktop_bar_height() as f32 + ui.pt(DK_TOP_GAP_PT), full)
    }

    pub(super) fn desk_detail_layout(&self, d: Detail) -> Layout {
        let ui = &self.ui;
        let (x0, y0, full) = self.desk_column();
        let pad = ui.pt(DK_PAD_PT);
        let header = (x0, y0, full, ui.pt(DT_HEADER_PT));
        let switch_shown = match d {
            Detail::Wifi => self.desk_wifi().is_some(),
            Detail::Bluetooth => self.info.bt.state != BtState::Unavailable,
            _ => false,
        };
        let (sw, sh) = (ui.pt(SWITCH_W_PT), ui.pt(SWITCH_H_PT));
        let switch = switch_shown.then(|| (x0 + full - pad - sw, y0 + (header.3 - sh) / 2.0, sw, sh));
        let mut y = y0 + header.3;
        let slider_tile = match d {
            Detail::Display if self.cc.backlight => Some(Tile::Brightness),
            Detail::Sound => Some(Tile::Volume),
            _ => None,
        };
        let slider = slider_tile.map(|tile| {
            let r = (x0, y, full, ui.pt(DT_SLIDER_PT));
            y += r.3;
            (tile, r)
        });
        let (mut art, mut text, mut transport) = (None, None, None);
        if d == Detail::NowPlaying {
            let side = (full - 2.0 * pad).min(ui.pt(DT_ART_PT));
            art = Some((x0 + (full - side) / 2.0, y, side, side));
            y += side + ui.pt(8.0);
            text = Some((x0 + pad, y, full - 2.0 * pad, ui.pt(DT_TEXT_PT)));
            y += ui.pt(DT_TEXT_PT);
            transport = Some((x0, y, full, ui.pt(DT_TRANSPORT_PT)));
            y += ui.pt(DT_TRANSPORT_PT);
        }
        let footer_h = if d.settings().is_some() { ui.pt(DT_FOOTER_PT) } else { 0.0 };
        // the list stops above the screen's bottom edge: the rest is in Settings
        let bottom = self.h as f32 - ui.pt(20.0) - footer_h - pad / 2.0;
        let mut rows = vec![];
        for row in self.desk_detail_rows(d) {
            let h = row.height(ui);
            if y + h > bottom {
                break;
            }
            rows.push((row, (x0 + pad / 2.0, y, full - pad, h)));
            y += h;
        }
        let footer = d.settings().map(|(label, page)| {
            let r = (x0 + pad / 2.0, y + ui.pt(6.0), full - pad, footer_h - ui.pt(6.0));
            y += footer_h;
            (label, page, r)
        });
        let panel = (x0, y0, full, y - y0 + pad / 2.0);
        Layout { panel, header, switch, slider, art, text, transport, rows, footer }
    }

    /// Opens the detail of `tile`'s module in place of the modules (nothing for a module without one).
    pub(super) fn open_desk_detail(&mut self, tile: Tile) {
        if let Some(d) = Detail::of(tile) {
            self.cc.route_menu = None;
            self.cc.up_next = None;
            self.cc.desk_detail = Some(d);
            self.dirty = true;
        }
    }

    /// Pointer input while a detail shows. A press beside it goes back to the modules.
    pub(super) fn desk_detail_event(&mut self, d: Detail, ev: Event, t: f64) -> Vec<Effect> {
        let layout = self.desk_detail_layout(d);
        let slop = self.ui.pt(4.0);
        match ev {
            Event::Down(x, y) => {
                self.dirty = true;
                if !inside(layout.panel, x, y) {
                    self.cc.desk_detail = None;
                    self.cc.touch = None;
                    return vec![];
                }
                if layout.switch.is_some_and(|(sx, sy, sw, sh)| inside((sx - slop, sy - slop, sw + 2.0 * slop, sh + 2.0 * slop), x, y)) {
                    return match d {
                        Detail::Wifi => {
                            let on = !self.desk_wifi().is_some_and(|w| w.0);
                            if let Some(n) = self.info.desktop_network.as_mut() {
                                n.wifi_on = on;
                            }
                            vec![Effect::Wifi(WifiAction::Enable(on))]
                        }
                        Detail::Bluetooth => self.toggle_bluetooth(),
                        _ => vec![],
                    };
                }
                if let Some((tile, r)) = layout.slider.filter(|&(_, r)| on_track(&self.ui, r, x, y)) {
                    self.cc.touch = Some(Touch::Slider(tile));
                    let (l, rr, _) = detail_track(&self.ui, r);
                    return self.set_slider(tile, ((x - l) / (rr - l)).clamp(0.0, 1.0));
                }
                if let Some(r) = layout.transport.filter(|&r| inside(r, x, y)) {
                    self.cc.touch = Some(Touch::Tile(desk_media_button(r, x)));
                    return vec![];
                }
                if let Some(action) = layout.rows.iter().find(|(_, r)| inside(*r, x, y)).map(|(row, _)| row.action.clone()) {
                    return self.run_detail_action(action, t);
                }
                if let Some((_, page, _)) = layout.footer.filter(|&(_, _, r)| inside(r, x, y)) {
                    return self.desk_detail_settings(page, t);
                }
                vec![]
            }
            Event::Move(x, _) => match (self.cc.touch, layout.slider) {
                (Some(Touch::Slider(tile)), Some((k, r))) if k == tile => {
                    let (l, rr, _) = detail_track(&self.ui, r);
                    self.set_slider(tile, ((x - l) / (rr - l)).clamp(0.0, 1.0))
                }
                _ => vec![],
            },
            Event::Up(x, y) => match self.cc.touch.take() {
                Some(Touch::Tile(tile @ (Tile::MediaPrev | Tile::MediaPlay | Tile::MediaNext)))
                    if layout.transport.is_some_and(|r| inside(r, x, y) && desk_media_button(r, x) == tile) =>
                {
                    self.activate_tile(tile, t)
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    fn run_detail_action(&mut self, action: RowAction, t: f64) -> Vec<Effect> {
        match action {
            RowAction::None => vec![],
            RowAction::Wifi(a) => vec![Effect::Wifi(a)],
            RowAction::Bluetooth(c) => vec![Effect::Bluetooth(c)],
            RowAction::Focus(id) => {
                self.set_focus_mode(id.as_deref());
                vec![]
            }
            RowAction::DarkMode => {
                self.toggle_appearance();
                vec![]
            }
            RowAction::NightShift(on) => {
                self.cc_set_night_shift(on);
                vec![]
            }
            RowAction::Route(r) => {
                // the binary owns the router: it takes the choice on its next pass
                self.cc.route_choice = Some(r);
                self.cc.route = r;
                vec![]
            }
            RowAction::Settings(page) => self.desk_detail_settings(page, t),
        }
    }

    /// Switches Night Shift on or off by hand, as Settings > Displays > Night Shift does, and keeps a retained Settings
    /// page in step.
    pub(super) fn cc_set_night_shift(&mut self, on: bool) {
        let mut night = NightShift::load(&self.prefs);
        night.manual = on;
        night.publish();
        night.save(&mut self.prefs);
        if let Some(st) = &mut self.settings {
            st.set_night_shift(&self.ui, night);
        }
        self.dirty = true;
    }

    /// Closes the panel and opens Settings on `page`.
    fn desk_detail_settings(&mut self, page: &str, t: f64) -> Vec<Effect> {
        self.close_desk_control_center(t);
        self.desk_open(AppId::Settings, t);
        match &mut self.settings {
            Some(st) => st.open_page(&self.ui, page, t),
            None => vec![],
        }
    }

    /// Draws the open detail over the live desktop, `lift` px up while the panel slides in.
    pub(super) fn draw_desk_detail(&self, s: &mut Surface, d: Detail, lift: f32, alpha: u32) {
        use super::super::ui::{LABEL, LABEL_2, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, SEPARATOR_C, SYSTEM_BLUE, TRACK_C};
        let ui = &self.ui;
        let a = |c: u32| scale(c, alpha);
        let up = |(x, y, w, h): R4| (x, y - lift, w, h);
        let layout = self.desk_detail_layout(d);
        let (px, py, pw, ph) = up(layout.panel);
        desk_module_glass(s, ShellSurface::ControlCentre, (px, py, pw, ph), ui.pt(DK_SLIDER_R_PT), alpha);
        let label = LABEL.now();
        let pad = ui.pt(DK_PAD_PT);
        let sep = (px + pad, pw - 2.0 * pad, a(SEPARATOR_C.now()));
        // the title row
        let (hx, hy, _, hh) = up(layout.header);
        let th = self.label(d.title(), MAC_HEADLINE_PT, Weight::Semibold, label, |im| im.h) as f32;
        self.label(d.title(), MAC_HEADLINE_PT, Weight::Semibold, label, |im| s.blit(im, (hx + pad + ui.pt(4.0)) as i32, (hy + (hh - th) / 2.0) as i32, 255, alpha));
        if let Some((sx, sy, _, _)) = layout.switch.map(up) {
            let on = match d {
                Detail::Wifi => self.desk_wifi().is_some_and(|w| w.0),
                _ => self.info.bt.on(),
            };
            draw_switch(s, ui, sx, sy, if on { 1.0 } else { 0.0 });
        }
        hairline(s, ui, sep, hy + hh);
        // the slider
        if let Some((tile, r)) = layout.slider {
            let r = up(r);
            let (l, rr, cy) = detail_track(ui, r);
            let v = if tile == Tile::Brightness { self.info.brightness } else { self.cc.volume };
            let tk = ui.pt(DK_TRACK_H_PT);
            s.fill_rrect(l, cy - tk / 2.0, rr - l, tk, tk / 2.0, a(TRACK_C.now()));
            s.fill_rrect(l, cy - tk / 2.0, (rr - l) * v, tk, tk / 2.0, a(label));
            let ends = (r.0 + ui.pt(DK_PAD_PT + 9.0), r.0 + r.2 - ui.pt(DK_PAD_PT + 9.0));
            if tile == Tile::Brightness {
                draw_sun(s, ends.0, cy, ui.s * 0.6, a(label));
                draw_sun(s, ends.1, cy, ui.s * 0.8, a(label));
            } else {
                draw_speaker(s, ends.0, cy, ui.s * 0.6, a(label));
                draw_speaker_waves(s, ends.1, cy, ui.s * 0.6, a(label), 3);
            }
        }
        // Now Playing: the artwork, the title and artist, the progress and the transport
        if let Some((ax, ay, aw, ah)) = layout.art.map(up) {
            let r = Rect { x: ax as i32, y: ay as i32, w: aw as i32, h: ah as i32 };
            draw_scaled(s, &icons::placeholder(256, 0.0), r, ui.pt(10.0), alpha);
        }
        if let Some((tx, ty, tw, _)) = layout.text.map(up) {
            let session = now_playing::current();
            let title = session.as_ref().and_then(|x| x.content().map(|c| c.title.clone())).unwrap_or_else(|| "Not Playing".to_string());
            let artist = session.as_ref().and_then(|x| x.content().map(|c| c.subtitle())).unwrap_or_default();
            let title = self.fit_chrome(&title, MAC_HEADLINE_PT, tw);
            let artist = self.fit_chrome(&artist, MAC_SUBHEAD_PT, tw);
            self.label(&title, MAC_HEADLINE_PT, Weight::Semibold, label, |im| s.blit(im, tx as i32, ty as i32, 255, alpha));
            self.label(&artist, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), |im| s.blit(im, tx as i32, (ty + ui.pt(18.0)) as i32, 255, alpha));
            if let Some(p) = now_playing::progress().filter(|p| p.duration_ms.is_some_and(|d| d > 0)) {
                let share = (p.position_ms as f32 / p.duration_ms.unwrap_or(1) as f32).clamp(0.0, 1.0);
                let (by, bh) = (ty + ui.pt(36.0), ui.pt(3.0));
                s.fill_rrect(tx, by, tw, bh, bh / 2.0, a(TRACK_C.now()));
                s.fill_rrect(tx, by, tw * share, bh, bh / 2.0, a(label));
            }
        }
        if let Some((tx, ty, tw, tt)) = layout.transport.map(up) {
            let playing = now_playing::current().is_some_and(|x| x.snapshot().state == PlaybackState::Playing) || (now_playing::current().is_none() && self.svc_active());
            let pressed = match self.cc.touch {
                Some(Touch::Tile(k)) => media_target(k),
                _ => None,
            };
            // draw_cc_controls centres the row at 72% of the rect: shift it so the row sits in the middle
            draw_cc_controls(s, ui, (tx, ty + tt * 0.5 - tt * 0.72, tw, tt), playing, a(label), pressed);
        }
        // the list
        for (row, r) in &layout.rows {
            let (rx, ry, rw, rh) = up(*r);
            let (pt, weight, ink) = match row.style {
                RowStyle::Heading => (MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now()),
                RowStyle::Note => (MAC_BODY_PT, Weight::Regular, LABEL_2.now()),
                _ => (MAC_BODY_PT, Weight::Regular, label),
            };
            let text_x = rx + pad / 2.0 + if row.style == RowStyle::Check { ui.pt(20.0) } else { 0.0 };
            let room = rx + rw - text_x - pad / 2.0 - if row.style == RowStyle::Toggle { ui.pt(SWITCH_W_PT + 8.0) } else { 0.0 };
            let text = self.fit_chrome(&row.title, pt, room.max(0.0));
            let h = self.label(&text, pt, weight, ink, |im| im.h) as f32;
            let ty = if row.style == RowStyle::Heading { ry + rh - h - ui.pt(2.0) } else { ry + (rh - h) / 2.0 };
            self.label(&text, pt, weight, ink, |im| s.blit(im, text_x as i32, ty as i32, 255, alpha));
            match row.style {
                RowStyle::Check if row.on => {
                    let ch = self.label("\u{2713}", MAC_BODY_PT, Weight::Semibold, SYSTEM_BLUE, |im| im.h) as f32;
                    self.label("\u{2713}", MAC_BODY_PT, Weight::Semibold, SYSTEM_BLUE, |im| s.blit(im, (rx + pad / 2.0) as i32, (ry + (rh - ch) / 2.0) as i32, 255, alpha));
                }
                RowStyle::Toggle => {
                    let (sw, sh) = (ui.pt(SWITCH_W_PT), ui.pt(SWITCH_H_PT));
                    draw_switch(s, ui, rx + rw - pad / 2.0 - sw, ry + (rh - sh) / 2.0, if row.on { 1.0 } else { 0.0 });
                }
                _ => {}
            }
        }
        // the Settings row
        if let Some((text, _, r)) = layout.footer {
            let (fx, fy, _, fh) = up(r);
            hairline(s, ui, sep, fy - ui.pt(3.0));
            let h = self.label(text, MAC_BODY_PT, Weight::Regular, label, |im| im.h) as f32;
            self.label(text, MAC_BODY_PT, Weight::Regular, label, |im| s.blit(im, (fx + pad / 2.0) as i32, (fy + (fh - h) / 2.0) as i32, 255, alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::desktop_network::Snapshot;
    use crate::shell::tests::{desktop_shell, settle};
    use crate::shell::Network;

    fn net(ssid: &str, saved: bool, secure: bool) -> Network {
        Network { ssid: ssid.into(), bars: 3, secure, saved }
    }

    /// A desktop shell with the panel open, a Wi-Fi adapter on (not connected) and no brightness control.
    fn desk() -> Shell {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.open_desk_control_center(1000.0);
        settle(&mut sh, 1000.0);
        assert!(sh.cc.is_shown(), "cc={}", sh.cc.describe());
        sh.info.desktop_network = Some(Snapshot {
            wifi_available: true,
            wifi_on: true,
            ssid: None,
            networks: vec![net("Home", true, true), net("Cafe", false, false), net("Locked", false, true)],
            ..Default::default()
        });
        sh.cc.backlight = false;
        sh
    }

    fn click(sh: &mut Shell, (x, y): (f32, f32)) -> Vec<Effect> {
        let mut fx = sh.cc_desk_event(Event::Down(x, y), 2000.0);
        fx.extend(sh.cc_desk_event(Event::Up(x, y), 2010.0));
        fx
    }

    fn module(sh: &Shell, tile: Tile) -> R4 {
        sh.cc_desk_tiles().into_iter().find(|(k, _)| *k == tile).map(|(_, r)| r).expect("the module is laid out")
    }

    /// A point on a capsule's label, right of the icon disc.
    fn label_of(sh: &Shell, tile: Tile) -> (f32, f32) {
        let r = module(sh, tile);
        (r.0 + r.2 - sh.ui.pt(20.0), r.1 + r.3 / 2.0)
    }

    fn row_centre(layout: &Layout, title: &str) -> (f32, f32) {
        let (_, (x, y, w, h)) = layout.rows.iter().find(|(row, _)| row.title == title).unwrap_or_else(|| panic!("a row {title}: {:?}", layout.rows));
        (x + w / 2.0, y + h / 2.0)
    }

    /// W5-10: the Wi-Fi disc still switches the radio; the label opens the network list, whose rows join or select a
    /// network, and a click beside the list goes back to the modules.
    #[test]
    fn the_wifi_label_opens_the_network_list_and_the_disc_still_toggles() {
        let mut sh = desk();
        let (dx, dy, d, _) = desk_disc(module(&sh, Tile::Wifi), &sh.ui);
        let fx = click(&mut sh, (dx + d / 2.0, dy + d / 2.0));
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Enable(false))], "the disc toggles the radio");
        assert_eq!(sh.cc.desk_detail, None, "and opens nothing");
        sh.info.desktop_network.as_mut().unwrap().wifi_on = true;

        let at = label_of(&sh, Tile::Wifi);
        assert!(click(&mut sh, at).is_empty(), "the label switches nothing");
        assert_eq!(sh.cc.desk_detail, Some(Detail::Wifi));
        let layout = sh.desk_detail_layout(Detail::Wifi);
        assert!(layout.switch.is_some(), "the radio's switch sits in the title row");
        assert_eq!(layout.footer.map(|f| f.0), Some("Wi-Fi Settings\u{2026}"));
        let fx = click(&mut sh, row_centre(&layout, "Home"));
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Select("Home".into()))], "a saved network is selected");
        let fx = click(&mut sh, row_centre(&layout, "Cafe"));
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Join { ssid: "Cafe".into(), psk: None })], "an open one is joined");
        assert_eq!(sh.cc.desk_detail, Some(Detail::Wifi), "the list stays open after a pick");
        let r = sh.cc_desk_rect().expect("the panel's bounds hold the detail");
        assert!(r.1 + r.3 >= layout.panel.1 + layout.panel.3);

        let (px, py, _, _) = layout.panel;
        let at = (px - sh.ui.pt(20.0), py + sh.ui.pt(10.0));
        assert!(click(&mut sh, at).is_empty());
        assert_eq!(sh.cc.desk_detail, None, "a click beside the detail goes back to the modules");
        assert!(sh.cc.is_shown(), "and keeps the panel open");
    }

    /// W5-10: Focus lists the modes; a row switches its mode on and the same row off again.
    #[test]
    fn the_focus_detail_lists_the_modes_and_a_row_switches_one() {
        let mut sh = desk();
        let at = label_of(&sh, Tile::Focus);
        click(&mut sh, at);
        assert_eq!(sh.cc.desk_detail, Some(Detail::Focus));
        let rows = sh.desk_detail_rows(Detail::Focus);
        assert_eq!(rows.len(), sh.notify.modes.list.len(), "one row per mode");
        let dnd = sh.notify.modes.list.iter().find(|m| m.id == crate::shell::notify::modes::DND_ID).map(|m| m.name.clone()).expect("Do Not Disturb");
        let layout = sh.desk_detail_layout(Detail::Focus);
        click(&mut sh, row_centre(&layout, &dnd));
        assert!(sh.dnd(), "the row switched Do Not Disturb on");
        assert!(sh.desk_detail_rows(Detail::Focus).iter().any(|r| r.title == dnd && r.on), "and it reads checked");
        click(&mut sh, row_centre(&layout, &dnd));
        assert!(!sh.dnd(), "the checked row switches it off");
    }

    /// W5-10: without a brightness control the Display detail has no slider, and offers Dark Mode and Night Shift.
    #[test]
    fn the_display_detail_offers_dark_mode_and_night_shift() {
        let mut sh = desk();
        let at = label_of(&sh, Tile::Brightness);
        click(&mut sh, at);
        assert_eq!(sh.cc.desk_detail, Some(Detail::Display));
        let layout = sh.desk_detail_layout(Detail::Display);
        assert!(layout.slider.is_none(), "nothing to drive, no slider");
        let titles: Vec<&str> = layout.rows.iter().map(|(r, _)| r.title.as_str()).collect();
        assert_eq!(titles, vec!["Dark Mode", "Night Shift"]);
        click(&mut sh, row_centre(&layout, "Night Shift"));
        assert_eq!(sh.prefs.get("night_manual"), Some("1"), "Night Shift is on by hand");
        click(&mut sh, row_centre(&layout, "Night Shift"));
        assert_eq!(sh.prefs.get("night_manual"), Some("0"));
        sh.cc.backlight = true;
        assert!(sh.desk_detail_layout(Detail::Display).slider.is_some(), "with a backlight or a DDC monitor the slider is there");
    }

    /// W5-10: a Bluetooth module without a controller still opens its detail, which says so and has no switch.
    #[test]
    fn bluetooth_without_a_controller_opens_a_detail_that_says_so() {
        let mut sh = desk();
        sh.info.bt.state = BtState::Unavailable;
        let (dx, dy, d, _) = desk_disc(module(&sh, Tile::Bluetooth), &sh.ui);
        assert!(click(&mut sh, (dx + d / 2.0, dy + d / 2.0)).is_empty(), "the disc has nothing to switch");
        assert_eq!(sh.cc.desk_detail, None);
        let at = label_of(&sh, Tile::Bluetooth);
        click(&mut sh, at);
        assert_eq!(sh.cc.desk_detail, Some(Detail::Bluetooth));
        let layout = sh.desk_detail_layout(Detail::Bluetooth);
        assert!(layout.switch.is_none());
        assert_eq!(layout.rows.len(), 1);
        assert_eq!(layout.rows[0].0.style, RowStyle::Note);
    }

    /// W5-11: in the Sound module only the track sets the volume; the whole right end opens the output menu, which
    /// hangs below the module; the title opens the Sound detail with the outputs.
    #[test]
    fn the_sound_module_sets_the_volume_on_the_track_only() {
        let mut sh = desk();
        sh.cc.volume = 0.644;
        let r = module(&sh, Tile::Volume);
        let (kx, ky, _, kh) = route_chevron(r, &sh.ui);
        // QA's press, 5 px left of the chevron: it used to set the volume to 0.914
        let fx = sh.cc_desk_event(Event::Down(kx - 5.0, ky + kh / 2.0), 2000.0);
        assert!(fx.is_empty() && sh.cc.volume == 0.644, "a press beside the chevron is not the slider: {fx:?}");
        sh.cc_desk_event(Event::Up(kx - 5.0, ky + kh / 2.0), 2010.0);
        assert_eq!(sh.cc.desk_detail, Some(Detail::Sound), "it is the module's title: the Sound detail");
        assert!(sh.desk_detail_rows(Detail::Sound).iter().any(|row| row.title == "Speaker" && row.on), "the outputs, the current one checked");
        sh.cc.desk_detail = None;

        // the whole right end, down to the module's bottom corner, opens the output menu
        let at = (r.0 + r.2 - sh.ui.pt(4.0), r.1 + r.3 - sh.ui.pt(4.0));
        let fx = click(&mut sh, at);
        assert!(fx.is_empty() && sh.cc.volume == 0.644);
        let menu = sh.cc.route_menu.as_ref().expect("the output menu opened");
        assert!(menu.panel.y >= r.1 + r.3, "the menu hangs below the module, clear of the slider");
        sh.cc.route_menu = None;

        // the track sets the level
        let (l, rr, cy) = sh.desk_track(r);
        let fx = sh.cc_desk_event(Event::Down((l + rr) / 2.0, cy), 3000.0);
        assert!(matches!(fx.as_slice(), [Effect::Volume(v)] if (v - 0.5).abs() < 0.01), "{fx:?}");
        sh.cc_desk_event(Event::Up((l + rr) / 2.0, cy), 3010.0);
    }

    /// W5-12: with no queue the Now Playing card has no Up Next button (it opened a 1 px sliver); the title opens the
    /// Now Playing detail with the artwork and the transport.
    #[test]
    fn up_next_needs_a_queue_and_the_title_opens_now_playing() {
        let mut sh = desk();
        let body = r#"{"title":"Memo","duration_ms":26000,"state":"paused","position_ms":0,"commands":{"play":"enabled","pause":"enabled"}}"#;
        sh.service_call("is.a.memos", "now_playing", "publish", body, 0.0).expect("published");
        let r = module(&sh, Tile::MediaPlay);
        let (bx, by, bw, bh) = sh.up_next_button(r);
        click(&mut sh, (bx + bw / 2.0, by + bh / 2.0));
        assert!(sh.cc.up_next.is_none(), "no queue, no Up Next picker");
        // the press landed on the title area: it opened the detail
        assert_eq!(sh.cc.desk_detail, Some(Detail::NowPlaying));
        let layout = sh.desk_detail_layout(Detail::NowPlaying);
        assert!(layout.art.is_some() && layout.transport.is_some() && layout.footer.is_none());
        sh.cc.desk_detail = None;

        let queued = r#"{"title":"Memo","duration_ms":26000,"state":"paused","position_ms":0,"queue":[{"title":"Next"}],"commands":{"play":"enabled"}}"#;
        sh.service_call("is.a.memos", "now_playing", "publish", queued, 0.0).expect("published");
        sh.cc_desk_event(Event::Down(bx + bw / 2.0, by + bh / 2.0), 3000.0);
        assert!(sh.cc.up_next.as_ref().is_some_and(|u| u.items.len() == 1), "with a queue the picker opens");
        sh.service_call("is.a.memos", "now_playing", "clear", "", 0.0).expect("cleared");
    }

    /// W5-10: "Not Connected" fits the Wi-Fi capsule (it read "Not Conne..."), at subhead or footnote size.
    #[test]
    fn not_connected_fits_the_wifi_capsule() {
        use crate::shell::ui::{MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT};
        let sh = desk();
        let r = module(&sh, Tile::Wifi);
        let (_, room) = desk_capsule_text(r, &sh.ui);
        let pt = sh.capsule_sub_pt("Not Connected", room);
        assert!(pt == MAC_SUBHEAD_PT || pt == MAC_FOOTNOTE_PT);
        assert!(sh.measure_chrome("Not Connected", pt, Weight::Regular) <= room, "room {room}");
        assert_eq!(sh.fit_chrome("Not Connected", pt, room), "Not Connected");
    }

    /// W5-10: in Light the modules are glass the desktop shows through, lighter than a menu.
    #[test]
    fn the_module_tint_is_lighter_than_a_menu() {
        use crate::shell::ui::{CC_MODULE_TINT, MENU_TINT};
        assert!(CC_MODULE_TINT.light >> 24 < MENU_TINT.light >> 24);
        assert!(CC_MODULE_TINT.light >> 24 >= 100, "but still a plate the labels read on");
    }
}
