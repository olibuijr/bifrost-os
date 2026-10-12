//! A fixed-size list of reusable GPU commands that are encoded once and executed later, as a pure model.
//!
//! The list holds `size` slots of one kind: all draw slots or all dispatch slots, chosen when it is made. A slot is
//! encoded by index, can be reset to empty, and a range of slots is executed in index order. Execution is checked as
//! a whole first, so a range that holds an empty slot runs nothing. The model holds no GPU state and submits nothing.

use std::ops::Range;

/// The kind of command a list holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Draw,
    Dispatch,
}

/// One encoded command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Draws `vertex_count` vertices starting at `first_vertex`.
    Draw { first_vertex: u32, vertex_count: u32 },
    /// Runs a compute grid of `groups` workgroups in x, y and z.
    Dispatch { groups: [u32; 3] },
}

impl Command {
    /// The kind this command belongs to.
    pub fn kind(&self) -> CommandKind {
        match self {
            Command::Draw { .. } => CommandKind::Draw,
            Command::Dispatch { .. } => CommandKind::Dispatch,
        }
    }
}

/// Why a call is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndirectError {
    /// The slot index is past the end of the list.
    OutOfRange { index: usize, size: usize },
    /// A command of the other kind was encoded into this list.
    WrongKind { expected: CommandKind },
    /// A range runs past the end of the list, or starts after it ends.
    BadRange { start: usize, end: usize, size: usize },
    /// A slot in the executed range has never been encoded.
    Unencoded { index: usize },
}

/// A list of `size` slots, all of one kind. Slots start empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndirectCommandList {
    kind: CommandKind,
    slots: Vec<Option<Command>>,
}

impl IndirectCommandList {
    /// An empty list of `size` slots that holds commands of `kind`.
    pub fn new(kind: CommandKind, size: usize) -> Self {
        IndirectCommandList { kind, slots: vec![None; size] }
    }

    /// The kind of command the list holds.
    pub fn kind(&self) -> CommandKind {
        self.kind
    }

    /// The number of slots.
    pub fn size(&self) -> usize {
        self.slots.len()
    }

    /// The command in a slot, or None when the slot is empty.
    pub fn command(&self, index: usize) -> Option<Command> {
        self.slots.get(index).copied().flatten()
    }

    /// Encodes `command` into slot `index`, replacing what was there. Refused for a slot past the end or a command of
    /// the other kind.
    pub fn encode(&mut self, index: usize, command: Command) -> Result<(), IndirectError> {
        if command.kind() != self.kind {
            return Err(IndirectError::WrongKind { expected: self.kind });
        }
        let size = self.slots.len();
        let slot = self.slots.get_mut(index).ok_or(IndirectError::OutOfRange { index, size })?;
        *slot = Some(command);
        Ok(())
    }

    /// Resets the slots in `range` to empty. The range may end at the list size.
    pub fn reset(&mut self, range: Range<usize>) -> Result<(), IndirectError> {
        self.check_range(&range)?;
        for slot in &mut self.slots[range] {
            *slot = None;
        }
        Ok(())
    }

    /// The commands in `range`, in index order. Refused as a whole when the range is bad or holds an empty slot.
    pub fn execute(&self, range: Range<usize>) -> Result<Vec<Command>, IndirectError> {
        self.check_range(&range)?;
        let mut out = Vec::with_capacity(range.len());
        for index in range {
            match self.slots[index] {
                Some(command) => out.push(command),
                None => return Err(IndirectError::Unencoded { index }),
            }
        }
        Ok(out)
    }

    fn check_range(&self, range: &Range<usize>) -> Result<(), IndirectError> {
        let size = self.slots.len();
        if range.start > range.end || range.end > size {
            return Err(IndirectError::BadRange { start: range.start, end: range.end, size });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(first: u32, count: u32) -> Command {
        Command::Draw { first_vertex: first, vertex_count: count }
    }

    #[test]
    fn new_list_has_empty_slots_of_one_kind() {
        let list = IndirectCommandList::new(CommandKind::Draw, 4);
        assert_eq!(list.size(), 4);
        assert_eq!(list.kind(), CommandKind::Draw);
        assert_eq!(list.command(0), None);
        assert_eq!(list.command(4), None);
    }

    #[test]
    fn encode_then_execute_returns_commands_in_index_order() {
        let mut list = IndirectCommandList::new(CommandKind::Draw, 3);
        list.encode(2, draw(6, 3)).unwrap();
        list.encode(0, draw(0, 3)).unwrap();
        list.encode(1, draw(3, 3)).unwrap();
        let run = list.execute(0..3).unwrap();
        assert_eq!(run, vec![draw(0, 3), draw(3, 3), draw(6, 3)]);
    }

    #[test]
    fn encode_past_the_end_is_refused() {
        let mut list = IndirectCommandList::new(CommandKind::Draw, 2);
        assert_eq!(list.encode(2, draw(0, 1)), Err(IndirectError::OutOfRange { index: 2, size: 2 }));
    }

    #[test]
    fn a_dispatch_cannot_go_into_a_draw_list() {
        let mut list = IndirectCommandList::new(CommandKind::Draw, 2);
        let grid = Command::Dispatch { groups: [8, 1, 1] };
        assert_eq!(list.encode(0, grid), Err(IndirectError::WrongKind { expected: CommandKind::Draw }));
        assert_eq!(list.command(0), None);
    }

    #[test]
    fn execute_refuses_a_range_with_an_empty_slot_and_runs_nothing() {
        let mut list = IndirectCommandList::new(CommandKind::Dispatch, 3);
        list.encode(0, Command::Dispatch { groups: [1, 1, 1] }).unwrap();
        assert_eq!(list.execute(0..3), Err(IndirectError::Unencoded { index: 1 }));
        assert_eq!(list.execute(0..1).unwrap().len(), 1);
    }

    #[test]
    fn reset_empties_only_the_range_and_bad_ranges_are_refused() {
        let mut list = IndirectCommandList::new(CommandKind::Draw, 4);
        for i in 0..4 {
            list.encode(i, draw(i as u32, 1)).unwrap();
        }
        list.reset(1..3).unwrap();
        assert_eq!(list.command(0), Some(draw(0, 1)));
        assert_eq!(list.command(1), None);
        assert_eq!(list.command(2), None);
        assert_eq!(list.command(3), Some(draw(3, 1)));
        assert_eq!(list.reset(3..5), Err(IndirectError::BadRange { start: 3, end: 5, size: 4 }));
        assert_eq!(list.execute(2..1), Err(IndirectError::BadRange { start: 2, end: 1, size: 4 }));
    }
}
