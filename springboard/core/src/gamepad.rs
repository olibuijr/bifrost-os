//! Gamepad discovery: from an input node's evdev capability bitmaps, decides whether the node is a game controller,
//! and keeps the list of connected controllers, reporting each connect and disconnect once.
//!
//! Pure: the caller reads `EVIOCGBIT` and `EVIOCGNAME` on each node and feeds the results in, and it learns of
//! nodes from the same hot-plug watch the recovery chord uses (`springboard/src/bare/chord_recovery.rs`). The
//! values a controller reports are modelled in `bifrost_ui::gamepad`. The codes below repeat the Linux input
//! codes that model uses, because this crate has no dependency on the UI kit.

/// `BTN_SOUTH`, the first gamepad button code (`BTN_GAMEPAD` range).
const BTN_SOUTH: u16 = 0x130;
/// `BTN_THUMBR`, the last gamepad button code.
const BTN_THUMBR: u16 = 0x13e;
/// `BTN_LEFT`: a pointer, not a controller.
const BTN_LEFT: u16 = 0x110;
/// `BTN_TOUCH`: a touch panel, not a controller.
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

/// Whether bit `code` is set in an `EVIOCGBIT` bitmap: byte `code / 8`, bit `code % 8`, least significant first.
pub fn has_bit(bitmap: &[u8], code: u16) -> bool {
    matches!(bitmap.get(usize::from(code / 8)), Some(byte) if (byte >> (code % 8)) & 1 == 1)
}

/// The key and absolute-axis bitmaps of one input node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub keys: Vec<u8>,
    pub abs: Vec<u8>,
}

impl Capabilities {
    pub fn has_key(&self, code: u16) -> bool {
        has_bit(&self.keys, code)
    }

    pub fn has_abs(&self, code: u16) -> bool {
        has_bit(&self.abs, code)
    }

    /// A game controller has a face button and both left-stick axes. A pointer or a touch panel can expose the same
    /// axes, so `BTN_LEFT` or `BTN_TOUCH` excludes a node.
    pub fn is_game_controller(&self) -> bool {
        (BTN_SOUTH..=BTN_THUMBR).any(|code| self.has_key(code))
            && self.has_abs(ABS_X)
            && self.has_abs(ABS_Y)
            && !self.has_key(BTN_LEFT)
            && !self.has_key(BTN_TOUCH)
    }
}

/// A connected controller. `id` is handed out once and never reused, so an app can keep it across unplugs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    pub id: u32,
    /// The input node, for example `/dev/input/event5`.
    pub path: String,
    /// The device name the kernel reports, trimmed.
    pub name: String,
}

/// A connection change, reported once per device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Connected(Controller),
    Disconnected(Controller),
}

/// The connected controllers in the order they arrived.
#[derive(Debug, Default)]
pub struct Registry {
    last_id: u32,
    controllers: Vec<Controller>,
}

impl Registry {
    /// Records a node that appeared. Returns the event for a new controller; `None` for a node that is not a
    /// controller, or a path that is already connected. Only a controller takes an id.
    pub fn connected(&mut self, path: &str, name: &str, caps: &Capabilities) -> Option<Event> {
        if !caps.is_game_controller() || self.controllers.iter().any(|c| c.path == path) {
            return None;
        }
        self.last_id += 1;
        let controller = Controller { id: self.last_id, path: path.to_string(), name: name.trim().to_string() };
        self.controllers.push(controller.clone());
        Some(Event::Connected(controller))
    }

    /// Records a node that went away. Returns the removed controller, or `None` when the path was not connected.
    pub fn disconnected(&mut self, path: &str) -> Option<Event> {
        let at = self.controllers.iter().position(|c| c.path == path)?;
        Some(Event::Disconnected(self.controllers.remove(at)))
    }

    pub fn controllers(&self) -> &[Controller] {
        &self.controllers
    }

    pub fn get(&self, id: u32) -> Option<&Controller> {
        self.controllers.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap with the given event codes set, sized like a `KEY_MAX` capability read.
    fn bitmap(codes: &[u16]) -> Vec<u8> {
        let mut bits = vec![0u8; 96];
        for &code in codes {
            bits[usize::from(code / 8)] |= 1u8 << (code % 8);
        }
        bits
    }

    fn pad() -> Capabilities {
        Capabilities { keys: bitmap(&[BTN_SOUTH, 0x13b]), abs: bitmap(&[ABS_X, ABS_Y, 0x02]) }
    }

    #[test]
    fn classifies_pads_and_rejects_pointers_keyboards_and_touch_panels() {
        assert!(pad().is_game_controller());
        // a keyboard reports keys and no axes
        let keyboard = Capabilities { keys: bitmap(&[30, 31]), abs: Vec::new() };
        assert!(!keyboard.is_game_controller());
        // a mouse has BTN_LEFT and relative axes only
        let mouse = Capabilities { keys: bitmap(&[0x110]), abs: Vec::new() };
        assert!(!mouse.is_game_controller());
        // a touch panel has absolute axes and BTN_TOUCH, with no face button
        let touch = Capabilities { keys: bitmap(&[0x14a]), abs: bitmap(&[ABS_X, ABS_Y]) };
        assert!(!touch.is_game_controller());
        // a pad that also reports a pointer button is not treated as a controller
        let odd = Capabilities { keys: bitmap(&[BTN_SOUTH, BTN_LEFT]), abs: bitmap(&[ABS_X, ABS_Y]) };
        assert!(!odd.is_game_controller());
    }

    #[test]
    fn bitmap_bits_are_least_significant_first_per_byte() {
        // code 0x130 (304) is bit 0 of byte 38
        let mut bits = vec![0u8; 96];
        bits[38] = 0x01;
        assert!(has_bit(&bits, 0x130));
        assert!(!has_bit(&bits, 0x131));
        assert!(!has_bit(&bits, 0x138));
        // a code past the end of the bitmap is not set
        assert!(!has_bit(&[], 0));
        assert!(!has_bit(&bits, 0x2ff * 2));
    }

    #[test]
    fn connect_assigns_ids_and_reports_each_path_once() {
        let mut reg = Registry::default();
        let first = reg.connected("/dev/input/event5", "  Pad A  ", &pad());
        assert_eq!(
            first,
            Some(Event::Connected(Controller { id: 1, path: "/dev/input/event5".into(), name: "Pad A".into() }))
        );
        // the same node reported again is not a second controller
        assert_eq!(reg.connected("/dev/input/event5", "Pad A", &pad()), None);
        match reg.connected("/dev/input/event6", "Pad B", &pad()) {
            Some(Event::Connected(c)) => assert_eq!(c.id, 2),
            other => panic!("expected a second controller, got {other:?}"),
        }
        assert_eq!(reg.controllers().len(), 2);
    }

    #[test]
    fn nodes_that_are_not_controllers_take_no_id() {
        let mut reg = Registry::default();
        let keyboard = Capabilities { keys: bitmap(&[30]), abs: Vec::new() };
        assert_eq!(reg.connected("/dev/input/event1", "Keyboard", &keyboard), None);
        assert!(reg.controllers().is_empty());
        // the next real controller is still id 1
        match reg.connected("/dev/input/event2", "Pad", &pad()) {
            Some(Event::Connected(c)) => assert_eq!(c.id, 1),
            other => panic!("expected a controller, got {other:?}"),
        }
    }

    #[test]
    fn disconnect_removes_the_controller_and_ids_are_not_reused() {
        let mut reg = Registry::default();
        reg.connected("/dev/input/event5", "Pad", &pad());
        match reg.disconnected("/dev/input/event5") {
            Some(Event::Disconnected(c)) => assert_eq!((c.id, c.path.as_str()), (1, "/dev/input/event5")),
            other => panic!("expected a disconnect, got {other:?}"),
        }
        assert!(reg.controllers().is_empty());
        // a path that is not connected reports nothing
        assert_eq!(reg.disconnected("/dev/input/event5"), None);
        // the same node back is a new controller with a fresh id
        match reg.connected("/dev/input/event5", "Pad", &pad()) {
            Some(Event::Connected(c)) => assert_eq!(c.id, 2),
            other => panic!("expected a reconnect, got {other:?}"),
        }
    }

    #[test]
    fn controllers_keep_arrival_order_and_look_up_by_id() {
        let mut reg = Registry::default();
        reg.connected("/dev/input/event5", "Pad A", &pad());
        reg.connected("/dev/input/event6", "Pad B", &pad());
        let ids: Vec<u32> = reg.controllers().iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(reg.get(2).map(|c| c.name.as_str()), Some("Pad B"));
        assert_eq!(reg.get(9), None);
        // removing the first keeps the second's id
        reg.disconnected("/dev/input/event5");
        assert_eq!(reg.get(2).map(|c| c.path.as_str()), Some("/dev/input/event6"));
        assert_eq!(reg.get(1), None);
    }
}
