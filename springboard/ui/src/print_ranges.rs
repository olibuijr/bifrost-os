//! Page ranges typed in the print sheet ("1-3, 5, 7-"): parsing, merging and applying them to a [`PrintInfo`].
//!
//! The sheet's Pages control offers All, or a list of pages and ranges. [`parse_page_ranges`] reads the typed text against
//! the document's page count, sorts it and merges overlapping or touching ranges. A job can carry one range as the
//! `page-ranges` attribute ([`PrintInfo::pages`]); a list of several ranges cannot be sent that way yet, so
//! [`PageSelection::apply_to`] refuses it rather than printing the wrong pages.

use crate::print::PrintInfo;

/// Which pages a job prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSelection {
    /// Every page.
    All,
    /// Sorted, non-overlapping, 1-based inclusive ranges.
    Ranges(Vec<(u32, u32)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// Text that is not a number or a `first-last` pair.
    Malformed(String),
    /// A range whose last page is before its first.
    Reversed(String),
    /// A page number that is zero or past the last page.
    OutOfRange(u32),
    /// The document has no pages.
    NoPages,
    /// A list of several ranges, which the job's `page-ranges` attribute cannot carry.
    Multiple,
}

fn number(text: &str) -> Result<u32, RangeError> {
    text.trim().parse::<u32>().map_err(|_| RangeError::Malformed(text.trim().to_string()))
}

/// Parses a page list such as `1-3, 5, 7-` for a document of `total` pages. Empty text and `all` mean every page. An
/// open start (`-4`) begins at page 1 and an open end (`7-`) runs to the last page. A last page past the end is cut to
/// the last page; a first page past the end is refused.
pub fn parse_page_ranges(text: &str, total: u32) -> Result<PageSelection, RangeError> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("all") {
        return Ok(PageSelection::All);
    }
    if total == 0 {
        return Err(RangeError::NoPages);
    }
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for item in text.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(RangeError::Malformed(text.to_string()));
        }
        let (first, last) = match item.split_once('-') {
            None => {
                let n = number(item)?;
                (n, n)
            }
            Some((a, b)) => {
                let first = if a.trim().is_empty() { 1 } else { number(a)? };
                let last = if b.trim().is_empty() { total } else { number(b)? };
                (first, last)
            }
        };
        if first == 0 || first > total {
            return Err(RangeError::OutOfRange(first));
        }
        if last < first {
            return Err(RangeError::Reversed(item.to_string()));
        }
        ranges.push((first, last.min(total)));
    }
    ranges.sort();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (first, last) in ranges {
        match merged.last_mut() {
            Some(prev) if first <= prev.1 + 1 => prev.1 = prev.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    Ok(PageSelection::Ranges(merged))
}

impl PageSelection {
    /// Whether page `page` (1-based) is in the selection. `All` holds every page.
    pub fn contains(&self, page: u32) -> bool {
        match self {
            PageSelection::All => true,
            PageSelection::Ranges(r) => r.iter().any(|(f, l)| (*f..=*l).contains(&page)),
        }
    }

    /// How many pages the selection prints out of a document of `total` pages.
    pub fn count(&self, total: u32) -> u32 {
        match self {
            PageSelection::All => total,
            PageSelection::Ranges(r) => r.iter().map(|(f, l)| l - f + 1).sum(),
        }
    }

    /// Writes the selection into `info.pages`: `All` clears the range, one range sets it. A list of several ranges is
    /// refused with [`RangeError::Multiple`] and `info` is left as it was.
    pub fn apply_to(&self, info: &mut PrintInfo) -> Result<(), RangeError> {
        match self {
            PageSelection::All => info.pages = None,
            PageSelection::Ranges(r) if r.len() == 1 => info.pages = Some(r[0]),
            PageSelection::Ranges(r) if r.is_empty() => return Err(RangeError::NoPages),
            PageSelection::Ranges(_) => return Err(RangeError::Multiple),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_of_pages_and_ranges_is_sorted_and_open_ends_run_to_the_edges() {
        assert_eq!(parse_page_ranges("1-3, 5, 7-", 10), Ok(PageSelection::Ranges(vec![(1, 3), (5, 5), (7, 10)])));
        assert_eq!(parse_page_ranges("9, 2", 10), Ok(PageSelection::Ranges(vec![(2, 2), (9, 9)])));
        assert_eq!(parse_page_ranges("-4", 10), Ok(PageSelection::Ranges(vec![(1, 4)])));
    }

    #[test]
    fn empty_text_and_all_mean_every_page() {
        assert_eq!(parse_page_ranges("", 10), Ok(PageSelection::All));
        assert_eq!(parse_page_ranges("  All ", 10), Ok(PageSelection::All));
        assert_eq!(parse_page_ranges("all", 0), Ok(PageSelection::All), "an empty document still means all");
    }

    #[test]
    fn overlapping_and_touching_ranges_merge() {
        assert_eq!(parse_page_ranges("1-3, 2-5, 6", 10), Ok(PageSelection::Ranges(vec![(1, 6)])));
        assert_eq!(parse_page_ranges("4, 1-2", 10), Ok(PageSelection::Ranges(vec![(1, 2), (4, 4)])), "a gap at 3 keeps them apart");
    }

    #[test]
    fn a_last_page_past_the_end_is_cut_and_a_first_page_past_it_is_refused() {
        assert_eq!(parse_page_ranges("2-99", 10), Ok(PageSelection::Ranges(vec![(2, 10)])));
        assert_eq!(parse_page_ranges("11", 10), Err(RangeError::OutOfRange(11)));
        assert_eq!(parse_page_ranges("12-", 10), Err(RangeError::OutOfRange(12)));
        assert_eq!(parse_page_ranges("0", 10), Err(RangeError::OutOfRange(0)));
        assert_eq!(parse_page_ranges("1", 0), Err(RangeError::NoPages));
    }

    #[test]
    fn malformed_and_reversed_ranges_are_named() {
        assert_eq!(parse_page_ranges("x", 10), Err(RangeError::Malformed("x".into())));
        assert_eq!(parse_page_ranges("1,,2", 10), Err(RangeError::Malformed("1,,2".into())));
        assert_eq!(parse_page_ranges("4-2", 10), Err(RangeError::Reversed("4-2".into())));
        assert_eq!(parse_page_ranges("1-x", 10), Err(RangeError::Malformed("x".into())));
    }

    #[test]
    fn contains_and_count_follow_the_merged_ranges() {
        let sel = parse_page_ranges("1-3, 7-", 10).unwrap();
        assert!(sel.contains(3) && !sel.contains(4) && sel.contains(10));
        assert_eq!(sel.count(10), 3 + 4);
        assert_eq!(PageSelection::All.count(10), 10);
        assert!(PageSelection::All.contains(1));
    }

    #[test]
    fn one_range_goes_into_the_print_info_and_several_are_refused_unchanged() {
        let mut info = PrintInfo::default();
        parse_page_ranges("2-4", 10).unwrap().apply_to(&mut info).unwrap();
        assert_eq!(info.pages, Some((2, 4)));
        parse_page_ranges("all", 10).unwrap().apply_to(&mut info).unwrap();
        assert_eq!(info.pages, None);
        let several = parse_page_ranges("1, 3", 10).unwrap();
        assert_eq!(several.apply_to(&mut info), Err(RangeError::Multiple));
        assert_eq!(info.pages, None, "a refused selection changes nothing");
        assert_eq!(PageSelection::Ranges(Vec::new()).apply_to(&mut info), Err(RangeError::NoPages));
    }
}
