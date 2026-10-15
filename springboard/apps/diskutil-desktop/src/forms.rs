//! The form sheets of Disk Utility and what their OK buttons do: Rename, Erase (name, format and scheme), Restore, the
//! RAID Assistant, and the disk image sheets (New Blank Image, Convert Image, Resize Image). Each sheet is built from
//! the selected row or the disks, and OK turns what was entered into a privileged request (checked against the disks,
//! then confirmed in an alert sheet), a plan that runs on a thread, or a save panel.

use crate::actions;
use crate::flow::{SaveOp, Want};
use crate::form::{Form, FormHit, FORM_CANCEL, FORM_OK};
use crate::images::{self, BlankSpec, ImageFormat};
use crate::model::{self, Kind, Node};
use crate::window::DiskUtility;
use bifrost_core::disk_format::{Personality, RaidLevel, Scheme};
use bifrost_core::disk_request::DiskRequest;

/// What a form was opened for, with what OK needs besides the entered values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormKind {
    /// Rename the volume `device`.
    Rename { device: String },
    /// Erase `device`; `whole` is set for a disk, which also picks a partition scheme.
    Erase { device: String, whole: bool },
    /// Copy one of the `sources` (device names, in list order) onto `device`.
    Restore { device: String, sources: Vec<String> },
    /// Create an array from some of the `members` (device names, in list order).
    Raid { members: Vec<String> },
    NewBlank,
    /// Convert the image file `src`.
    Convert { src: String },
    /// Resize the image file `path`, now `size` bytes long.
    Resize { path: String, size: u64, ext: bool },
}

/// The open form sheet.
#[derive(Clone, Debug)]
pub struct OpenForm {
    pub form: Form,
    pub kind: FormKind,
}

/// The file systems the format choices list, as their titles.
fn fs_titles() -> Vec<&'static str> {
    Personality::ALL.iter().map(|f| f.title()).collect()
}

/// A one-line description of a disk or volume for a list row: `sdb1 \u{b7} 8.0 GB \u{b7} Games`.
pub fn row_text(node: &Node) -> String {
    let mut text = format!("{} \u{b7} {}", node.name, model::format_size(node.size));
    if let Some(label) = node.label.as_deref().filter(|l| !l.is_empty()) {
        text.push_str(&format!(" \u{b7} {label}"));
    } else if !node.model.is_empty() {
        text.push_str(&format!(" \u{b7} {}", node.model));
    }
    text
}

impl DiskUtility {
    /// Whether a text field has the keyboard, so the shell should bring its keyboard up.
    pub fn wants_keyboard(&self) -> bool {
        self.form.as_ref().is_some_and(|f| f.form.takes_text())
    }

    /// Opens a form sheet over the window.
    fn show_form(&mut self, form: Form, kind: FormKind) -> bool {
        self.error = None;
        self.notice = None;
        self.form = Some(OpenForm { form, kind });
        true
    }

    /// Disk > Rename: the name of the selected volume. A refusal the disks already give (a mounted volume, no file
    /// system) is the footer's error and no sheet opens.
    pub fn open_rename_form(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let probe = DiskRequest::RenameVolume { device: node.name.clone(), label: "X".to_string() };
        if let Err(sentence) = actions::check_request(&probe, &self.nodes) {
            self.error = Some(sentence);
            return true;
        }
        let limit = node.fs_type.as_deref().and_then(Personality::from_probe_type).map_or(32, Personality::label_limit);
        let form = Form::new(&format!("Rename {}", node.name), "Rename", false)
            .text("Name", node.label.as_deref().unwrap_or(""), "Untitled", limit)
            .note(&format!("The name can hold {limit} letters, digits, spaces, - and _."));
        self.show_form(form, FormKind::Rename { device: node.name })
    }

    /// Disk > Erase: the name, format and (for a disk) partition scheme to erase the selected row with.
    pub fn open_erase_form(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let whole = node.kind == Kind::Disk;
        let fs = self.erase_fs;
        let first = if whole {
            DiskRequest::EraseDisk { device: node.name.clone(), fs, label: String::new(), scheme: Scheme::Gpt }
        } else {
            DiskRequest::EraseVolume { device: node.name.clone(), fs, label: String::new() }
        };
        if let Err(sentence) = actions::check_request(&first, &self.nodes) {
            self.error = Some(sentence);
            return true;
        }
        let kept = if whole { String::new() } else { model::erase_label(node.label.as_deref(), fs.label_limit()) };
        let index = Personality::ALL.iter().position(|p| *p == fs).unwrap_or(0);
        let mut form = Form::new(&format!("Erase {}?", node.name), "Erase", true)
            .text("Name", &kept, "Untitled", 32)
            .choice("Format", &fs_titles(), index);
        if whole {
            form = form.choice("Scheme", &[Scheme::Gpt.title(), Scheme::Mbr.title()], 0);
        }
        let form = form.note(&format!("Erasing {} deletes everything on it.", node.name));
        self.show_form(form, FormKind::Erase { device: node.name, whole })
    }

    /// The volumes `node` could be restored from: not itself, not its disk or its partitions, and small enough to fit.
    fn restore_sources(&self, node: &Node) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                n.name != node.name
                    && n.size > 0
                    && n.size <= node.size
                    && n.parent.as_deref() != Some(node.name.as_str())
                    && node.parent.as_deref() != Some(n.name.as_str())
            })
            .collect()
    }

    /// Disk > Restore: copy another volume or an attached disk image onto the selected volume.
    pub fn open_restore_form(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let Some(node) = self.selected_node().cloned() else { return false };
        let picks = self.restore_sources(&node);
        if picks.is_empty() {
            self.error = Some(format!("No volume is small enough to restore onto {}.", node.name));
            return true;
        }
        let sources: Vec<String> = picks.iter().take(crate::form::LIST_MAX).map(|&i| self.nodes[i].name.clone()).collect();
        let rows: Vec<String> = picks.iter().map(|&i| row_text(&self.nodes[i])).collect();
        let form = Form::new(&format!("Restore {}?", node.name), "Restore", true)
            .list("Restore from", rows, true)
            .note(&format!("Everything on {} is replaced by a copy of the volume you choose.", node.name))
            .note("Attach a disk image first to restore from it.");
        self.show_form(form, FormKind::Restore { device: node.name, sources })
    }

    /// Disk > RAID Assistant: a level, a name and the members of a new array.
    pub fn open_raid_form(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let picks: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                !n.read_only
                    && !model::is_raid_array(&n.name)
                    && n.mount_point.is_none()
                    && !self.nodes.iter().any(|c| c.parent.as_deref() == Some(n.name.as_str()) && c.mount_point.is_some())
            })
            .collect();
        if picks.len() < RaidLevel::Raid0.min_members() {
            self.error = Some("A RAID array needs at least two disks or volumes that are not in use.".to_string());
            return true;
        }
        let members: Vec<String> = picks.iter().take(crate::form::LIST_MAX).map(|&i| self.nodes[i].name.clone()).collect();
        let rows: Vec<String> = picks.iter().map(|&i| row_text(&self.nodes[i])).collect();
        let titles: Vec<&str> = RaidLevel::ALL.iter().map(|l| l.title()).collect();
        let form = Form::new("RAID Assistant", "Create", true)
            .choice("Level", &titles, 1)
            .text("Name", "md0", "md0", 6)
            .list("Members", rows, false)
            .note(RaidLevel::Raid1.summary())
            .note("The members are erased.");
        self.show_form(form, FormKind::Raid { members })
    }

    /// File > New Image > Blank Image: a name, a size and an optional empty file system.
    pub fn open_new_blank_form(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let mut formats = vec!["None"];
        formats.extend(fs_titles());
        let form = Form::new("New Blank Image", "Choose Location", false)
            .text("Name", "Untitled", "Untitled", 64)
            .text("Size", "100m", "512m, 4g", 12)
            .choice("Format", &formats, 1)
            .text("Volume name", "", "Untitled", 32)
            .note("The image is a sparse raw disk image: it takes space as it is filled.");
        self.show_form(form, FormKind::NewBlank)
    }

    /// Image > Convert Image: the format to write a chosen image in.
    pub fn open_convert_form(&mut self, src: String) -> bool {
        let titles: Vec<&str> = ImageFormat::ALL.iter().map(|f| f.title()).collect();
        let form = Form::new("Convert Image", "Choose Location", false)
            .choice("Format", &titles, 1)
            .note(&format!("Converts {} into a new file.", images::base_name(&src)));
        self.show_form(form, FormKind::Convert { src })
    }

    /// Image > Resize Image: the new size of a chosen image.
    pub fn open_resize_form(&mut self, path: String, size: u64, ext: bool) -> bool {
        let note = if ext {
            "The ext file system inside is checked and resized with it. The image may shrink."
        } else {
            "The file system inside is not resized, and the image can only grow."
        };
        let form = Form::new("Resize Image", "Resize", false)
            .text("New size", &images::size_text(size), "e.g. 2g", 12)
            .note(&format!("{} is {} now.", images::base_name(&path), images::size_text(size)))
            .note(note);
        self.show_form(form, FormKind::Resize { path, size, ext })
    }

    /// A press on the form: a button, a text field or an option.
    pub fn form_activate(&mut self, hit: FormHit) -> bool {
        match hit {
            FormHit::Nothing => false,
            FormHit::Button(FORM_CANCEL) => {
                self.form = None;
                true
            }
            FormHit::Button(FORM_OK) => self.submit_form(),
            FormHit::Button(_) => false,
            FormHit::Text(row) => self.form.as_mut().is_some_and(|f| f.form.focus_row(row)),
            FormHit::Pick(row, index) => {
                let Some(open) = self.form.as_mut() else { return false };
                let changed = open.form.pick(row, index);
                if changed {
                    if let (FormKind::Raid { .. }, 0) = (&open.kind, row) {
                        let level = RaidLevel::ALL[open.form.selected(0).min(RaidLevel::ALL.len() - 1)];
                        open.form.notes[0] = level.summary().to_string();
                    }
                }
                changed
            }
        }
    }

    /// Typed text goes to the focused field.
    pub fn form_text(&mut self, text: &str) -> bool {
        self.form.as_mut().is_some_and(|f| f.form.type_text(text))
    }

    /// An editing key (shell key codes): 8 backspace, 9 tab, 13 return, 1 or 27 escape. True when the window changed.
    pub fn form_code(&mut self, code: u32) -> bool {
        match code {
            8 => self.form.as_mut().is_some_and(|f| f.form.backspace()),
            9 => self.form.as_mut().is_some_and(|f| f.form.focus_next()),
            13 => self.submit_form(),
            1 | 27 => {
                self.form = None;
                true
            }
            _ => false,
        }
    }

    /// Reports why OK did not go on, inside the sheet.
    fn refuse(&mut self, sentence: &str) -> bool {
        if let Some(open) = self.form.as_mut() {
            open.form.set_error(sentence);
        }
        true
    }

    /// Checks `request` against the disks and, when they allow it, replaces the form with its confirmation sheet.
    fn confirm_form_request(&mut self, title: String, request: DiskRequest, button: &str, destructive: bool) -> bool {
        if let Err(sentence) = actions::check_request(&request, &self.nodes) {
            return self.refuse(&sentence);
        }
        self.form = None;
        self.confirm(title, request, button, destructive);
        true
    }

    /// OK: turns the entered values into the next step. A value that is not acceptable stays in the sheet with the
    /// reason under it.
    pub fn submit_form(&mut self) -> bool {
        let Some(OpenForm { form, kind }) = self.form.clone() else { return false };
        match kind {
            FormKind::Rename { device } => {
                let label = form.value(0);
                self.confirm_form_request(format!("Rename {device}?"), DiskRequest::RenameVolume { device, label }, "Rename", false)
            }
            FormKind::Erase { device, whole } => {
                let fs = Personality::ALL[form.selected(1).min(Personality::ALL.len() - 1)];
                let label = form.value(0);
                let request = if whole {
                    let scheme = if form.selected(2) == 1 { Scheme::Mbr } else { Scheme::Gpt };
                    DiskRequest::EraseDisk { device: device.clone(), fs, label, scheme }
                } else {
                    DiskRequest::EraseVolume { device: device.clone(), fs, label }
                };
                self.erase_fs = fs;
                self.confirm_form_request(format!("Erase {device}?"), request, "Erase", true)
            }
            FormKind::Restore { device, sources } => {
                let Some(source) = form.picked(0).first().and_then(|&i| sources.get(i)).cloned() else {
                    return self.refuse("Choose the volume to restore from.");
                };
                self.confirm_form_request(format!("Restore onto {device}?"), DiskRequest::RestoreVolume { device, source }, "Restore", true)
            }
            FormKind::Raid { members } => {
                let level = RaidLevel::ALL[form.selected(0).min(RaidLevel::ALL.len() - 1)];
                let name = form.value(1);
                let chosen: Vec<String> = form.picked(2).iter().filter_map(|&i| members.get(i).cloned()).collect();
                self.confirm_form_request(format!("Create {name}?"), DiskRequest::CreateRaid { name, level, members: chosen }, "Create", true)
            }
            FormKind::NewBlank => {
                let bytes = match images::parse_size(&form.value(1)) {
                    Ok(bytes) => bytes,
                    Err(sentence) => return self.refuse(&sentence),
                };
                let fs = form.selected(2).checked_sub(1).and_then(|i| Personality::ALL.get(i).copied());
                let spec = BlankSpec { bytes, fs, label: form.value(3) };
                if let Err(sentence) = images::check_blank(&spec) {
                    return self.refuse(&sentence);
                }
                self.form = None;
                self.want = Some(Want::Save { suggested: images::suggested_file(&form.value(0), "img"), filters: vec!["img"], op: SaveOp::Blank(spec) });
                true
            }
            FormKind::Convert { src } => {
                let format = ImageFormat::ALL[form.selected(0).min(ImageFormat::ALL.len() - 1)];
                self.form = None;
                let suggested = images::suggested_file(images::stem(images::base_name(&src)), format.extension());
                self.want = Some(Want::Save { suggested, filters: vec![format.extension()], op: SaveOp::Convert { src, format } });
                true
            }
            FormKind::Resize { path, size, ext } => {
                let new = match images::parse_size(&form.value(0)) {
                    Ok(bytes) => bytes,
                    Err(sentence) => return self.refuse(&sentence),
                };
                let plan = match images::resize_plan(&path, size, new, ext) {
                    Ok(plan) => plan,
                    Err(sentence) => return self.refuse(&sentence),
                };
                self.form = None;
                // a check that fixes something exits 1, which is a success here
                self.start_plan("Resizing", images::base_name(&path).to_string(), plan, true);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stick() -> DiskUtility {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), removable: true, size: 8_000_000_000, model: "Stick".into(), ..Node::default() },
            Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), size: 4_000_000_000, fs_type: Some("vfat".into()), label: Some("GAMES".into()), ..Node::default() },
            Node { name: "sdb2".into(), kind: Kind::Partition, parent: Some("sdb".into()), size: 3_000_000_000, fs_type: Some("ext4".into()), ..Node::default() },
            Node { name: "sdc".into(), removable: true, size: 16_000_000_000, ..Node::default() },
        ];
        w
    }

    fn type_into(w: &mut DiskUtility, text: &str) {
        assert!(w.form_text(text));
    }

    #[test]
    fn rename_opens_with_the_current_name_and_confirms_the_new_one() {
        let mut w = stick();
        w.selected = Some("sdb1".into());
        assert!(w.open_rename_form());
        assert_eq!(w.form.as_ref().unwrap().form.value(0), "GAMES");
        assert!(w.form_code(8) && w.form_code(8));
        type_into(&mut w, "ME");
        assert!(w.form_activate(FormHit::Button(FORM_OK)));
        assert!(w.form.is_none());
        let pending = w.sheet.as_ref().expect("confirmation");
        assert_eq!(pending.requests[0].1, DiskRequest::RenameVolume { device: "sdb1".into(), label: "GAMME".into() });
        assert!(pending.sheet.message.contains("Rename"));
    }

    #[test]
    fn rename_of_a_mounted_volume_or_one_without_a_file_system_opens_no_sheet() {
        let mut w = stick();
        w.nodes[1].mount_point = Some("/mnt/a".into());
        w.selected = Some("sdb1".into());
        assert!(w.open_rename_form());
        assert!(w.form.is_none() && w.error.as_deref().is_some_and(|e| e.contains("/mnt/a")));
        w.error = None;
        w.selected = Some("sdb".into());
        assert!(w.open_rename_form());
        assert!(w.form.is_none() && w.error.as_deref().is_some_and(|e| e.contains("no file system")));
    }

    #[test]
    fn a_name_the_file_system_refuses_stays_in_the_sheet_with_the_reason() {
        let mut w = stick();
        w.selected = Some("sdb1".into());
        w.open_rename_form();
        assert!(w.form_code(8));
        type_into(&mut w, "a/b");
        w.submit_form();
        let open = w.form.as_ref().expect("the sheet stays");
        assert!(open.form.error.as_deref().is_some_and(|e| e.contains("not valid")));
        assert!(w.sheet.is_none());
    }

    #[test]
    fn erase_of_a_volume_offers_name_and_format_and_keeps_a_valid_label() {
        let mut w = stick();
        w.selected = Some("sdb1".into());
        assert!(w.set_erase_fs(Personality::ExFat));
        assert!(w.open_erase_form());
        let open = w.form.as_ref().expect("the erase sheet");
        assert_eq!(open.form.rows.len(), 2, "a volume has no partition scheme");
        assert_eq!(open.form.value(0), "GAMES");
        assert_eq!(open.form.selected(1), 1);
        assert!(w.form_activate(FormHit::Pick(1, 4)));
        assert!(w.form_activate(FormHit::Button(FORM_OK)));
        let (_, request) = &w.sheet.as_ref().expect("confirmation").requests[0];
        assert_eq!(*request, DiskRequest::EraseVolume { device: "sdb1".into(), fs: Personality::Btrfs, label: "GAMES".into() });
        assert_eq!(w.erase_fs, Personality::Btrfs, "the next erase starts from this format");
    }

    #[test]
    fn erase_of_a_disk_also_picks_the_partition_scheme() {
        let mut w = stick();
        w.selected = Some("sdc".into());
        assert!(w.open_erase_form());
        assert_eq!(w.form.as_ref().unwrap().form.rows.len(), 3);
        assert!(w.form_activate(FormHit::Pick(2, 1)));
        assert!(w.form_activate(FormHit::Pick(1, 3)));
        type_into(&mut w, "Backup");
        w.submit_form();
        let (_, request) = &w.sheet.as_ref().expect("confirmation").requests[0];
        assert_eq!(*request, DiskRequest::EraseDisk { device: "sdc".into(), fs: Personality::Ntfs, label: "Backup".into(), scheme: Scheme::Mbr });
        assert!(w.sheet.as_ref().unwrap().sheet.message.contains("Record"));
    }

    #[test]
    fn a_refused_erase_opens_no_sheet_and_cancel_closes_one() {
        let mut w = stick();
        w.nodes[1].mount_point = Some("/mnt/a".into());
        w.selected = Some("sdb".into());
        assert!(w.open_erase_form());
        assert!(w.form.is_none() && w.error.as_deref().is_some_and(|e| e.contains("/mnt/a")));
        w.error = None;
        w.selected = Some("sdc".into());
        assert!(w.open_erase_form());
        assert!(w.form.is_some() && w.wants_keyboard());
        assert!(w.form_activate(FormHit::Button(FORM_CANCEL)));
        assert!(w.form.is_none() && !w.wants_keyboard());
    }

    #[test]
    fn restore_lists_the_volumes_that_fit_and_skips_related_ones() {
        let mut w = stick();
        w.selected = Some("sdc".into());
        assert!(w.open_restore_form());
        match &w.form.as_ref().unwrap().kind {
            FormKind::Restore { device, sources } => {
                assert_eq!(device, "sdc");
                assert_eq!(sources, &vec!["sdb".to_string(), "sdb1".to_string(), "sdb2".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        // a disk skips its own partitions and anything larger than itself
        w.form = None;
        w.selected = Some("sdb".into());
        assert!(w.open_restore_form());
        assert!(w.form.is_none() && w.error.is_some());
    }

    #[test]
    fn restore_sends_the_chosen_source_and_needs_one() {
        let mut w = stick();
        w.selected = Some("sdb1".into());
        w.nodes[2].size = 1_000_000_000;
        assert!(w.open_restore_form());
        // the only volume small enough is sdb2
        assert!(w.form_activate(FormHit::Button(FORM_OK)));
        let (_, request) = &w.sheet.as_ref().expect("confirmation").requests[0];
        assert_eq!(*request, DiskRequest::RestoreVolume { device: "sdb1".into(), source: "sdb2".into() });
    }

    #[test]
    fn restore_with_nothing_small_enough_is_the_footer_error() {
        let mut w = DiskUtility::new();
        w.nodes = vec![Node { name: "sdb1".into(), kind: Kind::Partition, parent: Some("sdb".into()), size: 1_000, ..Node::default() }];
        w.selected = Some("sdb1".into());
        assert!(w.open_restore_form());
        assert!(w.form.is_none() && w.error.as_deref().is_some_and(|e| e.contains("small enough")));
    }

    #[test]
    fn the_raid_assistant_creates_an_array_from_the_members_that_are_on() {
        let mut w = stick();
        w.nodes.push(Node { name: "sdd".into(), size: 16_000_000_000, ..Node::default() });
        assert!(w.open_raid_form());
        match &w.form.as_ref().unwrap().kind {
            FormKind::Raid { members } => assert_eq!(members, &["sdb", "sdb1", "sdb2", "sdc", "sdd"].map(String::from).to_vec()),
            other => panic!("{other:?}"),
        }
        assert_eq!(w.form.as_ref().unwrap().form.notes[0], RaidLevel::Raid1.summary());
        assert!(w.form_activate(FormHit::Pick(0, 0)));
        assert_eq!(w.form.as_ref().unwrap().form.notes[0], RaidLevel::Raid0.summary());
        assert!(w.form_activate(FormHit::Pick(2, 3)));
        assert!(w.form_activate(FormHit::Pick(2, 4)));
        assert!(w.form_activate(FormHit::Button(FORM_OK)));
        let (_, request) = &w.sheet.as_ref().expect("confirmation").requests[0];
        assert_eq!(*request, DiskRequest::CreateRaid { name: "md0".into(), level: RaidLevel::Raid0, members: vec!["sdc".into(), "sdd".into()] });
    }

    #[test]
    fn an_array_with_too_few_members_stays_in_the_sheet() {
        let mut w = stick();
        assert!(w.open_raid_form());
        assert!(w.form_activate(FormHit::Pick(2, 3)));
        w.submit_form();
        assert!(w.sheet.is_none());
        assert!(w.form.as_ref().unwrap().form.error.as_deref().is_some_and(|e| e.contains("at least")));
    }

    #[test]
    fn an_existing_array_is_listed_but_never_offered_as_a_member() {
        let mut w = DiskUtility::new();
        w.nodes = vec![
            Node { name: "sdb".into(), size: 1_000_000_000, ..Node::default() },
            Node { name: "sdc".into(), size: 1_000_000_000, ..Node::default() },
            Node { name: "md0".into(), size: 2_000_000_000, ..Node::default() },
        ];
        assert!(w.open_raid_form());
        match &w.form.as_ref().unwrap().kind {
            FormKind::Raid { members } => assert_eq!(members, &["sdb", "sdc"].map(String::from).to_vec()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_raid_assistant_needs_two_free_devices() {
        let mut w = DiskUtility::new();
        w.nodes = vec![Node { name: "sdb".into(), size: 1_000_000_000, ..Node::default() }];
        assert!(w.open_raid_form());
        assert!(w.form.is_none() && w.error.is_some());
    }

    #[test]
    fn a_new_blank_image_asks_for_a_place_after_its_size_and_label_check_out() {
        let mut w = DiskUtility::new();
        assert!(w.open_new_blank_form());
        w.form_code(9); // the size field
        assert!(w.form_activate(FormHit::Pick(2, 3)));
        w.form_code(9); // the volume name field
        type_into(&mut w, "DISK");
        w.submit_form();
        assert!(w.form.is_none());
        let spec = BlankSpec { bytes: 100 << 20, fs: Some(Personality::Fat32), label: "DISK".into() };
        assert_eq!(w.want, Some(Want::Save { suggested: "Untitled.img".into(), filters: vec!["img"], op: SaveOp::Blank(spec) }));
    }

    #[test]
    fn a_bad_size_or_label_is_told_inside_the_blank_image_sheet() {
        let mut w = DiskUtility::new();
        w.open_new_blank_form();
        w.form_code(9);
        for _ in 0..4 {
            w.form_code(8);
        }
        type_into(&mut w, "lots");
        w.submit_form();
        assert!(w.want.is_none());
        assert!(w.form.as_ref().unwrap().form.error.as_deref().is_some_and(|e| e.contains("not a number")));
        let mut w = DiskUtility::new();
        w.open_new_blank_form();
        assert!(w.form_activate(FormHit::Pick(2, 0)));
        w.form_code(9);
        w.form_code(9);
        type_into(&mut w, "X");
        w.submit_form();
        assert!(w.form.as_ref().unwrap().form.error.as_deref().is_some_and(|e| e.contains("no file system")));
    }

    #[test]
    fn convert_names_the_copy_after_the_source_and_the_format() {
        let mut w = DiskUtility::new();
        assert!(w.open_convert_form("/home/me/disk.img".into()));
        assert!(w.form_activate(FormHit::Pick(0, 2)));
        w.submit_form();
        assert_eq!(
            w.want,
            Some(Want::Save {
                suggested: "disk.vmdk".into(),
                filters: vec!["vmdk"],
                op: SaveOp::Convert { src: "/home/me/disk.img".into(), format: ImageFormat::Vmdk }
            })
        );
    }

    #[test]
    fn resize_starts_a_plan_that_checks_then_resizes_an_ext_image() {
        let mut w = DiskUtility::new();
        assert!(w.open_resize_form("/home/me/a.img".into(), 64 << 20, true));
        assert_eq!(w.form.as_ref().unwrap().form.value(0), "64m");
        for _ in 0..3 {
            w.form_code(8);
        }
        type_into(&mut w, "32m");
        w.submit_form();
        assert!(w.form.is_none());
        let job = w.job.as_ref().expect("the resize job");
        assert_eq!((job.verb, job.name.as_str()), ("Resizing", "a.img"));
    }

    #[test]
    fn resize_refuses_the_same_size_and_a_shrunk_image_of_another_file_system() {
        let mut w = DiskUtility::new();
        w.open_resize_form("/a.img".into(), 64 << 20, false);
        w.submit_form();
        assert!(w.form.as_ref().unwrap().form.error.as_deref().is_some_and(|e| e.contains("already")));
        for _ in 0..3 {
            w.form_code(8);
        }
        type_into(&mut w, "32m");
        w.submit_form();
        assert!(w.form.as_ref().unwrap().form.error.as_deref().is_some_and(|e| e.contains("can shrink")));
        assert!(w.job.is_none());
    }

    #[test]
    fn escape_closes_the_sheet_and_return_submits_it() {
        let mut w = stick();
        w.selected = Some("sdb1".into());
        w.open_rename_form();
        assert!(w.form_code(27));
        assert!(w.form.is_none());
        w.open_rename_form();
        assert!(w.form_code(13));
        assert!(w.sheet.is_some(), "return is OK");
        assert!(!w.form_code(8), "no sheet takes the key");
    }

    #[test]
    fn list_rows_name_the_device_size_and_label() {
        let w = stick();
        assert_eq!(row_text(&w.nodes[1]), "sdb1 \u{b7} 4.0 GB \u{b7} GAMES");
        assert_eq!(row_text(&w.nodes[0]), "sdb \u{b7} 8.0 GB \u{b7} Stick");
        assert_eq!(row_text(&w.nodes[2]), "sdb2 \u{b7} 3.0 GB");
    }
}
