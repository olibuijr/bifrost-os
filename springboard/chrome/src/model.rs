//! The chrome model: what the shell tells the menu bar, Dock and panels to show, and what they tell the shell back.
//! Plain data with a compact binary codec (`encode`/`decode`, owner `chrome-core`, hostile-input safe like
//! `bifrost_layers::codec`). Add fields and variants only at the end (the codec carries a version byte).

/// One menu item. A separator has an empty `label`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    /// Shortcut as the menu shows it ("cmd+s"), empty for none.
    pub shortcut: String,
    pub enabled: bool,
    pub checked: bool,
    pub submenu: Vec<MenuItem>,
}

/// A top-level menu of the menu bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub id: u32,
    pub title: String,
    pub items: Vec<MenuItem>,
}

/// A status item on the right of the menu bar (clock, Wi-Fi, Now Playing, input source ...).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusItem {
    pub id: u32,
    /// A symbol name from the shell's symbol set, or empty.
    pub symbol: String,
    pub text: String,
    pub menu: Vec<MenuItem>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuBarModel {
    /// The front app's name (bold, after the system menu).
    pub app_name: String,
    pub menus: Vec<Menu>,
    pub status: Vec<StatusItem>,
}

/// One Dock item.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DockItem {
    pub app_id: String,
    pub title: String,
    /// Path of the icon image the shell resolved (PNG or the shell's raw icon), read by the Dock client.
    pub icon_path: String,
    pub running: bool,
    pub active: bool,
    pub badge: String,
    /// Launch or download progress 0..1, negative for none.
    pub progress: f32,
    /// Kept in the Dock (the context menu offers Remove from Dock). Added after v1: the codec carries it at the end.
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockPosition {
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DockModel {
    pub items: Vec<DockItem>,
    /// Icon size in points and the magnified size (0 disables magnification).
    pub size: f32,
    pub magnified: f32,
    pub position: DockPosition,
    pub autohide: bool,
}

/// A notification as Notification Center shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notification {
    pub id: u32,
    pub app_id: String,
    pub title: String,
    pub body: String,
    pub time: String,
    pub actions: Vec<(u32, String)>,
}

/// A Control Center module.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlModule {
    pub id: u32,
    pub title: String,
    pub symbol: String,
    pub on: bool,
    /// A slider value 0..1, negative for a toggle.
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PanelsModel {
    pub notifications: Vec<Notification>,
    pub controls: Vec<ControlModule>,
}

/// Shell to chrome.
#[derive(Debug, Clone, PartialEq)]
pub enum ToChrome {
    MenuBar(MenuBarModel),
    Dock(DockModel),
    Panels(PanelsModel),
    Appearance { dark: bool, accent: u32, reduce_motion: bool, reduce_transparency: bool },
    /// Open a panel ("notifications", "control", "search") or close it ("").
    ShowPanel(String),
}

/// Chrome to shell.
#[derive(Debug, Clone, PartialEq)]
pub enum FromChrome {
    /// A menu bar item was chosen: the menu id and the item id (status items use their own id as the menu).
    MenuAction { menu: u32, item: u32 },
    DockActivate { app_id: String },
    /// The Dock's context menu asked for an action ("quit", "keep", "remove", "show").
    DockCommand { app_id: String, command: String },
    /// Where an app's Dock icon is, in output logical pixels, for the window server's minimize animation.
    MinimizeTarget { app_id: String, rect: [f32; 4] },
    ControlChanged { id: u32, on: bool, value: f32 },
    NotificationAction { id: u32, action: u32 },
    /// The client is up and wants the current models.
    Hello { client: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    BadVersion,
    UnknownTag(u8),
    BadValue,
    TooLong,
}

/// The largest accepted message.
pub const MAX_LEN: usize = 4 << 20;

/// The codec's version byte, first in every message.
pub const VERSION: u8 = 1;

/// Nesting depth allowed for submenus (a hostile message cannot recurse deeper).
const MAX_DEPTH: u32 = 8;

const TO_MENU_BAR: u8 = 1;
const TO_DOCK: u8 = 2;
const TO_PANELS: u8 = 3;
const TO_APPEARANCE: u8 = 4;
const TO_SHOW_PANEL: u8 = 5;

const FROM_MENU_ACTION: u8 = 1;
const FROM_DOCK_ACTIVATE: u8 = 2;
const FROM_DOCK_COMMAND: u8 = 3;
const FROM_MINIMIZE_TARGET: u8 = 4;
const FROM_CONTROL_CHANGED: u8 = 5;
const FROM_NOTIFICATION_ACTION: u8 = 6;
const FROM_HELLO: u8 = 7;

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn new(tag: u8) -> Writer {
        Writer { out: vec![VERSION, tag] }
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.out.extend_from_slice(s.as_bytes());
    }
    fn count(&mut self, n: usize) {
        self.u32(n as u32);
    }
    fn menu_items(&mut self, items: &[MenuItem]) {
        self.count(items.len());
        for it in items {
            self.menu_item(it);
        }
    }
    fn menu_item(&mut self, it: &MenuItem) {
        self.u32(it.id);
        self.str(&it.label);
        self.str(&it.shortcut);
        self.bool(it.enabled);
        self.bool(it.checked);
        self.menu_items(&it.submenu);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        if end > self.b.len() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        let v = f32::from_bits(self.u32()?);
        if v.is_nan() {
            return Err(DecodeError::BadValue);
        }
        Ok(v)
    }
    fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let n = self.u32()? as usize;
        if n > MAX_LEN {
            return Err(DecodeError::TooLong);
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::BadValue)
    }
    /// A count of elements that each take at least one byte, so it cannot exceed what is left.
    fn count(&mut self) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > self.b.len() - self.pos {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }
    fn menu_items(&mut self, depth: u32) -> Result<Vec<MenuItem>, DecodeError> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.menu_item(depth)?);
        }
        Ok(v)
    }
    fn menu_item(&mut self, depth: u32) -> Result<MenuItem, DecodeError> {
        if depth >= MAX_DEPTH {
            return Err(DecodeError::BadValue);
        }
        Ok(MenuItem {
            id: self.u32()?,
            label: self.str()?,
            shortcut: self.str()?,
            enabled: self.bool()?,
            checked: self.bool()?,
            submenu: self.menu_items(depth + 1)?,
        })
    }
    fn menus(&mut self) -> Result<Vec<Menu>, DecodeError> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(Menu { id: self.u32()?, title: self.str()?, items: self.menu_items(0)? });
        }
        Ok(v)
    }
    fn status(&mut self) -> Result<Vec<StatusItem>, DecodeError> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(StatusItem { id: self.u32()?, symbol: self.str()?, text: self.str()?, menu: self.menu_items(0)? });
        }
        Ok(v)
    }
    fn finish(&self) -> Result<(), DecodeError> {
        if self.pos == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::BadValue)
        }
    }
}

fn reader(b: &[u8]) -> Result<Reader<'_>, DecodeError> {
    if b.len() > MAX_LEN {
        return Err(DecodeError::TooLong);
    }
    let mut r = Reader { b, pos: 0 };
    if r.u8()? != VERSION {
        return Err(DecodeError::BadVersion);
    }
    Ok(r)
}

fn dock_position_tag(p: DockPosition) -> u8 {
    match p {
        DockPosition::Bottom => 0,
        DockPosition::Left => 1,
        DockPosition::Right => 2,
    }
}

pub fn encode_to_chrome(m: &ToChrome) -> Vec<u8> {
    match m {
        ToChrome::MenuBar(mb) => {
            let mut w = Writer::new(TO_MENU_BAR);
            w.str(&mb.app_name);
            w.count(mb.menus.len());
            for menu in &mb.menus {
                w.u32(menu.id);
                w.str(&menu.title);
                w.menu_items(&menu.items);
            }
            w.count(mb.status.len());
            for s in &mb.status {
                w.u32(s.id);
                w.str(&s.symbol);
                w.str(&s.text);
                w.menu_items(&s.menu);
            }
            w.out
        }
        ToChrome::Dock(d) => {
            let mut w = Writer::new(TO_DOCK);
            w.count(d.items.len());
            for it in &d.items {
                w.str(&it.app_id);
                w.str(&it.title);
                w.str(&it.icon_path);
                w.bool(it.running);
                w.bool(it.active);
                w.str(&it.badge);
                w.f32(it.progress);
            }
            w.f32(d.size);
            w.f32(d.magnified);
            w.u8(dock_position_tag(d.position));
            w.bool(d.autohide);
            w.out
        }
        ToChrome::Panels(p) => {
            let mut w = Writer::new(TO_PANELS);
            w.count(p.notifications.len());
            for n in &p.notifications {
                w.u32(n.id);
                w.str(&n.app_id);
                w.str(&n.title);
                w.str(&n.body);
                w.str(&n.time);
                w.count(n.actions.len());
                for (aid, label) in &n.actions {
                    w.u32(*aid);
                    w.str(label);
                }
            }
            w.count(p.controls.len());
            for c in &p.controls {
                w.u32(c.id);
                w.str(&c.title);
                w.str(&c.symbol);
                w.bool(c.on);
                w.f32(c.value);
            }
            w.out
        }
        ToChrome::Appearance { dark, accent, reduce_motion, reduce_transparency } => {
            let mut w = Writer::new(TO_APPEARANCE);
            w.bool(*dark);
            w.u32(*accent);
            w.bool(*reduce_motion);
            w.bool(*reduce_transparency);
            w.out
        }
        ToChrome::ShowPanel(name) => {
            let mut w = Writer::new(TO_SHOW_PANEL);
            w.str(name);
            w.out
        }
    }
}

pub fn decode_to_chrome(b: &[u8]) -> Result<ToChrome, DecodeError> {
    let mut r = reader(b)?;
    let m = match r.u8()? {
        TO_MENU_BAR => {
            let app_name = r.str()?;
            let menus = r.menus()?;
            let status = r.status()?;
            ToChrome::MenuBar(MenuBarModel { app_name, menus, status })
        }
        TO_DOCK => {
            let n = r.count()?;
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(DockItem {
                    app_id: r.str()?,
                    title: r.str()?,
                    icon_path: r.str()?,
                    running: r.bool()?,
                    active: r.bool()?,
                    badge: r.str()?,
                    progress: r.f32()?,
                });
            }
            let size = r.f32()?;
            let magnified = r.f32()?;
            let position = match r.u8()? {
                0 => DockPosition::Bottom,
                1 => DockPosition::Left,
                2 => DockPosition::Right,
                t => return Err(DecodeError::UnknownTag(t)),
            };
            let autohide = r.bool()?;
            ToChrome::Dock(DockModel { items, size, magnified, position, autohide })
        }
        TO_PANELS => {
            let n = r.count()?;
            let mut notifications = Vec::with_capacity(n);
            for _ in 0..n {
                let id = r.u32()?;
                let app_id = r.str()?;
                let title = r.str()?;
                let body = r.str()?;
                let time = r.str()?;
                let na = r.count()?;
                let mut actions = Vec::with_capacity(na);
                for _ in 0..na {
                    actions.push((r.u32()?, r.str()?));
                }
                notifications.push(Notification { id, app_id, title, body, time, actions });
            }
            let nc = r.count()?;
            let mut controls = Vec::with_capacity(nc);
            for _ in 0..nc {
                controls.push(ControlModule { id: r.u32()?, title: r.str()?, symbol: r.str()?, on: r.bool()?, value: r.f32()? });
            }
            ToChrome::Panels(PanelsModel { notifications, controls })
        }
        TO_APPEARANCE => ToChrome::Appearance {
            dark: r.bool()?,
            accent: r.u32()?,
            reduce_motion: r.bool()?,
            reduce_transparency: r.bool()?,
        },
        TO_SHOW_PANEL => ToChrome::ShowPanel(r.str()?),
        t => return Err(DecodeError::UnknownTag(t)),
    };
    r.finish()?;
    Ok(m)
}

pub fn encode_from_chrome(m: &FromChrome) -> Vec<u8> {
    match m {
        FromChrome::MenuAction { menu, item } => {
            let mut w = Writer::new(FROM_MENU_ACTION);
            w.u32(*menu);
            w.u32(*item);
            w.out
        }
        FromChrome::DockActivate { app_id } => {
            let mut w = Writer::new(FROM_DOCK_ACTIVATE);
            w.str(app_id);
            w.out
        }
        FromChrome::DockCommand { app_id, command } => {
            let mut w = Writer::new(FROM_DOCK_COMMAND);
            w.str(app_id);
            w.str(command);
            w.out
        }
        FromChrome::MinimizeTarget { app_id, rect } => {
            let mut w = Writer::new(FROM_MINIMIZE_TARGET);
            w.str(app_id);
            for v in rect {
                w.f32(*v);
            }
            w.out
        }
        FromChrome::ControlChanged { id, on, value } => {
            let mut w = Writer::new(FROM_CONTROL_CHANGED);
            w.u32(*id);
            w.bool(*on);
            w.f32(*value);
            w.out
        }
        FromChrome::NotificationAction { id, action } => {
            let mut w = Writer::new(FROM_NOTIFICATION_ACTION);
            w.u32(*id);
            w.u32(*action);
            w.out
        }
        FromChrome::Hello { client } => {
            let mut w = Writer::new(FROM_HELLO);
            w.str(client);
            w.out
        }
    }
}

pub fn decode_from_chrome(b: &[u8]) -> Result<FromChrome, DecodeError> {
    let mut r = reader(b)?;
    let m = match r.u8()? {
        FROM_MENU_ACTION => FromChrome::MenuAction { menu: r.u32()?, item: r.u32()? },
        FROM_DOCK_ACTIVATE => FromChrome::DockActivate { app_id: r.str()? },
        FROM_DOCK_COMMAND => FromChrome::DockCommand { app_id: r.str()?, command: r.str()? },
        FROM_MINIMIZE_TARGET => {
            let app_id = r.str()?;
            let rect = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
            FromChrome::MinimizeTarget { app_id, rect }
        }
        FROM_CONTROL_CHANGED => FromChrome::ControlChanged { id: r.u32()?, on: r.bool()?, value: r.f32()? },
        FROM_NOTIFICATION_ACTION => FromChrome::NotificationAction { id: r.u32()?, action: r.u32()? },
        FROM_HELLO => FromChrome::Hello { client: r.str()? },
        t => return Err(DecodeError::UnknownTag(t)),
    };
    r.finish()?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, label: &str, submenu: Vec<MenuItem>) -> MenuItem {
        MenuItem { id, label: label.into(), shortcut: "cmd+s".into(), enabled: true, checked: id % 2 == 0, submenu }
    }

    fn sample_to_chrome() -> Vec<ToChrome> {
        vec![
            ToChrome::MenuBar(MenuBarModel {
                app_name: "Files".into(),
                menus: vec![Menu {
                    id: 1,
                    title: "File".into(),
                    items: vec![item(10, "New", vec![]), item(11, "", vec![item(12, "Deep", vec![])])],
                }],
                status: vec![StatusItem { id: 90, symbol: "wifi".into(), text: "".into(), menu: vec![] }],
            }),
            ToChrome::Dock(DockModel {
                items: vec![DockItem {
                    app_id: "settings".into(),
                    title: "Settings".into(),
                    icon_path: "/x/icon.png".into(),
                    running: true,
                    active: false,
                    badge: "3".into(),
                    progress: 0.5,
                }],
                size: 48.0,
                magnified: 72.0,
                position: DockPosition::Left,
                autohide: true,
            }),
            ToChrome::Panels(PanelsModel {
                notifications: vec![Notification {
                    id: 7,
                    app_id: "mail".into(),
                    title: "Hi".into(),
                    body: "Body text".into(),
                    time: "09:41".into(),
                    actions: vec![(1, "Reply".into())],
                }],
                controls: vec![ControlModule { id: 2, title: "Wi-Fi".into(), symbol: "wifi".into(), on: true, value: -1.0 }],
            }),
            ToChrome::Appearance { dark: true, accent: 0x0a84ff, reduce_motion: false, reduce_transparency: true },
            ToChrome::ShowPanel("control".into()),
        ]
    }

    fn sample_from_chrome() -> Vec<FromChrome> {
        vec![
            FromChrome::MenuAction { menu: 1, item: 10 },
            FromChrome::DockActivate { app_id: "terminal".into() },
            FromChrome::DockCommand { app_id: "terminal".into(), command: "keep".into() },
            FromChrome::MinimizeTarget { app_id: "files".into(), rect: [10.0, 900.5, 64.0, 64.0] },
            FromChrome::ControlChanged { id: 3, on: false, value: 0.25 },
            FromChrome::NotificationAction { id: 7, action: 1 },
            FromChrome::Hello { client: "chrome-dock".into() },
        ]
    }

    #[test]
    fn every_to_chrome_variant_round_trips() {
        for m in sample_to_chrome() {
            let bytes = encode_to_chrome(&m);
            assert_eq!(bytes[0], VERSION);
            assert_eq!(decode_to_chrome(&bytes), Ok(m));
        }
    }

    #[test]
    fn every_from_chrome_variant_round_trips() {
        for m in sample_from_chrome() {
            let bytes = encode_from_chrome(&m);
            assert_eq!(decode_from_chrome(&bytes), Ok(m));
        }
    }

    #[test]
    fn empty_models_round_trip_and_bytes_are_stable() {
        let empty = ToChrome::Panels(PanelsModel::default());
        let bytes = encode_to_chrome(&empty);
        // Version, tag, then two u32 counts (notifications, controls), both zero.
        assert_eq!(bytes, vec![VERSION, TO_PANELS, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(decode_to_chrome(&bytes), Ok(empty));
        let hello = encode_from_chrome(&FromChrome::Hello { client: "x".into() });
        assert_eq!(hello, vec![VERSION, FROM_HELLO, 1, 0, 0, 0, b'x']);
    }

    #[test]
    fn every_truncation_is_rejected_without_panic() {
        for m in sample_to_chrome() {
            let bytes = encode_to_chrome(&m);
            for cut in 0..bytes.len() {
                assert!(decode_to_chrome(&bytes[..cut]).is_err(), "prefix {cut} of {m:?} decoded");
            }
        }
        for m in sample_from_chrome() {
            let bytes = encode_from_chrome(&m);
            for cut in 0..bytes.len() {
                assert!(decode_from_chrome(&bytes[..cut]).is_err(), "prefix {cut} of {m:?} decoded");
            }
        }
    }

    #[test]
    fn bad_version_unknown_tag_and_trailing_bytes_are_rejected() {
        assert_eq!(decode_to_chrome(&[2, TO_SHOW_PANEL, 0, 0, 0, 0]), Err(DecodeError::BadVersion));
        assert_eq!(decode_to_chrome(&[VERSION, 99]), Err(DecodeError::UnknownTag(99)));
        assert_eq!(decode_from_chrome(&[VERSION, 99]), Err(DecodeError::UnknownTag(99)));
        let mut bytes = encode_from_chrome(&FromChrome::NotificationAction { id: 1, action: 2 });
        bytes.push(0);
        assert_eq!(decode_from_chrome(&bytes), Err(DecodeError::BadValue));
        assert_eq!(decode_to_chrome(&[]), Err(DecodeError::Truncated));
    }

    #[test]
    fn hostile_values_are_rejected() {
        // A bool byte of 2 (version, tag, id: 4 bytes, then the bool).
        let mut bytes = encode_from_chrome(&FromChrome::ControlChanged { id: 1, on: true, value: 0.0 });
        bytes[6] = 2;
        assert_eq!(decode_from_chrome(&bytes), Err(DecodeError::BadValue));
        // A NaN float after the bool.
        let mut nan = encode_from_chrome(&FromChrome::ControlChanged { id: 1, on: true, value: 0.0 });
        nan[7..11].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert_eq!(decode_from_chrome(&nan), Err(DecodeError::BadValue));
        // Invalid UTF-8 in a string.
        assert_eq!(decode_from_chrome(&[VERSION, FROM_HELLO, 1, 0, 0, 0, 0xff]), Err(DecodeError::BadValue));
        // A dock position of 7 (the autohide bool follows it, so it sits two bytes from the end).
        let mut bad_pos = encode_to_chrome(&ToChrome::Dock(DockModel::default()));
        let pos_index = bad_pos.len() - 2;
        bad_pos[pos_index] = 7;
        assert_eq!(decode_to_chrome(&bad_pos), Err(DecodeError::UnknownTag(7)));
    }

    #[test]
    fn huge_lengths_do_not_allocate() {
        // A string claiming 4 GiB.
        assert_eq!(decode_from_chrome(&[VERSION, FROM_HELLO, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError::TooLong));
        // A string claiming 3 MiB in a message that is only a few bytes.
        let claim = (3u32 << 20).to_le_bytes();
        let bytes = [VERSION, FROM_HELLO, claim[0], claim[1], claim[2], claim[3]];
        assert_eq!(decode_from_chrome(&bytes), Err(DecodeError::Truncated));
        // A menu list claiming a million entries in a short message.
        let count = 1_000_000u32.to_le_bytes();
        let bytes = [VERSION, TO_MENU_BAR, 0, 0, 0, 0, count[0], count[1], count[2], count[3]];
        assert_eq!(decode_to_chrome(&bytes), Err(DecodeError::Truncated));
        // Oversized messages are refused before parsing.
        let big = vec![VERSION; MAX_LEN + 1];
        assert_eq!(decode_to_chrome(&big), Err(DecodeError::TooLong));
    }

    #[test]
    fn submenu_depth_is_limited() {
        let mut deep = item(0, "leaf", vec![]);
        for depth in 1..=(MAX_DEPTH + 2) {
            deep = item(depth, "level", vec![deep]);
        }
        let too_deep = ToChrome::MenuBar(MenuBarModel {
            app_name: String::new(),
            menus: vec![Menu { id: 1, title: "t".into(), items: vec![deep] }],
            status: vec![],
        });
        assert_eq!(decode_to_chrome(&encode_to_chrome(&too_deep)), Err(DecodeError::BadValue));
        // Items at the top level are depth 0; a leaf at depth MAX_DEPTH - 1 still decodes.
        let mut ok = item(0, "leaf", vec![]);
        for depth in 1..MAX_DEPTH {
            ok = item(depth, "level", vec![ok]);
        }
        let limit = ToChrome::MenuBar(MenuBarModel {
            app_name: String::new(),
            menus: vec![Menu { id: 1, title: "t".into(), items: vec![ok] }],
            status: vec![],
        });
        assert_eq!(decode_to_chrome(&encode_to_chrome(&limit)), Ok(limit));
    }
}
