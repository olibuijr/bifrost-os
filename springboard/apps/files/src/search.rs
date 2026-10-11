//! Search: the folder-tree query running on its own thread and streaming its hits.
use crate::*;

/// A search of the folder tree under the open folder, streaming its hits.
pub(crate) struct Search {
    pub(crate) rx: Receiver<(Vec<Entry>, bool)>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) query: String,
}

impl Files {
    pub(crate) fn stop_search(&mut self) {
        if let Some(search) = self.search.take() {
            search.cancel.store(true, Ordering::Relaxed);
        }
        self.hits = None;
    }
    /// The typed query changed: search the folder tree for it, or show the folder again.
    pub(crate) fn query_changed(&mut self) {
        self.stop_search();
        self.scroll = 0.;
        self.selection.clear();
        let q = self.query.trim().to_string();
        if q.is_empty() || self.in_recents() || self.in_bin() {
            self.refresh_rows();
            return;
        }
        let (tx, rx) = feed();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let root = self.path.clone();
        let search_for = q.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let mut file_manager = FileManager::new(&root, &search_for, TypeFilter::All);
            let mut out = vec![];
            while !file_manager.done() && !flag.load(Ordering::Relaxed) {
                out.extend(file_manager.step(16));
                let _ = tx.send((out.clone(), false));
            }
            let _ = tx.send((out, true));
        });
        self.search = Some(Search { rx, cancel, query: q });
        self.revision += 1;
    }
    /// Takes the newest hits from the search thread. Returns true when the screen changed.
    pub(crate) fn poll_search(&mut self) -> bool {
        let Some(search) = self.search.as_ref() else { return false };
        let mut latest = None;
        let mut done = false;
        while let Ok((hits, finished)) = search.rx.try_recv() {
            latest = Some(hits);
            done = finished;
            if finished {
                break;
            }
        }
        let Some(mut hits) = latest else { return false };
        let for_query = search.query.clone();
        if for_query != self.query.trim() {
            return false;
        }
        self.order(&mut hits);
        self.hits = Some(hits);
        self.refresh_rows();
        if done {
            self.search = None;
        }
        self.revision += 1;
        true
    }
}
