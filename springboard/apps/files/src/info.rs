//! Get Info and Tags: the info panel, the Tags panel and the colour tag dots.
use crate::*;

/// Tags panel layout: the first colour row's top, the row pitch, and the panel height.
pub(crate) const TAG_ROWS_TOP: f32 = 124.;

pub(crate) const TAG_ROW: f32 = 30.;

pub(crate) const TAG_PANEL_H: f32 = 330.;

/// Folder totals for Get Info stop counting after this long.
pub(crate) const INFO_BUDGET: Duration = Duration::from_millis(800);

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
        let rows = springboard::files::info::rows(&entry, summary, created, &tags, wall.unix, wall.offset);
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
