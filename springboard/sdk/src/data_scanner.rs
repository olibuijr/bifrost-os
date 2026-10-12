//! Barcode and QR scanning for apps (`DataScannerViewController`, `RecognizedItem` barcode items, and the
//! `DataScannerViewControllerDelegate` callbacks).
//!
//! [`DataScanner`] takes one camera frame at a time as 8-bit luminance and decodes the QR code in it with the
//! decoder the system Camera app uses. Results go through [`ScanSession`], so an app receives the same added,
//! updated and removed delta that a scanner delegate gets. The item id is a hash of the payload, so one code keeps
//! its id from frame to frame. Limits: QR model 2 only, one code per frame, and no perspective correction. The
//! outline of an item is the whole frame, because the decoder does not report the position of the code.
//!
//! [`Stabilizer`] (from `bifrost_ui::camera_region`) keeps a region of the frame on screen while the target moves.

use springboard::shell::camera::qr::decode_luma;

pub use bifrost_ui::camera_region::{Stabilizer, DEFAULT_DEAD_BAND};
pub use bifrost_ui::scan::{Changes, Item, ItemKind, Point, Quad, ScanSession};

/// FNV-1a, 64 bit: a hash that does not change between runs or releases, unlike `std`'s default hasher.
fn payload_id(payload: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in payload.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The four corners of a `w` x `h` frame, clockwise from the top left.
fn frame_quad(w: usize, h: usize) -> Quad {
    let (w, h) = (w as f32, h as f32);
    Quad([Point { x: 0.0, y: 0.0 }, Point { x: w, y: 0.0 }, Point { x: w, y: h }, Point { x: 0.0, y: h }])
}

/// Barcode scanning over a stream of camera frames.
#[derive(Debug, Default)]
pub struct DataScanner {
    session: ScanSession,
}

impl DataScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes one frame (`w` x `h` luminance, row-major) and reports the change since the previous frame. A frame
    /// without a readable code removes the previous item. A frame that is too small or too short is treated the same.
    pub fn scan_luma(&mut self, luma: &[u8], w: usize, h: usize) -> Changes {
        let observed = match decode_luma(luma, w, h) {
            Some(payload) => vec![Item {
                id: payload_id(&payload),
                kind: ItemKind::Barcode,
                payload,
                outline: frame_quad(w, h),
            }],
            None => Vec::new(),
        };
        self.session.update(observed)
    }

    /// The items visible in the last frame, ordered by id.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.session.items()
    }

    /// Ends the scan and reports every item as removed.
    pub fn stop(&mut self) -> Changes {
        self.session.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_id_is_fnv1a() {
        // FNV-1a 64 offset basis for the empty string.
        assert_eq!(payload_id(""), 0xcbf2_9ce4_8422_2325);
        // One step of FNV-1a from the basis: (basis ^ 0x61) * prime.
        assert_eq!(payload_id("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn same_payload_gives_same_id() {
        assert_eq!(payload_id("https://example.org"), payload_id("https://example.org"));
        assert_ne!(payload_id("ab"), payload_id("ba"));
    }

    #[test]
    fn frame_quad_covers_the_frame_clockwise() {
        let q = frame_quad(640, 480);
        assert_eq!(q.0[0], Point { x: 0.0, y: 0.0 });
        assert_eq!(q.0[1], Point { x: 640.0, y: 0.0 });
        assert_eq!(q.0[2], Point { x: 640.0, y: 480.0 });
        assert_eq!(q.0[3], Point { x: 0.0, y: 480.0 });
    }

    #[test]
    fn blank_frame_reports_nothing() {
        let mut s = DataScanner::new();
        let flat = vec![200u8; 100 * 100];
        assert!(s.scan_luma(&flat, 100, 100).is_empty());
        assert_eq!(s.items().count(), 0);
    }

    #[test]
    fn short_or_tiny_frames_report_nothing() {
        let mut s = DataScanner::new();
        // Buffer shorter than w * h, and a frame below the 21 pixel minimum for a QR code.
        assert!(s.scan_luma(&[0u8; 50], 100, 100).is_empty());
        assert!(s.scan_luma(&[0u8; 400], 20, 20).is_empty());
    }

    #[test]
    fn stop_on_an_idle_scanner_changes_nothing() {
        let mut s = DataScanner::new();
        assert!(s.stop().is_empty());
    }
}
