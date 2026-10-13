//! The Disk Utility window's state, geometry and hit tests: the disks and volumes (sidebar), the toolbar, the info pane
//! of the selected row and the footer. Layout is in points and derives from the window size; the drawing code and the
//! hit tests read the same rectangles from here, so a tap lands on what is drawn. The disks are re-read every
//! `REFRESH_S` seconds, and usage every `USAGE_S` seconds.

use crate::actions::{self, Action};
use crate::model::{self, Kind, Node};
use crate::{probe, sysfs};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_core::disk_format::Personality;
use bifrost_core::disk_request::DiskRequest;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};

/// The sidebar width (points).
pub const SIDE_PT: f32 = 220.0;
/// The gutter between the sidebar and the info pane, and at the window's bottom and right (points).
pub const PAD_PT: f32 = 20.0;
/// A sidebar row and a section header (points).
pub const ROW_PT: f32 = 26.0;
pub const HEAD_PT: f32 = 24.0;
/// One line of the info card (points), and the width of its key column.
pub const LINE_PT: f32 = 28.0;
pub const KEY_PT: f32 = 130.0;
/// How often the disks are re-read, and how often usage is measured (seconds).
pub const REFRESH_S: f64 = 2.0;
pub const USAGE_S: f64 = 10.0;

pub const B_MOUNT: u32 = 1;
pub const B_UNMOUNT: u32 = 2;
pub const B_EJECT: u32 = 3;
pub const B_REFRESH: u32 = 4;
pub const B_DETACH: u32 = 5;
pub const B_ERASE: u32 = 6;
pub const B_FIRST_AID: u32 = 7;
/// The confirmation sheet's buttons: Cancel closes it; the others answer with the request they name.
pub const SHEET_CANCEL: u32 = 10;
pub const SHEET_CONFIRM: u32 = 11;
pub const SHEET_VERIFY: u32 = 12;
pub const SHEET_REPAIR: u32 = 13;

/// Accessibility and sidebar ids: a row's id is `ROW_BASE` plus its index in `DiskUtility::nodes`.
pub const ROW_BASE: u64 = 100;

/// What a point or an accessibility id stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// An index into `DiskUtility::nodes`.
    Row(usize),
    /// A button of the confirmation sheet.
    Sheet(u32),
}

/// The erase or first-aid action the user asked for on the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Erase,
    FirstAid,
}

/// A confirmation sheet and the requests its buttons start, by button id.
pub struct Pending {
    pub sheet: Sheet,
    pub requests: Vec<(u32, DiskRequest)>,
}

/// A mount, unmount, eject or privileged request that is running on its own thread. `verb` is its progress word.
pub struct Job {
    pub verb: &'static str,
    pub name: String,
    pub rx: Receiver<Result<(), String>>,
}

/// The window.
pub struct DiskUtility {
    /// Every disk, with its partitions after it.
    pub nodes: Vec<Node>,
    /// The device name of the row the info pane shows.
    pub selected: Option<String>,
    /// The sentence for the last failed action or refusal.
    pub error: Option<String>,
    pub job: Option<Job>,
    /// The confirmation sheet, while one is open.
    pub sheet: Option<Pending>,
    /// SMART health by device name, read once per disk (Refresh clears it).
    pub smart: BTreeMap<String, String>,
    pub labels: TextCache,
    /// The window size in points.
    pub size: (f32, f32),
    pub last_refresh: f64,
    pub last_usage: f64,
    /// The target a press started on; the release activates it only when it lands on the same one.
    pub press: Target,
    /// The clock time of the last event, frame or tick (seconds).
    pub now: f64,
}

impl DiskUtility {
    pub fn new() -> DiskUtility {
        DiskUtility {
            nodes: Vec::new(),
            selected: None,
            error: None,
            job: None,
            sheet: None,
            smart: BTreeMap::new(),
            labels: TextCache::default(),
            size: (1200.0, 780.0),
            last_refresh: f64::NEG_INFINITY,
            last_usage: f64::NEG_INFINITY,
            press: Target::Nothing,
            now: 0.0,
        }
    }

    /// Reads the disks, the mount table and (when due) usage. True when anything the window shows changed.
    pub fn refresh(&mut self, t: f64, force_usage: bool) -> bool {
        self.last_refresh = t;
        let before = self.nodes.clone();
        let mut nodes = sysfs::read_nodes(Path::new("/sys"), Path::new("/run/udev/data"));
        model::apply_mounts(&mut nodes, &model::parse_mounts(&probe::read_mounts()));
        if force_usage || t - self.last_usage >= USAGE_S {
            let points: Vec<String> = nodes.iter().filter_map(|n| n.mount_point.clone()).collect();
            model::apply_usage(&mut nodes, &probe::read_df(&points));
            self.last_usage = t;
        } else {
            model::carry_usage(&mut nodes, &self.nodes);
        }
        for node in nodes.iter_mut().filter(|n| n.kind == Kind::Disk) {
            node.smart = self.smart.get(&node.name).cloned();
        }
        self.nodes = nodes;
        if let Some(name) = &self.selected {
            if !self.nodes.iter().any(|n| &n.name == name) {
                self.selected = None;
            }
        }
        self.probe_smart();
        self.nodes != before
    }

    /// Reads the SMART health of the selected disk, once. Partitions have none.
    fn probe_smart(&mut self) {
        let Some(name) = self.selected.clone() else { return };
        let is_disk = self.nodes.iter().any(|n| n.name == name && n.kind == Kind::Disk);
        if !is_disk || self.smart.contains_key(&name) {
            return;
        }
        let health = probe::read_smart(&name);
        if let Some(node) = self.nodes.iter_mut().find(|n| n.name == name) {
            node.smart = Some(health.clone());
        }
        self.smart.insert(name, health);
    }

    /// The row the info pane shows.
    pub fn selected_node(&self) -> Option<&Node> {
        let name = self.selected.as_ref()?;
        self.nodes.iter().find(|n| &n.name == name)
    }

    /// Whether the selected row is a loop device that holds a disk image.
    pub fn selected_image(&self) -> bool {
        self.selected_node().is_some_and(|n| n.image_file.is_some())
    }

    fn selected_index(&self) -> Option<usize> {
        let name = self.selected.as_ref()?;
        self.nodes.iter().position(|n| &n.name == name)
    }

    /// The mounted partitions of a disk, by device name.
    fn mounted_partitions(&self, disk: &str) -> Vec<String> {
        self.nodes.iter().filter(|n| n.parent.as_deref() == Some(disk) && n.mount_point.is_some()).map(|n| n.name.clone()).collect()
    }

    /// Starts an action on the selected row. Refusals from the disk session become the footer's error.
    fn start(&mut self, action: Action) -> bool {
        if self.job.is_some() {
            return false;
        }
        let Some(name) = self.selected.clone() else { return false };
        let disks = actions::disks_of(&self.nodes);
        let mounted = self.mounted_partitions(&name);
        match actions::plan(action, &disks, &name, &mounted) {
            Ok(plan) => {
                self.error = None;
                self.job = Some(Job { verb: action.verb(), name, rx: actions::spawn(plan) });
            }
            Err(refusal) => self.error = Some(refusal.sentence()),
        }
        true
    }

    /// Takes the result of a running action when it has one. True when the window changed.
    pub fn poll(&mut self, t: f64) -> bool {
        let Some(job) = &self.job else { return false };
        let outcome = match job.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The operation ended without a result.".to_string()),
        };
        self.error = outcome.err();
        self.job = None;
        self.refresh(t, true);
        true
    }

    /// The footer line: the progress of a running action, or the last error. `(text, is_error)`.
    pub fn footer(&self) -> Option<(String, bool)> {
        if let Some(error) = &self.error {
            return Some((error.clone(), true));
        }
        self.job.as_ref().map(|job| (format!("{} {}\u{2026}", job.verb, job.name), false))
    }

    /// Opens the confirmation sheet for an erase or a first-aid request on the selected row. A request the disks already
    /// refuse (a mounted volume, a partition where a whole disk is needed) becomes the footer's error and no sheet opens.
    fn ask(&mut self, ask: Ask) -> bool {
        if self.job.is_some() {
            return false;
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let name = node.name.clone();
        let (title, message, buttons, requests) = match ask {
            Ask::Erase => {
                let request = if node.kind == Kind::Disk {
                    DiskRequest::EraseDisk { device: name.clone(), fs: Personality::Fat32, label: String::new() }
                } else {
                    DiskRequest::EraseVolume { device: name.clone(), fs: Personality::Fat32, label: String::new() }
                };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Erase", ButtonRole::Destructive)];
                (format!("Erase {name}?"), request.confirm_text(), buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::FirstAid => {
                let buttons = vec![
                    SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel),
                    SheetButton::new(SHEET_VERIFY, "Check Only", ButtonRole::Normal),
                    SheetButton::new(SHEET_REPAIR, "Repair", ButtonRole::Default),
                ];
                let message = format!("Check {name} for errors. Repair fixes what it finds; Check Only changes nothing.");
                let requests = vec![(SHEET_VERIFY, DiskRequest::VerifyVolume { device: name.clone() }), (SHEET_REPAIR, DiskRequest::RepairVolume { device: name.clone() })];
                (format!("First Aid on {name}?"), message, buttons, requests)
            }
        };
        if let Some(sentence) = requests.iter().find_map(|(_, r)| actions::check_request(r, &self.nodes).err()) {
            self.error = Some(sentence);
            return true;
        }
        self.error = None;
        self.sheet = Some(Pending { sheet: Sheet::alert(&title, &message, buttons), requests });
        true
    }

    /// The sheet's answer. Cancel closes it; another button starts the request it names.
    fn answer(&mut self, id: u32) -> bool {
        let Some(pending) = self.sheet.take() else { return false };
        if id != SHEET_CANCEL {
            if let Some((_, request)) = pending.requests.into_iter().find(|(button, _)| *button == id) {
                self.start_request(request);
            }
        }
        true
    }

    /// Starts a privileged request on its own thread, with its progress word in the footer.
    fn start_request(&mut self, request: DiskRequest) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        let job = Job { verb: request.progress_word(), name: request.device().to_string(), rx: actions::spawn_request(request) };
        self.job = Some(job);
    }

    /// The whole window in points, the parent of a sheet.
    pub fn window_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: self.size.1 }
    }

    /// Activates a target. True when the window needs a new frame.
    pub fn activate(&mut self, target: Target, t: f64) -> bool {
        match target {
            Target::Nothing => false,
            Target::Toolbar(B_MOUNT) => self.start(Action::Mount),
            Target::Toolbar(B_UNMOUNT) => self.start(Action::Unmount),
            Target::Toolbar(B_EJECT) => self.start(Action::Eject),
            Target::Toolbar(B_DETACH) => self.start(Action::Detach),
            Target::Toolbar(B_ERASE) => self.ask(Ask::Erase),
            Target::Toolbar(B_FIRST_AID) => self.ask(Ask::FirstAid),
            Target::Sheet(id) => self.answer(id),
            Target::Toolbar(B_REFRESH) => {
                self.smart.clear();
                self.refresh(t, true);
                true
            }
            Target::Toolbar(_) => false,
            Target::Row(i) => match self.nodes.get(i).map(|n| n.name.clone()) {
                Some(name) => {
                    self.selected = Some(name);
                    self.probe_smart();
                    true
                }
                None => false,
            },
        }
    }

    /// The toolbar for the current state. Actions need a selected row and no running action.
    pub fn toolbar(&self) -> Toolbar {
        let idle = self.job.is_none();
        let node = self.selected_node();
        let mountable = node.is_some_and(|n| n.mount_point.is_none() && n.fs_type.is_some());
        let mounted = node.is_some_and(|n| n.mount_point.is_some());
        let ejectable = node.is_some_and(|n| n.kind == Kind::Disk && n.removable);
        Toolbar::new(vec![
            ToolbarItem::button(B_MOUNT, "Mount").enabled(idle && mountable),
            ToolbarItem::button(B_UNMOUNT, "Unmount").enabled(idle && mounted),
            ToolbarItem::button(B_EJECT, "Eject").enabled(idle && ejectable),
            ToolbarItem::button(B_DETACH, "Detach").enabled(idle && self.selected_image()),
            ToolbarItem::button(B_ERASE, "Erase").enabled(idle && node.is_some()),
            ToolbarItem::button(B_FIRST_AID, "First Aid").enabled(idle && node.is_some()),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(B_REFRESH, "Refresh", "arrow.clockwise"),
        ])
    }

    /// The sidebar: internal disks, then removable ones, each disk followed by its partitions. Row ids are
    /// `ROW_BASE + index`.
    pub fn sidebar(&self) -> SourceList {
        let mut items = Vec::new();
        for (removable, header) in [(false, "Internal"), (true, "External")] {
            let disks: Vec<usize> = (0..self.nodes.len()).filter(|&i| self.nodes[i].kind == Kind::Disk && self.nodes[i].removable == removable).collect();
            if disks.is_empty() {
                continue;
            }
            items.push(SidebarItem::header(header));
            for i in disks {
                let disk = &self.nodes[i];
                items.push(SidebarItem::row(ROW_BASE as u32 + i as u32, &model::display_name(disk)).with_symbol("internaldrive").with_trailing(&model::format_size(disk.size)));
                for (j, part) in self.nodes.iter().enumerate() {
                    if part.parent.as_deref() == Some(disk.name.as_str()) {
                        let row = SidebarItem::row(ROW_BASE as u32 + j as u32, &model::display_name(part)).with_symbol("folder");
                        items.push(row.with_trailing(&model::format_size(part.size)));
                    }
                }
            }
        }
        let mut list = SourceList::new(items, ROW_PT, HEAD_PT);
        list.set_selected(self.selected_index().map(|i| ROW_BASE as u32 + i as u32));
        list
    }

    /// The window's rectangles in points.
    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    pub fn sidebar_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT, h: (self.size.1 - UNIFIED_BAR_PT).max(0.0) }
    }

    /// The left edge of the info pane, and its width.
    pub fn content_x(&self) -> f32 {
        SIDE_PT + PAD_PT
    }

    pub fn content_w(&self) -> f32 {
        (self.size.0 - SIDE_PT - 2.0 * PAD_PT).max(0.0)
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// What the point is over. Points, with the window's origin at the top left.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        // an open sheet blocks the window: only its buttons answer
        if let Some(pending) = &self.sheet {
            return match pending.sheet.hit(self.window_rect(), UNIFIED_BAR_PT, 1.0, x, y) {
                Hit::Button(id) => Target::Sheet(id),
                _ => Target::Nothing,
            };
        }
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
            return match list.hit(self.sidebar_rect(), 0.0, x, y) {
                Some(id) if id as u64 >= ROW_BASE => Target::Row((id as u64 - ROW_BASE) as usize),
                _ => Target::Nothing,
            };
        }
        Target::Nothing
    }

    /// The accessibility target for a published node id (`Input::Activate`).
    pub fn target_of_id(id: u64) -> Target {
        match id {
            1..=99 => Target::Toolbar(id as u32),
            x if x >= ROW_BASE && x < ROW_BASE + 100_000 => Target::Row((x - ROW_BASE) as usize),
            _ => Target::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_ids_map_back_to_node_indices() {
        assert_eq!(DiskUtility::target_of_id(ROW_BASE + 3), Target::Row(3));
        assert_eq!(DiskUtility::target_of_id(B_EJECT as u64), Target::Toolbar(B_EJECT));
        assert_eq!(DiskUtility::target_of_id(0), Target::Nothing);
    }

    #[test]
    fn the_toolbar_follows_the_selection() {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, size: 8_000_000_000, ..Node::default() },
            Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), fs_type: Some("vfat".into()), mount_point: Some("/mnt/a".into()), ..Node::default() },
        ];
        w.selected = Some("sdb".into());
        let bar = w.toolbar();
        let enabled = |id: u32| bar.items.iter().find(|i| i.id == id).is_some_and(|i| i.enabled);
        assert!(!enabled(B_MOUNT));
        assert!(!enabled(B_UNMOUNT));
        assert!(enabled(B_EJECT));
        w.selected = Some("sdb1".into());
        let bar = w.toolbar();
        let enabled = |id: u32| bar.items.iter().find(|i| i.id == id).is_some_and(|i| i.enabled);
        assert!(enabled(B_UNMOUNT));
        assert!(!enabled(B_EJECT));
    }

    #[test]
    fn detach_is_enabled_only_for_a_selected_disk_image() {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, ..Node::default() },
            Node { name: "loop3".into(), image_file: Some("/home/me/a.iso".into()), fs_type: Some("iso9660".into()), ..Node::default() },
        ];
        let detach_enabled = |w: &DiskUtility| w.toolbar().items.iter().find(|i| i.id == B_DETACH).is_some_and(|i| i.enabled);
        w.selected = Some("sdb".into());
        assert!(!w.selected_image());
        assert!(!detach_enabled(&w));
        w.selected = Some("loop3".into());
        assert!(w.selected_image());
        assert!(detach_enabled(&w));
    }

    #[test]
    fn the_sidebar_groups_by_removability_and_nests_partitions() {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, ..Node::default() },
            Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), ..Node::default() },
            Node { name: "nvme0n1".into(), ..Node::default() },
        ];
        let list = w.sidebar();
        let ids: Vec<u32> = list.items.iter().filter_map(|i| if let SidebarItem::Row { id, .. } = i { Some(*id) } else { None }).collect();
        assert_eq!(ids, vec![ROW_BASE as u32 + 2, ROW_BASE as u32, ROW_BASE as u32 + 1]);
    }

    #[test]
    fn a_failed_refusal_becomes_the_footer_error() {
        let mut w = DiskUtility::new();
        w.selected = Some("sdz".into());
        assert!(w.activate(Target::Toolbar(B_UNMOUNT), 1.0));
        assert_eq!(w.footer().map(|(_, e)| e), Some(true));
        assert!(w.job.is_none());
    }

    fn one_stick() -> DiskUtility {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, size: 8_000_000_000, ..Node::default() },
            Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), fs_type: Some("vfat".into()), ..Node::default() },
        ];
        w
    }

    #[test]
    fn a_refused_erase_is_the_footer_error_and_opens_no_sheet() {
        let mut w = one_stick();
        w.nodes[1].mount_point = Some("/mnt/a".into());
        w.selected = Some("sdb1".into());
        assert!(w.activate(Target::Toolbar(B_ERASE), 1.0));
        assert!(w.sheet.is_none() && w.job.is_none());
        let footer = w.footer().expect("footer");
        assert!(footer.1 && footer.0.contains("/mnt/a"), "{footer:?}");
    }

    #[test]
    fn an_erase_asks_first_and_cancel_closes_the_sheet_without_a_job() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.activate(Target::Toolbar(B_ERASE), 1.0));
        let pending = w.sheet.as_ref().expect("sheet");
        assert!(pending.requests.iter().any(|(id, r)| *id == SHEET_CONFIRM && r.method() == "disk.erase_disk"));
        assert!(w.activate(Target::Sheet(SHEET_CANCEL), 1.0));
        assert!(w.sheet.is_none() && w.job.is_none());
    }

    #[test]
    fn a_sheet_blocks_the_toolbar_under_it() {
        let mut w = one_stick();
        w.selected = Some("sdb1".into());
        assert!(w.activate(Target::Toolbar(B_FIRST_AID), 1.0));
        let parent = w.window_rect();
        let pending = w.sheet.as_ref().expect("sheet");
        let (cx, cy) = pending.sheet.layout(parent, UNIFIED_BAR_PT, 1.0).buttons.iter().find(|(id, _)| *id == SHEET_REPAIR).map(|(_, r)| (r.x + r.w / 2.0, r.y + r.h / 2.0)).expect("repair button");
        assert_eq!(pending.sheet.hit(parent, UNIFIED_BAR_PT, 1.0, cx, cy), Hit::Button(SHEET_REPAIR));
    }
}
