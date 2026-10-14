//! Switch state of an input device: the lid switch, tablet mode and the headset jack switches, as the
//! kernel reports them in the switch bitmap. This is the pure model. The system layer reads the bitmap
//! at start-up and passes each new bitmap to `SwitchState::update`; the model returns the switches that
//! changed, so a caller can react to a lid closing or a headset being plugged in. No device is opened here.

/// Switch codes, numbered as the kernel numbers them: each code is a bit position in the bitmap.
pub const SW_LID: u16 = 0x00;
pub const SW_TABLET_MODE: u16 = 0x01;
pub const SW_HEADPHONE_INSERT: u16 = 0x02;
pub const SW_MICROPHONE_INSERT: u16 = 0x04;

/// One switch that changed state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchEdge {
    pub code: u16,
    pub on: bool,
}

/// The switch bitmap of one device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchState {
    bits: u32,
}

impl SwitchState {
    /// The state from a bitmap read at start-up. Bits above 31 are not switches and are ignored.
    pub fn from_bits(bits: u32) -> SwitchState {
        SwitchState { bits }
    }

    pub fn bits(self) -> u32 {
        self.bits
    }

    /// Whether the switch with `code` is on. Codes above 31 are never on.
    pub fn is_on(self, code: u16) -> bool {
        code < 32 && (self.bits >> code) & 1 != 0
    }

    /// The lid is closed when the lid switch is on.
    pub fn lid_closed(self) -> bool {
        self.is_on(SW_LID)
    }

    /// A headset is plugged in when either the headphone or the microphone insert switch is on.
    pub fn headset_plugged(self) -> bool {
        self.is_on(SW_HEADPHONE_INSERT) || self.is_on(SW_MICROPHONE_INSERT)
    }

    /// The device is in tablet mode when the tablet mode switch is on (the keyboard is folded away).
    pub fn tablet_mode(self) -> bool {
        self.is_on(SW_TABLET_MODE)
    }

    /// Replaces the bitmap with `bits` and returns the switches that changed, in code order.
    /// An unchanged bitmap returns no edges.
    pub fn update(&mut self, bits: u32) -> Vec<SwitchEdge> {
        let changed = self.bits ^ bits;
        let mut edges = Vec::new();
        for code in 0..32u16 {
            if (changed >> code) & 1 != 0 {
                edges.push(SwitchEdge { code, on: (bits >> code) & 1 != 0 });
            }
        }
        self.bits = bits;
        edges
    }
}

/// The new state of `code` in an edge list, if that switch changed.
pub fn edge_for(edges: &[SwitchEdge], code: u16) -> Option<bool> {
    edges.iter().find(|e| e.code == code).map(|e| e.on)
}

/// The switches an observer asked for, as a bit set over the switch codes. An observer receives only the edges of
/// the switches it watches, so a headset observer is not woken by a lid change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchWatch {
    mask: u32,
}

impl SwitchWatch {
    /// A watch for the listed codes. Codes above 31 are not switches and are left out.
    pub fn new(codes: &[u16]) -> SwitchWatch {
        let mask = codes.iter().filter(|&&code| code < 32).fold(0u32, |mask, &code| mask | (1u32 << code));
        SwitchWatch { mask }
    }

    /// A watch for every switch.
    pub fn all() -> SwitchWatch {
        SwitchWatch { mask: u32::MAX }
    }

    /// Whether this watch includes the switch `code`.
    pub fn watches(self, code: u16) -> bool {
        code < 32 && (self.mask >> code) & 1 != 0
    }

    /// The edges of the watched switches, in the order given.
    pub fn select(self, edges: &[SwitchEdge]) -> Vec<SwitchEdge> {
        edges.iter().copied().filter(|e| self.watches(e.code)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lid_and_headset_read_their_bits() {
        assert!(SwitchState::from_bits(1 << SW_LID).lid_closed());
        assert!(!SwitchState::from_bits(0).lid_closed());
        assert!(SwitchState::from_bits(1 << SW_HEADPHONE_INSERT).headset_plugged());
        assert!(SwitchState::from_bits(1 << SW_MICROPHONE_INSERT).headset_plugged());
        // The lid alone is not a headset.
        assert!(!SwitchState::from_bits(1 << SW_LID).headset_plugged());
    }

    #[test]
    fn lid_closing_gives_one_edge() {
        let mut s = SwitchState::default();
        let edges = s.update(1 << SW_LID);
        assert_eq!(edges, vec![SwitchEdge { code: SW_LID, on: true }]);
        assert!(s.lid_closed());
    }

    #[test]
    fn mixed_change_reports_each_switch_in_code_order() {
        // Before: lid (bit 0) and microphone (bit 4) on. After: headphone (bit 2) only.
        let mut s = SwitchState::from_bits(0b10001);
        let edges = s.update(0b00100);
        // Changed bits are 0, 2 and 4: lid off, headphone on, microphone off.
        assert_eq!(
            edges,
            vec![
                SwitchEdge { code: 0, on: false },
                SwitchEdge { code: 2, on: true },
                SwitchEdge { code: 4, on: false },
            ]
        );
        assert_eq!(edge_for(&edges, SW_HEADPHONE_INSERT), Some(true));
        assert_eq!(edge_for(&edges, SW_TABLET_MODE), None);
        assert!(s.headset_plugged());
    }

    #[test]
    fn unchanged_bitmap_gives_no_edges() {
        let mut s = SwitchState::from_bits(0b110);
        assert!(s.update(0b110).is_empty());
        assert_eq!(s.bits(), 0b110);
    }

    #[test]
    fn codes_above_31_are_never_on() {
        let s = SwitchState::from_bits(u32::MAX);
        assert!(!s.is_on(32));
        assert!(!s.is_on(200));
        assert!(s.is_on(31));
    }
}
