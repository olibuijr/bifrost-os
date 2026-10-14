//! Open documents. A picture (PNG or JPEG) is one page, decoded when it opens. A PDF is opened on its own worker thread
//! (`pdfjob`), which reports the page sizes and outline at once; pages and sidebar thumbnails are drawn by the worker
//! when the window asks for them, and kept here until they are evicted. The page order, deletions and turns are kept
//! for the session and written by Export PDF. Markup lives in a sidecar file under the app's data directory, keyed by
//! the document's path.

use crate::model::{self, fit_box, Adjust, Mark};
use crate::pdfjob::{self, Inbox, Job, Key, Opened, Outline, PdfInfo, Reply, PAGE_BOX};
use bifrost_app::image_filter::{Filter, FilterChain};
use bifrost_app::image_source::ImageSource;
use bifrost_app::kit::Image;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Sender;

/// Thumbnails fit a box this many pixels wide and twice as tall.
pub const THUMB_W: usize = 132;
/// Drawn PDF pictures kept in memory; they are dropped when more than this many would be kept (thumbnails stay).
const PDF_KEPT: usize = 6;

/// Where a document's pages come from.
enum Source {
    /// A decoded picture, the only page of a PNG or JPEG.
    Picture(Rc<Image>),
    /// A PDF: its worker takes the jobs, and its replies come back through the inbox.
    Pdf { jobs: Sender<Job>, inbox: Inbox },
}

/// An open document: its pages and the markup saved for it.
pub struct Document {
    /// Shown as the window title once the SDK can set one (apps have no window title API yet).
    #[allow(dead_code)]
    pub title: String,
    pub path: PathBuf,
    pub is_pdf: bool,
    /// Each source page's size in picture pixels, before rotation.
    pub sizes: Vec<(usize, usize)>,
    /// The source page each shown page is, in shown order. A deleted page is left out; Move Page changes it.
    pub order: Vec<usize>,
    /// The quarter turns of each source page, set by the rotate actions and saved by Export PDF.
    pub turns: Vec<u8>,
    /// The mirror bits of each source page (`model::FLIP_H`, `FLIP_V`).
    pub flips: Vec<u8>,
    /// The markup, keyed by source page.
    pub marks: Vec<Mark>,
    /// The PDF outline, in reading order. Empty for a picture.
    pub outline: Vec<Outline>,
    /// The bookmarked source pages, kept beside the markup.
    pub bookmarks: Vec<usize>,
    /// How many times a picture's pixels were replaced (crop, resize, mask); Undo compares it.
    pub pic_rev: u32,
    /// The facts about a PDF file that the worker reported; None for a picture.
    pub pdf_info: Option<PdfInfo>,
    source: Source,
    /// The pictures and thumbnails drawn so far, by key.
    drawn: RefCell<HashMap<Key, Rc<Image>>>,
    /// The keys asked of the worker and not answered yet, so each is asked once.
    asked: RefCell<HashSet<Key>>,
}

impl Document {
    fn new(path: &Path, data_dir: &Path, sizes: Vec<(usize, usize)>, source: Source, outline: Vec<Outline>, is_pdf: bool) -> Document {
        let n = sizes.len();
        let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Document").to_string();
        Document {
            title,
            path: path.to_path_buf(),
            is_pdf,
            sizes,
            order: (0..n).collect(),
            turns: vec![0; n],
            flips: vec![0; n],
            marks: load_marks(data_dir, path),
            outline,
            bookmarks: load_bookmarks(data_dir, path),
            pic_rev: 0,
            pdf_info: None,
            source,
            drawn: RefCell::new(HashMap::new()),
            asked: RefCell::new(HashSet::new()),
        }
    }

    /// The number of pages shown.
    pub fn count(&self) -> usize {
        self.order.len()
    }

    /// The source page that shown page `page` is.
    pub fn source_of(&self, page: usize) -> Option<usize> {
        self.order.get(page).copied()
    }

    /// The shown page of source page `src`, or None when it was deleted.
    pub fn display_of(&self, src: usize) -> Option<usize> {
        self.order.iter().position(|&s| s == src)
    }

    /// The size of shown page `page` in picture pixels, before rotation.
    pub fn page_size(&self, page: usize) -> Option<(usize, usize)> {
        self.sizes.get(self.source_of(page)?).copied()
    }

    /// The box the thumbnail of shown page `page` fits, known without drawing it. The sidebar lays rows out from this.
    pub fn thumb_box(&self, page: usize) -> Option<(usize, usize)> {
        let (w, h) = self.page_size(page)?;
        Some(fit_box(w, h, THUMB_W, 2 * THUMB_W))
    }

    /// The quarter turns of shown page `page`.
    pub fn turn_of(&self, page: usize) -> u8 {
        self.source_of(page).and_then(|s| self.turns.get(s).copied()).unwrap_or(0)
    }

    /// The orientation of shown page `page`: its quarter turns plus the mirror bits, as `model::rot_point` takes it.
    pub fn orient_of(&self, page: usize) -> u8 {
        let flips = self.source_of(page).and_then(|s| self.flips.get(s).copied()).unwrap_or(0);
        self.turn_of(page) | (flips & (model::FLIP_H | model::FLIP_V))
    }

    /// Mirrors shown page `page` left to right (`horizontal`) or top to bottom.
    pub fn toggle_flip(&mut self, page: usize, horizontal: bool) {
        let bit = if horizontal { model::FLIP_H } else { model::FLIP_V };
        if let Some(f) = self.source_of(page).and_then(|s| self.flips.get_mut(s)) {
            *f ^= bit;
        }
    }

    /// Sets the quarter turns of shown page `page`.
    pub fn set_turn(&mut self, page: usize, quarters: u8) {
        if let Some(s) = self.source_of(page) {
            if let Some(t) = self.turns.get_mut(s) {
                *t = quarters % 4;
            }
        }
    }

    /// The shown places of the bookmarked pages, in no particular order (a deleted page has none).
    pub fn bookmark_places(&self) -> Vec<usize> {
        self.bookmarks.iter().filter_map(|&s| self.display_of(s)).collect()
    }

    /// Moves shown page `page` by `delta` places. Returns the page's new place (unchanged when it cannot move).
    pub fn move_page(&mut self, page: usize, delta: i32) -> usize {
        let target = page as i64 + delta as i64;
        if page >= self.order.len() || target < 0 || target as usize >= self.order.len() {
            return page;
        }
        self.order.swap(page, target as usize);
        target as usize
    }

    /// Deletes shown page `page`. The last page cannot be deleted. Returns whether a page was deleted.
    pub fn delete_page(&mut self, page: usize) -> bool {
        if self.order.len() <= 1 || page >= self.order.len() {
            return false;
        }
        self.order.remove(page);
        true
    }

    /// The thumbnail of shown page `page`. It is asked of the worker the first time and drawn when the answer comes
    /// back; None until then.
    pub fn thumb(&self, page: usize) -> Option<Rc<Image>> {
        let src = self.source_of(page)?;
        let (w, h) = self.thumb_box(page)?;
        let key = Key { thumb: true, page: src, w, h };
        if let Some(t) = self.drawn.borrow().get(&key) {
            return Some(t.clone());
        }
        match &self.source {
            Source::Picture(img) => {
                let t = Rc::new(img.resized(w, h));
                self.drawn.borrow_mut().insert(key, t.clone());
                Some(t)
            }
            Source::Pdf { .. } => {
                self.request(key);
                None
            }
        }
    }

    /// The picture of shown page `page`, at the size the window draws it. A PDF page is drawn by the worker: None
    /// until the picture arrives.
    pub fn picture(&self, page: usize) -> Option<Rc<Image>> {
        let src = self.source_of(page)?;
        match &self.source {
            Source::Picture(img) => Some(img.clone()),
            Source::Pdf { .. } => {
                let key = Key { thumb: false, page: src, w: PAGE_BOX.0, h: PAGE_BOX.1 };
                if let Some(img) = self.drawn.borrow().get(&key) {
                    return Some(img.clone());
                }
                self.request(key);
                None
            }
        }
    }

    /// Asks the worker for the picture `key` names, once.
    fn request(&self, key: Key) {
        let Source::Pdf { jobs, .. } = &self.source else { return };
        if self.drawn.borrow().contains_key(&key) || !self.asked.borrow_mut().insert(key) {
            return;
        }
        let _ = jobs.send(pdfjob::render(key));
    }

    /// Sends a job to the PDF worker. False for a picture, or once the worker has stopped.
    pub fn send(&self, job: Job) -> bool {
        match &self.source {
            Source::Pdf { jobs, .. } => jobs.send(job).is_ok(),
            Source::Picture(_) => false,
        }
    }

    /// Takes the worker's replies. Drawn pictures and thumbnails are kept here; the rest are returned, with a flag that
    /// says whether anything arrived.
    pub fn take_replies(&self) -> (bool, Vec<Reply>) {
        let Source::Pdf { inbox, .. } = &self.source else { return (false, Vec::new()) };
        let replies = pdfjob::take(inbox);
        let arrived = !replies.is_empty();
        let mut rest = Vec::new();
        for reply in replies {
            match reply {
                Reply::Rendered(key, img) => self.store(key, img),
                other => rest.push(other),
            }
        }
        (arrived, rest)
    }

    /// Keeps a drawn picture or thumbnail. Pictures beyond `PDF_KEPT` are dropped first, so a long document stays small.
    fn store(&self, key: Key, img: Option<Image>) {
        let Some(img) = img else { return };
        let mut drawn = self.drawn.borrow_mut();
        let mut asked = self.asked.borrow_mut();
        if !key.thumb && drawn.keys().filter(|k| !k.thumb).count() >= PDF_KEPT {
            drawn.retain(|k, _| k.thumb);
            asked.retain(|k| k.thumb);
        }
        drawn.insert(key, Rc::new(img));
    }
}

impl Document {
    /// The picture's pixels now, for the undo history. No picture for a PDF.
    pub fn pic_state(&self) -> model::PicState {
        let image = match &self.source {
            Source::Picture(img) => Some(img.clone()),
            Source::Pdf { .. } => None,
        };
        model::PicState { rev: self.pic_rev, image }
    }

    /// Puts a picture back from the undo history. A state with no picture (a PDF) changes nothing.
    pub fn restore_pic(&mut self, state: &model::PicState) {
        let Some(img) = &state.image else { return };
        self.set_picture(img.clone());
        self.pic_rev = state.rev;
    }

    /// Replaces a picture's pixels with an edited copy (crop, resize, mask). A new revision is counted so the edit can
    /// be undone. False for a PDF.
    pub fn replace_picture(&mut self, img: Image) -> bool {
        if !matches!(self.source, Source::Picture(_)) || img.w == 0 || img.h == 0 {
            return false;
        }
        self.set_picture(Rc::new(img));
        self.pic_rev += 1;
        true
    }

    fn set_picture(&mut self, img: Rc<Image>) {
        self.sizes = vec![(img.w, img.h)];
        self.source = Source::Picture(img);
        self.drawn.borrow_mut().clear();
        self.asked.borrow_mut().clear();
    }
}

/// Opens the picture at `path` (PNG or JPEG). `data_dir` is the app's private storage directory.
pub fn open_picture(path: &Path, data_dir: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let source = ImageSource::new(&bytes).map_err(|e| format!("{e:?}"))?;
    let img = source.decode().map_err(|e| format!("{e:?}"))?;
    let sizes = vec![(img.w, img.h)];
    Ok(Document::new(path, data_dir, sizes, Source::Picture(Rc::new(img)), Vec::new(), false))
}

/// The document of a PDF whose worker has opened it. `jobs` and `inbox` are the worker's.
pub fn from_pdf(path: &Path, data_dir: &Path, opened: Opened, jobs: Sender<Job>, inbox: Inbox) -> Document {
    let mut doc = Document::new(path, data_dir, opened.sizes, Source::Pdf { jobs, inbox }, opened.outline, true);
    doc.pdf_info = Some(opened.info);
    doc
}

/// The file that remembers the page `path` was left on.
fn page_path(data_dir: &Path, path: &Path) -> PathBuf {
    sidecar_path(data_dir, path).with_extension("page")
}

/// The shown page `path` was last left on, or None when it was never saved.
pub fn load_last_page(data_dir: &Path, path: &Path) -> Option<usize> {
    std::fs::read_to_string(page_path(data_dir, path)).ok()?.trim().parse().ok()
}

/// Remembers the shown page `path` is on.
pub fn save_last_page(data_dir: &Path, path: &Path, page: usize) -> std::io::Result<()> {
    let file = page_path(data_dir, path);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, page.to_string())
}

/// The sidecar file for `path` under `data_dir`.
pub fn sidecar_path(data_dir: &Path, path: &Path) -> PathBuf {
    data_dir.join("markup").join(model::sidecar_name(&path.to_string_lossy()))
}

/// The markup saved for `path`. A missing or unreadable file gives no marks.
pub fn load_marks(data_dir: &Path, path: &Path) -> Vec<Mark> {
    std::fs::read_to_string(sidecar_path(data_dir, path)).map(|t| model::parse_sidecar(&t)).unwrap_or_default()
}

/// Saves the markup for `path`.
pub fn save_marks(data_dir: &Path, path: &Path, marks: &[Mark]) -> std::io::Result<()> {
    let file = sidecar_path(data_dir, path);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, model::to_sidecar(marks))
}

/// The bookmarks file for `path` under `data_dir`.
fn bookmarks_path(data_dir: &Path, path: &Path) -> PathBuf {
    sidecar_path(data_dir, path).with_extension("bookmarks")
}

/// The bookmarked source pages saved for `path`. A missing or unreadable file gives none.
pub fn load_bookmarks(data_dir: &Path, path: &Path) -> Vec<usize> {
    std::fs::read_to_string(bookmarks_path(data_dir, path)).map(|t| model::parse_bookmarks(&t)).unwrap_or_default()
}

/// Saves the bookmarks for `path`. With none left the file is removed.
pub fn save_bookmarks(data_dir: &Path, path: &Path, pages: &[usize]) -> std::io::Result<()> {
    let file = bookmarks_path(data_dir, path);
    if pages.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, model::to_bookmarks(pages))
}

/// `img` with the colour adjustments applied. Returns a copy unchanged when nothing is set.
pub fn adjusted(img: &Image, adjust: &Adjust) -> Image {
    let mut chain = FilterChain::new();
    for (name, value) in adjust.settings() {
        if let Some(mut filter) = Filter::named(name) {
            if let Some(key) = filter.input_keys().first().copied() {
                filter.set(key, value);
            }
            chain.push(filter);
        }
    }
    if chain.is_empty() {
        return img.clone();
    }
    chain.apply(img).unwrap_or_else(|| img.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_doc(w: usize, h: usize) -> Document {
        let img = Image { w, h, px: vec![0; w * h] };
        Document::new(Path::new("/a/b.png"), Path::new("/nonexistent"), vec![(w, h)], Source::Picture(Rc::new(img)), Vec::new(), false)
    }

    #[test]
    fn thumbnail_boxes_follow_the_page_and_are_drawn_once() {
        let doc = picture_doc(400, 200);
        assert_eq!(doc.thumb_box(0), Some((THUMB_W, 66)));
        assert_eq!(doc.thumb_box(1), None);
        let a = doc.thumb(0).expect("a thumbnail");
        assert_eq!((a.w, a.h), (THUMB_W, 66));
        let b = doc.thumb(0).expect("a thumbnail");
        assert!(Rc::ptr_eq(&a, &b));
        assert!(doc.thumb(1).is_none());
    }

    #[test]
    fn pages_can_be_moved_and_deleted_but_not_all_removed() {
        let mut doc = picture_doc(4, 4);
        doc.sizes = vec![(4, 4), (5, 5), (6, 6)];
        doc.order = vec![0, 1, 2];
        doc.turns = vec![0, 0, 0];
        assert_eq!(doc.move_page(0, 1), 1);
        assert_eq!(doc.order, vec![1, 0, 2]);
        assert_eq!(doc.move_page(0, -1), 0);
        assert_eq!(doc.display_of(2), Some(2));
        assert!(doc.delete_page(1));
        assert_eq!(doc.order, vec![1, 2]);
        assert_eq!(doc.display_of(0), None);
        assert!(doc.delete_page(0));
        assert!(!doc.delete_page(0), "the last page stays");
    }

    #[test]
    fn turns_follow_the_source_page_not_its_place() {
        let mut doc = picture_doc(4, 4);
        doc.sizes = vec![(4, 4), (4, 4)];
        doc.order = vec![0, 1];
        doc.turns = vec![0, 0];
        doc.set_turn(0, 5);
        assert_eq!(doc.turns, vec![1, 0]);
        doc.move_page(0, 1);
        assert_eq!(doc.turn_of(1), 1);
        assert_eq!(doc.turn_of(0), 0);
    }

    #[test]
    fn bookmarks_follow_the_shown_places_of_their_pages() {
        let mut doc = picture_doc(4, 4);
        doc.sizes = vec![(4, 4), (4, 4), (4, 4)];
        doc.order = vec![2, 0, 1];
        doc.bookmarks = vec![0, 2];
        let mut places = doc.bookmark_places();
        places.sort();
        assert_eq!(places, vec![0, 1]);
        doc.delete_page(0);
        assert_eq!(doc.bookmark_places(), vec![0]);
    }

    #[test]
    fn bookmarks_are_saved_beside_the_markup_and_removed_when_empty() {
        let dir = std::env::temp_dir().join(format!("viewer-bookmarks-{}", std::process::id()));
        let path = Path::new("/a/b.pdf");
        save_bookmarks(&dir, path, &[3, 1]).expect("saved");
        assert_eq!(load_bookmarks(&dir, path), vec![3, 1]);
        save_bookmarks(&dir, path, &[]).expect("removed");
        assert!(load_bookmarks(&dir, path).is_empty());
        save_bookmarks(&dir, path, &[]).expect("removing nothing is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flips_follow_the_source_page_and_join_the_turns() {
        let mut doc = picture_doc(4, 4);
        doc.sizes = vec![(4, 4), (4, 4)];
        doc.order = vec![0, 1];
        doc.turns = vec![0, 0];
        doc.flips = vec![0, 0];
        doc.set_turn(0, 3);
        doc.toggle_flip(0, true);
        assert_eq!(doc.orient_of(0), 3 | model::FLIP_H);
        doc.toggle_flip(0, false);
        assert_eq!(doc.orient_of(0), 3 | model::FLIP_H | model::FLIP_V);
        doc.toggle_flip(0, true);
        doc.toggle_flip(0, false);
        assert_eq!(doc.orient_of(0), 3);
        doc.move_page(0, 1);
        assert_eq!(doc.orient_of(1), 3);
        assert_eq!(doc.orient_of(5), 0);
    }

    #[test]
    fn replacing_a_picture_counts_a_revision_and_can_be_put_back() {
        let mut doc = picture_doc(4, 4);
        let before = doc.pic_state();
        assert_eq!(before.rev, 0);
        assert!(doc.replace_picture(Image { w: 2, h: 3, px: vec![1; 6] }));
        assert_eq!((doc.pic_rev, doc.sizes.clone()), (1, vec![(2, 3)]));
        assert_ne!(doc.pic_state(), before);
        doc.restore_pic(&before);
        assert_eq!((doc.pic_rev, doc.sizes.clone()), (0, vec![(4, 4)]));
        assert!(!doc.replace_picture(Image { w: 0, h: 0, px: vec![] }));
    }

    #[test]
    fn the_last_page_is_remembered_per_document() {
        let dir = std::env::temp_dir().join(format!("viewer-lastpage-{}", std::process::id()));
        let path = Path::new("/a/b.pdf");
        assert_eq!(load_last_page(&dir, path), None);
        save_last_page(&dir, path, 7).expect("saved");
        assert_eq!(load_last_page(&dir, path), Some(7));
        assert_eq!(load_last_page(&dir, Path::new("/a/c.pdf")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_document_has_no_worker() {
        let doc = picture_doc(4, 4);
        assert!(!doc.send(pdfjob::run(|_| Reply::Written(Ok(PathBuf::new())))));
        let (arrived, rest) = doc.take_replies();
        assert!(!arrived && rest.is_empty());
    }
}
