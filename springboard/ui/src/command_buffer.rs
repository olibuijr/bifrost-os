//! The life of a GPU command buffer as a pure state machine: passes are recorded, ended, the buffer is committed, then
//! the queue schedules and completes it. The model holds no GPU state. It refuses the orders of calls that a GPU queue
//! would reject: a pass opened inside another pass, a commit with a pass still open, and a second commit.
//!
//! States run Recording, then Committed, then Scheduled, then Completed, and only forward. A buffer can be enqueued once,
//! while it is still Recording, to reserve its place in the queue. Passes are recorded only while Recording, one at a
//! time, and each ended pass is kept in the order it was recorded.

/// Where a buffer is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Recording,
    Committed,
    Scheduled,
    Completed,
}

/// The kind of work a pass does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassKind {
    Render,
    Compute,
    Blit,
}

/// Why a call is refused in the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// The call does not apply in this state.
    WrongState { state: State },
    /// A pass is already open.
    PassOpen,
    /// No pass is open to end.
    NoPassOpen,
    /// The buffer already holds its place in the queue.
    AlreadyEnqueued,
}

/// One command buffer. Create it with `new`, record passes, then commit and step it through the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandBuffer {
    state: State,
    enqueued: bool,
    open: Option<PassKind>,
    passes: Vec<PassKind>,
}

impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandBuffer {
    /// An empty buffer that is recording.
    pub fn new() -> Self {
        CommandBuffer { state: State::Recording, enqueued: false, open: None, passes: Vec::new() }
    }

    /// The current state.
    pub fn state(&self) -> State {
        self.state
    }

    /// True once the buffer has reserved its place in the queue.
    pub fn enqueued(&self) -> bool {
        self.enqueued
    }

    /// The ended passes, in the order they were recorded.
    pub fn passes(&self) -> &[PassKind] {
        &self.passes
    }

    /// Reserves the buffer's place in the queue. Only while recording, and only once.
    pub fn enqueue(&mut self) -> Result<(), CommandError> {
        self.require(State::Recording)?;
        if self.enqueued {
            return Err(CommandError::AlreadyEnqueued);
        }
        self.enqueued = true;
        Ok(())
    }

    /// Opens a pass. Only while recording, and only when no other pass is open.
    pub fn begin_pass(&mut self, kind: PassKind) -> Result<(), CommandError> {
        self.require(State::Recording)?;
        if self.open.is_some() {
            return Err(CommandError::PassOpen);
        }
        self.open = Some(kind);
        Ok(())
    }

    /// Ends the open pass and keeps it in the list of passes.
    pub fn end_pass(&mut self) -> Result<(), CommandError> {
        let kind = self.open.take().ok_or(CommandError::NoPassOpen)?;
        self.passes.push(kind);
        Ok(())
    }

    /// Submits the buffer. Only while recording, and never with a pass still open.
    pub fn commit(&mut self) -> Result<(), CommandError> {
        self.require(State::Recording)?;
        if self.open.is_some() {
            return Err(CommandError::PassOpen);
        }
        self.state = State::Committed;
        Ok(())
    }

    /// The queue has scheduled the committed buffer to run.
    pub fn schedule(&mut self) -> Result<(), CommandError> {
        self.require(State::Committed)?;
        self.state = State::Scheduled;
        Ok(())
    }

    /// The scheduled buffer has finished running.
    pub fn complete(&mut self) -> Result<(), CommandError> {
        self.require(State::Scheduled)?;
        self.state = State::Completed;
        Ok(())
    }

    fn require(&self, wanted: State) -> Result<(), CommandError> {
        if self.state == wanted {
            Ok(())
        } else {
            Err(CommandError::WrongState { state: self.state })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_buffer_is_recording_and_empty() {
        let cb = CommandBuffer::new();
        assert_eq!(cb.state(), State::Recording);
        assert!(!cb.enqueued());
        assert!(cb.passes().is_empty());
    }

    #[test]
    fn ended_passes_keep_their_recorded_order() {
        let mut cb = CommandBuffer::new();
        cb.begin_pass(PassKind::Render).unwrap();
        cb.end_pass().unwrap();
        cb.begin_pass(PassKind::Compute).unwrap();
        cb.end_pass().unwrap();
        assert_eq!(cb.passes(), &[PassKind::Render, PassKind::Compute]);
    }

    #[test]
    fn a_pass_cannot_open_inside_another() {
        let mut cb = CommandBuffer::new();
        cb.begin_pass(PassKind::Blit).unwrap();
        assert_eq!(cb.begin_pass(PassKind::Render), Err(CommandError::PassOpen));
    }

    #[test]
    fn ending_with_no_open_pass_is_refused() {
        let mut cb = CommandBuffer::new();
        assert_eq!(cb.end_pass(), Err(CommandError::NoPassOpen));
    }

    #[test]
    fn commit_waits_for_the_open_pass_to_end() {
        let mut cb = CommandBuffer::new();
        cb.begin_pass(PassKind::Compute).unwrap();
        assert_eq!(cb.commit(), Err(CommandError::PassOpen));
        cb.end_pass().unwrap();
        cb.commit().unwrap();
        assert_eq!(cb.state(), State::Committed);
    }

    #[test]
    fn the_queue_steps_run_forward_only() {
        let mut cb = CommandBuffer::new();
        assert_eq!(cb.schedule(), Err(CommandError::WrongState { state: State::Recording }));
        cb.commit().unwrap();
        assert_eq!(cb.commit(), Err(CommandError::WrongState { state: State::Committed }));
        cb.schedule().unwrap();
        cb.complete().unwrap();
        assert_eq!(cb.state(), State::Completed);
        assert_eq!(cb.complete(), Err(CommandError::WrongState { state: State::Completed }));
    }

    #[test]
    fn enqueue_happens_once_and_only_while_recording() {
        let mut cb = CommandBuffer::new();
        cb.enqueue().unwrap();
        assert!(cb.enqueued());
        assert_eq!(cb.enqueue(), Err(CommandError::AlreadyEnqueued));
        cb.commit().unwrap();
        assert_eq!(cb.enqueue(), Err(CommandError::WrongState { state: State::Committed }));
    }
}
