//! The Audio Setup window's state and its geometry: the device list (sidebar), the toolbar, the detail view of a
//! device, and the aggregate editor. Layout is in points and derives from the window size; the drawing code and the
//! hit tests read the same rectangles from here, so a tap lands on what is drawn.

use crate::device::Session;
use crate::draft::Draft;
use crate::model::{DeviceInfo, Kind};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// The sidebar width (points).
pub const SIDE_PT: f32 = 220.0;
/// Sidebar row and header heights (points).
pub const ROW_PT: f32 = 26.0;
pub const HEAD_PT: f32 = 24.0;
/// The margin of the detail view (points).
pub const PAD_PT: f32 = 20.0;
/// The editor's footer, with its Save and Cancel buttons (points).
pub const FOOT_PT: f32 = 56.0;
/// The height of one member row in the editor (points), and of one line in the detail card.
pub const MEMBER_PT: f32 = 44.0;
pub const LINE_PT: f32 = 28.0;
/// The label column of the detail card and of the editor's form (points).
pub const KEY_PT: f32 = 110.0;
/// How often the device list is read again (seconds).
pub const REFRESH_S: f64 = 2.0;

/// Toolbar item ids.
pub const B_NEW_AGG: u32 = 1;
pub const B_EDIT: u32 = 3;
pub const B_REMOVE: u32 = 4;
pub const B_REFRESH: u32 = 5;

/// Accessibility ids of the sidebar rows, the editor's controls and its member rows.
pub const ROW_BASE: u64 = 100;
pub const N_NAME: u64 = 200;
pub const N_KIND_AGG: u64 = 201;
pub const N_KIND_MULTI: u64 = 202;
pub const N_SAVE: u64 = 203;
pub const N_CANCEL: u64 = 204;
pub const MEMBER_BASE: u64 = 300;
/// Member ids run `MEMBER_BASE..MEMBER_BASE + MEMBER_SPAN`; anything above is not a published node.
pub const MEMBER_SPAN: u64 = 100;

/// What a point or an accessibility id stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// An index into `AudioSetup::devices`.
    Row(usize),
    Name,
    Kind(Kind),
    /// An index into `AudioSetup::editable()`.
    Member(usize),
    /// The clock of a member, by the same index.
    Clock(usize),
    Save,
    Cancel,
}

/// The aggregate editor: the draft, and the sentence under the form when the last save was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    pub draft: Draft,
    pub message: Option<String>,
}

/// The window.
pub struct AudioSetup {
    pub session: Session,
    pub devices: Vec<DeviceInfo>,
    /// A failure to read the list (the server is not running).
    pub error: Option<String>,
    /// The device the detail view shows, by UID.
    pub selected: Option<String>,
    pub editor: Option<Editor>,
    pub labels: TextCache,
    /// The window size in points.
    pub size: (f32, f32),
    pub last_refresh: f64,
    /// The target a press started on; the release activates it only when it lands on the same one.
    pub press: Target,
    /// The clock time of the last event, frame or tick (seconds), for the actions that read the list.
    pub now: f64,
}

impl AudioSetup {
    pub fn new() -> AudioSetup {
        AudioSetup {
            session: Session::new(),
            devices: Vec::new(),
            error: None,
            selected: None,
            editor: None,
            labels: TextCache::default(),
            size: (0.0, 0.0),
            last_refresh: f64::NEG_INFINITY,
            press: Target::Nothing,
            now: 0.0,
        }
    }

    /// Reads the device list. True when anything the window shows changed.
    pub fn refresh(&mut self, t: f64) -> bool {
        self.last_refresh = t;
        let before = (self.devices.clone(), self.error.clone());
        match self.session.devices() {
            Ok(list) => {
                self.devices = list;
                self.error = None;
            }
            Err(msg) => {
                self.devices.clear();
                self.error = Some(msg);
                self.session.drop_connection();
            }
        }
        if let Some(uid) = &self.selected {
            if !self.devices.iter().any(|d| &d.uid == uid) {
                self.selected = None;
            }
        }
        (self.devices.clone(), self.error.clone()) != before
    }

    /// The device the detail view shows.
    pub fn selected_device(&self) -> Option<&DeviceInfo> {
        let uid = self.selected.as_ref()?;
        self.devices.iter().find(|d| &d.uid == uid)
    }

    /// The hardware devices an aggregate can take: the live ones that are not made by the app.
    pub fn editable(&self) -> Vec<&DeviceInfo> {
        self.devices.iter().filter(|d| !d.kind.is_made() && d.alive).collect()
    }

    /// A default name for a new device: `Aggregate 2` when one already exists.
    fn default_name(&self, kind: Kind) -> String {
        let made = self.devices.iter().filter(|d| d.kind == kind).count();
        let base = if kind == Kind::MultiOutput { "Multi-Output" } else { "Aggregate" };
        format!("{} {}", base, made + 1)
    }

    pub fn open_editor(&mut self, kind: Kind) {
        let name = self.default_name(kind);
        self.editor = Some(Editor { draft: Draft::new(kind, &name), message: None });
    }

    /// Opens the editor on the selected made device, with the members the server reports.
    pub fn edit_selected(&mut self) {
        let Some(dev) = self.selected_device().filter(|d| d.kind.is_made()).cloned() else { return };
        match self.session.composition(dev.object) {
            Ok(mut draft) => {
                draft.uid = Some(dev.uid.clone());
                self.editor = Some(Editor { draft, message: None });
            }
            Err(msg) => self.error = Some(msg),
        }
    }

    /// Sends the draft to the server when it validates; the editor closes on success.
    pub fn save(&mut self, t: f64) {
        let Some(editor) = self.editor.as_mut() else { return };
        if let Err(e) = editor.draft.validate(&self.devices) {
            editor.message = Some(e.message());
            return;
        }
        let draft = editor.draft.clone();
        match self.session.save(&draft) {
            Ok(()) => {
                self.editor = None;
                self.refresh(t);
                if draft.uid.is_none() {
                    // the new device is the made one with the draft's name
                    self.selected = self.devices.iter().find(|d| d.kind.is_made() && d.name == draft.name.trim()).map(|d| d.uid.clone());
                }
            }
            Err(msg) => {
                if let Some(e) = self.editor.as_mut() {
                    e.message = Some(msg);
                }
            }
        }
    }

    /// Removes the selected made device.
    pub fn remove_selected(&mut self, t: f64) {
        let Some(dev) = self.selected_device().filter(|d| d.kind.is_made()).cloned() else { return };
        match self.session.remove(dev.object) {
            Ok(()) => {
                self.selected = None;
                self.refresh(t);
            }
            Err(msg) => self.error = Some(msg),
        }
    }

    /// Activates a target. True when the window needs a new frame.
    pub fn activate(&mut self, target: Target, t: f64) -> bool {
        match target {
            Target::Nothing | Target::Name => false,
            Target::Toolbar(B_NEW_AGG) => self.open_for_new(Kind::Aggregate),
            Target::Toolbar(B_EDIT) => {
                self.edit_selected();
                true
            }
            Target::Toolbar(B_REMOVE) => {
                self.remove_selected(t);
                true
            }
            Target::Toolbar(B_REFRESH) => {
                self.refresh(t);
                true
            }
            Target::Toolbar(_) => false,
            Target::Row(i) => {
                let uid = self.devices.get(i).map(|d| d.uid.clone());
                self.selected = uid;
                self.editor = None;
                true
            }
            Target::Kind(kind) => match self.editor.as_mut() {
                Some(e) => {
                    e.draft.kind = kind;
                    true
                }
                None => false,
            },
            Target::Member(i) => {
                let uid = self.editable().get(i).map(|d| d.uid.clone());
                match (self.editor.as_mut(), uid) {
                    (Some(e), Some(uid)) => {
                        e.draft.toggle(&uid);
                        true
                    }
                    _ => false,
                }
            }
            Target::Clock(i) => {
                let uid = self.editable().get(i).map(|d| d.uid.clone());
                match (self.editor.as_mut(), uid) {
                    (Some(e), Some(uid)) => e.draft.set_clock_source(&uid),
                    _ => false,
                }
            }
            Target::Save => {
                self.save(t);
                true
            }
            Target::Cancel => {
                self.editor = None;
                true
            }
        }
    }

    fn open_for_new(&mut self, kind: Kind) -> bool {
        self.open_editor(kind);
        true
    }

    /// The toolbar for the current state: the made-device actions need a made device selected and no open editor.
    pub fn toolbar(&self) -> Toolbar {
        let made = self.editor.is_none() && self.selected_device().is_some_and(|d| d.kind.is_made());
        // One add control: the editor's kind switch picks Aggregate or Multi-Output. Nothing can be made without a server.
        let can_add = self.editor.is_none() && self.error.is_none();
        Toolbar::new(vec![
            ToolbarItem::icon(B_NEW_AGG, "New Device", "plus").enabled(can_add),
            ToolbarItem::button(B_EDIT, "Edit").enabled(made),
            ToolbarItem::button(B_REMOVE, "Remove").enabled(made),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(B_REFRESH, "Refresh"),
        ])
    }

    /// The sidebar: hardware first, then the made devices, each under its header. Row ids are `ROW_BASE + index`.
    pub fn sidebar(&self) -> SourceList {
        let mut items = Vec::new();
        for kind in [Kind::Hardware, Kind::Aggregate] {
            let rows: Vec<usize> = (0..self.devices.len()).filter(|&i| self.devices[i].kind == kind || (kind == Kind::Aggregate && self.devices[i].kind == Kind::MultiOutput)).collect();
            if rows.is_empty() {
                continue;
            }
            items.push(SidebarItem::header(kind.section()));
            for i in rows {
                let d = &self.devices[i];
                let symbol = if d.kind.is_made() { "rectangle.stack" } else { "speaker.wave.2" };
                let mut row = SidebarItem::row(ROW_BASE as u32 + i as u32, &d.name).with_symbol(symbol);
                if !d.alive {
                    row = row.disabled();
                }
                items.push(row);
            }
        }
        let mut list = SourceList::new(items, ROW_PT, HEAD_PT);
        list.set_selected(self.selected_index().map(|i| ROW_BASE as u32 + i as u32));
        list
    }

    fn selected_index(&self) -> Option<usize> {
        let uid = self.selected.as_ref()?;
        self.devices.iter().position(|d| &d.uid == uid)
    }

    /// The window's rectangles in points.
    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    pub fn sidebar_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT, h: (self.size.1 - UNIFIED_BAR_PT).max(0.0) }
    }

    /// The left edge of the detail content, and its width.
    pub fn content_x(&self) -> f32 {
        SIDE_PT + PAD_PT
    }

    pub fn content_w(&self) -> f32 {
        (self.size.0 - SIDE_PT - 2.0 * PAD_PT).max(0.0)
    }

    /// The editor's rectangles. Name row, kind switch, the member card, the clock hint and the footer buttons.
    pub fn editor_layout(&self) -> EditorLayout {
        let x = self.content_x();
        let w = self.content_w();
        let top = UNIFIED_BAR_PT + PAD_PT + 40.0;
        let name = Rect { x: x + KEY_PT, y: top, w: (w - KEY_PT).max(0.0), h: 28.0 };
        let kind_y = name.y + 28.0 + 12.0;
        let kind_w = 300f32.min(name.w);
        let kind = Rect { x: x + KEY_PT, y: kind_y, w: kind_w, h: 26.0 };
        let members_top = kind.y + 26.0 + 36.0;
        let footer_top = (self.size.1 - FOOT_PT).max(members_top);
        let visible = ((footer_top - members_top - 12.0) / MEMBER_PT).floor().max(0.0) as usize;
        let member_rows = (0..visible)
            .map(|i| Rect { x, y: members_top + i as f32 * MEMBER_PT, w, h: MEMBER_PT })
            .collect();
        let save = Rect { x: self.size.0 - PAD_PT - 96.0, y: self.size.1 - FOOT_PT + 14.0, w: 96.0, h: 28.0 };
        let cancel = Rect { x: save.x - 8.0 - 96.0, y: save.y, w: 96.0, h: 28.0 };
        EditorLayout { name, kind, members_top, member_rows, footer_top, save, cancel }
    }

    /// The toolbar's layout (the rectangles of its visible items).
    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// What the point is over. Points, with the window's origin at the top left.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            let bar = self.toolbar();
            let layout = self.toolbar_layout(ui);
            return layout
                .visible
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| Target::Toolbar(i.id)))
                .unwrap_or(Target::Nothing);
        }
        if x < SIDE_PT {
            let list = self.sidebar();
            let rect = self.sidebar_rect();
            return match list.hit(rect, 0.0, x, y) {
                Some(id) if id as u64 >= ROW_BASE => Target::Row((id as u64 - ROW_BASE) as usize),
                _ => Target::Nothing,
            };
        }
        if self.editor.is_none() {
            return Target::Nothing;
        }
        let geo = self.editor_layout();
        if geo.save.contains(x, y) {
            return Target::Save;
        }
        if geo.cancel.contains(x, y) {
            return Target::Cancel;
        }
        if geo.name.contains(x, y) {
            return Target::Name;
        }
        if geo.kind.contains(x, y) {
            let half = geo.kind.w / 2.0;
            return Target::Kind(if x < geo.kind.x + half { Kind::Aggregate } else { Kind::MultiOutput });
        }
        for (i, r) in geo.member_rows.iter().enumerate() {
            if r.contains(x, y) {
                // the clock tag sits at the right end of a member row; a click there on a member sets the clock
                let clock_x = r.x + r.w - 96.0;
                if x >= clock_x && self.is_member(i) {
                    return Target::Clock(i);
                }
                return Target::Member(i);
            }
        }
        Target::Nothing
    }

    fn is_member(&self, i: usize) -> bool {
        match (&self.editor, self.editable().get(i)) {
            (Some(e), Some(d)) => e.draft.is_member(&d.uid),
            _ => false,
        }
    }


    /// The accessibility target for a published node id (`Input::Activate`).
    pub fn target_of_id(id: u64) -> Target {
        match id {
            1..=99 => Target::Toolbar(id as u32),
            N_NAME => Target::Name,
            N_KIND_AGG => Target::Kind(Kind::Aggregate),
            N_KIND_MULTI => Target::Kind(Kind::MultiOutput),
            N_SAVE => Target::Save,
            N_CANCEL => Target::Cancel,
            x if (ROW_BASE..MEMBER_BASE).contains(&x) => Target::Row((x - ROW_BASE) as usize),
            x if (MEMBER_BASE..MEMBER_BASE + MEMBER_SPAN).contains(&x) => Target::Member((x - MEMBER_BASE) as usize),
            _ => Target::Nothing,
        }
    }
}

/// The editor's rectangles in points (see `AudioSetup::editor_layout`).
pub struct EditorLayout {
    pub name: Rect,
    pub kind: Rect,
    pub members_top: f32,
    pub member_rows: Vec<Rect>,
    pub footer_top: f32,
    pub save: Rect,
    pub cancel: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(uid: &str, kind: Kind, outputs: u32) -> DeviceInfo {
        let mut d = DeviceInfo::new(uid.len() as u32 + 10, uid, uid);
        d.kind = kind;
        d.output_channels = outputs;
        d.rate = 48000.0;
        d
    }

    #[test]
    fn default_names_count_the_made_devices() {
        let mut w = AudioSetup::new();
        assert_eq!(w.default_name(Kind::Aggregate), "Aggregate 1");
        w.devices = vec![dev("hw", Kind::Hardware, 2), dev("agg", Kind::Aggregate, 2)];
        assert_eq!(w.default_name(Kind::Aggregate), "Aggregate 2");
        assert_eq!(w.default_name(Kind::MultiOutput), "Multi-Output 1");
    }

    #[test]
    fn toolbar_made_actions_need_a_made_device_and_no_editor() {
        let mut w = AudioSetup::new();
        w.devices = vec![dev("hw", Kind::Hardware, 2), dev("agg", Kind::Aggregate, 2)];
        let enabled = |bar: &Toolbar, id: u32| bar.items.iter().find(|i| i.id == id).unwrap().enabled;
        w.selected = Some("hw".into());
        let bar = w.toolbar();
        assert!(enabled(&bar, B_NEW_AGG));
        assert!(!enabled(&bar, B_EDIT));
        assert!(!enabled(&bar, B_REMOVE));
        w.selected = Some("agg".into());
        let bar = w.toolbar();
        assert!(enabled(&bar, B_EDIT) && enabled(&bar, B_REMOVE));
        w.open_editor(Kind::Aggregate);
        let bar = w.toolbar();
        assert!(!enabled(&bar, B_NEW_AGG) && !enabled(&bar, B_EDIT) && enabled(&bar, B_REFRESH));
    }

    #[test]
    fn the_toolbar_has_one_add_control_and_it_needs_a_server() {
        let mut w = AudioSetup::new();
        let bar = w.toolbar();
        assert_eq!(bar.items.iter().filter(|i| i.label.contains("New")).count(), 1);
        assert!(bar.items.iter().find(|i| i.id == B_NEW_AGG).unwrap().enabled);
        w.error = Some("The audio server is not running.".into());
        let bar = w.toolbar();
        assert!(!bar.items.iter().find(|i| i.id == B_NEW_AGG).unwrap().enabled);
    }

    #[test]
    fn the_member_clock_and_the_save_path() {
        let mut w = AudioSetup::new();
        w.devices = vec![dev("analog", Kind::Hardware, 2), dev("hdmi3", Kind::Hardware, 2), dev("agg", Kind::Aggregate, 2)];
        w.open_editor(Kind::Aggregate);
        // the editable list is the hardware devices, in order
        assert_eq!(w.editable().iter().map(|d| d.uid.as_str()).collect::<Vec<_>>(), vec!["analog", "hdmi3"]);
        assert!(w.activate(Target::Member(0), 0.0));
        assert!(w.activate(Target::Member(1), 0.0));
        assert!(w.activate(Target::Clock(1), 0.0));
        let e = w.editor.as_ref().unwrap();
        assert_eq!(e.draft.members, vec!["analog", "hdmi3"]);
        assert_eq!(e.draft.clock(), Some("hdmi3"));
        // a name the user cleared fails validation, and the editor stays open with the sentence
        w.editor.as_mut().unwrap().draft.name = " ".into();
        w.activate(Target::Save, 0.0);
        assert_eq!(w.editor.as_ref().unwrap().message.as_deref(), Some("Give the device a name."));
        assert!(w.activate(Target::Cancel, 0.0));
        assert!(w.editor.is_none());
    }

    #[test]
    fn target_ids_round_trip_the_published_nodes() {
        assert_eq!(AudioSetup::target_of_id(B_REFRESH as u64), Target::Toolbar(B_REFRESH));
        assert_eq!(AudioSetup::target_of_id(ROW_BASE + 3), Target::Row(3));
        assert_eq!(AudioSetup::target_of_id(MEMBER_BASE + 2), Target::Member(2));
        assert_eq!(AudioSetup::target_of_id(N_SAVE), Target::Save);
        assert_eq!(AudioSetup::target_of_id(N_KIND_MULTI), Target::Kind(Kind::MultiOutput));
        assert_eq!(AudioSetup::target_of_id(999), Target::Nothing);
    }

    #[test]
    fn the_editor_fits_the_window_height() {
        let mut w = AudioSetup::new();
        w.size = (1200.0, 780.0);
        let g = w.editor_layout();
        assert!(g.name.y > UNIFIED_BAR_PT);
        assert!(g.kind.y > g.name.y);
        assert!(g.member_rows.iter().all(|r| r.y + r.h <= g.footer_top));
        assert!(g.save.x > g.cancel.x + g.cancel.w);
        // a short window shows fewer member rows and never a negative one
        w.size = (600.0, 300.0);
        assert!(w.editor_layout().member_rows.len() < g.member_rows.len());
    }

    #[test]
    fn the_configured_pixel_size_becomes_points() {
        assert_eq!(crate::size_pt(1250, 750, 1.5625), (800.0, 480.0));
        assert_eq!(crate::size_pt(600, 400, 1.0), (600.0, 400.0));
    }
}
