//! The Disk Utility window's state, geometry and hit tests: the disks and volumes (sidebar), the toolbar, the info pane
//! of the selected row and the footer. Layout is in points and derives from the window size; the drawing code and the
//! hit tests read the same rectangles from here, so a tap lands on what is drawn. The disks are re-read every
//! `REFRESH_S` seconds, and usage every `USAGE_S` seconds.

use crate::actions::{self, Action};
use crate::flow::{Flow, Want};
use crate::form::FormHit;
use crate::forms::OpenForm;
use crate::model::{self, Kind, Node};
use crate::{probe, sysfs};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton, BUTTON_H_PT, SHEET_PAD_PT};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::{service, Ctx};
use bifrost_core::disk_scan::Volume;
use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_core::disk_commands::Command;
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

/// The longest line of an alert sheet's message, in characters (the alert sizes itself for this many).
pub const ALERT_CHARS: usize = 34;
/// The width of the Get Info sheet (points).
const INFO_W_PT: f32 = 460.0;

/// A Get Info sheet: one `Key: value` line per fact, in an alert wider than the usual one so a line is not cut short.
pub fn info_sheet(title: &str, lines: &[String]) -> Sheet {
    let mut sheet = Sheet::alert(title, &lines.join("\n"), vec![SheetButton::new(SHEET_CANCEL, "Done", ButtonRole::Cancel)]);
    // the alert sizes its height for text that wraps at its own narrow width; these lines are kept whole
    sheet.size = (INFO_W_PT, SHEET_PAD_PT * 2.0 + 18.0 + lines.len() as f32 * 15.0 + 8.0 + 16.0 + BUTTON_H_PT);
    sheet
}

/// What a point or an accessibility id stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// An index into `DiskUtility::nodes`.
    Row(usize),
    /// A button of the confirmation sheet.
    Sheet(u32),
    /// A button, field or option of the open form sheet.
    Form(FormHit),
}

/// The erase or first-aid action the user asked for on the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Erase,
    FirstAid,
    /// Get Info: every fact about the row in a sheet.
    Info,
    /// Partition the selected disk: the Partition sheet opens with this many equal parts filled in.
    Partition(usize),
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
    /// For a job that writes the file a save panel handed over: the panel's request, committed when the job succeeds
    /// and thrown away when it fails.
    pub commit: Option<u32>,
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
    /// The form sheet (Rename, Erase, Restore, RAID, the image sheets), while one is open.
    pub form: Option<OpenForm>,
    /// The open or save panel the logic wants shown next, and the one the window is waiting on (`flow.rs`).
    pub want: Option<Want>,
    pub flow: Option<Flow>,
    /// A save the last job wrote: its request, and whether the job succeeded. `flush` commits or aborts it.
    pub finished_save: Option<(u32, bool)>,
    /// The footer line after something finished well.
    pub notice: Option<String>,
    /// SMART health by device name, read once per disk (Refresh clears it).
    pub smart: BTreeMap<String, String>,
    pub labels: TextCache,
    /// View > Show All Devices (disks with their volumes) or Show Only Volumes.
    pub show_all: bool,
    /// The file system Erase and Partition write.
    pub erase_fs: Personality,
    /// The window size in points.
    pub size: (f32, f32),
    pub last_refresh: f64,
    pub last_usage: f64,
    /// The target a press started on; the release activates it only when it lands on the same one.
    pub press: Target,
    /// The clock time of the last event, frame or tick (seconds).
    pub now: f64,
    /// Set by `refresh` when this sandbox has no `/sys`: the window then asks the shell's `disks` service for the
    /// volumes (`ask_shell`) and takes the answer in `on_volumes`.
    pub shell_scan: bool,
    /// The `volumes` call waiting for its answer.
    pub volumes_request: Option<u32>,
    /// Whether the last `volumes` answer failed, so the footer sentence it set is cleared by the next good one.
    pub scan_failed: bool,
}

impl DiskUtility {
    pub fn new() -> DiskUtility {
        DiskUtility {
            nodes: Vec::new(),
            selected: None,
            error: None,
            job: None,
            sheet: None,
            form: None,
            want: None,
            flow: None,
            finished_save: None,
            notice: None,
            smart: BTreeMap::new(),
            labels: TextCache::default(),
            show_all: true,
            erase_fs: Personality::Fat32,
            // The app host configures the actual client size before the first frame.
            size: (0.0, 0.0),
            last_refresh: f64::NEG_INFINITY,
            last_usage: f64::NEG_INFINITY,
            press: Target::Nothing,
            now: 0.0,
            shell_scan: false,
            volumes_request: None,
            scan_failed: false,
        }
    }

    /// Reads the disks, the mount table and (when due) usage. True when anything the window shows changed.
    pub fn refresh(&mut self, t: f64, force_usage: bool) -> bool {
        self.last_refresh = t;
        let before = self.nodes.clone();
        let mut nodes = sysfs::read_nodes(Path::new("/sys"), Path::new("/run/udev/data"));
        if nodes.is_empty() && !Path::new("/sys/block").is_dir() {
            // The app sandbox has no `/sys`, no udev database and no host mount table: the shell reads them for us.
            self.shell_scan = true;
            return false;
        }
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

    /// Asks the shell for the disks when `refresh` found no device tree to read. One call at a time.
    pub fn ask_shell(&mut self, ctx: &Ctx) {
        if !self.shell_scan || self.volumes_request.is_some() {
            return;
        }
        self.shell_scan = false;
        self.volumes_request = Some(service::call(ctx, disk_wire::SERVICE, "volumes", &Json::Null));
    }

    /// Takes the shell's answer to the `volumes` call. True when the window changed. Usage is not part of the answer
    /// (a mount point is not visible in this sandbox), so the rows show capacity, file system and mount point only.
    pub fn on_volumes(&mut self, result: &Result<Json, String>) -> bool {
        self.volumes_request = None;
        let list = result.as_ref().ok().and_then(|body| body.get("volumes")).and_then(Json::as_array);
        let volumes: Option<Vec<Volume>> = list.and_then(|l| l.iter().map(disk_wire::decode_volume).collect());
        let Some(volumes) = volumes else {
            let why = result.as_ref().err().map(|e| format!(" ({e})")).unwrap_or_default();
            self.error = Some(format!("The disks could not be listed: the disk service is not available{why}."));
            self.scan_failed = true;
            return true;
        };
        if std::mem::take(&mut self.scan_failed) {
            self.error = None;
        }
        let before = self.nodes.clone();
        let mut nodes = model::nodes_from_volumes(&volumes);
        model::carry_usage(&mut nodes, &self.nodes);
        for node in nodes.iter_mut().filter(|n| n.kind == Kind::Disk) {
            node.smart = self.smart.get(&node.name).cloned();
        }
        self.nodes = nodes;
        if let Some(name) = &self.selected {
            if !self.nodes.iter().any(|n| &n.name == name) {
                self.selected = None;
            }
        }
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
        if self.busy() {
            return false;
        }
        let Some(name) = self.selected.clone() else { return false };
        let disks = actions::disks_of(&self.nodes);
        let mounted = self.mounted_partitions(&name);
        match actions::plan(action, &disks, &name, &mounted) {
            Ok(plan) => {
                self.error = None;
                self.notice = None;
                self.job = Some(Job { verb: action.verb(), name, rx: actions::spawn(plan), commit: None });
            }
            Err(refusal) => self.error = Some(refusal.sentence()),
        }
        true
    }

    /// Runs a plan of commands on a thread with `verb` and `name` in the footer. `repairing` lets a checker that exits 1
    /// after fixing something count as a success.
    pub fn start_plan(&mut self, verb: &'static str, name: String, plan: Vec<Command>, repairing: bool) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        self.job = Some(Job { verb, name, rx: actions::spawn_with(plan, repairing), commit: None });
    }

    /// Takes the result of a running action when it has one. True when the window changed.
    pub fn poll(&mut self, t: f64) -> bool {
        let Some(job) = &self.job else { return false };
        let outcome = match job.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The operation ended without a result.".to_string()),
        };
        if let Some(request) = job.commit {
            self.finished_save = Some((request, outcome.is_ok()));
        }
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
        if let Some(job) = &self.job {
            return Some((format!("{} {}\u{2026}", job.verb, job.name), false));
        }
        self.notice.clone().map(|text| (text, false))
    }

    /// Opens the confirmation sheet for an erase or a first-aid request on the selected row. A request the disks already
    /// refuse (a mounted volume, a partition where a whole disk is needed) becomes the footer's error and no sheet opens.
    fn ask(&mut self, ask: Ask) -> bool {
        if self.busy() {
            return false;
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let name = node.name.clone();
        let (title, message, buttons, requests) = match ask {
            // the name, format and scheme are asked for in a form sheet, which then confirms
            Ask::Erase => return self.open_erase_form(),
            Ask::Info => {
                let extra = model::extra_rows(&probe::read_lsblk(&name));
                self.error = None;
                self.notice = None;
                self.sheet = Some(Pending { sheet: info_sheet(&format!("Info for {name}"), &model::info_lines(&node, &extra)), requests: Vec::new() });
                return true;
            }
            // the sizes and names are asked for in a form sheet, which then asks for the disk's name
            Ask::Partition(count) => return self.open_partition_form(count),
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
        self.notice = None;
        self.sheet = Some(Pending { sheet: Sheet::alert(&title, &model::wrap_text(&message, ALERT_CHARS), buttons), requests });
        true
    }

    /// Replaces any form with the confirmation sheet for `request`: `button` starts it, Cancel drops it.
    pub fn confirm(&mut self, title: String, request: DiskRequest, button: &str, destructive: bool) {
        let role = if destructive { ButtonRole::Destructive } else { ButtonRole::Default };
        let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, button, role)];
        let message = model::wrap_text(&request.confirm_text(), ALERT_CHARS);
        self.error = None;
        self.notice = None;
        self.sheet = Some(Pending { sheet: Sheet::alert(&title, &message, buttons), requests: vec![(SHEET_CONFIRM, request)] });
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
    pub(crate) fn start_request(&mut self, request: DiskRequest) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        let job = Job { verb: request.progress_word(), name: request.device().to_string(), rx: actions::spawn_request(request), commit: None };
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
            Target::Form(hit) => self.form_activate(hit),
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

    /// Whether Mount can run on the selected row: an unmounted volume with a file system, and no running action. The
    /// toolbar and the File menu both ask these three predicates, so they always agree.
    pub fn can_mount(&self) -> bool {
        !self.busy() && self.selected_node().is_some_and(|n| n.mount_point.is_none() && n.fs_type.is_some())
    }

    /// Whether Unmount can run on the selected row: a mounted row, and no running action.
    pub fn can_unmount(&self) -> bool {
        !self.busy() && self.selected_node().is_some_and(|n| n.mount_point.is_some())
    }

    /// Whether Eject can run on the selected row: a removable whole disk, and no running action.
    pub fn can_eject(&self) -> bool {
        !self.busy() && self.selected_node().is_some_and(|n| n.kind == Kind::Disk && n.removable)
    }

    /// The toolbar for the current state. Actions need a selected row and no running action.
    pub fn toolbar(&self) -> Toolbar {
        let idle = !self.busy();
        let node = self.selected_node();
        Toolbar::new(vec![
            ToolbarItem::button(B_MOUNT, "Mount").enabled(self.can_mount()),
            ToolbarItem::button(B_UNMOUNT, "Unmount").enabled(self.can_unmount()),
            ToolbarItem::button(B_EJECT, "Eject").enabled(self.can_eject()),
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
        if !self.show_all {
            items.push(SidebarItem::header("Volumes"));
            for i in self.volume_indices() {
                let v = &self.nodes[i];
                items.push(SidebarItem::row(ROW_BASE as u32 + i as u32, &model::display_name(v)).with_symbol("folder").with_trailing(&model::format_size(v.size)));
            }
            let mut list = SourceList::new(items, ROW_PT, HEAD_PT);
            list.set_selected(self.selected_index().map(|i| ROW_BASE as u32 + i as u32));
            return list;
        }
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

    /// The rows Show Only Volumes lists: every partition, and a disk that holds a file system itself and has no partitions.
    pub fn volume_indices(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                n.kind == Kind::Partition || (n.fs_type.is_some() && !self.nodes.iter().any(|p| p.parent.as_deref() == Some(n.name.as_str())))
            })
            .collect()
    }

    /// Switches between Show Only Volumes and Show All Devices. A selected disk is dropped when volumes-only hides it.
    pub fn set_show_all(&mut self, all: bool) -> bool {
        if self.show_all == all {
            return false;
        }
        self.show_all = all;
        if !all {
            let hidden = self.selected_index().is_some_and(|i| !self.volume_indices().contains(&i));
            if hidden {
                self.selected = None;
            }
        }
        true
    }

    /// Chooses the file system Erase and Partition write.
    pub fn set_erase_fs(&mut self, fs: Personality) -> bool {
        let changed = self.erase_fs != fs;
        self.erase_fs = fs;
        changed
    }

    /// Opens the Get Info, erase, first-aid or partition sheet from a menu command.
    pub fn ask_menu(&mut self, ask: Ask) -> bool {
        self.ask(ask)
    }

    /// Starts Mount, Unmount or Eject from a menu command.
    pub fn run_action(&mut self, action: Action) -> bool {
        self.start(action)
    }

    /// The window's rectangles in points.
    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    /// The toolbar's rectangle: the bar to the right of the traffic lights the shell draws over the window's top left.
    pub fn toolbar_bounds(&self) -> Rect {
        let x = widgets::toolbar_lead_pt();
        Rect { x, y: 0.0, w: (self.size.0 - x - PAD_PT / 2.0).max(0.0), h: UNIFIED_BAR_PT }
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
        widgets::toolbar_layout(ui, self.toolbar_bounds(), &self.toolbar())
    }

    /// What the point is over while a form sheet is open (it blocks the window), or `None` when there is no form.
    pub fn form_target(&self, x: f32, y: f32) -> Option<Target> {
        let open = self.form.as_ref()?;
        Some(match open.form.hit(self.window_rect(), UNIFIED_BAR_PT, x, y) {
            FormHit::Nothing => Target::Nothing,
            hit => Target::Form(hit),
        })
    }

    /// What the point is over. Points, with the window's origin at the top left.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        // an open sheet blocks the window: only its buttons (and a form's fields) answer
        if let Some(target) = self.form_target(x, y) {
            return target;
        }
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
    fn the_toolbar_starts_after_the_traffic_lights() {
        use bifrost_app::layout::traffic_lights_trailing;
        let mut w = DiskUtility::new();
        w.size = (1000.0, 700.0);
        let bounds = w.toolbar_bounds();
        assert!(bounds.x >= traffic_lights_trailing(), "the first button is not under the window controls");
        assert!(bounds.x + bounds.w <= w.size.0 && bounds.h == UNIFIED_BAR_PT);
        w.size = (10.0, 10.0);
        assert_eq!(w.toolbar_bounds().w, 0.0, "a tiny window never gets a negative width");
    }

    #[test]
    fn the_mount_predicates_disable_every_mount_command_while_busy() {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, size: 8_000_000_000, ..Node::default() },
            Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), fs_type: Some("vfat".into()), ..Node::default() },
        ];
        w.selected = Some("sdb1".into());
        assert!(w.can_mount() && !w.can_unmount() && !w.can_eject(), "an unmounted volume mounts, and a partition does not eject");
        w.nodes[1].mount_point = Some("/mnt/a".into());
        assert!(w.can_unmount() && !w.can_mount());
        w.selected = Some("sdb".into());
        assert!(w.can_eject() && !w.can_unmount() && !w.can_mount(), "a disk without a file system neither mounts nor unmounts");
        w.job = Some(Job { verb: "Ejecting", name: "sdb".into(), rx: std::sync::mpsc::channel().1, commit: None });
        assert!(!w.can_eject() && !w.can_unmount(), "a running action blocks the commands");
        let bar = w.toolbar();
        assert!(bar.items.iter().filter(|i| matches!(i.id, B_MOUNT | B_UNMOUNT | B_EJECT)).all(|i| !i.enabled));
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
        w.size = (1200.0, 780.0);
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
    fn an_erase_asks_for_its_options_then_confirms_and_cancel_closes_without_a_job() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.activate(Target::Toolbar(B_ERASE), 1.0));
        assert!(w.form.is_some() && w.sheet.is_none(), "the options come first");
        assert!(w.activate(Target::Form(FormHit::Button(crate::form::FORM_OK)), 1.0));
        // a whole disk is erased only after its name is typed into the form again: the typed name is the confirmation
        assert!(w.form.is_some() && w.sheet.is_none(), "the name is asked first");
        assert!(w.activate(Target::Form(FormHit::Button(crate::form::FORM_CANCEL)), 1.0));
        assert!(w.form.is_none() && w.sheet.is_none() && w.job.is_none(), "cancelling the name sends nothing");
        // a volume asks for the options, then a confirmation sheet
        w.selected = Some("sdb1".into());
        assert!(w.activate(Target::Toolbar(B_ERASE), 1.0));
        assert!(w.form.is_some() && w.sheet.is_none(), "the options come first");
        assert!(w.activate(Target::Form(FormHit::Button(crate::form::FORM_OK)), 1.0));
        let pending = w.sheet.as_ref().expect("sheet");
        assert!(pending.requests.iter().any(|(id, r)| *id == SHEET_CONFIRM && r.method() == "disk.erase_volume"));
        assert!(w.form.is_none());
        assert!(w.activate(Target::Sheet(SHEET_CANCEL), 1.0));
        assert!(w.sheet.is_none() && w.job.is_none());
    }

    #[test]
    fn an_open_form_blocks_the_toolbar_and_the_sidebar_under_it() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.activate(Target::Toolbar(B_ERASE), 1.0));
        assert_eq!(w.form_target(100.0, 40.0), Some(Target::Nothing), "the dimmed window answers nothing");
        assert!(!w.activate(Target::Toolbar(B_FIRST_AID), 1.0), "busy");
        assert!(w.toolbar().items.iter().filter(|i| i.id == B_ERASE).all(|i| !i.enabled));
        let geo = w.form.as_ref().unwrap().form.geometry(w.window_rect(), UNIFIED_BAR_PT);
        let r = geo.rows[0].control;
        assert_eq!(w.form_target(r.x + 10.0, r.y + 5.0), Some(Target::Form(FormHit::Text(0))));
        w.form = None;
        assert_eq!(w.form_target(0.0, 0.0), None);
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

    #[test]
    fn show_only_volumes_lists_partitions_and_drops_a_hidden_selection() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.set_show_all(false));
        assert!(!w.set_show_all(false));
        assert_eq!(w.volume_indices(), vec![1]);
        assert!(w.selected.is_none());
        let ids: Vec<u32> = w.sidebar().items.iter().filter_map(|i| if let SidebarItem::Row { id, .. } = i { Some(*id) } else { None }).collect();
        assert_eq!(ids, vec![ROW_BASE as u32 + 1]);
        assert!(w.set_show_all(true));
    }

    #[test]
    fn get_info_opens_a_sheet_without_a_request() {
        let mut w = one_stick();
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Info));
        let pending = w.sheet.as_ref().expect("sheet");
        assert!(pending.requests.is_empty() && pending.sheet.message.contains("Device: /dev/sdb1"));
    }

    #[test]
    fn erase_uses_the_chosen_format_and_keeps_a_valid_label() {
        let mut w = one_stick();
        w.nodes[1].label = Some("Games".into());
        w.selected = Some("sdb1".into());
        assert!(w.set_erase_fs(Personality::ExFat));
        assert!(w.ask_menu(Ask::Erase));
        assert!(w.submit_form());
        let (_, request) = &w.sheet.as_ref().expect("sheet").requests[0];
        assert_eq!(*request, DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::ExFat, label: "Games".into() });
    }

    #[test]
    fn partition_opens_a_sheet_with_equal_parts_and_refuses_a_volume() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.ask_menu(Ask::Partition(2)));
        let open = w.form.as_ref().expect("the partition sheet");
        assert!(w.sheet.is_none(), "nothing is confirmed before the sizes are accepted");
        assert!(open.form.value(1).ends_with(", rest") && open.form.value(1).matches(',').count() == 1);
        w.form = None;
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Partition(2)));
        assert!(w.form.is_none() && w.error.is_some());
    }
}
