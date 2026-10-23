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
            let is_dir = md.is_dir() && !fs::is_package_name(&name);
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

/// What the listing worker sends for a folder: batches of rows as the folder is read (`partial`), then the whole
/// sorted folder with the folders above it (`partial` false), which replaces the batches.
pub(crate) struct Listing {
    pub(crate) path: PathBuf,
    pub(crate) result: Result<Vec<Entry>, String>,
    /// The folders above `path`, nearest last, for the columns view. Empty in a partial batch.
    pub(crate) ancestors: Vec<(PathBuf, Vec<Entry>)>,
    /// True for a batch of rows that the final listing will replace.
    pub(crate) partial: bool,
}

/// The nearest `ANCESTORS` folders above `path`, outermost first. Inside the home folder the columns start at the
/// home folder (as Files's column view starts at the place the sidebar opened), so the home folder shows one column
/// of its own and never the system folders (`/bin`, `/dev`, ...) to its left.
pub(crate) fn ancestor_chain(path: &Path, home: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if path == home {
        return out;
    }
    let inside_home = path.starts_with(home);
    let mut parent = path.parent();
    while let Some(folder) = parent {
        if out.len() == ANCESTORS || (inside_home && !folder.starts_with(home)) {
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
    /// Selects the items at `paths` again after the rows were rebuilt (indices move when the rows do). Paths no longer
    /// listed drop out.
    pub(crate) fn reselect(&mut self, paths: &[PathBuf]) {
        self.selection = self.rows.iter().enumerate().filter(|(_, e)| paths.contains(&e.path)).map(|(i, _)| i).collect();
    }
    pub(crate) fn sort_rows(&mut self) {
        // the selection follows its items to their new places, as the reference keeps it across a sort
        let keep = self.selected_paths();
        let mut listing = std::mem::take(&mut self.listing);
        self.order(&mut listing);
        self.listing = listing;
        if let Some(mut hits) = self.hits.take() {
            self.order(&mut hits);
            self.hits = Some(hits);
        }
        self.refresh_rows();
        self.reselect(&keep);
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    #[test]
    fn the_columns_start_at_the_home_folder_inside_it() {
        let home = Path::new("/home/ana");
        assert!(ancestor_chain(home, home).is_empty(), "the home folder is the first column");
        assert_eq!(ancestor_chain(&home.join("Documents"), home), vec![home.to_path_buf()]);
        assert_eq!(ancestor_chain(&home.join("a/b"), home), vec![home.to_path_buf(), home.join("a")]);
        // deep in the home folder the nearest ANCESTORS folders show, still never above home
        let deep = home.join("a/b/c/d");
        assert_eq!(ancestor_chain(&deep, home), vec![home.join("a"), home.join("a/b"), home.join("a/b/c")]);
    }

    #[test]
    fn outside_the_home_folder_the_columns_walk_up_to_the_root() {
        let home = Path::new("/home/ana");
        assert_eq!(ancestor_chain(Path::new("/usr/share"), home), vec![PathBuf::from("/"), PathBuf::from("/usr")]);
        assert!(ancestor_chain(Path::new("/"), home).is_empty());
        // a sibling whose name starts like the home folder is not inside it
        assert_eq!(ancestor_chain(Path::new("/home/anabel/x"), home), vec![PathBuf::from("/"), PathBuf::from("/home"), PathBuf::from("/home/anabel")]);
    }
}
