//! Reading list: web pages saved to read later. The model lives in `bifrost_ui::reading_list`; this module re-exports it
//! for `bifrost_app`. Saved items are kept by the caller's own store; there is no shared list between apps yet.

pub use bifrost_ui::reading_list::{Error, Item, ReadingList};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_page_is_listed_and_can_be_removed() {
        let mut list = ReadingList::new(10);
        list.add("https://example.com/article", "Article", "A preview", 100).unwrap();
        assert!(list.contains("https://example.com/article"));
        assert!(list.remove("https://example.com/article"));
        assert!(list.is_empty());
    }

    #[test]
    fn a_file_address_cannot_be_saved() {
        let mut list = ReadingList::new(10);
        assert_eq!(list.add("file:///etc/hosts", "", "", 1), Err(Error::UnsupportedUrl));
    }
}
