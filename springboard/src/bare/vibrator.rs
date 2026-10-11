//! Whether the device has a vibrator (most PCs have none): Settings only offers
//! Keyboard Haptics when it does.

use std::path::Path;

const NODES: [&str; 2] = ["/sys/class/leds/vibrator", "/sys/class/timed_output/vibrator"];

pub fn vibrator_available() -> bool {
    NODES.iter().any(|p| Path::new(p).exists())
}

/// Same name as the spec asks for.
pub fn available() -> bool {
    vibrator_available()
}
