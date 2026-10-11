//! The shared item model: listings (`Entry`), sorting, and the tag and ancestor listings every view draws.
use crate::*;

pub(crate) const ANCESTORS: usize = 3;

/// Every item that carries a colour tag (or only `only`), as listing entries (missing files are left out).
pub(crate) fn tagged_entries(index: &TagIndex, only: Option<Tag>) -> Vec<Entry> {
    let paths: BTreeSet<PathBuf> = Tag::ALL.into_iter().filter(|t| only.is_none_or(|o| o == *t)).flat_map(|t| index.with_tag(t)).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let md = std::fs::metadata(&path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            let is_dir = md.is_dir();
            Some(Entry {
                kind: fs::kind_of(&name, is_dir),
                name,
                is_dir,
                size: if is_dir { 0 } else { md.len() },
                mtime: md.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64),
                items: None,
                path,
            })
        })
        .collect()
}

pub(crate) struct Listing {
    pub(crate) path: PathBuf,
    pub(crate) result: Result<Vec<Entry>, String>,
    /// The folders above `path`, nearest last, for the columns view.
    pub(crate) ancestors: Vec<(PathBuf, Vec<Entry>)>,
}

/// The nearest `ANCESTORS` folders above `path`, outermost first.
pub(crate) fn ancestor_chain(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut parent = path.parent();
    while let Some(folder) = parent {
        if out.len() == ANCESTORS {
            break;
        }
        out.push(folder.to_path_buf());
        parent = folder.parent();
    }
    out.reverse();
    out
}

impl Files {
    pub(crate) fn visible(&self) -> Vec<usize> {
        // While a search is running, the folder is filtered by the typed name; hits already match.
        // (an empty query skips the per-row lowercase copy: 50k rows would allocate 50k strings per call)
        let typed = self.query.to_lowercase();
        let filter = self.hits.is_none() && !typed.is_empty();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, e)| (self.show_hidden || !e.name.starts_with('.')) && (!filter || e.name.to_lowercase().contains(&typed)))
            .map(|(i, _)| i)
            .collect()
    }
    pub(crate) fn refresh_rows(&mut self) {
        let base = self.hits.clone().unwrap_or_else(|| self.listing.clone());
        self.rows = self.expand_rows(base);
    }
    pub(crate) fn sort_rows(&mut self) {
        let mut listing = std::mem::take(&mut self.listing);
        self.order(&mut listing);
        self.listing = listing;
        if let Some(mut hits) = self.hits.take() {
            self.order(&mut hits);
            self.hits = Some(hits);
        }
        self.refresh_rows();
        self.selection.clear();
    }
}
