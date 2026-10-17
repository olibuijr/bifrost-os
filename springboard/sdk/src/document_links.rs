//! Document links for apps (contentlinkingd): one shared index of which documents link to which, so an app can list the
//! backlinks of a document. The index is `bifrost_core::link_index::LinkIndex`; this module re-exports it and adds the
//! app-facing backlink query that returns owned ids.

pub use bifrost_core::link_index::{wiki_links, LinkIndex};

/// Replaces the outgoing links of `document` with the link targets found in its `text` (`[[Target]]` syntax).
pub fn index_wiki_links(index: &mut LinkIndex, document: &str, text: &str) {
    index.set_links(document, wiki_links(text));
}

/// The documents that link to `target`, in sorted order, as owned ids.
pub fn backlinks_of(index: &LinkIndex, target: &str) -> Vec<String> {
    index.backlinks(target).into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_wiki_links_give_backlinks_to_their_targets() {
        let mut index = LinkIndex::new();
        index_wiki_links(&mut index, "Plans", "See [[Notes]] and [[Ideas]].");
        index_wiki_links(&mut index, "Diary", "Today: [[Notes]]");
        assert_eq!(backlinks_of(&index, "Notes"), vec!["Diary".to_string(), "Plans".to_string()]);
        assert_eq!(backlinks_of(&index, "Ideas"), vec!["Plans".to_string()]);
    }

    #[test]
    fn reindexing_a_document_replaces_its_old_links() {
        let mut index = LinkIndex::new();
        index_wiki_links(&mut index, "Plans", "[[Notes]]");
        index_wiki_links(&mut index, "Plans", "[[Ideas]]");
        assert!(backlinks_of(&index, "Notes").is_empty());
        assert_eq!(backlinks_of(&index, "Ideas"), vec!["Plans".to_string()]);
    }

    #[test]
    fn a_document_never_lists_itself_and_an_empty_text_removes_it() {
        let mut index = LinkIndex::new();
        index_wiki_links(&mut index, "Notes", "[[Notes]] and [[Ideas]]");
        assert!(backlinks_of(&index, "Notes").is_empty());
        index_wiki_links(&mut index, "Notes", "no links now");
        assert!(backlinks_of(&index, "Ideas").is_empty());
        assert!(index.is_empty());
    }
}
