//! What sits behind the album picker's More page and the Info sheet's Edit Details row: the search, album renaming,
//! the details of a photo (title, caption, keywords, place, capture time), the Keyword Manager, importing from a
//! folder, Undo and Redo, merging duplicates and pasting copied edits. The text each of these needs is typed into the
//! card of `entry.rs`; keys from a physical keyboard reach it through `Photos::key`.

use super::entry::Outcome;
use super::*;
use crate::shell::keyboard;
use crate::shell::ui::{CARD_BG, FIELD_BG, LABEL, SEPARATOR_C};

/// A photo's edit with the adjustments of `clip` put on it: Copy Edits carries the look (filter, brightness, and the
/// like) and leaves the target's own rotation, crop and flips alone.
pub(super) fn paste_edits(clip: Edit, target: Edit) -> Edit {
    Edit { turns: target.turns, aspect: target.aspect, crop_zoom: target.crop_zoom, crop_center: target.crop_center, straighten: target.straighten, flip_h: target.flip_h, flip_v: target.flip_v, ..clip }
}

impl Photos {
    // --- keys ---

    /// Typed text or a shortcut from a keyboard: it goes to the text card while one is open. Otherwise Undo and Redo
    /// step through the changes, Select All selects the grid and 0 to 5 rate the photo in the viewer.
    /// Whether typing reaches a text field: the entry card is modal and its field takes every key (see `key`).
    pub fn text_focused(&self) -> bool {
        self.entry.is_some()
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.now = t;
        self.dirty = true;
        if self.entry.is_some() {
            self.entry_key(ui, out, t);
            return;
        }
        let browsing = self.editor.is_none() && self.slideshow.is_none();
        match out {
            Out::Named("Undo") => self.undo(ui),
            Out::Named("Redo") => self.redo(ui),
            Out::Named("Copy") if browsing && self.viewer.is_some() => self.copy_current(),
            Out::Named("SelectAll") if browsing && self.viewer.is_none() && !self.view.is_empty() => {
                if !self.selection.active {
                    self.selection.begin();
                }
                let all = self.view.clone();
                self.selection.select_all(&all);
                self.confirm_purge = false;
            }
            Out::Text(text) if browsing && self.viewer.is_some() && self.album != Album::Trash => {
                let mut chars = text.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if let Some(n) = c.to_digit(10).filter(|&n| n <= u32::from(stars::MAX_STARS)) {
                        if let Some(p) = self.viewing().cloned() {
                            self.rate_photo(&p, n as u8);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Escape: closes the topmost thing, or steps back one level. False when there is nothing left to step out of.
    pub fn hw_back(&mut self, ui: &Ui, t: f64) -> bool {
        self.now = t;
        self.dirty = true;
        if self.entry.is_some() {
            self.finish_entry(ui, false);
            return true;
        }
        if self.keywords_open || self.more_menu || self.album_menu {
            self.keywords_open = false;
            self.more_menu = false;
            self.album_menu = false;
            return true;
        }
        if self.sheet.is_some() {
            self.sheet = None;
            return true;
        }
        if self.editor.is_some() {
            self.editor = None;
            return true;
        }
        if self.slideshow.is_some() {
            self.slideshow = None;
            return true;
        }
        if self.viewer.is_some() {
            self.viewer = None;
            return true;
        }
        if self.selection.active {
            self.selection.end();
            return true;
        }
        if !self.search.is_empty() {
            self.set_search(ui, "");
            return true;
        }
        if self.filter.is_some() {
            self.clear_filter(ui);
            return true;
        }
        false
    }

    /// Tells the app that a physical keyboard does the typing (a desktop), so no on-screen keyboard is raised.
    pub fn set_hardware_keyboard(&mut self, on: bool) {
        self.hardware_keyboard = on;
    }

    /// Sets the star rating of `p` and keeps the change for Undo.
    pub(super) fn rate_photo(&mut self, p: &Photo, rating: u8) {
        let before = self.pref_now(&format!("rate:{}", p.path.display()));
        self.set_rating(p, rating);
        self.record("Rating", vec![before]);
    }

    // --- settings and Undo ---

    /// The value the setting `key` has now, as a change that would put it back.
    pub(super) fn pref_now(&self, key: &str) -> Change {
        Change::pref(key, self.prefs.get(key).unwrap_or(""))
    }

    /// Keeps the changes one action made as a step for Undo.
    pub(super) fn record(&mut self, label: &str, changes: Vec<Change>) {
        self.stack.record(label, changes);
    }

    /// Writes several settings with one save. The changes that would put the old values back, or None when the file
    /// could not be written (nothing changes then).
    pub(super) fn write_prefs(&mut self, writes: &[(String, String)]) -> Option<Vec<Change>> {
        if writes.is_empty() {
            return Some(Vec::new());
        }
        let changes: Vec<Change> = writes.iter().map(|(k, _)| self.pref_now(k)).collect();
        let pairs: Vec<(&str, &str)> = writes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let mut next = self.prefs.clone();
        match next.set_many(&pairs) {
            Ok(()) => {
                self.prefs = next;
                Some(changes)
            }
            Err(e) => {
                eprintln!("photos: {PREFS_FILE}: {e}");
                None
            }
        }
    }

    /// `write_prefs` as one named step for Undo. True when it was written.
    pub(super) fn write_many(&mut self, label: &str, writes: Vec<(String, String)>) -> bool {
        match self.write_prefs(&writes) {
            Some(changes) => {
                self.record(label, changes);
                true
            }
            None => false,
        }
    }

    /// Takes back the newest step.
    pub(super) fn undo(&mut self, ui: &Ui) {
        self.more_menu = false;
        if let Some(step) = self.stack.pop_undo() {
            let inverse = self.apply_step(ui, &step);
            self.stack.push_redo(inverse);
        }
        self.dirty = true;
    }

    /// Does the newest undone step again.
    pub(super) fn redo(&mut self, ui: &Ui) {
        self.more_menu = false;
        if let Some(step) = self.stack.pop_redo() {
            let inverse = self.apply_step(ui, &step);
            self.stack.push_undo(inverse);
        }
        self.dirty = true;
    }

    /// Applies the changes of `step` newest first and gives the step that would take them back.
    fn apply_step(&mut self, ui: &Ui, step: &Step) -> Step {
        let anchor = self.viewer_anchor();
        let mut inverse = Vec::new();
        let mut moved = false;
        for change in step.changes.iter().rev() {
            match change {
                Change::Pref { key, was } => {
                    inverse.push(self.pref_now(key));
                    self.pref_set(key, was);
                    // a different edit needs its pictures decoded again
                    if let Some(path) = key.strip_prefix("edit:") {
                        let path = PathBuf::from(path);
                        if let Some(p) = self.all.iter().find(|p| p.path == path).cloned() {
                            self.invalidate(&p);
                        }
                    }
                }
                Change::Trash { path, trashed } => {
                    if let Some(c) = self.set_trashed(path, *trashed) {
                        inverse.push(c);
                        moved = true;
                    }
                }
            }
        }
        self.rebuild(ui);
        self.restore_viewer(anchor);
        if moved {
            self.rescan();
        }
        Step { label: step.label.clone(), changes: inverse }
    }

    /// Moves the photo at `path` to Recently Deleted (`trashed`) or back to the library; the change that undoes it.
    fn set_trashed(&mut self, path: &Path, trashed: bool) -> Option<Change> {
        let photo = Photo::from_path_at(path.to_path_buf(), self.offset)?;
        let moved = if trashed { self.trash_one(&photo)? } else { self.recover_photo(&photo)? };
        Some(Change::Trash { path: moved.path, trashed: !trashed })
    }

    // --- the text card ---

    /// Opens the text card for `purpose` with `initial` in the field. `photo` is the photo it is about, if any.
    pub(super) fn open_entry(&mut self, ui: &Ui, purpose: Purpose, initial: &str, photo: Option<Photo>, t: f64) {
        self.album_menu = false;
        self.more_menu = false;
        self.keywords_open = false;
        self.search_before = self.search.clone();
        self.entry_photo = photo;
        self.entry_error = None;
        self.entry_press = None;
        self.pressed = None;
        self.entry = Some(Entry::open(ui, self.w, purpose, initial, !self.hardware_keyboard, t));
        self.dirty = true;
    }

    /// Applies a key typed on either keyboard to the card.
    pub(super) fn entry_key(&mut self, ui: &Ui, out: Out, t: f64) {
        let Some(entry) = self.entry.as_mut() else { return };
        let outcome = entry.key(ui, out, t);
        let live = entry.purpose.live();
        let text = entry.field.text().to_string();
        self.entry_error = None;
        match outcome {
            Outcome::Commit => self.finish_entry(ui, true),
            Outcome::Cancel => self.finish_entry(ui, false),
            Outcome::Typing if live && text.trim() != self.search => self.set_search(ui, &text),
            Outcome::Typing => {}
        }
        self.dirty = true;
    }

    /// A touch while the card is open: it is modal, so every touch is the card's.
    pub(super) fn entry_touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        let (w, h, status, hardware) = (self.w, self.h, ui.pt(STATUS_PT), self.hardware_keyboard);
        let Some(e) = self.entry.as_mut() else { return };
        let frame = e.field_frame(ui, w, status);
        if e.field.touch(ui, ev, frame, w as f32, t) {
            if e.field.focused && !hardware {
                e.keyboard.set_shown(true, t);
            }
            self.dirty = true;
            return;
        }
        let (cancel, done, top) = (e.cancel_rect(ui, w, status), e.done_rect(ui, w, status), e.keyboard_top(ui, h));
        let keyboard_shown = e.keyboard.shown();
        match ev {
            Event::Down(..) => {
                self.entry_press = if keyboard_shown && y >= top {
                    Some(EntryPress::Keyboard)
                } else if within(x, y, cancel) {
                    Some(EntryPress::Cancel)
                } else if within(x, y, done) {
                    Some(EntryPress::Done)
                } else {
                    None
                };
                if self.entry_press == Some(EntryPress::Keyboard) {
                    if let Some(e) = self.entry.as_mut() {
                        e.keyboard.down(x, y - top, t);
                    }
                }
            }
            Event::Move(..) => {
                if self.entry_press == Some(EntryPress::Keyboard) {
                    if let Some(e) = self.entry.as_mut() {
                        e.keyboard.moved(x, y - top);
                    }
                }
            }
            _ => match self.entry_press.take() {
                Some(EntryPress::Keyboard) => {
                    let typed = self.entry.as_mut().and_then(|e| e.keyboard.up(x, y - top, t));
                    if let Some(out) = typed {
                        self.entry_key(ui, out, t);
                    }
                }
                Some(EntryPress::Cancel) if within(x, y, cancel) => self.finish_entry(ui, false),
                Some(EntryPress::Done) if within(x, y, done) => self.finish_entry(ui, true),
                _ => {}
            },
        }
        self.dirty = true;
    }

    /// Why the text typed for `purpose` cannot be used, if it cannot.
    fn refuse(&self, purpose: &Purpose, text: &str) -> Option<&'static str> {
        match purpose {
            Purpose::TakenAt if !text.trim().is_empty() => {
                let current = self.entry_photo.as_ref().map_or(0, |p| p.mtime);
                meta::parse_taken(text, self.offset, unix_now(), current).is_none().then_some("That is not a date or a time. Try 2026-10-04 14:30.")
            }
            Purpose::ImportFolder => (!Path::new(text.trim()).is_dir()).then_some("That folder does not exist."),
            Purpose::SwitchLibrary => {
                let path = Path::new(text.trim());
                (text.trim().is_empty() || !path.is_absolute() || path.is_file()).then_some("Type the full path of a folder, such as /home/you/Pictures.")
            }
            Purpose::SmartRules(_) => meta::Query::parse(text, self.offset, unix_now()).is_empty().then_some("Add at least one rule, such as #beach or is:favorite."),
            _ => None,
        }
    }

    /// Closes the card, using its text when `commit`. A text the card refuses keeps it open.
    pub(super) fn finish_entry(&mut self, ui: &Ui, commit: bool) {
        let Some(entry) = self.entry.as_ref() else { return };
        let purpose = entry.purpose.clone();
        let text = entry.field.text().to_string();
        if commit {
            if let Some(why) = self.refuse(&purpose, &text) {
                self.entry_error = Some(why);
                self.dirty = true;
                return;
            }
        }
        self.entry = None;
        self.entry_error = None;
        self.entry_press = None;
        let photo = self.entry_photo.take();
        if commit {
            self.apply_entry(ui, purpose, &text, photo);
        } else if purpose == Purpose::Search {
            let before = self.search_before.clone();
            self.set_search(ui, &before);
        } else if matches!(purpose, Purpose::RenameKeyword(_)) {
            self.open_keywords(ui);
        }
        self.dirty = true;
    }

    /// Does what the text typed for `purpose` asks.
    fn apply_entry(&mut self, ui: &Ui, purpose: Purpose, text: &str, photo: Option<Photo>) {
        match purpose {
            Purpose::Search => self.set_search(ui, text),
            Purpose::RenameAlbum(slot) => {
                let name = meta::clean(text, meta::MAX_TEXT);
                if !name.is_empty() {
                    self.write_many("Rename Album", vec![(format!("ua.{slot}.name"), name)]);
                    self.rebuild(ui);
                }
            }
            Purpose::Title | Purpose::Caption | Purpose::Keywords | Purpose::Place => {
                let field = match purpose {
                    Purpose::Title => meta::Field::Title,
                    Purpose::Caption => meta::Field::Caption,
                    Purpose::Keywords => meta::Field::Keywords,
                    _ => meta::Field::Place,
                };
                if let Some(p) = photo {
                    self.write_detail(ui, &p, field, text);
                }
            }
            Purpose::TakenAt => {
                if let Some(p) = photo {
                    self.write_taken(ui, &p, text);
                }
            }
            Purpose::RenameKeyword(old) => {
                let writes = meta::rename_keyword_writes(&self.prefs, &self.all, &old, text);
                self.write_many("Rename Keyword", writes);
                self.rebuild(ui);
                self.open_keywords(ui);
            }
            Purpose::AddKeyword => {
                let chosen = self.selected_photos();
                let writes = meta::add_keyword_writes(&self.prefs, &chosen, text);
                self.write_many("Add Keyword", writes);
                self.selection.end();
                self.rebuild(ui);
            }
            Purpose::SmartName(slot) => {
                let name = meta::clean(text, meta::MAX_TEXT);
                let name = if name.is_empty() { format!("Smart Album {}", slot + 1) } else { name };
                let writes = vec![(format!("sa.{slot}.name"), name), (format!("sa.{slot}.q"), self.search.clone())];
                if self.write_many("New Smart Album", writes) {
                    self.set_search(ui, "");
                    self.select_album(ui, Album::Smart(slot));
                }
            }
            Purpose::SmartRules(slot) => {
                self.write_many("Smart Album Rules", vec![(format!("sa.{slot}.q"), text.trim().to_string())]);
                self.rebuild(ui);
            }
            Purpose::SwitchLibrary => {
                if !self.switch_library(ui, PathBuf::from(text.trim())) {
                    self.note = Some(("The library cannot be opened.".to_string(), self.now + NOTE_MS * 20.0));
                }
            }
            Purpose::ImportFolder => {
                let (from, into) = (PathBuf::from(text.trim()), self.pictures.clone());
                self.note = Some(("Importing…".to_string(), self.now + NOTE_MS * 20.0));
                self.pool.spawn(move || Done::Imported { report: import::import(&from, &into).map_err(|e| e.to_string()) });
            }
        }
    }

    // --- details of a photo ---

    /// Saves one field of the details of `p`; a search in force is run again so it follows.
    fn write_detail(&mut self, ui: &Ui, p: &Photo, field: meta::Field, text: &str) {
        let write = meta::field_write(&p.path, field, text);
        if !self.write_many("Details", vec![write]) || self.query.is_empty() {
            return;
        }
        let anchor = self.viewer_anchor();
        self.rebuild(ui);
        self.restore_viewer(anchor);
    }

    /// Gives `p` the capture time typed in `text` (empty = the time of the file again) and puts it in its place in
    /// the library.
    fn write_taken(&mut self, ui: &Ui, p: &Photo, text: &str) {
        let text = text.trim();
        let value = if text.is_empty() {
            String::new()
        } else {
            match meta::parse_taken(text, self.offset, unix_now(), p.mtime) {
                Some(t) => t.to_string(),
                None => return,
            }
        };
        let write = meta::field_write(&p.path, meta::Field::Taken, &value);
        if !self.write_many("Date and Time", vec![write]) {
            return;
        }
        if value.is_empty() {
            // the time of the file is read again by the scan
            self.rescan();
            return;
        }
        let anchor = self.viewer_anchor();
        meta::apply_taken(&self.prefs, &mut self.all);
        self.rebuild(ui);
        self.restore_viewer(anchor);
    }

    /// The place of the open photo: the name a person gave it, else the position in its EXIF block.
    fn details_place(&self, p: &Photo) -> String {
        let named = meta::details(&self.prefs, &p.path).place;
        if !named.is_empty() {
            return named;
        }
        self.info.iter().find(|(label, _)| label == "Location").map(|(_, v)| v.clone()).unwrap_or_default()
    }

    /// The rows of the Details sheet for the open photo: what they edit, their label and what they hold now.
    pub(super) fn details_rows(&self) -> Vec<(DetailRow, &'static str, String)> {
        let Some(p) = self.viewing() else { return Vec::new() };
        let d = meta::details(&self.prefs, &p.path);
        let place = self.details_place(p);
        let mut rows = vec![
            (DetailRow::Title, "Title", d.title),
            (DetailRow::Caption, "Caption", d.caption),
            (DetailRow::Keywords, "Keywords", meta::join_keywords(&d.keywords)),
            (DetailRow::Place, "Location", place.clone()),
            (DetailRow::Taken, "Date and Time", meta::format_taken(p.mtime, self.offset)),
        ];
        if !place.is_empty() {
            rows.push((DetailRow::CopyPlace, "Copy Location", String::new()));
        }
        rows.push((DetailRow::CopyLink, "Copy Link", String::new()));
        rows
    }

    /// The Details sheet's size in px: a title row and a row for each detail.
    pub(super) fn details_size(&self, ui: &Ui) -> (f32, f32) {
        (ui.pt(INFO_W_PT).min(self.w as f32 - 2.0 * ui.pt(MARGIN_PT)), ui.pt(INFO_ROW_PT) * (1 + self.details_rows().len()) as f32)
    }

    /// What a touch at (`x`, `y`) lands on in the Details sheet.
    pub(super) fn details_hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let rect = self.sheet_rect(ui, Sheet::Details);
        if !within(x, y, rect) {
            return Target::SheetCancel;
        }
        let k = ((y - rect.1) / ui.pt(INFO_ROW_PT)) as usize;
        k.checked_sub(1).and_then(|i| self.details_rows().get(i).map(|r| Target::Detail(r.0))).unwrap_or(Target::SheetCancel)
    }

    /// A tap on a row of the Details sheet: the card for that detail, or Copy Location.
    pub(super) fn activate_detail(&mut self, ui: &Ui, row: DetailRow, t: f64) {
        let Some(p) = self.viewing().cloned() else { return };
        let d = meta::details(&self.prefs, &p.path);
        match row {
            DetailRow::Title => self.open_entry(ui, Purpose::Title, &d.title, Some(p), t),
            DetailRow::Caption => self.open_entry(ui, Purpose::Caption, &d.caption, Some(p), t),
            DetailRow::Keywords => self.open_entry(ui, Purpose::Keywords, &meta::join_keywords(&d.keywords), Some(p), t),
            DetailRow::Place => self.open_entry(ui, Purpose::Place, &d.place, Some(p), t),
            DetailRow::Taken => {
                let shown = meta::format_taken(p.mtime, self.offset);
                self.open_entry(ui, Purpose::TakenAt, &shown, Some(p), t);
            }
            DetailRow::CopyPlace => {
                let place = self.details_place(&p);
                if !place.is_empty() {
                    crate::shell::clipboard::set_text(&place);
                }
                self.sheet = None;
            }
            DetailRow::CopyLink => {
                crate::shell::clipboard::set_text(&link::photo_url(&p.path));
                self.sheet = None;
            }
        }
    }

    // --- links ---

    /// Follows a `photos:` link: shows a picture, opens an album or runs a search. False when the link is not one of
    /// ours or names something that is not there. While the library is still being read the link waits for it.
    pub fn open_url(&mut self, ui: &Ui, url: &str) -> bool {
        let Some(l) = link::parse(url) else { return false };
        if self.scanning && l != link::Link::App {
            self.pending_link = Some(l);
            return true;
        }
        self.follow_link(ui, l)
    }

    /// Does what a link asks.
    pub(super) fn follow_link(&mut self, ui: &Ui, l: link::Link) -> bool {
        self.album_menu = false;
        self.more_menu = false;
        self.keywords_open = false;
        self.dirty = true;
        match l {
            link::Link::App => true,
            link::Link::Photo(path) => self.show(ui, &path),
            link::Link::Album(name) => {
                let want = name.to_lowercase();
                let found = self.menu_albums().into_iter().find(|&a| self.album_title(a).to_lowercase() == want);
                match found {
                    Some(album) => {
                        self.set_search(ui, "");
                        self.select_album(ui, album);
                        true
                    }
                    None => false,
                }
            }
            link::Link::Search(text) => {
                self.set_search(ui, &text);
                true
            }
        }
    }

    // --- export ---

    /// The rows of the export sheet: what they set, their label and what they hold now.
    pub(super) fn export_rows(&self) -> Vec<(ExportRow, &'static str, String)> {
        let o = &self.export;
        vec![
            (ExportRow::Kind, "Format", o.kind.label().to_string()),
            (ExportRow::Quality, "Quality", if o.kind == export::Kind::Png { "Lossless".to_string() } else { o.quality.label().to_string() }),
            (ExportRow::Size, "Size", o.size.label().to_string()),
            (ExportRow::Details, "Include Details", if o.details { "On" } else { "Off" }.to_string()),
            (ExportRow::Go, "Export to Files", String::new()),
        ]
    }

    /// The export sheet's size in px: a title row and a row for each choice.
    pub(super) fn export_size(&self, ui: &Ui) -> (f32, f32) {
        (ui.pt(INFO_W_PT).min(self.w as f32 - 2.0 * ui.pt(MARGIN_PT)), ui.pt(INFO_ROW_PT) * (1 + self.export_rows().len()) as f32)
    }

    /// What a touch at (`x`, `y`) lands on in the export sheet.
    pub(super) fn export_hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let rect = self.sheet_rect(ui, Sheet::Export);
        if !within(x, y, rect) {
            return Target::SheetCancel;
        }
        let k = ((y - rect.1) / ui.pt(INFO_ROW_PT)) as usize;
        k.checked_sub(1).and_then(|i| self.export_rows().get(i).map(|r| Target::Export(r.0))).unwrap_or(Target::SheetCancel)
    }

    /// A tap on a row of the export sheet: the choice steps to its next value, Export writes the file.
    pub(super) fn activate_export(&mut self, row: ExportRow) {
        match row {
            ExportRow::Kind => self.export.kind = self.export.kind.next(),
            ExportRow::Quality if self.export.kind == export::Kind::Jpeg => self.export.quality = self.export.quality.next(),
            ExportRow::Quality => {}
            ExportRow::Size => self.export.size = self.export.size.next(),
            ExportRow::Details => self.export.details = !self.export.details,
            ExportRow::Go => {
                self.sheet = None;
                self.start_export();
            }
        }
        let writes = self.export.writes();
        self.write_prefs(&writes);
    }

    /// Writes the open photo, with its edit applied, as a copy in Files under the choices of the export sheet. The
    /// work runs on a worker thread; a note says how it went.
    fn start_export(&mut self) {
        let Some(photo) = self.viewing().cloned() else { return };
        let (path, name, recipe) = (photo.path.clone(), photo.name(), self.recipe(&photo));
        let (options, details, dir) = (self.export, meta::details(&self.prefs, &photo.path), self.data.join("Files"));
        self.note = Some((format!("Exporting {name}\u{2026}"), self.now + NOTE_MS * 20.0));
        self.pool.spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let image = decode_edited(&bytes, recipe).ok_or("the picture could not be read")?;
                let data = export::encode(&image, &options, &details).ok_or("the picture could not be written")?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let (out, mut file) = crate::net::share::create_unique(&dir, &export::file_name(&name, options.kind)).map_err(|e| e.to_string())?;
                std::io::Write::write_all(&mut file, &data).map_err(|e| e.to_string())?;
                Ok(out)
            })();
            Done::Exported { name, result }
        });
    }

    // --- Keyword Manager ---

    /// Opens the Keyword Manager over the grid.
    pub(super) fn open_keywords(&mut self, ui: &Ui) {
        self.keywords = meta::keyword_counts(&self.prefs, &self.all);
        self.kw_scroll = Scroller::new();
        self.album_menu = false;
        self.more_menu = false;
        self.keywords_open = true;
        self.fit_keywords(ui);
        self.dirty = true;
    }

    /// The Keyword Manager's panel (x, y, w, h).
    pub(super) fn keyword_panel(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let margin = ui.pt(MARGIN_PT);
        let top = self.top(ui) + ui.pt(8.0);
        let rows = self.keywords.len().max(1) as f32 * ui.pt(KEYWORD_ROW_PT);
        let room = (self.content_bottom(ui) - top - ui.pt(8.0)).max(ui.pt(KEYWORD_ROW_PT));
        (margin, top, ui.pt(320.0).min(self.w as f32 - 2.0 * margin), rows.min(room))
    }

    /// Lets the list scroll exactly as far as it is longer than the panel.
    fn fit_keywords(&mut self, ui: &Ui) {
        let (_, _, _, h) = self.keyword_panel(ui);
        self.kw_scroll.set_max(self.keywords.len().max(1) as f32 * ui.pt(KEYWORD_ROW_PT) - h);
    }

    /// What a touch at (`x`, `y`) lands on while the Keyword Manager is open.
    pub(super) fn keywords_hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        let panel = self.keyword_panel(ui);
        if !within(x, y, panel) {
            return Target::MoreCancel;
        }
        let i = ((y - panel.1 + self.kw_scroll.offset()) / ui.pt(KEYWORD_ROW_PT)) as usize;
        if i >= self.keywords.len() {
            return Target::Content;
        }
        if x >= panel.0 + panel.2 - ui.pt(88.0) {
            Target::KeywordRename(i)
        } else {
            Target::KeywordUse(i)
        }
    }

    // --- picker pages ---

    /// The `k`-th row of the picker's second page.
    pub(super) fn more_row(&self, ui: &Ui, k: usize) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.album_row(ui, Album::All);
        (x, y + h * k as f32, w, h)
    }

    /// The title, colour and value text of a row of the second page.
    fn more_label(&self, target: Target) -> (String, u32, String) {
        let shorten = |text: &str| -> String {
            let head: String = text.chars().take(16).collect();
            if text.chars().count() > 16 { format!("{head}…") } else { head }
        };
        match target {
            Target::SearchOpen => ("Search".to_string(), WHITE, shorten(&self.search)),
            Target::SearchClear => ("Clear Search".to_string(), BLUE, String::new()),
            Target::KeywordsOpen => ("Keywords".to_string(), WHITE, String::new()),
            Target::PlayAlbum => ("Play Slideshow".to_string(), BLUE, String::new()),
            Target::MemoriesToggle => ("Memories".to_string(), WHITE, if self.prefs.get("memories") == Some("0") { "Off" } else { "On" }.to_string()),
            Target::ImportOpen => ("Import from Folder".to_string(), WHITE, String::new()),
            Target::UndoLast => (format!("Undo {}", self.stack.undo_label().unwrap_or("")), BLUE, String::new()),
            Target::RedoLast => (format!("Redo {}", self.stack.redo_label().unwrap_or("")), BLUE, String::new()),
            Target::RenameAlbum => ("Rename Album".to_string(), WHITE, String::new()),
            Target::SaveSmart => ("Save Search as Smart Album".to_string(), BLUE, String::new()),
            Target::EditRules => ("Edit Smart Album Rules".to_string(), WHITE, String::new()),
            Target::DeleteSmart => ("Delete Smart Album".to_string(), RED, String::new()),
            Target::AddKeywordOpen => ("Add Keyword to Selection".to_string(), WHITE, String::new()),
            Target::PasteEdits => ("Paste Edits to Selection".to_string(), WHITE, String::new()),
            _ => ("Merge Duplicates".to_string(), RED, self.duplicates.len().to_string()),
        }
    }

    // --- duplicates and copied edits ---

    /// Merge Duplicates: keeps the best copy of each group, folds the marks of the others into it and moves them to
    /// Recently Deleted.
    pub(super) fn merge_duplicates(&mut self, ui: &Ui) {
        self.more_menu = false;
        let present: HashSet<PathBuf> = self.visible.iter().map(|p| p.path.clone()).collect();
        let groups = dupes::members(&self.dupe_groups, &present);
        let candidates: Vec<Vec<dupes::Candidate>> = groups
            .iter()
            .map(|g| {
                g.iter()
                    .filter_map(|path| self.visible.iter().find(|p| &p.path == path))
                    .map(|p| dupes::Candidate { path: p.path.clone(), taken: p.mtime, favourite: self.is_favourite(p), rating: self.rating(p), edited: self.recipe(p).is_some() })
                    .collect()
            })
            .collect();
        let plan = dupes::plan(&candidates);
        let mut writes: Vec<(String, String)> = Vec::new();
        for m in &plan {
            if m.favourite {
                writes.push((format!("fav:{}", m.keep.display()), "1".to_string()));
            }
            if m.rating > 0 {
                writes.push((format!("rate:{}", m.keep.display()), m.rating.to_string()));
            }
            let mut keywords = meta::details(&self.prefs, &m.keep).keywords;
            for other in &m.drop {
                for k in meta::details(&self.prefs, other).keywords {
                    if keywords.len() < meta::MAX_KEYWORDS && !keywords.iter().any(|have| have.to_lowercase() == k.to_lowercase()) {
                        keywords.push(k);
                    }
                }
            }
            if !keywords.is_empty() {
                writes.push((meta::Field::Keywords.key(&m.keep), meta::join_keywords(&keywords)));
            }
        }
        let mut changes = self.write_prefs(&writes).unwrap_or_default();
        let mut merged = 0;
        for m in &plan {
            for path in &m.drop {
                let Some(photo) = Photo::from_path_at(path.clone(), self.offset) else { continue };
                if let Some(t) = self.trash_one(&photo) {
                    changes.push(Change::Trash { path: t.path, trashed: false });
                    merged += 1;
                }
            }
        }
        if merged > 0 {
            self.record("Merge Duplicates", changes);
            self.note = Some((format!("Merged {merged} {} into Recently Deleted.", if merged == 1 { "duplicate" } else { "duplicates" }), self.now + NOTE_MS));
            self.rescan();
        }
        self.rebuild(ui);
    }

    /// Deletes the open smart album; its rules go with it, and Undo brings them back.
    pub(super) fn delete_smart(&mut self, ui: &Ui) {
        self.more_menu = false;
        if let Album::Smart(slot) = self.album {
            let writes = vec![(format!("sa.{slot}.name"), String::new()), (format!("sa.{slot}.q"), String::new())];
            if self.write_many("Delete Smart Album", writes) {
                self.select_album(ui, Album::All);
            }
        }
    }

    /// Paste Edits: puts the copied adjustments on every selected photo.
    pub(super) fn paste_to_selection(&mut self, ui: &Ui) {
        self.more_menu = false;
        let chosen = self.selected_photos();
        self.paste_edits_to(ui, chosen);
        self.selection.end();
        self.rebuild(ui);
    }

    /// Pastes the copied edit onto `photos`, keeping each one's own geometry, as one step for Undo.
    pub(super) fn paste_edits_to(&mut self, ui: &Ui, photos: Vec<Photo>) {
        let Some(clip) = self.edit_clip else { return };
        if photos.is_empty() {
            return;
        }
        let mut changes = Vec::new();
        for p in &photos {
            changes.push(self.pref_now(&format!("edit:{}", p.path.display())));
            let target = self.recipe(p).unwrap_or_default();
            self.set_recipe(p, paste_edits(clip, target));
            self.invalidate(p);
        }
        self.record("Paste Edits", changes);
        self.rebuild(ui);
    }

    // --- drawing ---

    /// Text whose right end is at `right`.
    fn label_right(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, right: f32, cy: f32) {
        let img = self.text.get(ui, text, pt, weight, color);
        let (w, h) = (img.w as i32, img.h as i32);
        s.blit(img, right as i32 - w, cy as i32 - h / 2, 255, 255);
    }

    /// The text card over everything, with the keyboard under it.
    pub(super) fn draw_entry(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (w, h) = (self.w, self.h);
        let (wf, hf) = (w as f32, h as f32);
        let status = ui.pt(STATUS_PT);
        let Some(e) = &self.entry else { return };
        let card = e.card(ui, w, status);
        let (cancel, done) = (e.cancel_rect(ui, w, status), e.done_rect(ui, w, status));
        let (field, frame, hint) = (e.field_rect(ui, w, status), e.field_frame(ui, w, status), e.hint_rect(ui, w, status));
        let (heading, hint_text) = (e.purpose.heading(), e.purpose.hint());
        s.fill_rrect(0.0, 0.0, wf, hf, 0.0, rgba(0, 0, 0, 130));
        s.fill_rrect(card.0, card.1, card.2, card.3, flat(ui), CARD_BG.now());
        let mid = cancel.1 + cancel.3 / 2.0;
        let dim = |c: u32, target: EntryPress, pressed: Option<EntryPress>| if pressed == Some(target) { crate::gfx::scale(c, 140) } else { c };
        let pressed = self.entry_press;
        self.label(s, ui, heading, 17.0, Weight::Semibold, LABEL.now(), card.0 + card.2 / 2.0, mid);
        self.label_left(s, ui, "Cancel", 17.0, Weight::Regular, dim(BLUE, EntryPress::Cancel, pressed), cancel.0 + ui.pt(16.0), mid);
        self.label_right(s, ui, "Done", 17.0, Weight::Semibold, dim(BLUE, EntryPress::Done, pressed), done.0 + done.2 - ui.pt(16.0), mid);
        s.fill_rect(card.0 as usize, (card.1 + cancel.3) as usize, card.2 as usize, 1, SEPARATOR_C.now());
        s.fill_rrect(field.0, field.1 + ui.pt(4.0), field.2, field.3 - ui.pt(8.0), flat(ui), FIELD_BG.now());
        let (text, color) = match self.entry_error {
            Some(why) => (why, RED),
            None => (hint_text, SECONDARY),
        };
        for (i, line) in ui.wrap(text, 13.0, Weight::Regular, hint.2).iter().take(2).enumerate() {
            self.label_left(s, ui, line, 13.0, Weight::Regular, color, hint.0, hint.1 + ui.pt(12.0 + 17.0 * i as f32));
        }
        if let Some(e) = self.entry.as_mut() {
            e.field.draw(s, ui, frame, t);
            if e.keyboard.shown() || e.keyboard.animating() {
                let top = hf - home_zone(ui) - e.keyboard.h();
                e.keyboard.draw(s, ui, top, t);
                let below = top + e.keyboard.h() + e.keyboard.offset(t);
                s.fill_rrect(0.0, below, wf, hf - below, 0.0, keyboard::BG);
            }
        }
    }

    /// The line of news under the navigation bar.
    pub(super) fn draw_note(&mut self, s: &mut Surface, ui: &Ui) {
        let Some((text, _)) = self.note.clone() else { return };
        let (top, h) = (self.top(ui), ui.pt(36.0));
        s.fill_rect(0, top as usize, self.w as usize, h as usize, SHEET);
        self.label(s, ui, &text, 14.0, Weight::Regular, WHITE, self.w as f32 / 2.0, top + h / 2.0);
    }

    /// The picker's second page.
    pub(super) fn draw_more(&mut self, s: &mut Surface, ui: &Ui) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 80));
        let rows = self.more_rows();
        let (x, y, w, row_h) = self.album_row(ui, Album::All);
        s.fill_rrect(x, y, w, row_h * rows.len() as f32, flat(ui), SHEET);
        for (k, target) in rows.into_iter().enumerate() {
            let (_, ry, _, _) = self.more_row(ui, k);
            if self.pressed == Some(target) {
                s.fill_rrect(x, ry, w, row_h, flat(ui), rgba(255, 255, 255, 20));
            }
            if k > 0 {
                s.fill_rect((x + ui.pt(14.0)) as usize, ry as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            }
            let cy = ry + row_h / 2.0;
            let (title, color, value) = self.more_label(target);
            self.label_left(s, ui, &title, 17.0, Weight::Regular, color, x + ui.pt(16.0), cy);
            if !value.is_empty() {
                self.label_right(s, ui, &value, 14.0, Weight::Regular, SECONDARY, x + w - ui.pt(16.0), cy);
            }
        }
    }

    /// The Keyword Manager: each keyword with how many photos carry it, Rename on the right.
    pub(super) fn draw_keywords(&mut self, s: &mut Surface, ui: &Ui) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 80));
        let (x, y, w, h) = self.keyword_panel(ui);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let (row, off) = (ui.pt(KEYWORD_ROW_PT), self.kw_scroll.offset());
        let rows = self.keywords.clone();
        let pressed = self.pressed;
        let mut band = s.rows(y as usize, (y + h) as usize);
        if rows.is_empty() {
            self.label(&mut band, ui, "No Keywords", 17.0, Weight::Regular, SECONDARY, x + w / 2.0, y + row / 2.0);
        }
        for (i, (name, count)) in rows.iter().enumerate() {
            let ry = y + i as f32 * row - off;
            if ry + row < y || ry >= y + h {
                continue;
            }
            let cy = ry + row / 2.0;
            if pressed == Some(Target::KeywordUse(i)) {
                band.fill_rrect(x, ry, w, row, flat(ui), rgba(255, 255, 255, 20));
            }
            if i > 0 {
                band.fill_rect((x + ui.pt(14.0)) as usize, ry as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            }
            let shown = ui.fit(name, 17.0, w - ui.pt(190.0));
            self.label_left(&mut band, ui, &shown, 17.0, Weight::Regular, WHITE, x + ui.pt(16.0), cy);
            self.label_right(&mut band, ui, &count.to_string(), 14.0, Weight::Regular, SECONDARY, x + w - ui.pt(100.0), cy);
            let rename = if pressed == Some(Target::KeywordRename(i)) { crate::gfx::scale(BLUE, 140) } else { BLUE };
            self.label_right(&mut band, ui, "Rename", 15.0, Weight::Regular, rename, x + w - ui.pt(16.0), cy);
        }
    }

    /// The export sheet: a title row, then each choice with its value, and Export.
    pub(super) fn draw_export(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.sheet_rect(ui, Sheet::Export);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let row = ui.pt(INFO_ROW_PT);
        self.label(s, ui, "Export", 17.0, Weight::Semibold, WHITE, x + w / 2.0, y + row / 2.0);
        let pressed = self.pressed;
        for (i, (kind, label, value)) in self.export_rows().into_iter().enumerate() {
            let cy = y + row * (i + 1) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            let dim = |c: u32| if pressed == Some(Target::Export(kind)) { crate::gfx::scale(c, 140) } else { c };
            if kind == ExportRow::Go {
                self.label(s, ui, label, 15.0, Weight::Semibold, dim(BLUE), x + w / 2.0, cy);
                continue;
            }
            self.label_left(s, ui, label, 14.0, Weight::Regular, SECONDARY, x + ui.pt(16.0), cy);
            let color = if kind == ExportRow::Quality && self.export.kind == export::Kind::Png { SECONDARY } else { dim(BLUE) };
            self.label_right(s, ui, &value, 15.0, Weight::Regular, color, x + w - ui.pt(16.0), cy);
        }
    }

    /// The Details sheet: a title row, then each detail with its value.
    pub(super) fn draw_details(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.sheet_rect(ui, Sheet::Details);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let row = ui.pt(INFO_ROW_PT);
        self.label(s, ui, "Details", 17.0, Weight::Semibold, WHITE, x + w / 2.0, y + row / 2.0);
        let pressed = self.pressed;
        for (i, (kind, label, value)) in self.details_rows().into_iter().enumerate() {
            let cy = y + row * (i + 1) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            if matches!(kind, DetailRow::CopyPlace | DetailRow::CopyLink) {
                let color = if pressed == Some(Target::Detail(kind)) { crate::gfx::scale(BLUE, 140) } else { BLUE };
                self.label(s, ui, label, 15.0, Weight::Regular, color, x + w / 2.0, cy);
                continue;
            }
            self.label_left(s, ui, label, 14.0, Weight::Regular, SECONDARY, x + ui.pt(16.0), cy);
            let right = x + w - ui.pt(16.0);
            if value.is_empty() {
                self.label_right(s, ui, "Add", 15.0, Weight::Regular, BLUE, right, cy);
            } else {
                let shown = ui.fit(&value, 15.0, w - ui.pt(150.0));
                self.label_right(s, ui, &shown, 15.0, Weight::Regular, WHITE, right, cy);
            }
        }
    }
}

/// Height of a row of the Keyword Manager, in points.
const KEYWORD_ROW_PT: f32 = 56.0;

#[cfg(test)]
mod tests {
    use super::super::library::tests::picture;
    use super::super::tests::{choose_album, frame, photos, pump_until, tap};
    use super::*;

    /// Types `text` the way a physical keyboard does.
    fn type_text(p: &mut Photos, ui: &Ui, text: &str, t: f64) {
        for c in text.chars() {
            p.key(ui, Out::Text(c.to_string()), t);
        }
    }

    /// The row of the picker's second page that does `target`, as a tap position.
    fn more_tap(p: &mut Photos, ui: &Ui, target: Target, t: f64) {
        assert!(p.more_menu, "the second page is open");
        let k = p.more_rows().iter().position(|&r| r == target).unwrap_or_else(|| panic!("{target:?} is not on the page: {:?}", p.more_rows()));
        let (x, y, w, h) = p.more_row(ui, k);
        tap(p, ui, x + w / 2.0, y + h / 2.0, t);
    }

    /// Opens the picker, then its second page.
    fn open_more(p: &mut Photos, ui: &Ui, t: f64) {
        tap(p, ui, ui.pt(40.0), nav_mid(ui), t);
        assert!(p.album_menu);
        let (x, y, w, h) = p.extra_row_of(ui, Target::More);
        tap(p, ui, x + w / 2.0, y + h / 2.0, t + 100.0);
        assert!(p.more_menu && !p.album_menu);
    }

    fn open_viewer_info(p: &mut Photos, ui: &Ui, t: f64) {
        let cy = p.bar_top(ui) + ui.pt(BAR_PT) / 2.0;
        tap(p, ui, 1200.0 * 0.31, cy, t);
        assert_eq!(p.sheet, Some(Sheet::Info));
        frame(p, ui);
    }

    fn open_details(p: &mut Photos, ui: &Ui, t: f64) {
        open_viewer_info(p, ui, t);
        let (x, y, w, h) = p.info_details_row(ui);
        tap(p, ui, x + w / 2.0, y + h / 2.0, t + 100.0);
        assert_eq!(p.sheet, Some(Sheet::Details));
        frame(p, ui);
    }

    fn detail_tap(p: &mut Photos, ui: &Ui, row: DetailRow, t: f64) {
        let i = p.details_rows().iter().position(|r| r.0 == row).unwrap();
        let (x, y, w, _) = p.sheet_rect(ui, Sheet::Details);
        let h = ui.pt(INFO_ROW_PT);
        tap(p, ui, x + w / 2.0, y + h * (i + 1) as f32 + h / 2.0, t);
    }

    fn export_tap(p: &mut Photos, ui: &Ui, row: ExportRow, t: f64) {
        let i = p.export_rows().iter().position(|r| r.0 == row).unwrap();
        let (x, y, w, _) = p.sheet_rect(ui, Sheet::Export);
        let h = ui.pt(INFO_ROW_PT);
        tap(p, ui, x + w / 2.0, y + h * (i + 1) as f32 + h / 2.0, t);
    }

    #[test]
    fn a_search_is_saved_as_a_smart_album_whose_rules_can_change() {
        let (mut p, dir, ui) = photos("smart", 12);
        // saving needs a search to save
        open_more(&mut p, &ui, 500.0);
        assert!(!p.more_rows().contains(&Target::SaveSmart));
        assert!(p.hw_back(&ui, 600.0));
        p.set_search(&ui, "October 2026");
        open_more(&mut p, &ui, 1000.0);
        more_tap(&mut p, &ui, Target::SaveSmart, 1200.0);
        assert_eq!(p.entry.as_ref().map(|e| e.field.text().to_string()), Some("Smart Album 1".to_string()));
        for _ in 0.."Smart Album 1".len() {
            p.key(&ui, Out::Backspace, 1300.0);
        }
        type_text(&mut p, &ui, "Autumn", 1400.0);
        p.key(&ui, Out::Enter, 1500.0);
        assert_eq!(p.album(), Album::Smart(0));
        assert_eq!(p.search(), "", "the search became the album");
        assert_eq!(p.album_title(p.album()), "Autumn");
        assert_eq!(p.photos().len(), 2);
        assert_eq!(p.prefs.get("sa.0.q"), Some("October 2026"));
        assert!(p.menu_albums().contains(&Album::Smart(0)));
        frame(&mut p, &ui);
        // the album follows the library: a favourite rule picks up what is marked later
        open_more(&mut p, &ui, 2000.0);
        more_tap(&mut p, &ui, Target::EditRules, 2200.0);
        assert_eq!(p.entry.as_ref().map(|e| e.field.text().to_string()), Some("October 2026".to_string()));
        for _ in 0.."October 2026".len() {
            p.key(&ui, Out::Backspace, 2250.0);
        }
        p.key(&ui, Out::Enter, 2300.0);
        assert!(p.entry.is_some() && p.entry_error.is_some(), "an empty rule is refused");
        type_text(&mut p, &ui, "is:favorite", 2350.0);
        p.key(&ui, Out::Enter, 2400.0);
        assert!(p.entry.is_none());
        assert!(p.photos().is_empty());
        let first = p.all[0].clone();
        p.write_favourite(&first, true);
        p.rebuild(&ui);
        assert_eq!(p.photos().iter().map(|q| q.path.clone()).collect::<Vec<_>>(), [first.path.clone()]);
        // a search inside the album narrows it further
        p.set_search(&ui, "img_9999");
        assert!(p.photos().is_empty());
        p.set_search(&ui, "");
        // deleting leaves the library alone, and Undo brings the album back
        open_more(&mut p, &ui, 3000.0);
        more_tap(&mut p, &ui, Target::DeleteSmart, 3200.0);
        assert_eq!(p.album(), Album::All);
        assert!(!p.menu_albums().contains(&Album::Smart(0)));
        assert_eq!(p.photos().len(), 12);
        p.key(&ui, Out::Named("Undo"), 3300.0);
        assert!(p.menu_albums().contains(&Album::Smart(0)));
        assert_eq!(p.album_title(Album::Smart(0)), "Autumn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn photos_links_show_a_picture_open_an_album_and_search() {
        let (mut p, dir, ui) = photos("links", 4);
        let target = p.photos()[2].clone();
        // a picture
        assert!(p.open_url(&ui, &link::photo_url(&target.path)));
        assert_eq!(p.viewing().map(|q| q.path.clone()), Some(target.path.clone()));
        p.viewer = None;
        // an album, by name in any case
        assert!(p.open_url(&ui, "photos://album/favorites"));
        assert_eq!(p.album(), Album::Favorites);
        assert!(p.open_url(&ui, &link::album_url("Recently Deleted")));
        assert_eq!(p.album(), Album::Trash);
        assert!(!p.open_url(&ui, "photos://album/No%20Such%20Album"));
        assert_eq!(p.album(), Album::Trash, "an unknown album changes nothing");
        // a search, which also leaves the album it was in
        assert!(p.open_url(&ui, &link::search_url("img_0001")));
        assert_eq!(p.search(), "img_0001");
        assert_eq!(p.album(), Album::Trash, "the search stays in the open album");
        p.select_album(&ui, Album::All);
        assert_eq!(p.photos().len(), 1);
        // not ours, or a picture that is not in the library
        assert!(!p.open_url(&ui, "https://example.com/"));
        assert!(!p.open_url(&ui, "photos://photo/no/such/picture.png"));
        assert!(p.open_url(&ui, "photos:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_link_that_arrives_during_the_first_scan_waits_for_it() {
        let ui = crate::shell::tests::ui();
        let dir = super::super::library::tests::scratch("link-wait");
        let pics = dir.join("Pictures");
        picture(&pics.join("IMG_0000.png"), 1_790_000_000);
        picture(&pics.join("IMG_0001.png"), 1_790_086_400);
        let mut p = Photos::new(&ui, 1200, 1920, dir.clone(), vec![pics.clone()], 0).unwrap();
        assert!(p.scanning && p.photos().is_empty());
        assert!(p.open_url(&ui, &link::photo_url(&pics.join("IMG_0001.png"))));
        assert!(p.viewer.is_none(), "nothing to show yet");
        pump_until(&mut p, &ui, |p| p.viewer.is_some());
        assert_eq!(p.viewing().map(Photo::name), Some("IMG_0001.png".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_link_puts_a_link_to_the_open_photo_on_the_pasteboard() {
        let _clip = crate::shell::clipboard::tests::exclusive();
        let (mut p, dir, ui) = photos("copy-link", 2);
        p.open_viewer(1);
        let photo = p.photos()[1].clone();
        open_details(&mut p, &ui, 1000.0);
        detail_tap(&mut p, &ui, DetailRow::CopyLink, 1200.0);
        assert_eq!(p.sheet, None);
        let url = crate::shell::clipboard::text().expect("a link on the pasteboard");
        assert_eq!(link::parse(&url), Some(link::Link::Photo(photo.path)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_months_are_memories_that_play_as_a_slideshow() {
        let (mut p, dir, ui) = photos("memories", 12);
        // twelve pictures a day apart from 21 September: ten in September, two in October
        assert_eq!(p.memories.len(), 1);
        assert_eq!(p.album_title(Album::Memory(0)), "Memory: September 2026");
        assert!(p.menu_albums().contains(&Album::Memory(0)));
        choose_album(&mut p, &ui, Album::Memory(0), 1000.0);
        assert_eq!(p.photos().len(), 10);
        frame(&mut p, &ui);
        // the picker plays the album as a slideshow
        open_more(&mut p, &ui, 2000.0);
        more_tap(&mut p, &ui, Target::PlayAlbum, 2200.0);
        assert!(p.slideshow.as_ref().is_some_and(|s| s.photos.len() == 10 && s.index == 0));
        assert!(!p.more_menu);
        p.slideshow = None;
        // Memories can be switched off, and then they are gone
        open_more(&mut p, &ui, 3000.0);
        assert_eq!(p.more_label(Target::MemoriesToggle).2, "On");
        more_tap(&mut p, &ui, Target::MemoriesToggle, 3200.0);
        assert!(p.memories.is_empty());
        assert_eq!(p.album(), Album::All, "the open memory is gone");
        assert_eq!(p.prefs.get("memories"), Some("0"));
        assert!(!p.menu_albums().iter().any(|a| matches!(a, Album::Memory(_))));
        open_more(&mut p, &ui, 4000.0);
        more_tap(&mut p, &ui, Target::MemoriesToggle, 4200.0);
        assert_eq!(p.memories.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn screenshots_are_not_memories() {
        let (mut p, dir, ui) = photos("memories-shots", 2);
        for i in 0..6 {
            picture(&dir.join(format!("Pictures/Screenshot-2026110{}-100000.png", i + 1)), 1_793_000_000 + i * 86_400);
        }
        p.rescan();
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.screenshots.len(), 6);
        assert!(p.memories.is_empty(), "six screenshots in a month do not make a memory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_share_sheet_exports_a_copy_with_the_chosen_format_and_details() {
        let (mut p, dir, ui) = photos("export", 1);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        assert!(p.write_many("Details", vec![meta::field_write(&photo.path, meta::Field::Title, "Harbour")]));
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
        assert_eq!(p.sheet, Some(Sheet::Share));
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 5.5 / 8.0, 1100.0);
        assert_eq!(p.sheet, Some(Sheet::Export));
        frame(&mut p, &ui);
        // each tap steps a choice; quality means nothing for a PNG
        export_tap(&mut p, &ui, ExportRow::Kind, 1200.0);
        assert_eq!(p.export.kind, export::Kind::Png);
        export_tap(&mut p, &ui, ExportRow::Quality, 1250.0);
        assert_eq!(p.export.quality, export::Quality::High);
        export_tap(&mut p, &ui, ExportRow::Details, 1300.0);
        assert!(!p.export.details);
        export_tap(&mut p, &ui, ExportRow::Details, 1350.0);
        assert!(p.export.details);
        assert_eq!(export::Options::load(&Prefs::load(dir.join(PREFS_FILE))), p.export, "the choices are kept");
        // Export writes the file in Files and says so
        export_tap(&mut p, &ui, ExportRow::Go, 1400.0);
        assert_eq!(p.sheet, None);
        pump_until(&mut p, &ui, |p| p.note.as_ref().is_some_and(|(t, _)| t.starts_with("Exported")));
        assert_eq!(p.note.as_ref().map(|(t, _)| t.as_str()), Some("Exported IMG_0000.png to Files."));
        let out = dir.join("Files/IMG_0000.png");
        let bytes = std::fs::read(&out).expect("the copy is in Files");
        assert_eq!(crate::cg::imageio::pixel_size(&bytes), Some((4, 4)));
        assert!(bytes.windows(7).any(|w| w == b"Harbour"), "the title is in the file");
        assert!(photo.path.exists(), "the original stays");
        // a second export does not overwrite the first
        p.activate(&ui, Target::SheetExport, 1500.0);
        export_tap(&mut p, &ui, ExportRow::Go, 1600.0);
        pump_until(&mut p, &ui, |p| p.note.as_ref().is_some_and(|(t, _)| t.starts_with("Exported IMG_0000 2")));
        assert!(dir.join("Files/IMG_0000 2.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_search_follows_the_typing_and_cancel_puts_the_old_one_back() {
        let (mut p, dir, ui) = photos("search", 12);
        picture(&dir.join("Pictures/Holiday_beach.png"), 1_790_000_000);
        p.rescan();
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.photos().len(), 13);
        open_more(&mut p, &ui, 1000.0);
        more_tap(&mut p, &ui, Target::SearchOpen, 1200.0);
        assert!(p.entry.is_some() && !p.more_menu);
        type_text(&mut p, &ui, "beach", 1300.0);
        assert_eq!(p.search(), "beach");
        assert_eq!(p.photos().iter().map(Photo::name).collect::<Vec<_>>(), ["Holiday_beach.png"]);
        assert!(p.describe().contains("photos_search=beach"));
        frame(&mut p, &ui);
        // Return keeps the search; the title shows it
        p.key(&ui, Out::Enter, 1400.0);
        assert!(p.entry.is_none());
        assert_eq!(p.photos().len(), 1);
        frame(&mut p, &ui);
        // tapping the title edits it; Cancel puts the text from before back
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 1500.0);
        assert!(p.entry.is_some());
        type_text(&mut p, &ui, "x", 1600.0);
        assert_eq!(p.photos().len(), 0, "beachx matches nothing");
        frame(&mut p, &ui);
        let (cx, cy, cw, ch) = p.entry.as_ref().unwrap().cancel_rect(&ui, 1200, ui.pt(STATUS_PT));
        tap(&mut p, &ui, cx + cw / 2.0, cy + ch / 2.0, 1700.0);
        assert!(p.entry.is_none());
        assert_eq!(p.search(), "beach");
        assert_eq!(p.photos().len(), 1);
        // Escape steps out of the search
        assert!(p.hw_back(&ui, 1800.0));
        assert_eq!(p.search(), "");
        assert_eq!(p.photos().len(), 13);
        assert!(!p.hw_back(&ui, 1900.0), "nothing left to step out of");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_search_can_be_a_date_a_flag_or_a_rating() {
        let (mut p, dir, ui) = photos("search-ops", 12);
        // twelve pictures a day apart from 2026-09-21: the last two are in October
        p.set_search(&ui, "October 2026");
        assert_eq!(p.photos().len(), 2);
        p.set_search(&ui, "2026-09-25");
        assert_eq!(p.photos().len(), 1);
        let first = p.photos()[0].clone();
        p.set_search(&ui, "is:favorite");
        assert!(p.photos().is_empty());
        p.write_favourite(&first, true);
        p.set_search(&ui, "is:favorite");
        assert_eq!(p.photos().len(), 1);
        p.set_search(&ui, "rating:3");
        assert!(p.photos().is_empty());
        p.set_rating(&first, 4);
        p.set_search(&ui, "rating:3 kind:png");
        assert_eq!(p.photos().len(), 1);
        p.set_search(&ui, "kind:jpg");
        assert!(p.photos().is_empty());
        p.set_search(&ui, "");
        assert_eq!(p.photos().len(), 12);
        frame(&mut p, &ui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_search_result_says_so_and_keeps_the_library_page_away() {
        let (mut p, dir, ui) = photos("search-empty", 3);
        p.set_search(&ui, "nothing like this");
        assert!(p.photos().is_empty() && !p.blank_library());
        frame(&mut p, &ui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn albums_are_renamed_through_the_card() {
        let (mut p, dir, ui) = photos("rename-album", 2);
        let slot = p.new_album().unwrap();
        p.rebuild(&ui);
        p.select_album(&ui, Album::User(slot as u8));
        open_more(&mut p, &ui, 1000.0);
        more_tap(&mut p, &ui, Target::RenameAlbum, 1200.0);
        assert_eq!(p.entry.as_ref().map(|e| e.field.text().to_string()), Some("Album 1".to_string()), "the card starts with the old name");
        for _ in 0.."Album 1".len() {
            p.key(&ui, Out::Backspace, 1300.0);
        }
        type_text(&mut p, &ui, "Trip", 1400.0);
        p.key(&ui, Out::Enter, 1500.0);
        assert_eq!(p.album_title(Album::User(slot as u8)), "Trip");
        assert_eq!(p.prefs.get("ua.0.name"), Some("Trip"));
        // an empty name keeps the old one
        open_more(&mut p, &ui, 2000.0);
        more_tap(&mut p, &ui, Target::RenameAlbum, 2200.0);
        for _ in 0.."Trip".len() {
            p.key(&ui, Out::Backspace, 2300.0);
        }
        p.key(&ui, Out::Enter, 2400.0);
        assert_eq!(p.album_title(Album::User(slot as u8)), "Trip");
        // and the rename can be taken back
        p.key(&ui, Out::Named("Undo"), 2500.0);
        assert_eq!(p.album_title(Album::User(slot as u8)), "Album 1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_details_sheet_edits_title_caption_keywords_and_place_and_search_finds_them() {
        let _clip = crate::shell::clipboard::tests::exclusive();
        let (mut p, dir, ui) = photos("details", 3);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        open_details(&mut p, &ui, 1000.0);
        detail_tap(&mut p, &ui, DetailRow::Title, 1200.0);
        assert!(p.entry.is_some());
        type_text(&mut p, &ui, "Harbour", 1250.0);
        p.key(&ui, Out::Enter, 1300.0);
        detail_tap(&mut p, &ui, DetailRow::Keywords, 1400.0);
        type_text(&mut p, &ui, "sea, Boats", 1450.0);
        p.key(&ui, Out::Enter, 1500.0);
        detail_tap(&mut p, &ui, DetailRow::Place, 1600.0);
        type_text(&mut p, &ui, "Reykjavik", 1650.0);
        p.key(&ui, Out::Enter, 1700.0);
        detail_tap(&mut p, &ui, DetailRow::Caption, 1800.0);
        type_text(&mut p, &ui, "Evening light", 1850.0);
        p.key(&ui, Out::Enter, 1900.0);
        let d = meta::details(&p.prefs, &photo.path);
        assert_eq!((d.title.as_str(), d.place.as_str(), d.caption.as_str()), ("Harbour", "Reykjavik", "Evening light"));
        assert_eq!(d.keywords, ["sea", "Boats"]);
        assert_eq!(p.sheet, Some(Sheet::Details), "the sheet stays under the card");
        assert!(p.details_rows().iter().any(|r| r.0 == DetailRow::CopyPlace), "a place can be copied");
        frame(&mut p, &ui);
        // the pasteboard gets the place
        detail_tap(&mut p, &ui, DetailRow::CopyPlace, 2000.0);
        assert_eq!(p.sheet, None);
        assert_eq!(crate::shell::clipboard::text().as_deref(), Some("Reykjavik"));
        // search matches the details, and a keyword by #
        p.viewer = None;
        for (query, hits) in [("harbour", 1), ("evening", 1), ("reykjavik", 1), ("#boats", 1), ("tag:sea", 1), ("#harbour", 0), ("img_0001", 1)] {
            p.set_search(&ui, query);
            assert_eq!(p.photos().len(), hits, "{query}");
        }
        // the whole step can be undone
        p.set_search(&ui, "");
        p.key(&ui, Out::Named("Undo"), 2100.0);
        assert_eq!(meta::details(&p.prefs, &photo.path).caption, "");
        p.key(&ui, Out::Named("Redo"), 2200.0);
        assert_eq!(meta::details(&p.prefs, &photo.path).caption, "Evening light");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrected_capture_time_moves_the_photo_and_a_bad_one_is_refused() {
        let (mut p, dir, ui) = photos("extras-taken", 3);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        open_details(&mut p, &ui, 1000.0);
        detail_tap(&mut p, &ui, DetailRow::Taken, 1200.0);
        // the card starts with the time of the photo
        assert_eq!(p.entry.as_ref().map(|e| e.field.text().to_string()), Some(meta::format_taken(photo.mtime, 0)));
        for _ in 0..40 {
            p.key(&ui, Out::Backspace, 1250.0);
        }
        type_text(&mut p, &ui, "someday", 1300.0);
        p.key(&ui, Out::Enter, 1350.0);
        assert!(p.entry.is_some(), "the card stays open");
        assert!(p.entry_error.is_some());
        frame(&mut p, &ui);
        for _ in 0..40 {
            p.key(&ui, Out::Backspace, 1400.0);
        }
        type_text(&mut p, &ui, "2027-01-02 03:04", 1450.0);
        p.key(&ui, Out::Enter, 1500.0);
        assert!(p.entry.is_none());
        let moved = p.all.iter().find(|q| q.path == photo.path).unwrap();
        assert_eq!(meta::format_taken(moved.mtime, 0), "2027-01-02 03:04");
        assert_eq!(p.all.last().map(|q| q.path.clone()), Some(photo.path.clone()), "it is the newest now");
        assert_eq!(p.photos().len(), 3);
        // the viewer follows the photo
        assert_eq!(p.viewing().map(|q| q.path.clone()), Some(photo.path.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_keyword_manager_lists_searches_renames_and_deletes() {
        let (mut p, dir, ui) = photos("keywords", 4);
        let (a, b) = (p.photos()[0].clone(), p.photos()[1].clone());
        let writes = vec![meta::field_write(&a.path, meta::Field::Keywords, "Beach, Sun Set"), meta::field_write(&b.path, meta::Field::Keywords, "beach")];
        assert!(p.write_many("Details", writes));
        open_more(&mut p, &ui, 1000.0);
        more_tap(&mut p, &ui, Target::KeywordsOpen, 1200.0);
        assert!(p.keywords_open && !p.more_menu);
        assert_eq!(p.keywords, [("Beach".to_string(), 2), ("Sun Set".to_string(), 1)]);
        frame(&mut p, &ui);
        // a row searches by that keyword
        let (px, py, _, _) = p.keyword_panel(&ui);
        let row = ui.pt(KEYWORD_ROW_PT);
        tap(&mut p, &ui, px + 30.0, py + row * 1.5, 1300.0);
        assert!(!p.keywords_open);
        assert_eq!(p.search(), "#Sun_Set");
        assert_eq!(p.photos().len(), 1);
        p.set_search(&ui, "");
        // Rename folds Sun Set into Beach
        open_more(&mut p, &ui, 2000.0);
        more_tap(&mut p, &ui, Target::KeywordsOpen, 2200.0);
        let (px, py, pw, _) = p.keyword_panel(&ui);
        tap(&mut p, &ui, px + pw - 20.0, py + row * 1.5, 2300.0);
        assert_eq!(p.entry.as_ref().map(|e| e.purpose.clone()), Some(Purpose::RenameKeyword("Sun Set".to_string())));
        for _ in 0.."Sun Set".len() {
            p.key(&ui, Out::Backspace, 2350.0);
        }
        type_text(&mut p, &ui, "beach", 2360.0);
        p.key(&ui, Out::Enter, 2400.0);
        assert!(p.keywords_open, "the manager is back");
        assert_eq!(p.keywords, [("Beach".to_string(), 2)]);
        // an empty name deletes the keyword, and cancel leaves it
        tap(&mut p, &ui, px + pw - 20.0, py + row * 0.5, 2500.0);
        for _ in 0.."Beach".len() {
            p.key(&ui, Out::Backspace, 2550.0);
        }
        p.key(&ui, Out::Enter, 2600.0);
        assert!(p.keywords.is_empty());
        assert!(meta::details(&p.prefs, &a.path).keywords.is_empty());
        frame(&mut p, &ui);
        // a tap outside closes it
        tap(&mut p, &ui, 1190.0, 1800.0, 2700.0);
        assert!(!p.keywords_open);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_keyword_can_be_added_to_the_selection() {
        let (mut p, dir, ui) = photos("add-keyword", 4);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 96.0 + 38.0), nav_mid(&ui), 1000.0);
        assert!(p.selection.active);
        p.key(&ui, Out::Named("SelectAll"), 1100.0);
        assert_eq!(p.selection.len(), 4);
        open_more(&mut p, &ui, 1200.0);
        more_tap(&mut p, &ui, Target::AddKeywordOpen, 1400.0);
        type_text(&mut p, &ui, "family", 1450.0);
        p.key(&ui, Out::Enter, 1500.0);
        assert_eq!(meta::keyword_counts(&p.prefs, &p.all), [("family".to_string(), 4)]);
        assert!(!p.selection.active);
        p.key(&ui, Out::Named("Undo"), 1600.0);
        assert!(meta::keyword_counts(&p.prefs, &p.all).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_and_redo_step_through_favourites_ratings_and_deleting() {
        let (mut p, dir, ui) = photos("undo", 3);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        // favourite from the viewer bar
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.5, cy, 1000.0);
        assert!(p.is_favourite(&photo));
        // a rating with a number key
        p.key(&ui, Out::Text("4".into()), 1100.0);
        assert_eq!(p.rating(&photo), 4);
        p.key(&ui, Out::Text("0".into()), 1150.0);
        assert_eq!(p.rating(&photo), 0);
        p.key(&ui, Out::Text("9".into()), 1160.0);
        assert_eq!(p.rating(&photo), 0, "only 0 to 5 rate");
        assert_eq!(p.stack.undo_label(), Some("Rating"));
        p.key(&ui, Out::Named("Undo"), 1200.0);
        assert_eq!(p.rating(&photo), 4);
        p.key(&ui, Out::Named("Undo"), 1300.0);
        assert_eq!(p.rating(&photo), 0);
        p.key(&ui, Out::Named("Undo"), 1400.0);
        assert!(!p.is_favourite(&photo));
        p.key(&ui, Out::Named("Redo"), 1500.0);
        assert!(p.is_favourite(&photo));
        assert_eq!(p.stack.redo_label(), Some("Rating"));
        // a new change drops what could be redone
        p.key(&ui, Out::Text("2".into()), 1600.0);
        assert!(p.stack.redo_label().is_none());
        // deleting goes to Recently Deleted and comes back
        p.viewer = None;
        p.open_viewer(1);
        let doomed = p.photos()[1].clone();
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.88, cy, 2000.0);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Delete);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 2100.0);
        assert_eq!(p.trash.len(), 1);
        assert_eq!(p.stack.undo_label(), Some("Delete"));
        p.key(&ui, Out::Named("Undo"), 2200.0);
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert!(p.trash.is_empty());
        assert!(p.all.iter().any(|q| q.path == doomed.path), "the file is back where it was");
        assert!(doomed.path.exists());
        p.key(&ui, Out::Named("Redo"), 2300.0);
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.trash.len(), 1);
        assert!(!doomed.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_picker_lists_undo_and_redo_with_what_they_do() {
        let (mut p, dir, ui) = photos("undo-rows", 2);
        p.open_viewer(0);
        p.key(&ui, Out::Text("3".into()), 1000.0);
        p.viewer = None;
        open_more(&mut p, &ui, 1100.0);
        assert!(p.more_rows().contains(&Target::UndoLast) && !p.more_rows().contains(&Target::RedoLast));
        assert_eq!(p.more_label(Target::UndoLast).0, "Undo Rating");
        more_tap(&mut p, &ui, Target::UndoLast, 1300.0);
        assert!(!p.more_menu);
        assert_eq!(p.rating(&p.photos()[0].clone()), 0);
        open_more(&mut p, &ui, 1400.0);
        assert_eq!(p.more_label(Target::RedoLast).0, "Redo Rating");
        frame(&mut p, &ui);
        more_tap(&mut p, &ui, Target::RedoLast, 1600.0);
        assert_eq!(p.rating(&p.photos()[0].clone()), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_second_page_closes_on_a_tap_outside_and_on_escape() {
        let (mut p, dir, ui) = photos("more-close", 2);
        open_more(&mut p, &ui, 1000.0);
        frame(&mut p, &ui);
        tap(&mut p, &ui, 1190.0, 1800.0, 1200.0);
        assert!(!p.more_menu);
        open_more(&mut p, &ui, 2000.0);
        assert!(p.hw_back(&ui, 2100.0));
        assert!(!p.more_menu);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identical_files_make_a_duplicates_album_and_merge_keeps_one() {
        let (mut p, dir, ui) = photos("extras-merge-dupes", 3);
        // the three fixture pictures are the same bytes
        assert_eq!(p.dupe_groups.len(), 1);
        assert!(p.menu_albums().contains(&Album::Duplicates));
        assert_eq!(p.album_list(Album::Duplicates).len(), 3);
        let favourite = p.photos()[2].clone();
        p.write_favourite(&favourite, true);
        p.set_rating(&p.photos()[0].clone(), 5);
        let wrote = vec![meta::field_write(&p.photos()[1].path, meta::Field::Keywords, "kept")];
        assert!(p.write_many("Details", wrote));
        choose_album(&mut p, &ui, Album::Duplicates, 1000.0);
        assert_eq!(p.photos().len(), 3);
        frame(&mut p, &ui);
        open_more(&mut p, &ui, 1100.0);
        more_tap(&mut p, &ui, Target::MergeDuplicates, 1300.0);
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.all.len(), 1, "one copy stays");
        assert_eq!(p.trash.len(), 2);
        assert_eq!(p.album(), Album::All, "no duplicates are left");
        assert!(!p.menu_albums().contains(&Album::Duplicates));
        let kept = p.all[0].clone();
        assert!(p.is_favourite(&kept), "the favourite mark of a copy comes along");
        assert_eq!(p.rating(&kept), 5);
        assert_eq!(meta::details(&p.prefs, &kept.path).keywords, ["kept"]);
        // undo brings the copies back
        p.key(&ui, Out::Named("Undo"), 2000.0);
        pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 3);
        assert!(p.trash.is_empty());
        assert_eq!(p.album_list(Album::Duplicates).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copied_edits_paste_in_the_editor_and_onto_a_selection() {
        let (mut p, dir, ui) = photos("copy-edits", 3);
        p.open_viewer(0);
        p.tick(&ui, 1000.0);
        // The viewer decodes the shown picture and its neighbour on worker threads, in no fixed order; Edit only opens
        // once the shown one is there, so wait for that one and not for any picture.
        let shown = p.viewing().expect("the viewer shows a photo").key();
        pump_until(&mut p, &ui, |p| p.full.contains(&shown));
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.69, cy, 2000.0);
        assert!(p.editor.is_some(), "the Edit button opens the editor on the shown picture");
        let (row, top) = (ui.pt(EDIT_ROW_PT), p.edit_panel_top(&ui));
        tap(&mut p, &ui, 1200.0 / 4.0 * 3.5, top + row * 2.5, 2100.0);
        assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Tools));
        // rotate once and brighten
        tap(&mut p, &ui, 1200.0 / 6.0, top + row * 3.5, 2200.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.turns), Some(3), "Rotate Left is a quarter back");
        tap(&mut p, &ui, 1200.0 / 4.0 * 0.5, top + row * 2.5, 2250.0);
        assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Adjust));
        let (_, x1) = p.slider_track(&ui);
        tap(&mut p, &ui, x1, top + row * 3.5, 2300.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.brightness), Some(1.0));
        tap(&mut p, &ui, 1200.0 / 4.0 * 3.5, top + row * 2.5, 2350.0);
        assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Tools));
        tap(&mut p, &ui, 300.0, top + row * 5.5, 2400.0);
        let clip = p.edit_clip.expect("Copy Edits keeps the edit");
        assert_eq!((clip.turns, clip.brightness), (3, 1.0));
        // paste on a fresh copy of the editor: the look arrives, the rotation stays
        p.editor.as_mut().unwrap().edit = Edit { turns: 1, ..Edit::default() };
        tap(&mut p, &ui, 900.0, top + row * 5.5, 2500.0);
        let pasted = p.editor.as_ref().unwrap().edit;
        assert_eq!((pasted.turns, pasted.brightness), (1, 1.0));
        frame(&mut p, &ui);
        tap(&mut p, &ui, 100.0, ui.pt(STATUS_PT) + 5.0, 2600.0);
        p.viewer = None;
        // paste onto two selected photos from the picker
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 96.0 + 38.0), nav_mid(&ui), 3000.0);
        p.selection.select_range(&p.view.clone(), 1, 2);
        open_more(&mut p, &ui, 3100.0);
        more_tap(&mut p, &ui, Target::PasteEdits, 3300.0);
        for i in [1, 2] {
            let photo = p.photos()[i].clone();
            let edit = p.recipe(&photo).expect("pasted");
            assert_eq!((edit.turns, edit.brightness), (0, 1.0), "the look, not the rotation");
        }
        assert!(p.recipe(&p.photos()[0].clone()).is_none());
        p.key(&ui, Out::Named("Undo"), 3400.0);
        assert!(p.recipe(&p.photos()[1].clone()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_keeps_the_target_geometry() {
        let clip = Edit { turns: 2, flip_h: true, brightness: 0.5, auto: true, filter: Filter::Mono, ..Edit::default() };
        let target = Edit { turns: 1, flip_v: true, aspect: Aspect::Square, ..Edit::default() };
        let got = paste_edits(clip, target);
        assert_eq!((got.turns, got.flip_h, got.flip_v, got.aspect), (1, false, true, Aspect::Square));
        assert_eq!((got.brightness, got.auto, got.filter), (0.5, true, Filter::Mono));
    }

    #[test]
    fn importing_copies_a_folder_and_reports() {
        // not "import": the importer's own test scratches `<tests>/import-<pid>` in this same process, and two tests
        // removing and refilling one directory race each other
        let (mut p, dir, ui) = photos("extras-import", 1);
        let source = dir.join("card");
        picture(&source.join("DCIM/IMG_9001.png"), 1_790_500_000);
        std::fs::write(source.join("notes.txt"), b"x").unwrap();
        open_more(&mut p, &ui, 1000.0);
        more_tap(&mut p, &ui, Target::ImportOpen, 1200.0);
        let field = p.entry.as_ref().map(|e| e.field.text().to_string()).unwrap();
        for _ in 0..field.chars().count() {
            p.key(&ui, Out::Backspace, 1250.0);
        }
        // a folder that is not there is refused
        type_text(&mut p, &ui, "/no/such/folder", 1300.0);
        p.key(&ui, Out::Enter, 1350.0);
        assert!(p.entry.is_some() && p.entry_error.is_some());
        for _ in 0.."/no/such/folder".len() {
            p.key(&ui, Out::Backspace, 1400.0);
        }
        type_text(&mut p, &ui, &source.display().to_string(), 1450.0);
        p.key(&ui, Out::Enter, 1500.0);
        assert!(p.entry.is_none());
        pump_until(&mut p, &ui, |p| p.note.as_ref().is_some_and(|(t, _)| t.starts_with("Imported")) && !p.scanning);
        assert_eq!(p.note.as_ref().map(|(t, _)| t.as_str()), Some("Imported 1 photo, 0 already in the library."));
        assert_eq!(p.all.len(), 2);
        assert!(dir.join("Pictures/IMG_9001.png").exists());
        frame(&mut p, &ui);
        // the note goes away by itself
        p.tick(&ui, 1500.0 + NOTE_MS * 2.0);
        assert!(p.note.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_all_and_escape_work_from_the_keyboard() {
        let (mut p, dir, ui) = photos("keys", 5);
        p.key(&ui, Out::Named("SelectAll"), 1000.0);
        assert!(p.selection.active);
        assert_eq!(p.selection.len(), 5);
        assert!(p.hw_back(&ui, 1100.0));
        assert!(!p.selection.active);
        p.open_viewer(0);
        p.key(&ui, Out::Named("SelectAll"), 1200.0);
        assert!(!p.selection.active, "not while a photo is open");
        assert!(p.hw_back(&ui, 1300.0));
        assert!(p.viewer.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_duplicate_of_a_photo_keeps_its_details() {
        let (mut p, dir, ui) = photos("dup-details", 1);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        assert!(p.write_many("Details", vec![meta::field_write(&photo.path, meta::Field::Title, "Original")]));
        p.duplicate_current();
        pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 2);
        let copy = p.all.iter().find(|q| q.path != photo.path).unwrap().clone();
        assert_eq!(meta::details(&p.prefs, &copy.path).title, "Original");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
