//! The page Print lays notes out on: the paper and the orientation that File > Page Setup chooses. They are kept in the
//! app's defaults, so the choice lasts across launches.

use bifrost_app::defaults::UserDefaults;
use bifrost_app::print::{Orientation, PaperSize, PrintInfo};

const PAPER: &str = "stickies.page.paper";
const LANDSCAPE: &str = "stickies.page.landscape";

/// The papers Page Setup offers, in menu order. The first one is the default.
pub const PAPERS: [PaperSize; 5] = [PaperSize::A4, PaperSize::Letter, PaperSize::Legal, PaperSize::A5, PaperSize::A3];

/// Reads the page from the app's defaults. A paper that is not in the menu takes the default paper.
pub fn load(store: &UserDefaults) -> PrintInfo {
    let mut page = PrintInfo::default();
    if let Some(paper) = store.string(PAPER).and_then(|name| PAPERS.iter().copied().find(|p| p.media() == name)) {
        page.paper = paper;
    }
    if store.bool(LANDSCAPE) {
        page.orientation = Orientation::Landscape;
    }
    page
}

/// Writes the paper and the orientation of `page` to the app's defaults.
pub fn save(store: &mut UserDefaults, page: &PrintInfo) {
    store.set_string(PAPER, page.paper.media());
    store.set_bool(LANDSCAPE, page.orientation == Orientation::Landscape);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_gives_the_default_page() {
        let store = UserDefaults::memory();
        let page = load(&store);
        assert_eq!(page.paper, PAPERS[0]);
        assert_eq!(page.orientation, Orientation::Portrait);
    }

    #[test]
    fn the_paper_and_orientation_round_trip_through_the_defaults() {
        let mut store = UserDefaults::memory();
        let mut page = PrintInfo::default();
        page.paper = PaperSize::Legal;
        page.orientation = Orientation::Landscape;
        save(&mut store, &page);
        let back = load(&store);
        assert_eq!(back.paper, PaperSize::Legal);
        assert_eq!(back.orientation, Orientation::Landscape);
    }

    #[test]
    fn an_unknown_paper_name_takes_the_default_paper() {
        let mut store = UserDefaults::memory();
        store.set_string(PAPER, "Tabloid");
        assert_eq!(load(&store).paper, PAPERS[0]);
    }
}
