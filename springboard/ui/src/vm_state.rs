//! Execution state of a guest virtual machine and the lifecycle operations allowed in each state.
//! The state names (stopped, running, paused, error, starting, pausing, stopping, resuming, restoring, saving) and
//! the operations (start, pause, resume, stop, request stop, save, restore) follow the official Virtualization
//! documentation for its execution state. The transition rules are BifrOSt's model of that lifecycle.
//! This is a pure model: it starts, pauses and stops nothing. The hypervisor layer drives it and reports results
//! through `complete`.

use std::fmt;

/// Execution state of a guest. A transitional state (starting, pausing, ...) lasts until `complete` is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmState {
    Stopped,
    Starting,
    Running,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Saving,
    Restoring,
    Error,
}

/// A lifecycle operation a caller asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmOperation {
    Start,
    Pause,
    Resume,
    Stop,
    RequestStop,
    SaveState,
    RestoreState,
}

/// Why a lifecycle call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmStateError {
    /// The operation is not allowed in the current state.
    InvalidState {
        operation: VmOperation,
        state: VmState,
    },
    /// `complete` was called while no transition is in progress.
    NoTransition,
}

impl fmt::Display for VmStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmStateError::InvalidState { operation, state } => {
                write!(f, "{:?} is not allowed while {:?}", operation, state)
            }
            VmStateError::NoTransition => f.write_str("no transition is in progress"),
        }
    }
}

impl VmState {
    /// Whether the state is a transition in progress that waits for `complete`.
    pub fn is_transitional(self) -> bool {
        matches!(
            self,
            VmState::Starting
                | VmState::Pausing
                | VmState::Resuming
                | VmState::Stopping
                | VmState::Saving
                | VmState::Restoring
        )
    }
}

/// Lifecycle model for one guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualMachine {
    state: VmState,
}

impl Default for VirtualMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualMachine {
    /// A new guest is stopped.
    pub fn new() -> Self {
        Self {
            state: VmState::Stopped,
        }
    }

    pub fn state(&self) -> VmState {
        self.state
    }

    /// Whether `operation` may be requested now. Nothing changes.
    /// Start and restore need a stopped guest; a failed guest may be started again. Pause, stop and request-stop
    /// need a running guest (stop also accepts a paused one). Save needs a paused guest.
    pub fn can(&self, operation: VmOperation) -> bool {
        match (operation, self.state) {
            (VmOperation::Start, VmState::Stopped) | (VmOperation::Start, VmState::Error) => true,
            (VmOperation::RestoreState, VmState::Stopped) => true,
            (VmOperation::Pause, VmState::Running) => true,
            (VmOperation::Resume, VmState::Paused) => true,
            (VmOperation::Stop, VmState::Running) | (VmOperation::Stop, VmState::Paused) => true,
            (VmOperation::RequestStop, VmState::Running) => true,
            (VmOperation::SaveState, VmState::Paused) => true,
            _ => false,
        }
    }

    /// Starts an operation. Returns the state the guest is in now. A transitional state waits for `complete`.
    /// `RequestStop` asks the guest and leaves the state unchanged.
    pub fn request(&mut self, operation: VmOperation) -> Result<VmState, VmStateError> {
        if !self.can(operation) {
            return Err(VmStateError::InvalidState {
                operation,
                state: self.state,
            });
        }
        self.state = match operation {
            VmOperation::Start => VmState::Starting,
            VmOperation::Pause => VmState::Pausing,
            VmOperation::Resume => VmState::Resuming,
            VmOperation::Stop => VmState::Stopping,
            VmOperation::RequestStop => self.state,
            VmOperation::SaveState => VmState::Saving,
            VmOperation::RestoreState => VmState::Restoring,
        };
        Ok(self.state)
    }

    /// Finishes the transition in progress. `ok` false moves the guest to `Error`.
    /// Starting and resuming reach running; pausing, saving and restoring reach paused; stopping reaches stopped.
    pub fn complete(&mut self, ok: bool) -> Result<VmState, VmStateError> {
        if !self.state.is_transitional() {
            return Err(VmStateError::NoTransition);
        }
        self.state = if !ok {
            VmState::Error
        } else {
            match self.state {
                VmState::Starting | VmState::Resuming => VmState::Running,
                VmState::Pausing | VmState::Saving | VmState::Restoring => VmState::Paused,
                _ => VmState::Stopped,
            }
        };
        Ok(self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_guest_is_stopped_and_only_start_is_allowed() {
        let vm = VirtualMachine::new();
        assert_eq!(vm.state(), VmState::Stopped);
        assert!(vm.can(VmOperation::Start));
        assert!(vm.can(VmOperation::RestoreState));
        assert!(!vm.can(VmOperation::Pause));
        assert!(!vm.can(VmOperation::Stop));
    }

    #[test]
    fn start_waits_for_completion_and_blocks_other_calls_meanwhile() {
        let mut vm = VirtualMachine::new();
        assert_eq!(vm.request(VmOperation::Start), Ok(VmState::Starting));
        assert!(!vm.can(VmOperation::Start));
        assert!(matches!(
            vm.request(VmOperation::Stop),
            Err(VmStateError::InvalidState {
                operation: VmOperation::Stop,
                state: VmState::Starting
            })
        ));
        assert_eq!(vm.complete(true), Ok(VmState::Running));
        assert_eq!(vm.state(), VmState::Running);
    }

    #[test]
    fn pause_and_resume_cycle_returns_to_running() {
        let mut vm = VirtualMachine::new();
        vm.request(VmOperation::Start).unwrap();
        vm.complete(true).unwrap();
        assert_eq!(vm.request(VmOperation::Pause), Ok(VmState::Pausing));
        assert_eq!(vm.complete(true), Ok(VmState::Paused));
        assert!(vm.can(VmOperation::Resume));
        assert_eq!(vm.request(VmOperation::Resume), Ok(VmState::Resuming));
        assert_eq!(vm.complete(true), Ok(VmState::Running));
    }

    #[test]
    fn failed_start_enters_error_and_can_be_started_again() {
        let mut vm = VirtualMachine::new();
        vm.request(VmOperation::Start).unwrap();
        assert_eq!(vm.complete(false), Ok(VmState::Error));
        assert!(!vm.can(VmOperation::Pause));
        assert!(vm.can(VmOperation::Start));
    }

    #[test]
    fn request_stop_asks_the_guest_and_keeps_running() {
        let mut vm = VirtualMachine::new();
        vm.request(VmOperation::Start).unwrap();
        vm.complete(true).unwrap();
        assert_eq!(vm.request(VmOperation::RequestStop), Ok(VmState::Running));
        assert!(vm.can(VmOperation::Stop));
        assert!(!vm.can(VmOperation::SaveState));
    }

    #[test]
    fn stop_from_paused_then_complete_reaches_stopped() {
        let mut vm = VirtualMachine::new();
        vm.request(VmOperation::Start).unwrap();
        vm.complete(true).unwrap();
        vm.request(VmOperation::Pause).unwrap();
        vm.complete(true).unwrap();
        assert_eq!(vm.request(VmOperation::Stop), Ok(VmState::Stopping));
        assert_eq!(vm.complete(true), Ok(VmState::Stopped));
    }

    #[test]
    fn complete_without_a_transition_is_refused() {
        let mut vm = VirtualMachine::new();
        assert_eq!(vm.complete(true), Err(VmStateError::NoTransition));
        assert_eq!(vm.state(), VmState::Stopped);
    }
}
