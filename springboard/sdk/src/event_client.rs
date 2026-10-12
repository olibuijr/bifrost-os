//! Monitoring client lifecycle (parity row "Client creation and teardown"). A client is a handle that a root-run
//! monitor opens to receive system events and closes when it is done. This module is the bookkeeping only: it decides
//! whether a caller may open a client, hands out handles, and forgets them on teardown. It opens no socket, reads no
//! process table and starts no thread. The error names mirror the reference result set for a new client; the order
//! in which the checks run is BifrOSt's own.

use std::collections::BTreeSet;

/// Who asks for a client. The caller fills this in from its own credentials, manifest and policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub pid: u32,
    pub uid: u32,
    /// The caller's manifest grants the monitoring permission.
    pub entitled: bool,
    /// Policy (an administrator's allow list, for example) lets this caller monitor.
    pub permitted: bool,
}

/// Why a client was not created. Mirrors the reference result set; success is `Ok` in `open_client`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    Internal,
    InvalidArgument,
    NotEntitled,
    NotPermitted,
    NotPrivileged,
    TooManyClients,
}

impl OpenError {
    /// Stable lower-case name, used in logs and in the control socket.
    pub fn name(self) -> &'static str {
        match self {
            OpenError::Internal => "internal",
            OpenError::InvalidArgument => "invalid-argument",
            OpenError::NotEntitled => "not-entitled",
            OpenError::NotPermitted => "not-permitted",
            OpenError::NotPrivileged => "not-privileged",
            OpenError::TooManyClients => "too-many-clients",
        }
    }
}

/// An opaque handle to an open client. Handles are never reused within one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(u32);

impl ClientId {
    pub fn get(self) -> u32 {
        self.0
    }
}

/// The open clients of one monitor, with a limit on how many may be open at once.
#[derive(Debug)]
pub struct ClientTable {
    max_clients: usize,
    next_id: u32,
    open: BTreeSet<ClientId>,
}

impl ClientTable {
    pub fn new(max_clients: usize) -> Self {
        Self { max_clients, next_id: 1, open: BTreeSet::new() }
    }

    pub fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Opens a client for `caller`. The checks run in this order: a zero pid is an invalid argument; a caller
    /// without the entitlement is refused; then one without policy permission; then one that is not root; then a
    /// caller once the table is full. When handles run out, the result is `Internal`.
    pub fn open_client(&mut self, caller: Caller) -> Result<ClientId, OpenError> {
        if caller.pid == 0 {
            return Err(OpenError::InvalidArgument);
        }
        if !caller.entitled {
            return Err(OpenError::NotEntitled);
        }
        if !caller.permitted {
            return Err(OpenError::NotPermitted);
        }
        if caller.uid != 0 {
            return Err(OpenError::NotPrivileged);
        }
        if self.open.len() >= self.max_clients {
            return Err(OpenError::TooManyClients);
        }
        let Some(next) = self.next_id.checked_add(1) else {
            return Err(OpenError::Internal);
        };
        let id = ClientId(self.next_id);
        self.next_id = next;
        self.open.insert(id);
        Ok(id)
    }

    /// Tears one client down. Returns false when the handle is unknown or already closed.
    pub fn delete_client(&mut self, id: ClientId) -> bool {
        self.open.remove(&id)
    }

    /// Tears every client down, as when the monitor shuts down. Returns how many were closed.
    pub fn delete_all(&mut self) -> usize {
        let closed = self.open.len();
        self.open.clear();
        closed
    }

    pub fn is_open(&self, id: ClientId) -> bool {
        self.open.contains(&id)
    }

    /// The number of clients currently open.
    pub fn count(&self) -> usize {
        self.open.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(pid: u32) -> Caller {
        Caller { pid, uid: 0, entitled: true, permitted: true }
    }

    #[test]
    fn a_root_caller_with_entitlement_and_permission_gets_a_handle() {
        let mut t = ClientTable::new(4);
        let id = t.open_client(root(10)).unwrap();
        assert_eq!(id.get(), 1);
        assert!(t.is_open(id));
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn checks_run_in_order_and_report_the_first_failure() {
        let mut t = ClientTable::new(4);
        let bare = Caller { pid: 10, uid: 1000, entitled: false, permitted: false };
        assert_eq!(t.open_client(bare), Err(OpenError::NotEntitled));
        let entitled = Caller { entitled: true, ..bare };
        assert_eq!(t.open_client(entitled), Err(OpenError::NotPermitted));
        let permitted = Caller { permitted: true, ..entitled };
        assert_eq!(t.open_client(permitted), Err(OpenError::NotPrivileged));
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn a_zero_pid_is_an_invalid_argument_before_any_other_check() {
        let mut t = ClientTable::new(4);
        let kernel = Caller { pid: 0, uid: 1000, entitled: false, permitted: false };
        assert_eq!(t.open_client(kernel), Err(OpenError::InvalidArgument));
    }

    #[test]
    fn the_table_refuses_a_client_past_its_limit_and_ids_are_not_reused() {
        let mut t = ClientTable::new(1);
        let first = t.open_client(root(10)).unwrap();
        assert_eq!(t.open_client(root(11)), Err(OpenError::TooManyClients));
        assert!(t.delete_client(first));
        let second = t.open_client(root(11)).unwrap();
        assert_eq!(second.get(), 2, "a closed handle is never handed out again");
        assert_eq!(t.max_clients(), 1);
    }

    #[test]
    fn deleting_a_client_twice_or_an_unknown_one_returns_false() {
        let mut t = ClientTable::new(4);
        let id = t.open_client(root(10)).unwrap();
        assert!(t.delete_client(id));
        assert!(!t.delete_client(id));
        assert!(!t.delete_client(ClientId(99)));
        assert!(!t.is_open(id));
    }

    #[test]
    fn delete_all_closes_every_client_and_reports_the_count() {
        let mut t = ClientTable::new(4);
        t.open_client(root(10)).unwrap();
        t.open_client(root(11)).unwrap();
        assert_eq!(t.delete_all(), 2);
        assert_eq!(t.count(), 0);
        assert_eq!(t.delete_all(), 0);
    }

    #[test]
    fn exhausted_handles_are_an_internal_error_and_names_are_stable() {
        let mut t = ClientTable::new(4);
        t.next_id = u32::MAX;
        assert_eq!(t.open_client(root(10)), Err(OpenError::Internal));
        assert_eq!(t.count(), 0);
        assert_eq!(OpenError::Internal.name(), "internal");
        assert_eq!(OpenError::NotPrivileged.name(), "not-privileged");
        assert_eq!(OpenError::TooManyClients.name(), "too-many-clients");
    }
}
