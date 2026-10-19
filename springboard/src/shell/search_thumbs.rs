//! Thumbnails for the file rows of Spotlight: a file found by name shows a picture of its content in place of the
//! Files icon, drawn by the system thumbnail providers (`crate::thumbs`) and shared with Files and the preview panel
//! through the same disk cache.
//!
//! Drawing happens on worker threads (providers are not `Send`, so each worker builds its own generator over the
//! shared cache directory); the search card asks with [`FileThumbs::want`] while it lays the rows out, reads what is
//! ready with [`FileThumbs::ready`] while it draws, and calls [`FileThumbs::poll`] on every tick to take finished
//! pictures. A file whose type has no picture (or a folder) keeps its icon.

use crate::gfx::Image;
use crate::shell::photos::thumbs::DiskCache;
use crate::thumbs::{Answer, Generator, MAX_BYTES, MAX_FILES};
use bifrost_ui::thumb_service::{ThumbKind, ThumbRequest};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

/// Pictures drawn at the same time.
pub const WORKERS: usize = 2;
/// Files kept waiting for a worker; later asks are dropped (the rows that scrolled away are not worth drawing).
pub const MAX_WAITING: usize = 64;
/// Pictures kept in memory before the oldest answers are forgotten.
pub const MAX_KEPT: usize = 96;

type Key = (PathBuf, usize);

struct Done {
    key: Key,
    picture: Option<Image>,
}

pub struct FileThumbs {
    cache: PathBuf,
    kept: HashMap<Key, Option<Image>>,
    /// keys asked for and not yet answered (running or waiting)
    asked: HashSet<Key>,
    waiting: VecDeque<Key>,
    running: usize,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl Default for FileThumbs {
    fn default() -> Self {
        FileThumbs::with_cache(crate::thumbs::cache_dir())
    }
}

/// Draws the picture of `path` in a square of `px` pixels; None when the file has only an icon or cannot be read.
fn draw(cache: &Path, path: &Path, px: usize) -> Option<Image> {
    let generator = Generator::new(DiskCache::new(cache.to_path_buf(), MAX_FILES).with_max_bytes(MAX_BYTES));
    let req = ThumbRequest::new(path.to_str()?, px as u32, 1.0, ThumbKind::Thumbnail).ok()?;
    match crate::shell::services::thumbnails::contained(|| generator.generate(&req)) {
        Ok(Answer::Pixels { path, .. }) => Image::from_raw(&std::fs::read(path).ok()?),
        _ => None,
    }
}

impl FileThumbs {
    /// A tracker over the thumbnail cache in `cache`.
    pub fn with_cache(cache: PathBuf) -> Self {
        let (tx, rx) = channel();
        FileThumbs { cache, kept: HashMap::new(), asked: HashSet::new(), waiting: VecDeque::new(), running: 0, tx, rx }
    }

    /// Asks for the picture of `path` at `px` pixels, once. Relative paths and sizes of zero are ignored.
    pub fn want(&mut self, path: &Path, px: usize) {
        let key = (path.to_path_buf(), px);
        if px == 0 || !path.is_absolute() || self.kept.contains_key(&key) || self.asked.contains(&key) || self.waiting.len() >= MAX_WAITING {
            return;
        }
        self.asked.insert(key.clone());
        self.waiting.push_back(key);
        self.start();
    }

    /// The picture, when it has been drawn.
    pub fn ready(&self, path: &Path, px: usize) -> Option<&Image> {
        self.kept.get(&(path.to_path_buf(), px)).and_then(Option::as_ref)
    }

    /// True while a picture is being drawn or waits for a worker (the card keeps ticking).
    pub fn pending(&self) -> bool {
        !self.asked.is_empty()
    }

    /// Takes the finished pictures and starts the next waiting ones. True when a picture arrived, so the card redraws.
    pub fn poll(&mut self) -> bool {
        let mut arrived = false;
        while let Ok(done) = self.rx.try_recv() {
            self.running = self.running.saturating_sub(1);
            self.asked.remove(&done.key);
            arrived |= done.picture.is_some();
            if self.kept.len() >= MAX_KEPT {
                self.kept.clear();
            }
            self.kept.insert(done.key, done.picture);
        }
        self.start();
        arrived
    }

    fn start(&mut self) {
        while self.running < WORKERS {
            let Some((path, px)) = self.waiting.pop_front() else { break };
            self.running += 1;
            let (tx, cache) = (self.tx.clone(), self.cache.clone());
            std::thread::spawn(move || {
                let picture = draw(&cache, &path, px);
                let _ = tx.send(Done { key: (path, px), picture });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-searchthumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png(path: &Path) {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 40, 20);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&vec![90u8; 40 * 20 * 4]).unwrap();
        }
        std::fs::write(path, out).unwrap();
    }

    fn settle(t: &mut FileThumbs) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        let mut arrived = false;
        while t.pending() && Instant::now() < end {
            arrived |= t.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        arrived
    }

    #[test]
    fn a_picture_is_drawn_once_and_then_ready() {
        let dir = scratch("draw");
        let file = dir.join("a.png");
        png(&file);
        let mut t = FileThumbs::with_cache(dir.join("cache"));
        assert!(t.ready(&file, 32).is_none(), "nothing before the first ask");
        t.want(&file, 32);
        t.want(&file, 32);
        assert!(t.pending());
        assert!(settle(&mut t), "the finished picture is reported");
        let im = t.ready(&file, 32).expect("the picture is ready");
        assert_eq!((im.w, im.h), (32, 16), "40x20 fitted into 32 pixels, aspect kept");
        assert!(!t.pending());
        t.want(&file, 32);
        assert!(!t.pending(), "an answered ask is not repeated");
    }

    #[test]
    fn a_folder_a_missing_file_and_a_relative_path_keep_their_icons() {
        let dir = scratch("none");
        let mut t = FileThumbs::with_cache(dir.join("cache"));
        t.want(&dir, 32);
        t.want(&dir.join("gone.png"), 32);
        t.want(Path::new("relative.png"), 32);
        t.want(&dir.join("zero.png"), 0);
        settle(&mut t);
        assert!(t.ready(&dir, 32).is_none());
        assert!(t.ready(&dir.join("gone.png"), 32).is_none());
        assert!(!t.pending());
    }

    #[test]
    fn a_file_that_is_not_what_its_name_says_keeps_its_icon_and_does_not_stop_the_others() {
        let dir = scratch("broken");
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a picture").unwrap();
        let good = dir.join("good.png");
        png(&good);
        let mut t = FileThumbs::with_cache(dir.join("cache"));
        t.want(&bad, 32);
        t.want(&good, 32);
        settle(&mut t);
        assert!(t.ready(&bad, 32).is_none());
        assert!(t.ready(&good, 32).is_some());
    }

    #[test]
    fn the_waiting_list_is_bounded() {
        let dir = scratch("bound");
        let mut t = FileThumbs::with_cache(dir.join("cache"));
        for i in 0..(MAX_WAITING + WORKERS + 20) {
            t.want(&dir.join(format!("f{i}.png")), 32);
        }
        assert!(t.asked.len() <= MAX_WAITING + WORKERS);
        settle(&mut t);
    }
}
