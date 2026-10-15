//! The rule for books that arrive from outside the reader. A single PDF shared to Books opens at once, while books
//! chosen in File > Add to Library, or a batch of shared books, only join the shelf.

use std::path::{Path, PathBuf};

/// The book to open after a batch of imports. `added` holds each imported book with whether it arrived as an inbound
/// share. Only a lone shared book opens; anything else stays on the shelf.
pub fn book_to_open(added: &[(PathBuf, bool)]) -> Option<&Path> {
    match added {
        [(path, true)] => Some(path.as_path()),
        _ => None,
    }
}
