//! Exception handling mask (docs/parity-matrix, theme-20): which uncaught conditions a process monitors, handles,
//! logs or halts for a debugger, and the delegate that approves each handle and log decision.
//!
//! Pure model with no hooks installed: the panic hook in `springboard/src/bare/crashlog.rs` does not read it yet.
//! The halt bits only report a decision; nothing here suspends a process or attaches a debugger. The handle and log
//! masks start empty, so nothing is monitored until a caller sets a mask. The handling mask and the halt mask are set
//! separately, as the platform's two setters are, and each setter drops the bits that belong to the other.

use std::ops::BitOr;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// The class of a condition, from the most specific to the least.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An uncaught condition of the program itself.
    Uncaught,
    /// An uncaught condition raised by the system.
    UncaughtSystem,
    /// An uncaught runtime error.
    UncaughtRuntime,
    /// A condition that reaches the top-level handler.
    TopLevel,
    /// A condition caught below the top-level handler.
    Other,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Uncaught, Kind::UncaughtSystem, Kind::UncaughtRuntime, Kind::TopLevel, Kind::Other];

    fn log_bit(self) -> Mask {
        match self {
            Kind::Uncaught => LOG_UNCAUGHT,
            Kind::UncaughtSystem => LOG_UNCAUGHT_SYSTEM,
            Kind::UncaughtRuntime => LOG_UNCAUGHT_RUNTIME,
            Kind::TopLevel => LOG_TOP_LEVEL,
            Kind::Other => LOG_OTHER,
        }
    }

    fn handle_bit(self) -> Mask {
        match self {
            Kind::Uncaught => HANDLE_UNCAUGHT,
            Kind::UncaughtSystem => HANDLE_UNCAUGHT_SYSTEM,
            Kind::UncaughtRuntime => HANDLE_UNCAUGHT_RUNTIME,
            Kind::TopLevel => HANDLE_TOP_LEVEL,
            Kind::Other => HANDLE_OTHER,
        }
    }

    fn hang_bit(self) -> Mask {
        match self {
            Kind::Uncaught => HANG_UNCAUGHT,
            Kind::UncaughtSystem => HANG_UNCAUGHT_SYSTEM,
            Kind::UncaughtRuntime => HANG_UNCAUGHT_RUNTIME,
            Kind::TopLevel => HANG_TOP_LEVEL,
            Kind::Other => HANG_OTHER,
        }
    }
}

/// A set of mask bits. Bits 0 to 9 are the log and handle bits; bits 10 to 14 are the halt bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mask(u32);

pub const LOG_UNCAUGHT: Mask = Mask(1 << 0);
pub const HANDLE_UNCAUGHT: Mask = Mask(1 << 1);
pub const LOG_UNCAUGHT_SYSTEM: Mask = Mask(1 << 2);
pub const HANDLE_UNCAUGHT_SYSTEM: Mask = Mask(1 << 3);
pub const LOG_UNCAUGHT_RUNTIME: Mask = Mask(1 << 4);
pub const HANDLE_UNCAUGHT_RUNTIME: Mask = Mask(1 << 5);
pub const LOG_TOP_LEVEL: Mask = Mask(1 << 6);
pub const HANDLE_TOP_LEVEL: Mask = Mask(1 << 7);
pub const LOG_OTHER: Mask = Mask(1 << 8);
pub const HANDLE_OTHER: Mask = Mask(1 << 9);

pub const HANG_UNCAUGHT: Mask = Mask(1 << 10);
pub const HANG_UNCAUGHT_SYSTEM: Mask = Mask(1 << 11);
pub const HANG_UNCAUGHT_RUNTIME: Mask = Mask(1 << 12);
pub const HANG_TOP_LEVEL: Mask = Mask(1 << 13);
pub const HANG_OTHER: Mask = Mask(1 << 14);

const HANDLING_BITS: u32 = (1 << 10) - 1;
const HANGING_BITS: u32 = ((1 << 15) - 1) & !HANDLING_BITS;

impl Mask {
    /// Builds a mask from raw bits. Bits that name no constant are dropped.
    pub fn from_bits(bits: u32) -> Mask {
        Mask(bits & (HANDLING_BITS | HANGING_BITS))
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every bit of `other` is set in `self`.
    pub fn contains(self, other: Mask) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for Mask {
    type Output = Mask;

    fn bitor(self, rhs: Mask) -> Mask {
        Mask(self.0 | rhs.0)
    }
}

/// What the handler does with one condition of a given kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The condition is logged or handled.
    pub monitored: bool,
    /// The condition is logged, after the delegate approves it.
    pub log: bool,
    /// The condition is handled, after the delegate approves it.
    pub handle: bool,
    /// The process halts so a debugger can attach. Not subject to delegate approval.
    pub halt: bool,
}

/// Approves each handle and log decision. Both default to approval, so a delegate only overrides what it must.
pub trait Delegate: Send {
    /// `mask` is the handle bit that matched the condition.
    fn should_handle(&self, _kind: Kind, _mask: Mask) -> bool {
        true
    }

    /// `mask` is the log bit that matched the condition.
    fn should_log(&self, _kind: Kind, _mask: Mask) -> bool {
        true
    }
}

/// The handler: the masks, and the optional delegate that approves each decision.
#[derive(Default)]
pub struct Handler {
    handling: Mask,
    hanging: Mask,
    delegate: Option<Box<dyn Delegate>>,
}

impl Handler {
    pub fn new() -> Handler {
        Handler::default()
    }

    pub fn handling_mask(&self) -> Mask {
        self.handling
    }

    /// Sets the log and handle bits. Halt bits in `mask` are dropped.
    pub fn set_handling_mask(&mut self, mask: Mask) {
        self.handling = Mask::from_bits(mask.0 & HANDLING_BITS);
    }

    pub fn hanging_mask(&self) -> Mask {
        self.hanging
    }

    /// Sets the halt bits. Log and handle bits in `mask` are dropped.
    pub fn set_hanging_mask(&mut self, mask: Mask) {
        self.hanging = Mask::from_bits(mask.0 & HANGING_BITS);
    }

    pub fn delegate(&self) -> Option<&dyn Delegate> {
        self.delegate.as_deref()
    }

    /// Replaces the delegate. `None` removes it.
    pub fn set_delegate(&mut self, delegate: Option<Box<dyn Delegate>>) {
        self.delegate = delegate;
    }

    /// The decision for a condition of `kind`. A delegate is asked only about a bit that is set, and its answer can
    /// turn handling or logging off but never on.
    pub fn decide(&self, kind: Kind) -> Decision {
        let (log_bit, handle_bit) = (kind.log_bit(), kind.handle_bit());
        let log_on = self.handling.contains(log_bit);
        let handle_on = self.handling.contains(handle_bit);
        let (handle, log) = match &self.delegate {
            None => (handle_on, log_on),
            Some(d) => (
                handle_on && d.should_handle(kind, handle_bit),
                log_on && d.should_log(kind, log_bit),
            ),
        };
        Decision {
            monitored: log_on || handle_on,
            log,
            handle,
            halt: self.hanging.contains(kind.hang_bit()),
        }
    }
}

static DEFAULT: OnceLock<Mutex<Handler>> = OnceLock::new();

/// The shared handler, the one place a process keeps its exception policy. A poisoned lock is recovered, since the
/// masks are plain bits and stay valid after a panic.
pub fn default_handler() -> MutexGuard<'static, Handler> {
    DEFAULT.get_or_init(|| Mutex::new(Handler::new())).lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Veto;

    impl Delegate for Veto {
        fn should_handle(&self, _kind: Kind, _mask: Mask) -> bool {
            false
        }
        fn should_log(&self, _kind: Kind, _mask: Mask) -> bool {
            false
        }
    }

    struct LogOnly;

    impl Delegate for LogOnly {
        fn should_handle(&self, _kind: Kind, _mask: Mask) -> bool {
            false
        }
    }

    fn decision(monitored: bool, log: bool, handle: bool, halt: bool) -> Decision {
        Decision { monitored, log, handle, halt }
    }

    #[test]
    fn mask_bits_follow_the_documented_layout() {
        assert_eq!(LOG_UNCAUGHT.bits(), 1);
        assert_eq!(HANDLE_OTHER.bits(), 1 << 9);
        assert_eq!(HANG_OTHER.bits(), 1 << 14);
        assert_eq!(Mask::from_bits(u32::MAX).bits(), 0x7FFF);
        assert_eq!((LOG_OTHER | HANDLE_OTHER).bits(), 0x300);
        assert!((LOG_OTHER | HANDLE_OTHER).contains(LOG_OTHER));
        assert!(!LOG_OTHER.contains(HANDLE_OTHER));
    }

    #[test]
    fn a_new_handler_monitors_nothing() {
        let h = Handler::new();
        for kind in Kind::ALL {
            assert_eq!(h.decide(kind), decision(false, false, false, false), "{kind:?}");
        }
    }

    #[test]
    fn log_bit_alone_logs_without_handling() {
        let mut h = Handler::new();
        h.set_handling_mask(LOG_UNCAUGHT);
        assert_eq!(h.decide(Kind::Uncaught), decision(true, true, false, false));
        assert_eq!(h.decide(Kind::UncaughtSystem), decision(false, false, false, false));
    }

    #[test]
    fn handle_and_log_without_delegate_both_apply() {
        let mut h = Handler::new();
        h.set_handling_mask(LOG_TOP_LEVEL | HANDLE_TOP_LEVEL);
        assert_eq!(h.decide(Kind::TopLevel), decision(true, true, true, false));
    }

    #[test]
    fn delegate_can_veto_handling_but_not_add_it() {
        let mut h = Handler::new();
        h.set_delegate(Some(Box::new(LogOnly)));
        h.set_handling_mask(LOG_OTHER | HANDLE_OTHER);
        assert_eq!(h.decide(Kind::Other), decision(true, true, false, false));
        // An unmonitored condition stays unmonitored whatever the delegate says.
        assert_eq!(h.decide(Kind::Uncaught), decision(false, false, false, false));
    }

    #[test]
    fn halt_mask_is_independent_of_monitoring() {
        let mut h = Handler::new();
        h.set_hanging_mask(HANG_TOP_LEVEL);
        assert_eq!(h.decide(Kind::TopLevel), decision(false, false, false, true));
        // Even a veto does not cancel a halt.
        h.set_delegate(Some(Box::new(Veto)));
        assert_eq!(h.decide(Kind::TopLevel), decision(false, false, false, true));
    }

    #[test]
    fn each_setter_drops_the_other_masks_bits() {
        let mut h = Handler::new();
        h.set_handling_mask(HANG_OTHER | LOG_OTHER);
        assert_eq!(h.handling_mask(), LOG_OTHER);
        h.set_hanging_mask(HANDLE_OTHER | HANG_UNCAUGHT);
        assert_eq!(h.hanging_mask(), HANG_UNCAUGHT);
    }

    #[test]
    fn delegate_is_set_and_cleared() {
        let mut h = Handler::new();
        assert!(h.delegate().is_none());
        h.set_delegate(Some(Box::new(Veto)));
        assert!(h.delegate().is_some());
        h.set_delegate(None);
        assert!(h.delegate().is_none());
    }

    #[test]
    fn default_handler_keeps_its_masks_between_calls() {
        default_handler().set_handling_mask(LOG_OTHER);
        assert_eq!(default_handler().handling_mask(), LOG_OTHER);
        default_handler().set_handling_mask(Mask::default());
        assert!(default_handler().handling_mask().is_empty());
    }
}
