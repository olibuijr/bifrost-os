//! Screen Sharing: the viewer for VNC (RFB) servers. Type an address, connect, and the remote desktop shows in the
//! window; pointer and keyboard go to it unless View Only is on. The clipboard moves both ways through the shell's
//! pasteboard. This computer's own server is the Remote app, switched on in Settings > Sharing.

mod connection;
mod view;

use bifrost_app::kit::{pack, Damage, Image, Surface, Ui};
use bifrost_app::layout::Layout;
use bifrost_app::pasteboard_types::{self as pasteboard, Board, Item};
use bifrost_app::proto::CHORD_FLAG;
use bifrost_app::widgets;
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use bifrost_rfb::keysym;
use connection::{lock, Connection, Status};
use springboard::shell::ui::{Weight, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_TITLE2_PT, TINT, WHITE};
use std::sync::atomic::Ordering;
use view::{Area, View};

/// Window layout in points, below the 52 pt band that holds the traffic lights and the large title.
const MARGIN: f32 = 16.;
const FIELD_Y: f32 = 112.;
const FIELD_W: f32 = 420.;
const BUTTON_W: f32 = 120.;
const ROW_H: f32 = 34.;
const CHIP_Y: f32 = 160.;
const CHIP_H: f32 = 30.;
const STATUS_Y: f32 = 204.;
const AREA_Y: f32 = 240.;
/// The mouse button bits of a PointerEvent.
const BUTTON_LEFT: u8 = 1;
const BUTTON_RIGHT: u8 = 4;
/// Cache key for the scaled desktop: generation, origin and scale bits, then the visible device rectangle.
type CacheKey = (u64, u32, u32, u32, i32, i32, usize, usize);

struct ScreenSharing {
    address: String,
    conn: Option<Connection>,
    /// The status line under the controls: connection progress, the reason it ended, or the last clipboard result.
    message: String,
    fit: bool,
    view_only: bool,
    /// The left button is held down on the remote desktop (a drag is in progress).
    pressed: bool,
    /// The window size in points, from the last draw.
    window: (f32, f32),
    /// Left edge of the content column in points, from the last draw.
    col: f32,
    paste_req: Option<u32>,
    /// The last generation drawn, so `tick` knows when a new frame has arrived.
    drawn: u64,
    cache: Option<(CacheKey, Image)>,
}

/// Whether a point (in points) lies in the box (`x`, `y`, `w`, `h`).
fn hit(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn chip(s: &mut Surface, u: &Ui, x: f32, w: f32, label: &str, on: bool, enabled: bool) {
    let fill = if on { TINT } else { FIELD_BG.now() };
    let color = if on {
        WHITE
    } else if enabled {
        LABEL.now()
    } else {
        LABEL_2.now()
    };
    s.fill_rrect(u.pt(x), u.pt(CHIP_Y), u.pt(w), u.pt(CHIP_H), u.pt(8.), fill);
    s.blit(&u.text(label, MAC_BODY_PT, Weight::Regular, color), u.pt(x + 12.) as i32, u.pt(CHIP_Y + 7.) as i32, 255, 255);
}

impl ScreenSharing {
    fn area(&self) -> Area {
        Area { x: MARGIN, y: AREA_Y, w: (self.window.0 - 2. * MARGIN).max(0.), h: (self.window.1 - AREA_Y - MARGIN).max(0.) }
    }

    /// The remote desktop's size in pixels, once a connection has a framebuffer.
    fn desktop_size(&self) -> Option<(usize, usize)> {
        let c = self.conn.as_ref()?;
        let fb = lock(&c.shared.frame);
        if fb.w == 0 || fb.h == 0 {
            None
        } else {
            Some((fb.w, fb.h))
        }
    }

    fn placement(&self) -> Option<(View, usize, usize)> {
        let (w, h) = self.desktop_size()?;
        Some((view::place(w, h, self.area(), self.fit), w, h))
    }

    fn connect(&mut self) {
        let addr = self.address.trim().to_string();
        if addr.is_empty() {
            self.message = "Type the address of the computer to view.".to_string();
            return;
        }
        // Dropping the old connection closes its socket.
        self.conn = None;
        self.conn = Some(Connection::open(&addr));
        self.pressed = false;
        self.drawn = 0;
        self.cache = None;
        self.message = format!("Connecting to {addr}...");
    }

    fn disconnect(&mut self) {
        self.conn = None;
        self.pressed = false;
        self.cache = None;
        self.message = "Disconnected.".to_string();
    }

    fn request_paste(&mut self, ctx: &Ctx) {
        self.paste_req = Some(Board::general().string(ctx, pasteboard::STRING));
        self.message = "Sending the clipboard...".to_string();
    }

    /// Handles a touch. Controls answer on release; the desktop gets a pointer stream while connected.
    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let col = self.col;
        let connected = self.conn.is_some();
        if matches!(kind, TouchKind::Up) {
            if hit(x, y, col + FIELD_W + 16., FIELD_Y, BUTTON_W, ROW_H) {
                if connected {
                    self.disconnect();
                } else {
                    self.connect();
                }
                return true;
            }
            if hit(x, y, col, CHIP_Y, 140., CHIP_H) {
                self.fit = !self.fit;
                self.cache = None;
                return true;
            }
            if hit(x, y, col + 152., CHIP_Y, 130., CHIP_H) {
                self.view_only = !self.view_only;
                self.pressed = false;
                return true;
            }
            if hit(x, y, col + 294., CHIP_Y, 170., CHIP_H) {
                if connected {
                    self.request_paste(ctx);
                    return true;
                }
                return false;
            }
        }
        if self.view_only || (!self.area().contains(x, y) && !self.pressed) {
            return false;
        }
        let Some((v, fw, fh)) = self.placement() else { return false };
        let (rx, ry) = view::remote_point(v, fw, fh, x, y);
        let Some(c) = self.conn.as_ref() else { return false };
        match kind {
            TouchKind::Down => {
                self.pressed = true;
                c.pointer(BUTTON_LEFT, rx, ry);
            }
            TouchKind::Move => {
                if self.pressed {
                    c.pointer(BUTTON_LEFT, rx, ry);
                }
            }
            TouchKind::Up | TouchKind::Cancel => {
                if self.pressed {
                    self.pressed = false;
                    c.pointer(0, rx, ry);
                }
            }
        }
        true
    }

    /// Draws the remote desktop into the area, scaled and cropped to what is visible. The scaled copy is rebuilt only
    /// when a new frame or a new placement arrives.
    fn draw_desktop(&mut self, u: &Ui, s: &mut Surface) {
        let Some((v, fw, fh)) = self.placement() else { return };
        let Some(c) = self.conn.as_ref() else { return };
        let gen = c.shared.generation.load(Ordering::SeqCst);
        let ds = v.scale * u.s;
        let (ox, oy) = (v.x * u.s, v.y * u.s);
        let a = self.area();
        let area_px = Area { x: a.x * u.s, y: a.y * u.s, w: a.w * u.s, h: a.h * u.s };
        if let Some(rect) = view::visible(fw, fh, ox, oy, ds, area_px) {
            let key = (gen, ox.to_bits(), oy.to_bits(), ds.to_bits(), rect.0, rect.1, rect.2, rect.3);
            if self.cache.as_ref().map(|(k, _)| *k) != Some(key) {
                let px = view::sample(&lock(&c.shared.frame), ox, oy, ds, rect);
                let px: Vec<u32> = px.into_iter().map(|p| pack(((p >> 16) & 0xff) as u8, ((p >> 8) & 0xff) as u8, (p & 0xff) as u8, 255)).collect();
                self.cache = Some((key, Image { w: rect.2, h: rect.3, px }));
            }
            if let Some((_, img)) = &self.cache {
                s.blit(img, rect.0, rect.1, 255, 255);
            }
        }
        self.drawn = gen;
    }
}

impl App for ScreenSharing {
    fn id(&self) -> &str {
        "is.olibuijr.screensharing.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination();
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let s = ctx.ui.s;
                self.touch(ctx, kind, x / s, y / s)
            }
            // A secondary click on the desktop: press and release the right button at the point.
            Input::Context { x, y } => {
                let s = ctx.ui.s;
                let (x, y) = (x / s, y / s);
                if self.view_only || !self.area().contains(x, y) {
                    return false;
                }
                let Some((v, fw, fh)) = self.placement() else { return false };
                let (rx, ry) = view::remote_point(v, fw, fh, x, y);
                let Some(c) = self.conn.as_ref() else { return false };
                c.pointer(BUTTON_RIGHT, rx, ry);
                c.pointer(0, rx, ry);
                true
            }
            Input::Key(KeyInput::Text(t)) => {
                if let Some(c) = self.conn.as_ref() {
                    if self.view_only {
                        return false;
                    }
                    for ch in t.chars() {
                        let sym = keysym::from_char(ch);
                        c.key(true, sym);
                        c.key(false, sym);
                    }
                    return true;
                }
                // Not connected: typing fills in the address.
                let before = self.address.len();
                self.address.extend(t.chars().filter(|c| !c.is_control()));
                self.address.len() != before
            }
            Input::Key(KeyInput::Code(code)) => {
                if code & CHORD_FLAG != 0 {
                    return false;
                }
                if let Some(c) = self.conn.as_ref() {
                    if self.view_only {
                        return false;
                    }
                    return match keysym::from_code(code) {
                        Some(sym) => {
                            c.key(true, sym);
                            c.key(false, sym);
                            true
                        }
                        None => false,
                    };
                }
                match code {
                    13 => {
                        self.connect();
                        true
                    }
                    8 => self.address.pop().is_some(),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, _t: f64) -> bool {
        let mut changed = false;
        let mut ended: Option<String> = None;
        let mut texts = Vec::new();
        if let Some(c) = self.conn.as_ref() {
            while let Ok(t) = c.cut_text.try_recv() {
                texts.push(t);
            }
            match lock(&c.shared.status).clone() {
                Status::Connecting => {}
                Status::Connected { name } => {
                    if self.message.starts_with("Connecting") {
                        self.message = format!("Connected to {name}.");
                        changed = true;
                    }
                }
                Status::Closed(reason) => ended = Some(reason),
            }
            if c.shared.generation.load(Ordering::SeqCst) != self.drawn {
                changed = true;
            }
        }
        // The server's clipboard goes to the shell's clipboard, so the next paste anywhere gets it.
        for t in texts {
            Board::general().write(ctx, &[Item::default().with(pasteboard::STRING, t.as_bytes())]);
            self.message = "Copied the remote clipboard.".to_string();
            changed = true;
        }
        if let Some(reason) = ended {
            self.conn = None;
            self.pressed = false;
            self.cache = None;
            self.message = reason;
            changed = true;
        }
        changed
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        if self.paste_req != Some(request) {
            return false;
        }
        self.paste_req = None;
        match pasteboard::decode_string(result) {
            Some(text) if !text.is_empty() => {
                if let Some(c) = self.conn.as_ref() {
                    c.paste(&text);
                }
                self.message = "Sent the clipboard to the remote computer.".to_string();
            }
            _ => self.message = "The clipboard has no text to send.".to_string(),
        }
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let u = &ctx.ui;
        s.fill_rect(0, 0, s.w, s.h, DETAIL_BG.now());
        self.window = (s.w as f32 / u.pt(1.), s.h as f32 / u.pt(1.));
        let l = Layout::from_points(self.window.0, self.window.1, 0., 0.);
        let (x, _) = l.readable();
        self.col = x;
        widgets::large_title(s, u, &l, "Screen Sharing", 64.);

        // The address field, the Connect or Disconnect button, then the three option chips.
        let (text, color) = if self.address.is_empty() { ("Address, such as 192.168.1.20", LABEL_2.now()) } else { (self.address.as_str(), LABEL.now()) };
        s.fill_rrect(u.pt(x), u.pt(FIELD_Y), u.pt(FIELD_W), u.pt(ROW_H), u.pt(9.), FIELD_BG.now());
        s.blit(&u.text(text, MAC_TITLE2_PT, Weight::Regular, color), u.pt(x + 12.) as i32, u.pt(FIELD_Y + 6.) as i32, 255, 255);
        let connected = self.conn.is_some();
        let bx = x + FIELD_W + 16.;
        s.fill_rrect(u.pt(bx), u.pt(FIELD_Y), u.pt(BUTTON_W), u.pt(ROW_H), u.pt(9.), if connected { FIELD_BG.now() } else { TINT });
        let (label, label_color) = if connected { ("Disconnect", LABEL.now()) } else { ("Connect", WHITE) };
        s.blit(&u.text(label, MAC_TITLE2_PT, Weight::Regular, label_color), u.pt(bx + 18.) as i32, u.pt(FIELD_Y + 6.) as i32, 255, 255);
        chip(s, u, x, 140., "Scale to Fit", self.fit, true);
        chip(s, u, x + 152., 130., "View Only", self.view_only, true);
        chip(s, u, x + 294., 170., "Send Clipboard", false, connected);

        s.blit(&u.text(&self.message, MAC_BODY_PT, Weight::Regular, LABEL_2.now()), u.pt(x) as i32, u.pt(STATUS_Y) as i32, 255, 255);

        // The desktop area: black around the remote screen, which is drawn only while connected.
        let a = self.area();
        s.fill_rrect(u.pt(a.x), u.pt(a.y), u.pt(a.w), u.pt(a.h), u.pt(10.), DETAIL_BG.now());
        self.draw_desktop(u, s);

        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ScreenSharing {
        address: String::new(),
        conn: None,
        message: "Type the address of the computer to view, then Connect.".to_string(),
        fit: true,
        view_only: false,
        pressed: false,
        window: (1200., 780.),
        col: 32.,
        paste_req: None,
        drawn: 0,
        cache: None,
    });
}
