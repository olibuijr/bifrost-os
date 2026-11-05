//! The trace switch for window focus decisions (`touch $dir/trace` or `trace on` over the control socket, the same
//! switch that logs input): every focus, hand-back and activation the window layer reads or asks for, and every choice
//! the desk makes about it, is logged with the window ids and the reason, so an E2E journal shows why a window came
//! (or did not come) to the front.
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Turns the focus trace on or off (the shell's `trace` switch).
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Whether the focus trace is on.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Logs one focus decision while the trace is on.
pub fn log(what: std::fmt::Arguments) {
    if on() {
        eprintln!("springboard: focus {what}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_switch_turns_the_trace_on_and_off() {
        super::set(true);
        assert!(super::on());
        super::set(false);
        assert!(!super::on());
    }
}
