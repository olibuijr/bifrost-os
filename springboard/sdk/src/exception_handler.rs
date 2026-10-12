//! Failure monitoring for apps: which kinds of failure a handler monitors, whether it handles or logs each one, which
//! kinds halt for a debugger, a delegate that approves each decision, and a process-wide handler. Also reads the
//! crash records the shell keeps.
//!
//! Rust has no exception objects. A failure is a panic or another fault the caller classifies as a [`FailureKind`].
//! The shell owns the process panic hook (`springboard/src/bare/crashlog.rs`), and this module does not install one:
//! a caller builds a [`Failure`], asks [`ExceptionHandler::decide`] what to do, and acts on the [`Decision`]. Nothing
//! here stops a thread or halts a process. `handle` and `halt` are policy that the caller enforces.
//!
//! Each failure kind owns three bits: a logging bit and a handling bit in the handling mask ([`handling`]), and a
//! hanging bit in the hanging mask ([`hanging`]). A failure is monitored when its bit is set; the delegate is asked
//! only about monitored failures. The process-wide handler is [`default_handler`].

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use springboard::shell::diag::{self, CrashEntry};

/// The kinds of failure a handler can monitor, in the order their bits are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// A failure that reaches the top of its thread without being caught.
    Uncaught,
    /// An uncaught fault raised by the system layer.
    UncaughtSystem,
    /// An uncaught runtime error.
    UncaughtRuntime,
    /// A failure that reaches the outermost handler of a thread.
    TopLevel,
    /// A failure caught by an inner handler.
    Other,
}

impl FailureKind {
    /// Position in the bit layout: 0 for `Uncaught` up to 4 for `Other`.
    pub fn index(self) -> u32 {
        match self {
            FailureKind::Uncaught => 0,
            FailureKind::UncaughtSystem => 1,
            FailureKind::UncaughtRuntime => 2,
            FailureKind::TopLevel => 3,
            FailureKind::Other => 4,
        }
    }

    /// The bit that logs this kind of failure.
    pub fn log_bit(self) -> u32 {
        1 << (2 * self.index())
    }

    /// The bit that handles this kind of failure.
    pub fn handle_bit(self) -> u32 {
        1 << (2 * self.index() + 1)
    }

    /// Both handling bits (log and handle) of this kind.
    pub fn handling_bits(self) -> u32 {
        self.log_bit() | self.handle_bit()
    }

    /// The bit that halts for a debugger on this kind of failure.
    pub fn hang_bit(self) -> u32 {
        1 << self.index()
    }
}

/// Bits for [`ExceptionHandler::set_handling_mask`]: a log bit and a handle bit for each kind.
pub mod handling {
    pub const LOG_UNCAUGHT: u32 = 1 << 0;
    pub const HANDLE_UNCAUGHT: u32 = 1 << 1;
    pub const LOG_UNCAUGHT_SYSTEM: u32 = 1 << 2;
    pub const HANDLE_UNCAUGHT_SYSTEM: u32 = 1 << 3;
    pub const LOG_UNCAUGHT_RUNTIME: u32 = 1 << 4;
    pub const HANDLE_UNCAUGHT_RUNTIME: u32 = 1 << 5;
    pub const LOG_TOP_LEVEL: u32 = 1 << 6;
    pub const HANDLE_TOP_LEVEL: u32 = 1 << 7;
    pub const LOG_OTHER: u32 = 1 << 8;
    pub const HANDLE_OTHER: u32 = 1 << 9;
    /// Every defined bit.
    pub const ALL: u32 = (1 << 10) - 1;
}

/// Bits for [`ExceptionHandler::set_hanging_mask`]: one bit per kind of failure.
pub mod hanging {
    pub const UNCAUGHT: u32 = 1 << 0;
    pub const UNCAUGHT_SYSTEM: u32 = 1 << 1;
    pub const UNCAUGHT_RUNTIME: u32 = 1 << 2;
    pub const TOP_LEVEL: u32 = 1 << 3;
    pub const OTHER: u32 = 1 << 4;
    /// Every defined bit.
    pub const ALL: u32 = (1 << 5) - 1;
}

/// One failure as a handler sees it: its kind, the message and where it happened when known (`file:line:col`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
    pub location: Option<String>,
}

impl Failure {
    pub fn new(kind: FailureKind, message: &str, location: Option<&str>) -> Failure {
        Failure { kind, message: message.to_string(), location: location.map(String::from) }
    }
}

/// What the handler decided for one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The caller should handle the failure (for example, end only the thread it occurred on).
    pub handle: bool,
    /// The caller should write the failure to its log.
    pub log: bool,
    /// The caller should suspend execution so a debugger can attach.
    pub halt: bool,
}

/// Approves the handler's decisions. The handler asks only about failures its mask monitors. Each method receives the
/// failure and the bits of its kind that are set in the handling mask. The default answer is yes, so a delegate
/// overrides only the decisions it refuses.
pub trait ExceptionHandlerDelegate: Send {
    fn should_handle(&self, _failure: &Failure, _mask: u32) -> bool {
        true
    }

    fn should_log(&self, _failure: &Failure, _mask: u32) -> bool {
        true
    }
}

/// Which failures are monitored and what happens to them, with an optional delegate that approves each decision.
/// A new handler monitors nothing; the official pages do not state a default mask, so this one is a choice.
#[derive(Default)]
pub struct ExceptionHandler {
    handling: u32,
    hanging: u32,
    delegate: Option<Box<dyn ExceptionHandlerDelegate>>,
}

impl ExceptionHandler {
    pub fn new() -> ExceptionHandler {
        ExceptionHandler::default()
    }

    pub fn handling_mask(&self) -> u32 {
        self.handling
    }

    /// Sets the monitored kinds. Bits outside [`handling::ALL`] are dropped.
    pub fn set_handling_mask(&mut self, mask: u32) {
        self.handling = mask & handling::ALL;
    }

    pub fn hanging_mask(&self) -> u32 {
        self.hanging
    }

    /// Sets the kinds that halt. Bits outside [`hanging::ALL`] are dropped.
    pub fn set_hanging_mask(&mut self, mask: u32) {
        self.hanging = mask & hanging::ALL;
    }

    pub fn delegate(&self) -> Option<&dyn ExceptionHandlerDelegate> {
        self.delegate.as_deref()
    }

    pub fn set_delegate(&mut self, delegate: Option<Box<dyn ExceptionHandlerDelegate>>) {
        self.delegate = delegate;
    }

    /// The decision for one failure. A kind's log bit gates logging and its handle bit gates handling; the delegate
    /// is asked only when the matching bit is set. `halt` follows the hanging mask alone.
    pub fn decide(&self, failure: &Failure) -> Decision {
        let kind = failure.kind;
        let mask = kind.handling_bits() & self.handling;
        let log = self.handling & kind.log_bit() != 0 && self.approves(|d| d.should_log(failure, mask));
        let handle = self.handling & kind.handle_bit() != 0 && self.approves(|d| d.should_handle(failure, mask));
        let halt = self.hanging & kind.hang_bit() != 0;
        Decision { handle, log, halt }
    }

    /// True when there is no delegate or the delegate says yes.
    fn approves<F>(&self, ask: F) -> bool
    where
        F: Fn(&dyn ExceptionHandlerDelegate) -> bool,
    {
        match self.delegate.as_deref() {
            Some(d) => ask(d),
            None => true,
        }
    }
}

/// The process-wide handler. It starts with an empty mask and no delegate. A caller that holds the lock while it
/// decides must not call back into the handler.
pub fn default_handler() -> &'static Mutex<ExceptionHandler> {
    static HANDLER: OnceLock<Mutex<ExceptionHandler>> = OnceLock::new();
    HANDLER.get_or_init(|| Mutex::new(ExceptionHandler::new()))
}

/// Crash records in `dir` (crash.log, then crash.log.old), newest first. The shell keeps them in its own data
/// directory and does not grant apps that directory, so an app reads records only from a directory it is given.
pub fn crash_records(dir: &Path) -> Vec<CrashEntry> {
    diag::load(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Refuse {
        handle: bool,
        log: bool,
    }

    impl ExceptionHandlerDelegate for Refuse {
        fn should_handle(&self, _failure: &Failure, _mask: u32) -> bool {
            self.handle
        }

        fn should_log(&self, _failure: &Failure, _mask: u32) -> bool {
            self.log
        }
    }

    /// Approves logging only when the mask it receives is exactly the top-level log bit.
    struct OnlyTopLevelLog;

    impl ExceptionHandlerDelegate for OnlyTopLevelLog {
        fn should_log(&self, _failure: &Failure, mask: u32) -> bool {
            mask == handling::LOG_TOP_LEVEL
        }
    }

    fn uncaught() -> Failure {
        Failure::new(FailureKind::Uncaught, "boom", Some("src/a.rs:3:9"))
    }

    #[test]
    fn bits_match_the_named_constants() {
        assert_eq!(FailureKind::Uncaught.log_bit(), handling::LOG_UNCAUGHT);
        assert_eq!(FailureKind::Uncaught.handle_bit(), handling::HANDLE_UNCAUGHT);
        assert_eq!(FailureKind::UncaughtSystem.log_bit(), handling::LOG_UNCAUGHT_SYSTEM);
        assert_eq!(FailureKind::UncaughtRuntime.handle_bit(), handling::HANDLE_UNCAUGHT_RUNTIME);
        assert_eq!(FailureKind::TopLevel.log_bit(), handling::LOG_TOP_LEVEL);
        assert_eq!(FailureKind::Other.handle_bit(), handling::HANDLE_OTHER);
        assert_eq!(FailureKind::Other.hang_bit(), hanging::OTHER);
        assert_eq!(FailureKind::UncaughtSystem.hang_bit(), 2);
        assert_eq!(handling::ALL, 0b11_1111_1111);
        assert_eq!(hanging::ALL, 0b1_1111);
    }

    #[test]
    fn empty_mask_monitors_nothing() {
        let h = ExceptionHandler::new();
        assert_eq!(h.decide(&uncaught()), Decision { handle: false, log: false, halt: false });
    }

    #[test]
    fn log_bit_alone_logs_without_handling() {
        let mut h = ExceptionHandler::new();
        h.set_handling_mask(handling::LOG_UNCAUGHT);
        assert_eq!(h.decide(&uncaught()), Decision { handle: false, log: true, halt: false });
        let other = Failure::new(FailureKind::Other, "x", None);
        assert_eq!(h.decide(&other), Decision { handle: false, log: false, halt: false });
    }

    #[test]
    fn unmasked_mask_bits_are_dropped() {
        let mut h = ExceptionHandler::new();
        h.set_handling_mask(u32::MAX);
        assert_eq!(h.handling_mask(), handling::ALL);
        h.set_hanging_mask(u32::MAX);
        assert_eq!(h.hanging_mask(), hanging::ALL);
    }

    #[test]
    fn delegate_can_refuse_handling_but_not_logging() {
        let mut h = ExceptionHandler::new();
        h.set_handling_mask(handling::ALL);
        h.set_delegate(Some(Box::new(Refuse { handle: false, log: true })));
        assert!(h.delegate().is_some());
        assert_eq!(h.decide(&uncaught()), Decision { handle: false, log: true, halt: false });
    }

    #[test]
    fn delegate_sees_only_the_bits_of_its_kind() {
        let mut h = ExceptionHandler::new();
        h.set_handling_mask(handling::LOG_TOP_LEVEL | handling::HANDLE_OTHER);
        h.set_delegate(Some(Box::new(OnlyTopLevelLog)));
        let top = Failure::new(FailureKind::TopLevel, "t", None);
        assert!(h.decide(&top).log, "mask passed is LOG_TOP_LEVEL alone");
        let inner = Failure::new(FailureKind::Other, "o", None);
        assert!(!h.decide(&inner).log, "the other kind is not logged: its log bit is off");
    }

    #[test]
    fn handle_without_delegate_follows_the_mask() {
        let mut h = ExceptionHandler::new();
        h.set_handling_mask(handling::HANDLE_UNCAUGHT_SYSTEM);
        let sys = Failure::new(FailureKind::UncaughtSystem, "fault", None);
        assert_eq!(h.decide(&sys), Decision { handle: true, log: false, halt: false });
    }

    #[test]
    fn halt_follows_the_hanging_mask_alone() {
        let mut h = ExceptionHandler::new();
        h.set_hanging_mask(hanging::OTHER | hanging::UNCAUGHT);
        let other = Failure::new(FailureKind::Other, "o", None);
        assert!(h.decide(&other).halt);
        assert!(h.decide(&uncaught()).halt);
        let top = Failure::new(FailureKind::TopLevel, "t", None);
        assert!(!h.decide(&top).halt);
    }

    #[test]
    fn default_handler_is_one_shared_instance() {
        let a = default_handler() as *const _;
        let b = default_handler() as *const _;
        assert_eq!(a, b);
        assert_eq!(default_handler().lock().unwrap().handling_mask(), 0);
    }

    #[test]
    fn crash_records_reads_the_shell_log_format() {
        let dir = tempfile::tempdir().unwrap();
        let text = "[1700000000 12:00:00] thread 'main': panic: boom at src/a.rs:3:9\njunk line\n";
        std::fs::write(dir.path().join("crash.log"), text).unwrap();
        let records = crash_records(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].unix, 1700000000);
        assert_eq!(records[0].signature, "src/a.rs:3 boom");
        assert!(crash_records(&dir.path().join("missing")).is_empty());
    }
}
