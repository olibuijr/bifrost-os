//! Extension processes: the host-side table of processes that run app extensions, and the connection each one serves.
//!
//! A host asks for the process of an extension by its bundle identifier and point ([`Key`]). The table reuses a live
//! process for that key, or records a new one in [`State::Launching`]. The launcher starts the OS process and reports
//! back with [`ProcessTable::running`]; only a running process takes a [`ConnectionId`]. A connection is a pair of
//! in-memory message queues, host to extension and back. Closing a connection, or the process that owns it, drops the
//! messages still queued. This table starts no OS process itself: the shell's launcher does that.

use std::collections::VecDeque;

/// Which extension a process runs: the package's bundle identifier and the point it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub bundle_id: String,
    pub point: String,
}

impl Key {
    pub fn new(bundle_id: &str, point: &str) -> Key {
        Key { bundle_id: bundle_id.into(), point: point.into() }
    }
}

/// Where a process is in its life. A process leaves the table when it exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Launching,
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No process with this id is in the table (it never was, or it exited).
    NoProcess,
    /// The process is still launching; it takes no connections yet.
    NotRunning,
    /// No connection with this id is open (it was closed, or its process exited).
    NoConnection,
}

struct Process {
    id: ProcessId,
    key: Key,
    state: State,
}

struct Connection {
    id: ConnectionId,
    process: ProcessId,
    to_extension: VecDeque<Vec<u8>>,
    to_host: VecDeque<Vec<u8>>,
}

#[derive(Default)]
pub struct ProcessTable {
    processes: Vec<Process>,
    connections: Vec<Connection>,
    last_id: u32,
}

impl ProcessTable {
    pub fn new() -> Self {
        ProcessTable::default()
    }

    fn next_id(&mut self) -> u32 {
        self.last_id += 1;
        self.last_id
    }

    /// The live process for `key`, reused whether it is launching or running; otherwise a new one in `Launching`.
    pub fn acquire(&mut self, key: &Key) -> ProcessId {
        if let Some(p) = self.processes.iter().find(|p| &p.key == key) {
            return p.id;
        }
        let id = ProcessId(self.next_id());
        self.processes.push(Process { id, key: key.clone(), state: State::Launching });
        id
    }

    pub fn state(&self, id: ProcessId) -> Option<State> {
        self.processes.iter().find(|p| p.id == id).map(|p| p.state)
    }

    /// The launcher reports that the process is up and may take connections.
    pub fn running(&mut self, id: ProcessId) -> Result<(), Error> {
        let p = self.processes.iter_mut().find(|p| p.id == id).ok_or(Error::NoProcess)?;
        p.state = State::Running;
        Ok(())
    }

    /// Opens a connection to a running process.
    pub fn connect(&mut self, id: ProcessId) -> Result<ConnectionId, Error> {
        if self.state(id).ok_or(Error::NoProcess)? != State::Running {
            return Err(Error::NotRunning);
        }
        let conn = ConnectionId(self.next_id());
        self.connections.push(Connection { id: conn, process: id, to_extension: VecDeque::new(), to_host: VecDeque::new() });
        Ok(conn)
    }

    fn connection_mut(&mut self, conn: ConnectionId) -> Result<&mut Connection, Error> {
        self.connections.iter_mut().find(|c| c.id == conn).ok_or(Error::NoConnection)
    }

    /// Host side: queues a message for the extension.
    pub fn send(&mut self, conn: ConnectionId, message: Vec<u8>) -> Result<(), Error> {
        self.connection_mut(conn)?.to_extension.push_back(message);
        Ok(())
    }

    /// Extension side: the next message from the host, oldest first.
    pub fn take_inbound(&mut self, conn: ConnectionId) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.connection_mut(conn)?.to_extension.pop_front())
    }

    /// Extension side: queues a reply for the host.
    pub fn reply(&mut self, conn: ConnectionId, message: Vec<u8>) -> Result<(), Error> {
        self.connection_mut(conn)?.to_host.push_back(message);
        Ok(())
    }

    /// Host side: the next reply, oldest first.
    pub fn receive(&mut self, conn: ConnectionId) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.connection_mut(conn)?.to_host.pop_front())
    }

    /// Closes one connection and drops its queued messages. The process keeps running.
    pub fn invalidate(&mut self, conn: ConnectionId) -> Result<(), Error> {
        let before = self.connections.len();
        self.connections.retain(|c| c.id != conn);
        if self.connections.len() == before {
            return Err(Error::NoConnection);
        }
        Ok(())
    }

    /// The process exited or was killed: it leaves the table with all its connections.
    pub fn exit(&mut self, id: ProcessId) -> Result<(), Error> {
        let before = self.processes.len();
        self.processes.retain(|p| p.id != id);
        if self.processes.len() == before {
            return Err(Error::NoProcess);
        }
        self.connections.retain(|c| c.process != id);
        Ok(())
    }

    pub fn connection_count(&self, id: ProcessId) -> usize {
        self.connections.iter().filter(|c| c.process == id).count()
    }

    pub fn live_processes(&self) -> usize {
        self.processes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Key {
        Key::new("com.other.clip", "com.example.notes.share")
    }

    #[test]
    fn a_key_reuses_its_live_process_and_other_keys_get_their_own() {
        let mut t = ProcessTable::new();
        let first = t.acquire(&clip());
        assert_eq!(t.acquire(&clip()), first);
        let other = t.acquire(&Key::new("com.other.pdf", "com.example.notes.share"));
        assert_ne!(first, other);
        assert_eq!(t.live_processes(), 2);
        assert_eq!(t.state(first), Some(State::Launching));
    }

    #[test]
    fn a_launching_process_takes_no_connection_until_it_is_running() {
        let mut t = ProcessTable::new();
        let id = t.acquire(&clip());
        assert_eq!(t.connect(id), Err(Error::NotRunning));
        t.running(id).unwrap();
        assert_eq!(t.state(id), Some(State::Running));
        assert!(t.connect(id).is_ok());
        assert_eq!(t.connect(ProcessId(99)), Err(Error::NoProcess));
    }

    #[test]
    fn messages_cross_the_connection_in_order_each_way() {
        let mut t = ProcessTable::new();
        let id = t.acquire(&clip());
        t.running(id).unwrap();
        let c = t.connect(id).unwrap();

        t.send(c, b"one".to_vec()).unwrap();
        t.send(c, b"two".to_vec()).unwrap();
        assert_eq!(t.take_inbound(c).unwrap(), Some(b"one".to_vec()));
        assert_eq!(t.take_inbound(c).unwrap(), Some(b"two".to_vec()));
        assert_eq!(t.take_inbound(c).unwrap(), None);

        t.reply(c, b"ack".to_vec()).unwrap();
        assert_eq!(t.receive(c).unwrap(), Some(b"ack".to_vec()));
        assert_eq!(t.receive(c).unwrap(), None);
    }

    #[test]
    fn invalidating_one_connection_leaves_the_others_and_the_process() {
        let mut t = ProcessTable::new();
        let id = t.acquire(&clip());
        t.running(id).unwrap();
        let a = t.connect(id).unwrap();
        let b = t.connect(id).unwrap();
        t.send(a, b"queued".to_vec()).unwrap();

        t.invalidate(a).unwrap();
        assert_eq!(t.send(a, vec![]), Err(Error::NoConnection));
        assert_eq!(t.connection_count(id), 1);
        t.send(b, b"still open".to_vec()).unwrap();
        assert_eq!(t.take_inbound(b).unwrap(), Some(b"still open".to_vec()));
        assert_eq!(t.invalidate(a), Err(Error::NoConnection));
        assert_eq!(t.live_processes(), 1);
    }

    #[test]
    fn an_exited_process_drops_its_connections_and_the_next_acquire_starts_fresh() {
        let mut t = ProcessTable::new();
        let old = t.acquire(&clip());
        t.running(old).unwrap();
        let c = t.connect(old).unwrap();
        t.exit(old).unwrap();

        assert_eq!(t.take_inbound(c), Err(Error::NoConnection));
        assert_eq!(t.state(old), None);
        assert_eq!(t.exit(old), Err(Error::NoProcess));
        let fresh = t.acquire(&clip());
        assert_ne!(fresh, old);
        assert_eq!(t.state(fresh), Some(State::Launching));
    }
}
