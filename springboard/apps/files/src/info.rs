//! Get Info and Tags: the info panel, the Tags panel and the colour tag dots.
use crate::*;

/// Tags panel layout: the first colour row's top, the row pitch, and the panel height.
pub(crate) const TAG_ROWS_TOP: f32 = 124.;

pub(crate) const TAG_ROW: f32 = 30.;

pub(crate) const TAG_PANEL_H: f32 = 330.;

/// Folder totals for Get Info stop counting after this long.
pub(crate) const INFO_BUDGET: Duration = Duration::from_millis(800);

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

/// The Get Info panel: a name and labelled rows.
pub(crate) struct Info {
    pub(crate) name: String,
    pub(crate) rows: Vec<(&'static str, String)>,
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
    pub(crate) fn open_tags(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.preview = None;
        self.info = None;
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
        self.with_tags(|index| {
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
        }
        self.preview = None;
        self.info = Some(Info { name: entry.name.clone(), rows });
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
            let (x, y) = (self.side() + 16., 72.);
            rect(s, ui, x, y, w - x - 16., h - y - 64., CARD_BG.now());
            text(s, ui, &info.name, 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
            for (i, (label, value)) in info.rows.iter().enumerate() {
                let row_y = y + 56. + i as f32 * 26.;
                text(s, ui, label, 12., LABEL_2.now(), x + 20., row_y + 2., 120.);
                text(s, ui, value, 13., LABEL.now(), x + 150., row_y, w - x - 190.);
            }
            text(s, ui, "Press Escape or click to close", 11., LABEL_2.now(), x + 20., y + 56. + info.rows.len() as f32 * 26. + 12., w - x - 56.);
        }
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
    fn only_image_extensions_are_read() {
        assert!(disk_image_rows(std::path::Path::new("/nonexistent/notes.txt"), 10).is_empty());
    }
}
