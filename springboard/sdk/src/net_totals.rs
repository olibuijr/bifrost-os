//! Network byte totals for apps: how many bytes the device has moved over Wi-Fi and over cellular, split into
//! upload and download, and the change between two readings for a reporting interval.
//!
//! Pure counters. The shell feeds them from its interface byte counters; no service call is made here. Totals
//! saturate at the top of `u64` instead of wrapping. [`Totals::since`] treats a counter that went down (the
//! interface was reset) as nothing moved for that slot, rather than a negative amount.

/// The link a byte moved over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    WiFi,
    Cellular,
}

/// The direction of the transfer, from this device's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Upload,
    Download,
}

/// Byte totals for the four link and direction pairs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    wifi_up: u64,
    wifi_down: u64,
    cell_up: u64,
    cell_down: u64,
}

impl Totals {
    fn slot(&mut self, link: Link, dir: Direction) -> &mut u64 {
        match (link, dir) {
            (Link::WiFi, Direction::Upload) => &mut self.wifi_up,
            (Link::WiFi, Direction::Download) => &mut self.wifi_down,
            (Link::Cellular, Direction::Upload) => &mut self.cell_up,
            (Link::Cellular, Direction::Download) => &mut self.cell_down,
        }
    }

    /// The bytes recorded for one link and direction.
    pub fn get(&self, link: Link, dir: Direction) -> u64 {
        match (link, dir) {
            (Link::WiFi, Direction::Upload) => self.wifi_up,
            (Link::WiFi, Direction::Download) => self.wifi_down,
            (Link::Cellular, Direction::Upload) => self.cell_up,
            (Link::Cellular, Direction::Download) => self.cell_down,
        }
    }

    /// Adds bytes to one link and direction.
    pub fn add(&mut self, link: Link, dir: Direction, bytes: u64) {
        let slot = self.slot(link, dir);
        *slot = slot.saturating_add(bytes);
    }

    /// Upload plus download over one link.
    pub fn total(&self, link: Link) -> u64 {
        self.get(link, Direction::Upload).saturating_add(self.get(link, Direction::Download))
    }

    /// The bytes moved since an earlier reading, slot by slot. A slot that is smaller than in the earlier
    /// reading counts as zero.
    pub fn since(&self, earlier: &Totals) -> Totals {
        Totals {
            wifi_up: self.wifi_up.saturating_sub(earlier.wifi_up),
            wifi_down: self.wifi_down.saturating_sub(earlier.wifi_down),
            cell_up: self.cell_up.saturating_sub(earlier.cell_up),
            cell_down: self.cell_down.saturating_sub(earlier.cell_down),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_slot_accumulates_on_its_own() {
        let mut t = Totals::default();
        t.add(Link::WiFi, Direction::Upload, 100);
        t.add(Link::WiFi, Direction::Upload, 50);
        assert_eq!(t.get(Link::WiFi, Direction::Upload), 150);
        assert_eq!(t.get(Link::WiFi, Direction::Download), 0);
        assert_eq!(t.get(Link::Cellular, Direction::Upload), 0);
    }

    #[test]
    fn total_sums_both_directions_of_one_link_only() {
        let mut t = Totals::default();
        t.add(Link::WiFi, Direction::Upload, 10);
        t.add(Link::WiFi, Direction::Download, 20);
        t.add(Link::Cellular, Direction::Download, 7);
        assert_eq!(t.total(Link::WiFi), 30);
        assert_eq!(t.total(Link::Cellular), 7);
    }

    #[test]
    fn totals_saturate_instead_of_wrapping() {
        let mut t = Totals::default();
        t.add(Link::Cellular, Direction::Upload, u64::MAX);
        t.add(Link::Cellular, Direction::Upload, 1);
        assert_eq!(t.get(Link::Cellular, Direction::Upload), u64::MAX);
        assert_eq!(t.total(Link::Cellular), u64::MAX);
    }

    #[test]
    fn since_gives_the_interval_change() {
        let mut earlier = Totals::default();
        earlier.add(Link::WiFi, Direction::Upload, 100);
        let mut now = earlier;
        now.add(Link::WiFi, Direction::Upload, 150);
        now.add(Link::Cellular, Direction::Download, 40);
        let delta = now.since(&earlier);
        assert_eq!(delta.get(Link::WiFi, Direction::Upload), 150);
        assert_eq!(delta.get(Link::Cellular, Direction::Download), 40);
        assert_eq!(delta.total(Link::WiFi), 150);
    }

    #[test]
    fn a_counter_that_went_down_counts_as_zero() {
        let mut earlier = Totals::default();
        earlier.add(Link::WiFi, Direction::Download, 500);
        let mut now = Totals::default();
        now.add(Link::WiFi, Direction::Download, 30);
        assert_eq!(now.since(&earlier).get(Link::WiFi, Direction::Download), 0);
    }
}
