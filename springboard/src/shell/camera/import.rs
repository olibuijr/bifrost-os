//! Import from Camera: the pictures on a connected camera or SD card, listed from the device model
//! (`bifrost_ui::camera_device`), and copied into the Photos folder when the user picks some and taps
//! Import. The Camera app owns the volume browser and runs the copy on a thread; this view holds the
//! selection, the page and the status, and asks for the copy through `take_request`.

use super::devices;
use super::super::cache::TextCache;
use super::super::ui::{home_zone, Ui, Weight, CARD_BG, CONTENT_BG, LABEL, SECONDARY, TINT, accent_now, WHITE};
use super::super::Event;
use super::RADIUS;
use crate::gfx::Surface;
use bifrost_ui::camera_device::{CameraContents, EntryKind, Media};
use bifrost_ui::device_browser::DeviceEvent;
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::PathBuf;

/// What a touch asks the Camera app to do with the view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportAction {
    Stay,
    /// Back was tapped: the Camera app closes the view.
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Back,
    Source,
    Import,
    Prev,
    Next,
    /// a picture row, by its position in the listing (not on the page)
    Row(usize),
}

pub struct ImportView {
    w: i32,
    h: i32,
    /// id of the volume being read, when one is attached
    source: Option<String>,
    /// every attached volume id, in id order; `Source` cycles through them
    volumes: Vec<String>,
    contents: CameraContents,
    /// positions in the listing: the pictures of the source, in listing order
    files: Vec<usize>,
    picked: BTreeSet<usize>,
    page: usize,
    status: String,
    busy: bool,
    request: Option<Vec<PathBuf>>,
    pressed: Option<Target>,
    cache: TextCache,
}

/// Height of the header with Back, the title and Import.
fn header_h(ui: &Ui) -> f32 {
    ui.pt(56.0)
}

/// Height of one picture row.
fn row_h(ui: &Ui) -> f32 {
    ui.pt(44.0)
}

/// Height of the footer with Previous, the page line and Next.
fn footer_h(ui: &Ui) -> f32 {
    ui.pt(60.0)
}

/// Top of the picture list: below the header and the source button.
fn list_top(ui: &Ui) -> f32 {
    header_h(ui) + ui.pt(52.0)
}

/// How many pages a listing of `total` items takes at `per_page` items a page. Never less than one page.
pub fn page_count(total: usize, per_page: usize) -> usize {
    let per = per_page.max(1);
    total.div_ceil(per).max(1)
}

/// The listing positions shown on page `page` (0 based), clamped to the listing.
pub fn page_range(total: usize, per_page: usize, page: usize) -> Range<usize> {
    let per = per_page.max(1);
    let start = (page * per).min(total);
    start..(start + per).min(total)
}

/// A file size for a row: whole kilobytes below a megabyte, then one decimal in megabytes.
pub fn size_text(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

impl ImportView {
    /// An empty view for a `w` by `h` window. Volumes arrive through `on_event`.
    pub fn new(w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            source: None,
            volumes: Vec::new(),
            contents: CameraContents::new(),
            files: Vec::new(),
            picked: BTreeSet::new(),
            page: 0,
            status: String::new(),
            busy: false,
            request: None,
            pressed: None,
            cache: TextCache::default(),
        }
    }

    /// Feeds one volume change from the device browser. The first volume that appears is read.
    pub fn on_event(&mut self, event: &DeviceEvent) {
        match event {
            DeviceEvent::Added(d) => {
                if !self.volumes.contains(&d.id) {
                    self.volumes.push(d.id.clone());
                    self.volumes.sort();
                }
                if self.source.is_none() {
                    self.select(d.id.clone());
                }
            }
            DeviceEvent::Removed(d) => {
                self.volumes.retain(|id| *id != d.id);
                if self.source.as_deref() == Some(d.id.as_str()) {
                    self.source = None;
                    self.contents = CameraContents::new();
                    self.files.clear();
                    self.picked.clear();
                    self.page = 0;
                    self.status = "The card was removed".into();
                    if let Some(next) = self.volumes.first().cloned() {
                        self.select(next);
                    }
                }
            }
        }
    }

    /// Reads the volume `id`: its pictures are listed and the selection is cleared.
    fn select(&mut self, id: String) {
        self.contents = devices::volume_path(&id).map(|p| devices::contents_of(&p)).unwrap_or_default();
        self.files = self.contents.files_of(Media::Image);
        self.picked.clear();
        self.page = 0;
        self.status = match self.files.len() {
            0 => "No JPEG or PNG pictures on this one".into(),
            1 => "1 picture".into(),
            n => format!("{n} pictures"),
        };
        self.source = Some(id);
    }

    /// Volume ids the view knows, in order.
    pub fn volume_count(&self) -> usize {
        self.volumes.len()
    }

    /// The copy the Camera app should run: the paths of the picked pictures. Taken once per request.
    pub fn take_request(&mut self) -> Option<Vec<PathBuf>> {
        self.request.take()
    }

    /// True while a copy is running; the Camera app keeps the frame loop awake for it.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The copy finished with `copied` pictures in Photos.
    pub fn on_copied(&mut self, copied: usize) {
        self.busy = false;
        self.picked.clear();
        self.status = match copied {
            0 => "Nothing was imported".into(),
            1 => "Imported 1 picture to Photos".into(),
            n => format!("Imported {n} pictures to Photos"),
        };
    }

    fn per_page(&self, ui: &Ui) -> usize {
        let avail = self.h as f32 - home_zone(ui) - footer_h(ui) - list_top(ui);
        ((avail / row_h(ui)).floor().max(1.0)) as usize
    }

    fn pages(&self, ui: &Ui) -> usize {
        page_count(self.files.len(), self.per_page(ui))
    }

    fn back_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(12.0), ui.pt(10.0), ui.pt(90.0), ui.pt(36.0))
    }

    fn import_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (self.w as f32 - ui.pt(12.0) - ui.pt(120.0), ui.pt(10.0), ui.pt(120.0), ui.pt(36.0))
    }

    fn source_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(16.0), header_h(ui) + ui.pt(8.0), self.w as f32 - ui.pt(32.0), ui.pt(36.0))
    }

    fn row_rect(&self, ui: &Ui, slot: usize) -> (f32, f32, f32, f32) {
        (ui.pt(16.0), list_top(ui) + slot as f32 * row_h(ui), self.w as f32 - ui.pt(32.0), row_h(ui) - ui.pt(4.0))
    }

    fn footer_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - footer_h(ui)
    }

    fn prev_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(16.0), self.footer_top(ui) + ui.pt(12.0), ui.pt(96.0), ui.pt(36.0))
    }

    fn next_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (self.w as f32 - ui.pt(16.0) - ui.pt(96.0), self.footer_top(ui) + ui.pt(12.0), ui.pt(96.0), ui.pt(36.0))
    }

    fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= x && px < x + w && py >= y && py < y + h
    }

    fn import_enabled(&self) -> bool {
        !self.picked.is_empty() && !self.busy
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if Self::inside(self.back_rect(ui), x, y) {
            return Some(Target::Back);
        }
        if self.import_enabled() && Self::inside(self.import_rect(ui), x, y) {
            return Some(Target::Import);
        }
        if self.volumes.len() > 1 && Self::inside(self.source_rect(ui), x, y) {
            return Some(Target::Source);
        }
        let per = self.per_page(ui);
        let page = self.page.min(self.pages(ui) - 1);
        let shown = page_range(self.files.len(), per, page);
        for pos in shown.clone() {
            if Self::inside(self.row_rect(ui, pos - shown.start), x, y) {
                return Some(Target::Row(pos));
            }
        }
        if page > 0 && Self::inside(self.prev_rect(ui), x, y) {
            return Some(Target::Prev);
        }
        if page + 1 < self.pages(ui) && Self::inside(self.next_rect(ui), x, y) {
            return Some(Target::Next);
        }
        None
    }

    /// Picks or un-picks the listing position `pos`.
    fn toggle(&mut self, pos: usize) {
        if !self.picked.remove(&pos) {
            self.picked.insert(pos);
        }
    }

    /// Asks for the copy of every picked picture, with each path joined onto the volume's mount point.
    fn request_import(&mut self) {
        let Some(root) = self.source.as_deref().and_then(devices::volume_path) else { return };
        let paths: Vec<PathBuf> = self.picked.iter().filter_map(|&pos| self.contents.path(self.files[pos])).map(|p| root.join(p)).collect();
        if paths.is_empty() {
            return;
        }
        self.busy = true;
        self.status = format!("Importing {} to Photos", if paths.len() == 1 { "1 picture".to_string() } else { format!("{} pictures", paths.len()) });
        self.request = Some(paths);
    }

    /// Tapping Source moves to the next attached volume.
    fn next_source(&mut self) {
        if self.volumes.is_empty() {
            return;
        }
        let at = self.source.as_ref().and_then(|s| self.volumes.iter().position(|v| v == s)).unwrap_or(0);
        let next = self.volumes[(at + 1) % self.volumes.len()].clone();
        self.select(next);
    }

    pub fn touch(&mut self, ui: &Ui, event: Event) -> ImportAction {
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                ImportAction::Stay
            }
            Event::Move(x, y) => {
                if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                }
                ImportAction::Stay
            }
            Event::Up(x, y) => {
                let pressed = self.pressed.take();
                if pressed.is_none() || self.hit(ui, x, y) != pressed {
                    return ImportAction::Stay;
                }
                let page = self.page.min(self.pages(ui) - 1);
                match pressed.unwrap() {
                    Target::Back => return ImportAction::Close,
                    Target::Import => self.request_import(),
                    Target::Source => self.next_source(),
                    Target::Prev => self.page = page.saturating_sub(1),
                    Target::Next => self.page = page + 1,
                    Target::Row(pos) => self.toggle(pos),
                }
                ImportAction::Stay
            }
            _ => ImportAction::Stay,
        }
    }

    /// Draws a text in a rectangle, centred in it or at its left edge.
    fn put(s: &mut Surface, ui: &Ui, cache: &mut TextCache, text: &str, pt: f32, weight: Weight, color: u32, rect: (f32, f32, f32, f32), centred: bool) {
        let image = cache.get(ui, text, pt, weight, color);
        let x = if centred { rect.0 + (rect.2 - image.w as f32) / 2.0 } else { rect.0 };
        let y = rect.1 + (rect.3 - image.h as f32) / 2.0;
        s.blit(image, x as i32, y as i32, 255, 255);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        // the text cache is taken out for the frame so the layout helpers can borrow `self` freely
        let mut owned = std::mem::take(&mut self.cache);
        self.draw_with(s, ui, &mut owned);
        self.cache = owned;
    }

    fn draw_with(&self, s: &mut Surface, ui: &Ui, cache: &mut TextCache) {
        let bg = CONTENT_BG.now();
        s.fill_rect(0, 0, self.w as usize, self.h as usize, bg);

        // header: Back, the title and Import
        let back = self.back_rect(ui);
        let pressed_back = self.pressed == Some(Target::Back);
        Self::put(s, ui, cache, "Back", 17.0, Weight::Regular, if pressed_back { SECONDARY } else { TINT }, back, false);
        let title = (self.w as f32 / 4.0, ui.pt(10.0), self.w as f32 / 2.0, ui.pt(36.0));
        Self::put(s, ui, cache, "Import from Camera", 17.0, Weight::Semibold, LABEL.now(), title, true);
        let imp = self.import_rect(ui);
        let (fill, text) = if self.import_enabled() { (accent_now(), WHITE) } else { (CARD_BG.now(), SECONDARY) };
        let fill = if self.pressed == Some(Target::Import) { SECONDARY } else { fill };
        s.fill_rrect(imp.0, imp.1, imp.2, imp.3, RADIUS, fill);
        Self::put(s, ui, cache, "Import", 15.0, Weight::Semibold, text, imp, true);

        // the source button
        let src = self.source_rect(ui);
        s.fill_rrect(src.0, src.1, src.2, src.3, RADIUS, CARD_BG.now());
        let label = match &self.source {
            Some(id) if self.volumes.len() > 1 => {
                let at = self.volumes.iter().position(|v| v == id).unwrap_or(0);
                format!("{} ({} of {}) - tap for the next card", devices::volume_label(id), at + 1, self.volumes.len())
            }
            Some(id) => devices::volume_label(id),
            None => "No camera or SD card connected".to_string(),
        };
        let shown = ui.fit(&label, 15.0, src.2 - ui.pt(24.0));
        Self::put(s, ui, cache, &shown, 15.0, Weight::Regular, if self.source.is_some() { LABEL.now() } else { SECONDARY }, (src.0 + ui.pt(12.0), src.1, src.2 - ui.pt(24.0), src.3), false);

        if self.source.is_none() {
            let r = (ui.pt(16.0), list_top(ui), self.w as f32 - ui.pt(32.0), ui.pt(60.0));
            Self::put(s, ui, cache, "Connect a camera or an SD card with pictures on it", 15.0, Weight::Regular, SECONDARY, r, true);
        } else if self.files.is_empty() {
            let r = (ui.pt(16.0), list_top(ui), self.w as f32 - ui.pt(32.0), ui.pt(60.0));
            Self::put(s, ui, cache, "No JPEG or PNG pictures in the DCIM folder", 15.0, Weight::Regular, SECONDARY, r, true);
        }

        // the rows of the current page
        let per = self.per_page(ui);
        let page = self.page.min(self.pages(ui) - 1);
        for pos in page_range(self.files.len(), per, page) {
            let slot = pos - page * per;
            let r = self.row_rect(ui, slot);
            let picked = self.picked.contains(&pos);
            let pressed = self.pressed == Some(Target::Row(pos));
            let fill = match (picked, pressed) {
                (true, _) => accent_now(),
                (false, true) => SECONDARY,
                _ => CARD_BG.now(),
            };
            s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, fill);
            let ink = if picked { WHITE } else { LABEL.now() };
            let entry = self.contents.entry(self.files[pos]);
            let name = entry.map(|e| e.name.clone()).unwrap_or_default();
            let size = match entry.map(|e| e.kind) {
                Some(EntryKind::File { size, .. }) => size_text(size),
                _ => String::new(),
            };
            let name_w = r.2 - ui.pt(150.0);
            let shown = ui.fit(&name, 15.0, name_w);
            Self::put(s, ui, cache, &shown, 15.0, Weight::Regular, ink, (r.0 + ui.pt(14.0), r.1, name_w, r.3), false);
            let size_rect = (r.0 + r.2 - ui.pt(120.0), r.1, ui.pt(106.0), r.3);
            Self::put(s, ui, cache, &size, 13.0, Weight::Regular, if picked { WHITE } else { SECONDARY }, size_rect, false);
        }

        // footer: Previous, the status or page line, Next
        let total = self.pages(ui);
        if page > 0 {
            let r = self.prev_rect(ui);
            s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, if self.pressed == Some(Target::Prev) { SECONDARY } else { CARD_BG.now() });
            Self::put(s, ui, cache, "Previous", 15.0, Weight::Regular, TINT, r, true);
        }
        if page + 1 < total {
            let r = self.next_rect(ui);
            s.fill_rrect(r.0, r.1, r.2, r.3, RADIUS, if self.pressed == Some(Target::Next) { SECONDARY } else { CARD_BG.now() });
            Self::put(s, ui, cache, "Next", 15.0, Weight::Regular, TINT, r, true);
        }
        let line = if self.status.is_empty() { format!("Page {} of {}", page + 1, total) } else { self.status.clone() };
        let shown = ui.fit(&line, 14.0, self.w as f32 - ui.pt(440.0));
        let mid = (ui.pt(120.0), self.footer_top(ui) + ui.pt(12.0), self.w as f32 - ui.pt(240.0), ui.pt(36.0));
        Self::put(s, ui, cache, &shown, 14.0, Weight::Regular, SECONDARY, mid, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::device_browser::{Device, DeviceKind, Transport};

    fn volume(id: &str) -> DeviceEvent {
        DeviceEvent::Added(Device { id: id.into(), name: "SD".into(), kind: DeviceKind::Camera, transport: Transport::Other, serial: None })
    }

    #[test]
    fn pages_split_the_listing() {
        assert_eq!(page_count(0, 8), 1);
        assert_eq!(page_count(8, 8), 1);
        assert_eq!(page_count(9, 8), 2);
        assert_eq!(page_range(10, 4, 0), 0..4);
        assert_eq!(page_range(10, 4, 2), 8..10);
        assert_eq!(page_range(10, 4, 7), 10..10);
    }

    #[test]
    fn sizes_read_in_kilobytes_then_megabytes() {
        assert_eq!(size_text(0), "0 KB");
        assert_eq!(size_text(512 * 1024), "512 KB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn the_first_volume_is_read_and_removal_moves_to_the_next() {
        let mut view = ImportView::new(1200, 800);
        assert_eq!(view.volume_count(), 0);
        view.on_event(&volume("volume:/no/such/one"));
        assert_eq!(view.source.as_deref(), Some("volume:/no/such/one"));
        // a volume with no DCIM folder lists nothing
        assert_eq!(view.status, "No JPEG or PNG pictures on this one");
        view.on_event(&volume("volume:/no/such/two"));
        assert_eq!(view.volume_count(), 2);
        assert_eq!(view.source.as_deref(), Some("volume:/no/such/one"));
        view.on_event(&DeviceEvent::Removed(Device { id: "volume:/no/such/one".into(), name: "SD".into(), kind: DeviceKind::Camera, transport: Transport::Other, serial: None }));
        assert_eq!(view.source.as_deref(), Some("volume:/no/such/two"));
        assert_eq!(view.volume_count(), 1);
    }

    #[test]
    fn picking_rows_builds_a_copy_request_under_the_mount_point() {
        let root = std::env::temp_dir().join(format!("bifrost-camera-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shots = root.join("DCIM/100CANON");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join("IMG_0001.JPG"), b"one").unwrap();
        std::fs::write(shots.join("IMG_0002.JPG"), b"two").unwrap();
        let mut view = ImportView::new(1200, 800);
        view.on_event(&DeviceEvent::Added(Device { id: format!("volume:{}", root.display()), name: "x".into(), kind: DeviceKind::Camera, transport: Transport::Other, serial: None }));
        assert_eq!(view.files.len(), 2);
        assert!(!view.import_enabled(), "nothing picked yet");
        view.toggle(1);
        assert!(view.import_enabled());
        view.request_import();
        assert!(view.busy());
        let paths = view.take_request().unwrap();
        assert_eq!(paths, [root.join("DCIM/100CANON/IMG_0002.JPG")]);
        assert!(view.take_request().is_none(), "a request is taken once");
        view.on_copied(1);
        assert!(!view.busy());
        assert_eq!(view.status, "Imported 1 picture to Photos");
        let _ = std::fs::remove_dir_all(&root);
    }
}
