//! Bookmark import from an HTML bookmark export written by a browser. The parser lives in `bifrost_ui::bookmark_import`;
//! this module re-exports it for `bifrost_app`. The caller reads the file and writes the result into its own store.

pub use bifrost_ui::bookmark_import::{bookmarks, parse, Bookmark, Node};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_keeps_its_title_address_and_folder() {
        let html = "<DL><DT><H3>Work</H3><DL><DT><A HREF=\"https://a.example/\">Alpha</A></DL></DL>";
        let found = bookmarks(&parse(html));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://a.example/");
        assert_eq!(found[0].title, "Alpha");
        assert_eq!(found[0].folders, ["Work"]);
    }
}
