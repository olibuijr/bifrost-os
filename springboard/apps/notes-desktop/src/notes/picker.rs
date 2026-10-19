//! The camera-roll picker sheet of Notes: a bottom sheet with a grid of the
//! newest photos (thumbnails decoded off the UI thread). Tapping one attaches
//! it to the open note; the note stores the usual `![image](<file>)` line.

use super::attach;
use springboard::gfx::Image;
use springboard::shell::photos::library::{self, Photo};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Most photos offered (newest first).
pub const MAX_PHOTOS: usize = 60;
/// Grid columns and gap (pt).
pub const COLS: usize = 4;
pub const GAP_PT: f32 = 2.0;

pub struct Picker {
    pub photos: Vec<PathBuf>,
    pub thumbs: HashMap<usize, Image>,
    pub scroll: f32,
    /// thumbnails decoded off the UI thread, delivered to the main queue and drained by `poll`
    inbox: Arc<Mutex<Vec<(usize, Image)>>>,
}

impl Picker {
    /// Scans the camera roll and starts decoding thumbnails of `size` px.
    pub fn open(size: usize) -> Picker {
        let dirs: Vec<PathBuf> = library::dirs();
        let mut all: Vec<Photo> = library::scan(&dirs);
        all.reverse();
        all.truncate(MAX_PHOTOS);
        let photos: Vec<PathBuf> = all.into_iter().map(|p| p.path).collect();
        let inbox: Arc<Mutex<Vec<(usize, Image)>>> = Arc::default();
        let list = photos.clone();
        let sink = inbox.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            for (i, path) in list.iter().enumerate() {
                let Ok(bytes) = std::fs::read(path) else { continue };
                if let Some(img) = attach::decode_fitted(&bytes, size) {
                    let sink = sink.clone();
                    DispatchQueue::main().async_(move || sink.lock().unwrap_or_else(|e| e.into_inner()).push((i, img)));
                }
            }
        });
        Picker { photos, thumbs: HashMap::new(), scroll: 0.0, inbox }
    }

    /// Collects finished thumbnails; true when any arrived.
    pub fn poll(&mut self) -> bool {
        let mut batch = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        let any = !batch.is_empty();
        for (i, img) in batch.drain(..) {
            self.thumbs.insert(i, img);
        }
        any
    }

    /// True while thumbnails are still arriving.
    pub fn loading(&self) -> bool {
        self.thumbs.len() < self.photos.len()
    }

    /// Cell side and total content height for a grid `w` wide.
    pub fn metrics(&self, w: f32, gap: f32) -> (f32, f32) {
        let side = (w - gap * (COLS as f32 - 1.0)) / COLS as f32;
        let rows = self.photos.len().div_ceil(COLS);
        (side, rows as f32 * (side + gap))
    }

    /// The photo under `(x, y)` (grid-local, scroll applied).
    pub fn hit(&self, x: f32, y: f32, w: f32, gap: f32) -> Option<usize> {
        let (side, _) = self.metrics(w, gap);
        let (col, row) = ((x / (side + gap)) as usize, ((y + self.scroll) / (side + gap)) as usize);
        let i = row * COLS + col;
        (x >= 0.0 && col < COLS && i < self.photos.len()).then_some(i)
    }
}

/// The indices of the photos whose file name contains `query`, ignoring case, in the order given. A blank query matches
/// every photo. This is the model behind the picker's search field; the field itself is not drawn yet.
#[cfg(test)]
pub fn matching(photos: &[PathBuf], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    photos
        .iter()
        .enumerate()
        .filter(|(_, path)| {
            needle.is_empty()
                || path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().contains(&needle))
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from(format!("/pics/{n}"))).collect()
    }

    #[test]
    fn a_blank_query_matches_every_photo_in_order() {
        let photos = paths(&["a.jpg", "b.png"]);
        assert_eq!(matching(&photos, ""), vec![0, 1]);
        assert_eq!(matching(&photos, "   "), vec![0, 1]);
    }

    #[test]
    fn a_query_matches_file_names_ignoring_case() {
        let photos = paths(&["Holiday-01.JPG", "receipt.png", "holiday-02.png"]);
        assert_eq!(matching(&photos, "HOLIDAY"), vec![0, 2]);
        assert_eq!(matching(&photos, " receipt "), vec![1]);
    }

    #[test]
    fn no_match_gives_an_empty_list() {
        assert!(matching(&paths(&["a.jpg"]), "zzz").is_empty());
        assert!(matching(&[], "a").is_empty());
    }
}
