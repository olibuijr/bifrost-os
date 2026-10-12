//! DNS-SD operation references: the lifecycle of one browse, resolve, register or query (std only).
//!
//! Each DNS-SD call returns a reference. The caller collects replies by asking the reference to process its pending
//! result, and frees the reference when it is done. This module models that lifecycle with a queue of replies: the
//! daemon layer pushes each reply it receives, the app pulls them one at a time, and a deallocated reference refuses
//! further work. It opens no socket and talks to no daemon. The reference numbers stand in for the socket descriptor,
//! so a caller can tell references apart; they are not file descriptors.

use std::collections::VecDeque;
use std::fmt;

/// The DNS-SD operation a reference was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Browse,
    Resolve,
    Register,
    QueryRecord,
    GetAddrInfo,
    EnumerateDomains,
    NatPortMapping,
}

/// Why a reference refused an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefError {
    /// The reference was deallocated, so it can no longer receive or process replies.
    Deallocated,
}

impl fmt::Display for RefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefError::Deallocated => write!(f, "the DNS-SD reference was deallocated"),
        }
    }
}

/// One operation's reference. `T` is the reply type the operation produces.
#[derive(Debug)]
pub struct Ref<T> {
    kind: Kind,
    number: u32,
    pending: VecDeque<T>,
    open: bool,
}

/// Hands out reference numbers, one per reference, starting at 1 (0 is never used).
#[derive(Debug, Default)]
pub struct Registry {
    next: u32,
}

impl Registry {
    /// Creates a registry whose first reference is numbered 1.
    pub fn new() -> Self {
        Registry { next: 0 }
    }

    /// Opens a reference for an operation with a fresh number.
    pub fn open<T>(&mut self, kind: Kind) -> Ref<T> {
        self.next += 1;
        Ref { kind, number: self.next, pending: VecDeque::new(), open: true }
    }
}

impl<T> Ref<T> {
    /// The operation this reference was opened for.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The reference number, or `None` once the reference is deallocated. Stands in for the socket descriptor.
    pub fn number(&self) -> Option<u32> {
        if self.open {
            Some(self.number)
        } else {
            None
        }
    }

    /// Whether the reference can still receive and process replies.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Number of replies waiting to be processed.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Queues a reply the daemon sent for this operation.
    pub fn push(&mut self, reply: T) -> Result<(), RefError> {
        if !self.open {
            return Err(RefError::Deallocated);
        }
        self.pending.push_back(reply);
        Ok(())
    }

    /// Takes the oldest pending reply. `Ok(None)` means nothing is waiting, where the C call would block.
    pub fn process_result(&mut self) -> Result<Option<T>, RefError> {
        if !self.open {
            return Err(RefError::Deallocated);
        }
        Ok(self.pending.pop_front())
    }

    /// Frees the reference. Replies still pending are dropped and the number is retired. Returns how many were dropped.
    /// Deallocating twice is a no-op that drops nothing.
    pub fn deallocate(&mut self) -> usize {
        if !self.open {
            return 0;
        }
        self.open = false;
        let dropped = self.pending.len();
        self.pending.clear();
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_come_back_in_the_order_they_were_pushed() {
        let mut reg = Registry::new();
        let mut r: Ref<&str> = reg.open(Kind::Browse);
        r.push("a").unwrap();
        r.push("b").unwrap();
        assert_eq!(r.pending(), 2);
        assert_eq!(r.process_result(), Ok(Some("a")));
        assert_eq!(r.process_result(), Ok(Some("b")));
        assert_eq!(r.process_result(), Ok(None));
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn each_reference_gets_its_own_number_starting_at_one() {
        let mut reg = Registry::new();
        let a: Ref<u8> = reg.open(Kind::Resolve);
        let b: Ref<u8> = reg.open(Kind::Register);
        assert_eq!(a.number(), Some(1));
        assert_eq!(b.number(), Some(2));
        assert_eq!(a.kind(), Kind::Resolve);
        assert_eq!(b.kind(), Kind::Register);
    }

    #[test]
    fn deallocate_drops_pending_replies_and_retires_the_number() {
        let mut reg = Registry::new();
        let mut r: Ref<u32> = reg.open(Kind::QueryRecord);
        r.push(10).unwrap();
        r.push(20).unwrap();
        assert_eq!(r.deallocate(), 2);
        assert!(!r.is_open());
        assert_eq!(r.number(), None);
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn a_deallocated_reference_refuses_pushes_and_processing() {
        let mut reg = Registry::new();
        let mut r: Ref<u32> = reg.open(Kind::GetAddrInfo);
        r.deallocate();
        assert_eq!(r.push(1), Err(RefError::Deallocated));
        assert_eq!(r.process_result(), Err(RefError::Deallocated));
        assert_eq!(format!("{}", RefError::Deallocated), "the DNS-SD reference was deallocated");
    }

    #[test]
    fn deallocating_twice_drops_nothing_the_second_time() {
        let mut reg = Registry::new();
        let mut r: Ref<u32> = reg.open(Kind::EnumerateDomains);
        r.push(7).unwrap();
        assert_eq!(r.deallocate(), 1);
        assert_eq!(r.deallocate(), 0);
        assert!(!r.is_open());
    }

    #[test]
    fn numbers_keep_counting_after_a_reference_is_freed() {
        let mut reg = Registry::new();
        let mut a: Ref<u8> = reg.open(Kind::NatPortMapping);
        a.deallocate();
        let b: Ref<u8> = reg.open(Kind::Browse);
        assert_eq!(b.number(), Some(2));
    }
}
