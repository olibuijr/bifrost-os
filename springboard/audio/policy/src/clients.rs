//! The server's view of the applications that use audio: one process object per connected client, with the
//! application id the client said in its Hello, its pid (from the socket credentials), and the streams it has
//! running in each direction. This is what a process-list query answers ("which applications are playing or
//! recording right now") and what the menu-bar indicator and per-app volume key on.
//!
//! Pure bookkeeping: the daemon calls `connect` when a client says Hello, `stream_started` and `stream_stopped`
//! as streams run, and `disconnect` when the socket closes. Object ids are never reused within a run, so a client
//! that holds an id can tell its process went away.

use crate::sessions::Direction;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    /// The id clients use to refer to this process in the object model. Never reused.
    pub object_id: u32,
    pub app_id: String,
    pub pid: i32,
    output_streams: BTreeSet<u32>,
    input_streams: BTreeSet<u32>,
}

impl Process {
    pub fn is_running(&self, direction: Direction) -> bool {
        !self.streams(direction).is_empty()
    }

    fn streams(&self, direction: Direction) -> &BTreeSet<u32> {
        match direction {
            Direction::Output => &self.output_streams,
            Direction::Input => &self.input_streams,
        }
    }

    pub fn stream_count(&self, direction: Direction) -> usize {
        self.streams(direction).len()
    }
}

/// Why a call did not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownPid,
    /// The stream id is already running for another process.
    StreamTaken,
}

#[derive(Clone, Debug, Default)]
pub struct Applications {
    next_object: u32,
    by_pid: BTreeMap<i32, Process>,
    /// stream id -> pid
    owners: BTreeMap<u32, i32>,
}

impl Applications {
    pub fn new() -> Applications {
        Applications { next_object: 1, ..Default::default() }
    }

    /// A client said Hello. A second connection from the same pid returns the process it already has (an
    /// application may open several sockets) and keeps its first application id.
    pub fn connect(&mut self, app_id: &str, pid: i32) -> u32 {
        if let Some(p) = self.by_pid.get(&pid) {
            return p.object_id;
        }
        let object_id = self.next_object;
        self.next_object += 1;
        self.by_pid.insert(pid, Process { object_id, app_id: app_id.to_string(), pid, output_streams: BTreeSet::new(), input_streams: BTreeSet::new() });
        object_id
    }

    /// The process went away. Its streams are released; the process is returned for the last notice.
    pub fn disconnect(&mut self, pid: i32) -> Option<Process> {
        let p = self.by_pid.remove(&pid)?;
        self.owners.retain(|_, owner| *owner != pid);
        Some(p)
    }

    pub fn stream_started(&mut self, pid: i32, stream: u32, direction: Direction) -> Result<(), Error> {
        if matches!(self.owners.get(&stream), Some(owner) if *owner != pid) {
            return Err(Error::StreamTaken);
        }
        let p = self.by_pid.get_mut(&pid).ok_or(Error::UnknownPid)?;
        match direction {
            Direction::Output => p.output_streams.insert(stream),
            Direction::Input => p.input_streams.insert(stream),
        };
        self.owners.insert(stream, pid);
        Ok(())
    }

    /// A stream stopped; true when it was running.
    pub fn stream_stopped(&mut self, stream: u32) -> bool {
        let Some(pid) = self.owners.remove(&stream) else { return false };
        if let Some(p) = self.by_pid.get_mut(&pid) {
            p.output_streams.remove(&stream);
            p.input_streams.remove(&stream);
        }
        true
    }

    pub fn process(&self, pid: i32) -> Option<&Process> {
        self.by_pid.get(&pid)
    }

    pub fn by_object(&self, object_id: u32) -> Option<&Process> {
        self.by_pid.values().find(|p| p.object_id == object_id)
    }

    /// The processes in the order they connected.
    pub fn processes(&self) -> Vec<&Process> {
        let mut all: Vec<&Process> = self.by_pid.values().collect();
        all.sort_by_key(|p| p.object_id);
        all
    }

    /// The application id that owns a stream.
    pub fn app_for_stream(&self, stream: u32) -> Option<&str> {
        self.owners.get(&stream).and_then(|pid| self.by_pid.get(pid)).map(|p| p.app_id.as_str())
    }

    /// The distinct application ids with a stream running in `direction`, sorted. Several processes of one
    /// application count once.
    pub fn apps_running(&self, direction: Direction) -> Vec<&str> {
        let set: BTreeSet<&str> = self.by_pid.values().filter(|p| p.is_running(direction)).map(|p| p.app_id.as_str()).collect();
        set.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_gets_an_object_id_that_is_not_reused() {
        let mut a = Applications::new();
        let first = a.connect("is.olibuijr.music", 100);
        assert_eq!(a.connect("is.olibuijr.music", 100), first, "the same pid is the same process");
        let second = a.connect("is.olibuijr.notes", 200);
        assert_ne!(first, second);
        a.disconnect(100);
        let third = a.connect("is.olibuijr.music", 300);
        assert!(third > second, "ids only grow");
        assert!(a.by_object(first).is_none());
        assert_eq!(a.by_object(second).unwrap().app_id, "is.olibuijr.notes");
        let order: Vec<i32> = a.processes().iter().map(|p| p.pid).collect();
        assert_eq!(order, vec![200, 300]);
    }

    #[test]
    fn streams_make_a_process_running_in_that_direction() {
        let mut a = Applications::new();
        a.connect("player", 1);
        assert!(!a.process(1).unwrap().is_running(Direction::Output));
        a.stream_started(1, 10, Direction::Output).unwrap();
        a.stream_started(1, 11, Direction::Output).unwrap();
        a.stream_started(1, 12, Direction::Input).unwrap();
        let p = a.process(1).unwrap();
        assert_eq!((p.stream_count(Direction::Output), p.stream_count(Direction::Input)), (2, 1));
        assert_eq!(a.app_for_stream(11), Some("player"));
        assert!(a.stream_stopped(11));
        assert!(!a.stream_stopped(11), "a stopped stream is gone");
        assert!(a.process(1).unwrap().is_running(Direction::Output));
        assert!(a.stream_stopped(10));
        assert!(!a.process(1).unwrap().is_running(Direction::Output));
        assert!(a.process(1).unwrap().is_running(Direction::Input));
    }

    #[test]
    fn streams_belong_to_one_process_and_unknown_pids_are_refused() {
        let mut a = Applications::new();
        a.connect("a", 1);
        a.connect("b", 2);
        a.stream_started(1, 5, Direction::Output).unwrap();
        assert_eq!(a.stream_started(2, 5, Direction::Output), Err(Error::StreamTaken));
        assert_eq!(a.stream_started(9, 6, Direction::Output), Err(Error::UnknownPid));
        a.stream_started(1, 5, Direction::Output).unwrap();
    }

    #[test]
    fn disconnecting_releases_the_streams_and_apps_running_lists_each_app_once() {
        let mut a = Applications::new();
        a.connect("web", 1);
        a.connect("web", 2);
        a.connect("recorder", 3);
        a.stream_started(1, 1, Direction::Output).unwrap();
        a.stream_started(2, 2, Direction::Output).unwrap();
        a.stream_started(3, 3, Direction::Input).unwrap();
        assert_eq!(a.apps_running(Direction::Output), vec!["web"]);
        assert_eq!(a.apps_running(Direction::Input), vec!["recorder"]);
        let gone = a.disconnect(3).unwrap();
        assert_eq!(gone.app_id, "recorder");
        assert!(a.apps_running(Direction::Input).is_empty());
        assert_eq!(a.app_for_stream(3), None);
        assert!(a.disconnect(3).is_none());
    }
}
