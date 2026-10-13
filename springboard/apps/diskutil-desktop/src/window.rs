//! The Disk Utility window's state, geometry and hit tests: the disks and volumes (sidebar), the toolbar, the info pane
//! of the selected row and the footer. Layout is in points and derives from the window size; the drawing code and the
//! hit tests read the same rectangles from here, so a tap lands on what is drawn. The disks are re-read every
//! `REFRESH_S` seconds, and usage every `USAGE_S` seconds.

use crate::actions::{self, Action};
use crate::form::{Field, Form, FormKey};
use crate::ops::{self, ImageFormat};
use crate::model::{self, Kind, Node};
use crate::{probe, sysfs};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton, BUTTON_H_PT, SHEET_PAD_PT};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::KeyInput;
use bifrost_core::disk_commands::Command;
use bifrost_core::disk_format::{PartitionSpec, Personality};
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
pub const B_PARTITION: u32 = 8;
/// The confirmation sheet's buttons: Cancel closes it; the others answer with the request they name.
pub const SHEET_CANCEL: u32 = 10;
pub const SHEET_CONFIRM: u32 = 11;
pub const SHEET_VERIFY: u32 = 12;
pub const SHEET_REPAIR: u32 = 13;

/// Accessibility and sidebar ids: a row's id is `ROW_BASE` plus its index in `DiskUtility::nodes`.
pub const ROW_BASE: u64 = 100;
/// Accessibility ids of a sheet's text fields: `FIELD_BASE` plus the field's index.
pub const FIELD_BASE: u64 = 200_000;

/// A form sheet is wider than an alert, and each field adds this much height (points).
pub const FORM_W_PT: f32 = 340.0;
pub const FIELD_ROW_PT: f32 = 30.0;
pub const FIELD_H_PT: f32 = 22.0;
/// The width of the label column left of a field (points).
pub const FIELD_LABEL_PT: f32 = 70.0;

/// What a point or an accessibility id stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// An index into `DiskUtility::nodes`.
    Row(usize),
    /// A button of the confirmation sheet.
    Sheet(u32),
    /// A text field of the sheet.
    Field(usize),
}

/// The erase or first-aid action the user asked for on the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Erase,
    FirstAid,
    /// Get Info: every fact about the row in a sheet.
    Info,
    /// Partition the selected disk into this many equal parts.
    Partition(usize),
    /// Partition the selected disk into the sizes typed in the sheet.
    PartitionCustom,
    /// Rename the selected volume.
    Rename,
    /// Copy another volume onto the selected one.
    Restore,
    /// Make a blank disk image.
    NewImage,
}

/// What a sheet's fields turn into when the user confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Erase,
    Rename,
    Partition,
    Restore,
    NewImage,
}

/// What a confirmed form starts: a privileged request, or commands run as the user.
#[derive(Debug, PartialEq)]
pub enum Built {
    Request(DiskRequest),
    Plan { verb: &'static str, name: String, plan: Vec<Command> },
}

/// A confirmation sheet and the requests its buttons start, by button id.
pub struct Pending {
    pub sheet: Sheet,
    pub requests: Vec<(u32, DiskRequest)>,
    /// The text fields of the sheet and what they build, for the sheets that ask for a name or a size.
    pub form: Option<Form>,
    pub kind: Option<FormKind>,
    /// The device the sheet is about (the selected row when it opened).
    pub device: String,
}

impl Pending {
    /// A sheet with buttons only.
    pub fn plain(sheet: Sheet, requests: Vec<(u32, DiskRequest)>) -> Pending {
        Pending { sheet, requests, form: None, kind: None, device: String::new() }
    }
}

/// An alert sheet widened and made taller for `form`'s fields.
fn form_sheet(title: &str, message: &str, buttons: Vec<SheetButton>, form: &Form) -> Sheet {
    let mut sheet = Sheet::alert(title, message, buttons);
    sheet.size.0 = FORM_W_PT;
    sheet.size.1 += form.fields.len() as f32 * FIELD_ROW_PT;
    sheet
}

/// Where a sheet's fields sit, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct FormLayout {
    pub panel: Rect,
    /// The top of the button row.
    pub buttons_y: f32,
    pub fields: Vec<Rect>,
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
    /// View > Show All Devices (disks with their volumes) or Show Only Volumes.
    pub show_all: bool,
    /// The file system Erase and Partition write.
    pub erase_fs: Personality,
    /// The home folder, where new disk images are saved.
    pub home: String,
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
            show_all: true,
            erase_fs: Personality::Fat32,
            home: std::env::var("HOME").unwrap_or_default(),
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
        if ask == Ask::NewImage {
            return self.ask_new_image();
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let name = node.name.clone();
        let mut form: Option<(FormKind, Form)> = None;
        let (title, message, buttons, requests) = match ask {
            Ask::Erase => {
                let fs = self.erase_fs;
                let request = if node.kind == Kind::Disk {
                    DiskRequest::EraseDisk { device: name.clone(), fs, label: String::new() }
                } else {
                    let label = model::erase_label(node.label.as_deref(), fs.label_limit());
                    DiskRequest::EraseVolume { device: name.clone(), fs, label }
                };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Erase", ButtonRole::Destructive)];
                let initial = match &request {
                    DiskRequest::EraseVolume { label, .. } => label.clone(),
                    _ => String::new(),
                };
                form = Some((FormKind::Erase, Form::new(vec![Field::new("Name", &initial, "Untitled")])));
                (format!("Erase {name}?"), request.confirm_text(), buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::Info => {
                let message = model::detail_rows(&node).iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
                self.error = None;
                let done = vec![SheetButton::new(SHEET_CANCEL, "Done", ButtonRole::Cancel)];
                self.sheet = Some(Pending::plain(Sheet::alert(&format!("Info for {name}"), &message, done), Vec::new()));
                return true;
            }
            Ask::Partition(count) => {
                let sizes = model::equal_parts_mib(node.size, count);
                if node.kind != Kind::Disk || sizes.is_empty() {
                    self.error = Some(format!("{name} cannot be split into {count} partitions."));
                    return true;
                }
                let partitions = sizes.into_iter().map(|size_mib| PartitionSpec { size_mib, fs: self.erase_fs, label: String::new() }).collect();
                let request = DiskRequest::PartitionDisk { device: name.clone(), partitions };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Partition", ButtonRole::Destructive)];
                (format!("Partition {name}?"), request.confirm_text(), buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::PartitionCustom => {
                if node.kind != Kind::Disk {
                    self.error = Some(format!("{name} is a volume. Choose a disk to partition."));
                    return true;
                }
                let whole = PartitionSpec { size_mib: 0, fs: self.erase_fs, label: String::new() };
                let request = DiskRequest::PartitionDisk { device: name.clone(), partitions: vec![whole] };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Partition", ButtonRole::Destructive)];
                let message = format!(
                    "List the size of each new partition, separated by commas: 64 GB, 25%, rest. The last one may be rest. Each is formatted as {}. All data on {name} will be lost.",
                    model::fs_title(self.erase_fs)
                );
                form = Some((FormKind::Partition, Form::new(vec![Field::new("Sizes", "", "32 GB, rest")])));
                (format!("Partition {name}?"), message, buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::Rename => {
                if node.kind != Kind::Partition {
                    self.error = Some(format!("{name} is a disk. Choose a volume to rename."));
                    return true;
                }
                let current = node.label.clone().unwrap_or_default();
                let probe = if current.is_empty() { "X".to_string() } else { current.clone() };
                let request = DiskRequest::RenameVolume { device: name.clone(), label: probe };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Rename", ButtonRole::Default)];
                form = Some((FormKind::Rename, Form::new(vec![Field::new("Name", &current, "Untitled")])));
                (format!("Rename {name}"), format!("Enter a new name for {name}. The volume must not be mounted."), buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::Restore => {
                let sources = ops::restore_sources(&self.nodes, &node);
                let Some(first) = sources.first().cloned() else {
                    self.error = Some(format!("No unmounted device of the same kind and no larger than {name} can be copied onto it."));
                    return true;
                };
                let request = DiskRequest::RestoreVolume { device: name.clone(), source: first.clone() };
                let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Restore", ButtonRole::Destructive)];
                let shown: Vec<&str> = sources.iter().take(6).map(String::as_str).collect();
                let message = format!("Copy a device block for block onto {name}. All data on {name} will be lost. Available: {}.", shown.join(", "));
                form = Some((FormKind::Restore, Form::new(vec![Field::new("Copy from", &first, "device name")])));
                (format!("Restore {name}?"), message, buttons, vec![(SHEET_CONFIRM, request)])
            }
            Ask::NewImage => return false,
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
        self.sheet = Some(match form {
            Some((kind, form)) => Pending { sheet: form_sheet(&title, &message, buttons, &form), requests, form: Some(form), kind: Some(kind), device: name },
            None => Pending::plain(Sheet::alert(&title, &message, buttons), requests),
        });
        true
    }

    /// Opens the sheet that asks for the name and size of a new blank image.
    fn ask_new_image(&mut self) -> bool {
        let form = Form::new(vec![Field::new("Name", "", "Untitled"), Field::new("Size", "1 GB", "1 GB")]);
        let message = format!("The image is saved in your Documents folder, formatted as {}, and opened. Change the format in the Disk menu.", model::fs_title(self.erase_fs));
        let buttons = vec![SheetButton::new(SHEET_CANCEL, "Cancel", ButtonRole::Cancel), SheetButton::new(SHEET_CONFIRM, "Create", ButtonRole::Default)];
        let sheet = form_sheet("New Blank Image", &message, buttons, &form);
        self.error = None;
        self.sheet = Some(Pending { sheet, requests: Vec::new(), form: Some(form), kind: Some(FormKind::NewImage), device: String::new() });
        true
    }

    /// What the form's fields build for `kind` on `device`, or the sentence for the field that is wrong. The request is
    /// checked against the disks here, so a refusal shows in the sheet and nothing is sent.
    pub fn build_from_form(&self, kind: FormKind, device: &str, form: &Form) -> Result<Built, String> {
        let node = self.nodes.iter().find(|n| n.name == device);
        let checked = |request: DiskRequest| -> Result<Built, String> {
            actions::check_request(&request, &self.nodes)?;
            Ok(Built::Request(request))
        };
        match kind {
            FormKind::Erase => {
                let node = node.ok_or("The device is no longer connected.")?;
                let label = form.value(0);
                let fs = self.erase_fs;
                checked(if node.kind == Kind::Disk {
                    DiskRequest::EraseDisk { device: device.to_string(), fs, label }
                } else {
                    DiskRequest::EraseVolume { device: device.to_string(), fs, label }
                })
            }
            FormKind::Rename => {
                let label = form.value(0);
                if label.is_empty() {
                    return Err("Enter a name.".to_string());
                }
                checked(DiskRequest::RenameVolume { device: device.to_string(), label })
            }
            FormKind::Partition => {
                let node = node.ok_or("The device is no longer connected.")?;
                let sizes = ops::parse_partition_sizes(&form.value(0), node.size)?;
                let partitions = sizes.into_iter().map(|size_mib| PartitionSpec { size_mib, fs: self.erase_fs, label: String::new() }).collect();
                checked(DiskRequest::PartitionDisk { device: device.to_string(), partitions })
            }
            FormKind::Restore => {
                let source = form.value(0);
                if !self.nodes.iter().any(|n| n.name == source) {
                    return Err(format!("\u{201c}{source}\u{201d} is not a disk or volume."));
                }
                checked(DiskRequest::RestoreVolume { device: device.to_string(), source })
            }
            FormKind::NewImage => {
                let (path, plan) = ops::new_image_plan(&self.home, &form.value(0), &form.value(1), self.erase_fs)?;
                if Path::new(&path).exists() {
                    return Err("An image with that name already exists in Documents.".to_string());
                }
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                Ok(Built::Plan { verb: "Creating", name, plan })
            }
        }
    }

    /// Confirms the open form: starts what it builds, or shows why it cannot in the sheet.
    fn submit_form(&mut self) -> bool {
        let Some(pending) = &self.sheet else { return false };
        let (Some(kind), Some(form)) = (pending.kind, pending.form.clone()) else { return false };
        let device = pending.device.clone();
        match self.build_from_form(kind, &device, &form) {
            Ok(Built::Request(request)) => {
                self.sheet = None;
                self.start_request(request);
            }
            Ok(Built::Plan { verb, name, plan }) => {
                self.sheet = None;
                self.start_plan(verb, name, plan);
            }
            Err(sentence) => {
                if let Some(form) = self.sheet.as_mut().and_then(|p| p.form.as_mut()) {
                    form.error = Some(sentence);
                }
            }
        }
        true
    }

    /// Runs commands as the user on a thread, with `verb name` in the footer.
    pub fn start_plan(&mut self, verb: &'static str, name: String, plan: Vec<Command>) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        self.job = Some(Job { verb, name, rx: actions::spawn(plan) });
    }

    /// Attaches the image file at `path` (File > Open Disk Image).
    pub fn attach_image(&mut self, path: &str) -> bool {
        if self.job.is_some() {
            return false;
        }
        match ops::attach_plan(path) {
            Ok(plan) => {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                self.start_plan("Opening", name, plan);
            }
            Err(sentence) => self.error = Some(sentence),
        }
        true
    }

    /// Writes a converted copy of the selected disk image next to it (Images > Convert).
    pub fn convert_image(&mut self, format: ImageFormat) -> bool {
        if self.job.is_some() {
            return false;
        }
        let Some(source) = self.selected_node().and_then(|n| n.image_file.clone()) else { return false };
        match ops::convert_plan(&source, format) {
            Ok((target, plan)) => {
                if Path::new(&target).exists() {
                    self.error = Some(format!("{} already exists.", target.rsplit('/').next().unwrap_or(&target)));
                } else {
                    self.start_plan("Converting", source.rsplit('/').next().unwrap_or(&source).to_string(), plan);
                }
            }
            Err(sentence) => self.error = Some(sentence),
        }
        true
    }

    /// Sends a key to the open sheet: a form takes the typing, Return confirms and Escape cancels. True when the window
    /// needs a new frame.
    pub fn key(&mut self, key: &KeyInput) -> bool {
        if self.sheet.is_none() {
            return false;
        }
        if let Some(outcome) = self.sheet.as_mut().and_then(|p| p.form.as_mut()).map(|f| f.key(key)) {
            return match outcome {
                FormKey::Confirm => self.answer(SHEET_CONFIRM),
                FormKey::Cancel => self.answer(SHEET_CANCEL),
                FormKey::Edited => true,
                FormKey::Ignored => false,
            };
        }
        let (default, cancel) = self.sheet.as_ref().map(|p| (p.sheet.default_button(), p.sheet.cancel_button())).unwrap_or((None, None));
        match key {
            KeyInput::Code(13) => default.or(cancel).is_some_and(|id| self.answer(id)),
            KeyInput::Code(27) | KeyInput::Code(1) => cancel.is_some_and(|id| self.answer(id)),
            _ => false,
        }
    }

    /// The sheet's panel, button row and field rectangles, when the open sheet has a form.
    pub fn form_layout(&self) -> Option<FormLayout> {
        let pending = self.sheet.as_ref()?;
        let form = pending.form.as_ref()?;
        let l = pending.sheet.layout(self.window_rect(), UNIFIED_BAR_PT, 1.0);
        let buttons_y = l.buttons.first().map(|(_, r)| r.y).unwrap_or(l.panel.bottom() - SHEET_PAD_PT - BUTTON_H_PT);
        let n = form.fields.len();
        let x = l.panel.x + SHEET_PAD_PT + FIELD_LABEL_PT;
        let w = (l.panel.w - 2.0 * SHEET_PAD_PT - FIELD_LABEL_PT).max(0.0);
        let fields = (0..n).map(|i| Rect { x, y: buttons_y - (n - i) as f32 * FIELD_ROW_PT, w, h: FIELD_H_PT }).collect();
        Some(FormLayout { panel: l.panel, buttons_y, fields })
    }

    /// The sheet's answer. Cancel closes it; another button starts the request it names.
    fn answer(&mut self, id: u32) -> bool {
        if id == SHEET_CONFIRM && self.sheet.as_ref().is_some_and(|p| p.form.is_some()) {
            return self.submit_form();
        }
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
            Target::Field(i) => self.sheet.as_mut().and_then(|p| p.form.as_mut()).is_some_and(|f| f.set_focus(i)),
            Target::Toolbar(B_PARTITION) => self.ask(Ask::PartitionCustom),
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
            ToolbarItem::button(B_PARTITION, "Partition").enabled(idle && node.is_some_and(|n| n.kind == Kind::Disk)),
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
                Hit::Panel => self.form_layout().and_then(|l| l.fields.iter().position(|r| r.contains(x, y))).map(Target::Field).unwrap_or(Target::Nothing),
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
            x if (FIELD_BASE..FIELD_BASE + 16).contains(&x) => Target::Field((x - FIELD_BASE) as usize),
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
        let (_, request) = &w.sheet.as_ref().expect("sheet").requests[0];
        assert_eq!(*request, DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::ExFat, label: "Games".into() });
    }

    #[test]
    fn partition_splits_a_disk_equally_and_refuses_a_volume() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.ask_menu(Ask::Partition(2)));
        let (_, request) = &w.sheet.as_ref().expect("sheet").requests[0];
        match request {
            DiskRequest::PartitionDisk { partitions, .. } => assert_eq!(partitions.len(), 2),
            other => panic!("{other:?}"),
        }
        w.sheet = None;
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Partition(2)));
        assert!(w.sheet.is_none() && w.error.is_some());
    }

    #[test]
    fn rename_opens_a_name_field_for_a_volume_and_refuses_a_disk() {
        let mut w = one_stick();
        w.nodes[1].label = Some("Games".into());
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Rename));
        let pending = w.sheet.as_ref().expect("sheet");
        assert_eq!(pending.kind, Some(FormKind::Rename));
        assert_eq!(pending.form.as_ref().map(|f| f.value(0)), Some("Games".to_string()));
        w.sheet = None;
        w.selected = Some("sdb".into());
        assert!(w.ask_menu(Ask::Rename));
        assert!(w.sheet.is_none() && w.error.is_some());
    }

    #[test]
    fn typing_into_the_rename_sheet_builds_a_rename_request() {
        let mut w = one_stick();
        w.nodes[1].fs_type = Some("ext4".into());
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Rename));
        assert!(w.key(&KeyInput::Text("Photos".into())));
        let form = w.sheet.as_ref().and_then(|p| p.form.clone()).expect("form");
        assert_eq!(w.build_from_form(FormKind::Rename, "sdb1", &form), Ok(Built::Request(DiskRequest::RenameVolume { device: "sdb1".into(), label: "Photos".into() })));
        let empty = Form::new(vec![Field::new("Name", "  ", "")]);
        assert_eq!(w.build_from_form(FormKind::Rename, "sdb1", &empty), Err("Enter a name.".to_string()));
    }

    #[test]
    fn a_bad_name_stays_in_the_sheet_with_its_reason() {
        let mut w = one_stick();
        w.nodes[1].fs_type = Some("vfat".into());
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Rename));
        w.key(&KeyInput::Text("A name that is far too long".into()));
        assert!(w.key(&KeyInput::Code(13)));
        let pending = w.sheet.as_ref().expect("the sheet stays open");
        assert!(pending.form.as_ref().and_then(|f| f.error.as_ref()).is_some());
        assert!(w.job.is_none());
        assert!(w.key(&KeyInput::Code(27)));
        assert!(w.sheet.is_none());
    }

    #[test]
    fn the_erase_sheet_carries_a_name_field_that_feeds_the_label() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.ask_menu(Ask::Erase));
        let form = w.sheet.as_ref().and_then(|p| p.form.clone()).expect("form");
        let mut named = form.clone();
        named.fields[0].text = "STICK".into();
        assert_eq!(
            w.build_from_form(FormKind::Erase, "sdb", &named),
            Ok(Built::Request(DiskRequest::EraseDisk { device: "sdb".into(), fs: Personality::Fat32, label: "STICK".into() }))
        );
    }

    #[test]
    fn a_custom_partition_layout_is_parsed_from_the_sheet() {
        let mut w = one_stick();
        w.selected = Some("sdb".into());
        assert!(w.activate(Target::Toolbar(B_PARTITION), 1.0));
        assert_eq!(w.sheet.as_ref().and_then(|p| p.kind), Some(FormKind::Partition));
        let form = Form::new(vec![Field::new("Sizes", "2 GB, rest", "")]);
        match w.build_from_form(FormKind::Partition, "sdb", &form) {
            Ok(Built::Request(DiskRequest::PartitionDisk { partitions, .. })) => {
                assert_eq!(partitions.len(), 2);
                assert_eq!(partitions[1].size_mib, 0);
            }
            other => panic!("{other:?}"),
        }
        let too_big = Form::new(vec![Field::new("Sizes", "20 GB", "")]);
        assert!(w.build_from_form(FormKind::Partition, "sdb", &too_big).is_err());
    }

    #[test]
    fn the_partition_button_needs_a_disk() {
        let mut w = one_stick();
        let enabled = |w: &DiskUtility| w.toolbar().items.iter().find(|i| i.id == B_PARTITION).is_some_and(|i| i.enabled);
        w.selected = Some("sdb".into());
        assert!(enabled(&w));
        w.selected = Some("sdb1".into());
        assert!(!enabled(&w));
    }

    #[test]
    fn restore_offers_the_other_unmounted_volumes() {
        let mut w = one_stick();
        w.nodes.push(Node { name: "sdc1".into(), kind: Kind::Partition, parent: Some("sdc".into()), size: 1, ..Node::default() });
        w.nodes[1].size = 4_000_000_000;
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Restore));
        let form = w.sheet.as_ref().and_then(|p| p.form.clone()).expect("form");
        assert_eq!(form.value(0), "sdc1");
        let unknown = Form::new(vec![Field::new("Copy from", "sdq9", "")]);
        assert!(w.build_from_form(FormKind::Restore, "sdb1", &unknown).unwrap_err().contains("sdq9"));
    }

    #[test]
    fn a_new_image_form_builds_a_plan_in_the_documents_folder() {
        let mut w = DiskUtility::new();
        w.home = "/nonexistent-home".into();
        assert!(w.ask_menu(Ask::NewImage));
        assert_eq!(w.sheet.as_ref().and_then(|p| p.kind), Some(FormKind::NewImage));
        let form = Form::new(vec![Field::new("Name", "Backup", ""), Field::new("Size", "100 MB", "")]);
        match w.build_from_form(FormKind::NewImage, "", &form) {
            Ok(Built::Plan { verb, name, plan }) => {
                assert_eq!((verb, name.as_str()), ("Creating", "Backup.img"));
                assert_eq!(plan.first().map(|c| c.program), Some("mkdir"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_click_on_a_field_moves_the_focus() {
        let mut w = DiskUtility::new();
        assert!(w.ask_menu(Ask::NewImage));
        let layout = w.form_layout().expect("layout");
        assert_eq!(layout.fields.len(), 2);
        let r = layout.fields[1];
        let (px, py) = (r.x + 4.0, r.y + 4.0);
        assert_eq!(w.sheet.as_ref().expect("sheet").sheet.hit(w.window_rect(), UNIFIED_BAR_PT, 1.0, px, py), Hit::Panel);
        assert_eq!(layout.fields.iter().position(|f| f.contains(px, py)), Some(1));
        assert!(w.activate(Target::Field(1), 1.0));
        assert_eq!(w.sheet.as_ref().and_then(|p| p.form.as_ref()).map(|f| f.focus), Some(1));
    }

    #[test]
    fn return_closes_the_info_sheet() {
        let mut w = one_stick();
        w.selected = Some("sdb1".into());
        assert!(w.ask_menu(Ask::Info));
        assert!(w.key(&KeyInput::Code(13)));
        assert!(w.sheet.is_none());
    }
}
