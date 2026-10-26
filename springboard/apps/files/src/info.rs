//! Get Info and Tags: the info panel, the Tags panel and the colour tag dots.
use crate::*;

/// Tags panel layout: the first colour row's top, the row pitch, and the panel height.
pub(crate) const TAG_ROWS_TOP: f32 = 124.;

pub(crate) const TAG_ROW: f32 = 30.;

pub(crate) const TAG_PANEL_H: f32 = 330.;

/// Folder totals for Get Info stop counting after this long.
pub(crate) const INFO_BUDGET: Duration = Duration::from_millis(800);

/// The head of a JPEG read for its EXIF rows: the APP1 segment comes before the image data.
const EXIF_HEAD_BYTES: u64 = 128 * 1024;

/// The head of an image file read for Get Info: the 32 KB system area plus the volume descriptor.
const IMAGE_HEAD_BYTES: u64 = 34 * 1024;

/// Disk image rows for Get Info, read from the image's own descriptor (`.iso` and `.img` files only; other files get
/// none). The contents, the volume name, the declared volume size and whether the file holds all of it.
pub(crate) fn disk_image_rows(path: &Path, length: u64) -> Vec<(&'static str, String)> {
    let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "iso" | "img") {
        return Vec::new();
    }
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        use std::io::Read;
        if file.take(IMAGE_HEAD_BYTES).read_to_end(&mut head).is_err() {
            head.clear();
        }
    }
    image_rows(&head, length)
}

/// The Get Info rows for a disk image whose first bytes are `head` and whose file is `length` bytes long.
pub(crate) fn image_rows(head: &[u8], length: u64) -> Vec<(&'static str, String)> {
    use bifrost_app::controls::disk_image::{inspect, ImageContents};
    let props = inspect(head, length);
    let contents = match props.contents {
        ImageContents::Iso9660 => "ISO 9660 disc image",
        ImageContents::Raw => "Raw disk image",
    };
    let mut rows = vec![("Image", contents.to_string())];
    if let Some(name) = props.volume_name.clone() {
        rows.push(("Volume", name));
    }
    if let Some(size) = props.declared_size() {
        rows.push(("Volume Size", springboard::files::fs::size_text(size)));
        let status = if props.is_complete() { "Complete" } else { "Truncated: the file is shorter than its volume" };
        rows.push(("Image Status", status.to_string()));
    }
    rows
}

/// The EXIF rows for a JPEG in Get Info (date, camera, exposure, dimensions, location), read from the head of the
/// file, where the APP1 segment sits. Other files get none.
pub(crate) fn exif_rows(path: &Path) -> Vec<(&'static str, String)> {
    let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "jpg" | "jpeg") {
        return Vec::new();
    }
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        use std::io::Read;
        if file.take(EXIF_HEAD_BYTES).read_to_end(&mut head).is_err() {
            head.clear();
        }
    }
    exif_rows_from(&head)
}

/// The EXIF rows for the start of a JPEG. The reader is the Photos app's (`springboard::shell::photos::exif`); its
/// pixel size is labelled Dimensions here, because Get Info already has a Size row for the file.
pub(crate) fn exif_rows_from(head: &[u8]) -> Vec<(&'static str, String)> {
    springboard::shell::photos::exif::parse(head)
        .map(|e| e.rows())
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (if label == "Size" { "Dimensions" } else { label }, value))
        .collect()
}

/// The owner and group rows for Get Info: the account name from the password and group tables (`/etc/passwd`,
/// `/etc/group`), with the numeric id in brackets, or the id alone when the table has no such account.
pub(crate) fn permission_rows(st: &bifrost_app::file_stat::FileStat, passwd: &str, group: &str) -> Vec<(&'static str, String)> {
    let mode = st.mode.to_string();
    let octal = format!("{:o}", st.permissions().bits());
    vec![
        ("Permissions", format!("{mode} ({octal})")),
        ("Owner", named(account_name(passwd, st.user_id), st.user_id)),
        ("Group", named(account_name(group, st.group_id), st.group_id)),
    ]
}

/// The name for an id in a colon-separated account table, such as `/etc/passwd` (name, password, id, ...).
pub(crate) fn account_name(table: &str, id: u32) -> Option<String> {
    table.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        (fields.next()?.parse::<u32>().ok()? == id && !name.is_empty()).then(|| name.to_string())
    })
}

fn named(name: Option<String>, id: u32) -> String {
    match name {
        Some(name) => format!("{name} ({id})"),
        None => id.to_string(),
    }
}

/// The Get Info panel: a name and labelled rows.
pub(crate) struct Info {
    pub(crate) name: String,
    pub(crate) rows: Vec<(&'static str, String)>,
    /// The item Get Info describes, and its permission bits for the editable grid (`perms.rs`).
    pub(crate) path: PathBuf,
    pub(crate) mode: Option<u32>,
    /// True when the person owns the item (or is root), so a click on the grid changes the mode.
    pub(crate) editable: bool,
}

/// A tag's colour in the surface's pixel format (tags are stored as 0xRRGGBB).
pub(crate) fn tag_colour(tag: Tag) -> u32 {
    let c = tag.rgb();
    pack((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
}

/// The colour dots of an item, left to right, starting at (x, y).
pub(crate) fn tag_dots(s: &mut Surface, ui: &Ui, index: &TagIndex, path: &Path, x: f32, y: f32) {
    for (i, tag) in index.tags(path).into_iter().enumerate() {
        s.fill_rrect(ui.pt(x + i as f32 * 9.), ui.pt(y), ui.pt(SPACE_S), ui.pt(SPACE_S), ui.pt(SPACE_XS), tag_colour(tag));
    }
}

impl Files {
    /// Runs `change` on the tag index and saves it to the sidecar file.
    pub(crate) fn with_tags(&mut self, change: impl FnOnce(&mut TagIndex)) {
        change(&mut self.tag_index);
        if let Err(e) = self.tag_index.save(&sidecar_path(&self.home)) {
            self.notice = Some(format!("Could not save tags: {e}"));
        }
        self.revision += 1;
    }
    /// A tag change the person made, undoable with Cmd+Z: runs `change` on the index and, when it changed the colours
    /// of any of `paths`, records the tags they had before as one undo step.
    pub(crate) fn edit_tags(&mut self, paths: &[PathBuf], change: impl FnOnce(&mut TagIndex)) {
        let before: Vec<(PathBuf, Vec<Tag>)> = paths.iter().map(|p| (p.clone(), self.tag_index.tags(p))).collect();
        self.with_tags(change);
        let changed: Vec<(PathBuf, Vec<Tag>)> = before.into_iter().filter(|(p, old)| self.tag_index.tags(p) != *old).collect();
        if !changed.is_empty() {
            self.push_undo(Undo::Tags(changed));
        }
    }
    pub(crate) fn open_tags(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.preview = None;
        self.info = None;
        self.open_with = None;
        self.tag_panel = true;
        self.revision += 1;
    }
    /// Sets or clears one colour on every selected item: cleared when all have it, else set.
    pub(crate) fn toggle_tag(&mut self, tag: Tag) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let all = paths.iter().all(|p| self.tag_index.has(p, tag));
        self.edit_tags(&paths, |index| {
            for p in &paths {
                if index.has(p, tag) == all {
                    index.toggle(p, tag);
                }
            }
        });
    }
    /// Get Info: the focused item's kind, size, dates and tags. Folder totals stop counting
    /// after INFO_BUDGET, so the panel never waits on a huge tree.
    pub(crate) fn get_info(&mut self) {
        let Some(entry) = self.focus_entry().cloned() else {
            return;
        };
        self.open_with = None;
        let wall = Wall::sample(bifrost_app::proto::sys::now_ms());
        let summary = entry.is_dir.then(|| {
            let deadline = Instant::now() + INFO_BUDGET;
            springboard::files::info::summarize(&entry.path, &mut || Instant::now() < deadline)
        });
        let tags = TagIndex::load(&sidecar_path(&self.home));
        let created = springboard::files::info::created_at(&entry.path);
        let mut rows = springboard::files::info::rows(&entry, summary, created, &tags, wall.unix, wall.offset);
        if !entry.is_dir {
            rows.extend(disk_image_rows(&entry.path, entry.size));
            rows.extend(exif_rows(&entry.path));
        }
        let (mut mode, mut editable) = (None, false);
        if let Ok(st) = bifrost_app::file_stat::stat(&entry.path) {
            let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
            let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
            rows.extend(permission_rows(&st, &passwd, &group));
            mode = Some(st.permissions().bits() & 0o7777);
            // provider folders are stand-in paths with no mode of their own to change
            editable = perms::can_edit(st.user_id, perms::current_uid()) && !provider::is_path(&entry.path);
        }
        self.preview = None;
        self.info = Some(Info { name: entry.name.clone(), rows, path: entry.path.clone(), mode, editable });
        // the reference shows Get Info in a small window of its own; the panel stays as the fallback
        self.info_wanted = !self.info_window;
        self.revision += 1;
    }
    /// The Tags panel.
    pub(crate) fn paint_tag_panel(&self, ui: &Ui, s: &mut Surface) {
        let w = self.w;
        if self.tag_panel {
            let (x, y) = (self.side() + 16., 72.);
            rect(s, ui, x, y, w - x - 16., TAG_PANEL_H, CARD_BG.now());
            text(s, ui, "Tags", 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
            let picked: Vec<&PathBuf> = self.selection.iter().filter_map(|&i| self.rows.get(i)).map(|e| &e.path).collect();
            for (i, tag) in Tag::ALL.into_iter().enumerate() {
                let row_y = TAG_ROWS_TOP + i as f32 * TAG_ROW;
                s.fill_rrect(ui.pt(x + 20.), ui.pt(row_y + 7.), ui.pt(SPACE_L), ui.pt(SPACE_L), ui.pt(SPACE_S), tag_colour(tag));
                text(s, ui, tag.label(), 13., LABEL.now(), x + 44., row_y + 6., 200.);
                if !picked.is_empty() && picked.iter().all(|p| self.tag_index.has(p, tag)) {
                    text(s, ui, "On", 13., TINT, x + w - 120., row_y + 6., 60.);
                }
            }
            text(s, ui, "Click a colour to turn it on or off for the selection. Click elsewhere to close.", 11., LABEL_2.now(), x + 20., TAG_ROWS_TOP + Tag::ALL.len() as f32 * TAG_ROW + 14., w - x - 56.);
        }
    }
    /// The Get Info panel.
    pub(crate) fn paint_info(&self, ui: &Ui, s: &mut Surface) {
        let (w, h) = (self.w, self.h);
        if let Some(info) = &self.info {
            let (x, y) = self.info_origin();
            if !self.info_window {
                rect(s, ui, x, y, w - x - 16., h - y - 64., CARD_BG.now());
            }
            text(s, ui, &info.name, 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
            for (i, (label, value)) in info.rows.iter().enumerate() {
                let row_y = y + 56. + i as f32 * 26.;
                text(s, ui, label, 12., LABEL_2.now(), x + 20., row_y + 2., 120.);
                text(s, ui, value, 13., LABEL.now(), x + 150., row_y, w - x - 190.);
            }
            self.paint_perm_grid(ui, s);
            let grid_h = if info.mode.is_some() { perms::GRID_H + 14. } else { 0. };
            let hint = if self.info_window && !info.editable {
                ""
            } else if self.info_window {
                "Click a box to change who can read, write or run it."
            } else if info.mode.is_none() {
                "Press Escape or click to close"
            } else if info.editable {
                "Click a box to change who can read, write or run it. Press Escape to close"
            } else {
                "Only the owner can change the permissions. Press Escape to close"
            };
            text(s, ui, hint, 11., LABEL_2.now(), x + 20., y + 56. + info.rows.len() as f32 * 26. + 12. + grid_h, w - x - 56.);
        }
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    use bifrost_app::file_stat::{FileStat, Timestamp};
    use bifrost_core::file_mode::FileMode;

    fn stat_with(mode: u32, uid: u32, gid: u32) -> FileStat {
        let t = Timestamp { secs: 0, nanos: 0 };
        FileStat {
            device: 1,
            inode: 2,
            mode: FileMode::from_raw(mode),
            link_count: 1,
            user_id: uid,
            group_id: gid,
            size: 0,
            blocks_allocated: 0,
            block_size: 4096,
            accessed: t,
            modified: t,
            changed: t,
        }
    }

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nolibuijr:x:1000:1000::/home/olibuijr:/bin/zsh\n";
    const GROUP: &str = "root:x:0:\nusers:x:100:olibuijr\n";

    #[test]
    fn a_regular_file_shows_its_mode_bits_and_owner() {
        let rows = permission_rows(&stat_with(0o100644, 1000, 100), PASSWD, GROUP);
        assert_eq!(rows[0], ("Permissions", "-rw-r--r-- (644)".to_string()));
        assert_eq!(rows[1], ("Owner", "olibuijr (1000)".to_string()));
        assert_eq!(rows[2], ("Group", "users (100)".to_string()));
    }

    #[test]
    fn a_directory_and_a_set_user_id_file_show_their_type_and_special_bits() {
        let dir = permission_rows(&stat_with(0o040755, 0, 0), PASSWD, GROUP);
        assert_eq!(dir[0].1, "drwxr-xr-x (755)");
        let suid = permission_rows(&stat_with(0o104755, 0, 0), PASSWD, GROUP);
        assert_eq!(suid[0].1, "-rwsr-xr-x (4755)");
    }

    #[test]
    fn an_id_without_an_account_shows_the_number_alone() {
        let rows = permission_rows(&stat_with(0o100600, 1234, 4321), PASSWD, GROUP);
        assert_eq!(rows[0].1, "-rw------- (600)");
        assert_eq!(rows[1], ("Owner", "1234".to_string()));
        assert_eq!(rows[2], ("Group", "4321".to_string()));
    }

    #[test]
    fn account_names_come_from_the_id_field_only() {
        assert_eq!(account_name(PASSWD, 0).as_deref(), Some("root"));
        assert_eq!(account_name(PASSWD, 1000).as_deref(), Some("olibuijr"));
        assert_eq!(account_name(PASSWD, 42), None);
        assert_eq!(account_name("", 0), None);
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    /// A 2048-byte-block ISO head: the primary descriptor at 32 KB, volume "BIFROST", 100 blocks.
    fn iso_head() -> Vec<u8> {
        let mut h = vec![0u8; IMAGE_HEAD_BYTES as usize];
        let pvd = 16 * 2048;
        h[pvd] = 1;
        h[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        h[pvd + 6] = 1;
        let name = b"BIFROST";
        h[pvd + 40..pvd + 72].fill(b' ');
        h[pvd + 40..pvd + 40 + name.len()].copy_from_slice(name);
        h[pvd + 80..pvd + 84].copy_from_slice(&100u32.to_le_bytes());
        h[pvd + 128..pvd + 130].copy_from_slice(&2048u16.to_le_bytes());
        h
    }

    #[test]
    fn complete_iso_reports_its_volume() {
        let rows = image_rows(&iso_head(), 204_800);
        assert_eq!(rows[0], ("Image", "ISO 9660 disc image".to_string()));
        assert_eq!(rows[1], ("Volume", "BIFROST".to_string()));
        assert_eq!(rows[2], ("Volume Size", springboard::files::fs::size_text(204_800)));
        assert_eq!(rows[3], ("Image Status", "Complete".to_string()));
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn short_iso_is_flagged_as_truncated() {
        let rows = image_rows(&iso_head(), 150_000);
        assert_eq!(rows.last().unwrap().1, "Truncated: the file is shorter than its volume");
    }

    #[test]
    fn raw_image_has_no_volume_rows() {
        let rows = image_rows(&[0u8; 64], 4096);
        assert_eq!(rows, vec![("Image", "Raw disk image".to_string())]);
    }

    #[test]
    fn a_jpeg_without_exif_shows_its_pixel_size_as_dimensions() {
        // SOI, then a baseline frame header (SOF0, length 11): 8-bit, 10 lines high, 20 samples wide, one component
        let head = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x0A, 0x00, 0x14, 0x01, 0x01, 0x11, 0x00];
        assert_eq!(exif_rows_from(&head), vec![("Dimensions", "20 x 10".to_string())]);
    }

    #[test]
    fn a_file_that_is_not_a_jpeg_has_no_exif_rows() {
        assert!(exif_rows_from(b"\x89PNG\r\n\x1a\n").is_empty());
        assert!(exif_rows(std::path::Path::new("/nonexistent/photo.png")).is_empty());
    }

    #[test]
    fn only_image_extensions_are_read() {
        assert!(disk_image_rows(std::path::Path::new("/nonexistent/notes.txt"), 10).is_empty());
    }
}
