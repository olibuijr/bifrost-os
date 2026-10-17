//! Media library queries for apps: run a [`Query`] over the records the app supplies and get the matching
//! positions and the sections (one per grouping key). The app keeps its own library index; this module never reads
//! the system library. The model is `bifrost_ui::media_query`.

pub use bifrost_ui::media_query::{Comparison, Grouping, LibraryItem, Predicate, Property, Query, Section, Value};

/// The result of one query: the matching positions in the library slice, in query order, and the sections over them.
#[derive(Clone, Debug, PartialEq)]
pub struct Results {
    pub items: Vec<usize>,
    pub sections: Vec<Section>,
}

impl Results {
    /// The records of the result, in query order, borrowed from the library slice they came from.
    pub fn records<'a>(&self, library: &'a [LibraryItem]) -> Vec<&'a LibraryItem> {
        self.items.iter().filter_map(|&i| library.get(i)).collect()
    }
}

/// Runs a query over the library. The result is empty for an empty library.
pub fn run(library: &[LibraryItem], query: &Query) -> Results {
    Results { items: query.items(library), sections: query.sections(library) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artist: &str, album: &str, track: u32) -> LibraryItem {
        let mut item = LibraryItem::new(title);
        item.artist = artist.to_string();
        item.album = album.to_string();
        item.track_number = track;
        item
    }

    fn library() -> Vec<LibraryItem> {
        vec![
            song("Zeta", "Ann", "First", 2),
            song("Alpha", "Ann", "First", 1),
            song("Beta", "Bo", "Second", 1),
        ]
    }

    #[test]
    fn an_empty_query_returns_the_whole_library_ordered_by_title() {
        let lib = library();
        let res = run(&lib, &Query::new());
        let titles: Vec<&str> = res.records(&lib).iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Beta", "Zeta"]);
        assert_eq!(res.sections.len(), 1, "without a grouping the result is one section");
    }

    #[test]
    fn a_filter_keeps_only_the_matching_records() {
        let lib = library();
        let mut q = Query::new();
        assert!(q.add_filter(Predicate::equal_to(Property::Artist, Value::Text("ann".into()))));
        let res = run(&lib, &q);
        assert_eq!(res.items.len(), 2, "text comparison ignores case");
        assert!(res.records(&lib).iter().all(|i| i.artist == "Ann"));
    }

    #[test]
    fn an_album_grouping_gives_one_section_per_album() {
        let lib = library();
        let res = run(&lib, &Query::albums());
        let titles: Vec<&str> = res.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["First", "Second"]);
        assert_eq!(res.sections[0].range, 0..2);
        assert_eq!(res.sections[1].range, 2..3);
    }

    #[test]
    fn an_empty_library_gives_an_empty_result() {
        let res = run(&[], &Query::songs());
        assert!(res.items.is_empty());
        assert!(res.sections.is_empty());
    }
}
