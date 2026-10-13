//! Audio processes (the reference's `AudioHardwareProcess`): the processes that use audio, with their pid, bundle
//! identifier and whether each one is running input or output. This is the pure model of the list: the caller
//! feeds it from the server's snapshot and keeps it current with `upsert` and `remove`. Entries are kept sorted by
//! pid, so lookups are a binary search.

/// One process that has, or had, audio IO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioProcess {
    pub pid: i32,
    pub bundle_id: String,
    pub running_input: bool,
    pub running_output: bool,
}

impl AudioProcess {
    /// True while the process runs input or output.
    pub fn is_running(&self) -> bool {
        self.running_input || self.running_output
    }
}

/// Why a process entry was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessError {
    /// Process ids are positive.
    InvalidPid(i32),
}

/// The audio processes, sorted by pid, one entry per pid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessList {
    processes: Vec<AudioProcess>,
}

impl ProcessList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.processes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    /// All entries in pid order.
    pub fn as_slice(&self) -> &[AudioProcess] {
        &self.processes
    }

    /// The entry for `pid`, if listed.
    pub fn get(&self, pid: i32) -> Option<&AudioProcess> {
        self.index_of(pid).ok().map(|i| &self.processes[i])
    }

    /// Inserts the entry, or replaces the entry with the same pid. Refuses a pid that is not positive.
    pub fn upsert(&mut self, process: AudioProcess) -> Result<(), ProcessError> {
        if process.pid <= 0 {
            return Err(ProcessError::InvalidPid(process.pid));
        }
        match self.index_of(process.pid) {
            Ok(i) => self.processes[i] = process,
            Err(i) => self.processes.insert(i, process),
        }
        Ok(())
    }

    /// Removes and returns the entry for `pid`.
    pub fn remove(&mut self, pid: i32) -> Option<AudioProcess> {
        match self.index_of(pid) {
            Ok(i) => Some(self.processes.remove(i)),
            Err(_) => None,
        }
    }

    /// Sets the input and output running flags of a listed process. Returns false when the pid is not listed.
    pub fn set_running(&mut self, pid: i32, input: bool, output: bool) -> bool {
        match self.index_of(pid) {
            Ok(i) => {
                self.processes[i].running_input = input;
                self.processes[i].running_output = output;
                true
            }
            Err(_) => false,
        }
    }

    /// The processes running input, in pid order.
    pub fn running_input(&self) -> impl Iterator<Item = &AudioProcess> {
        self.processes.iter().filter(|p| p.running_input)
    }

    /// The processes running output, in pid order.
    pub fn running_output(&self) -> impl Iterator<Item = &AudioProcess> {
        self.processes.iter().filter(|p| p.running_output)
    }

    fn index_of(&self, pid: i32) -> Result<usize, usize> {
        self.processes.binary_search_by_key(&pid, |p| p.pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, bundle: &str, input: bool, output: bool) -> AudioProcess {
        AudioProcess { pid, bundle_id: bundle.to_string(), running_input: input, running_output: output }
    }

    #[test]
    fn upsert_keeps_pid_order_and_replaces_same_pid() {
        let mut list = ProcessList::new();
        assert!(list.is_empty());
        list.upsert(proc(300, "b.third", false, true)).unwrap();
        list.upsert(proc(100, "b.first", false, false)).unwrap();
        list.upsert(proc(200, "b.second", true, false)).unwrap();
        let pids: Vec<i32> = list.as_slice().iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![100, 200, 300]);
        assert_eq!(list.len(), 3);

        list.upsert(proc(200, "b.second", true, true)).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(200).unwrap().running_output, true);
    }

    #[test]
    fn refuses_non_positive_pid() {
        let mut list = ProcessList::new();
        assert_eq!(list.upsert(proc(0, "x", true, true)), Err(ProcessError::InvalidPid(0)));
        assert_eq!(list.upsert(proc(-4, "x", true, true)), Err(ProcessError::InvalidPid(-4)));
        assert!(list.is_empty());
    }

    #[test]
    fn get_and_remove() {
        let mut list = ProcessList::new();
        list.upsert(proc(10, "a", false, false)).unwrap();
        assert!(list.get(10).is_some());
        assert!(list.get(11).is_none());
        assert_eq!(list.remove(10).map(|p| p.bundle_id), Some("a".to_string()));
        assert_eq!(list.remove(10), None);
        assert!(list.is_empty());
    }

    #[test]
    fn set_running_only_touches_listed_processes() {
        let mut list = ProcessList::new();
        list.upsert(proc(10, "a", false, false)).unwrap();
        assert!(list.set_running(10, true, false));
        assert!(!list.set_running(11, true, true));
        let p = list.get(10).unwrap();
        assert!(p.running_input);
        assert!(!p.running_output);
        assert!(p.is_running());
    }

    #[test]
    fn running_filters_follow_the_flags() {
        let mut list = ProcessList::new();
        list.upsert(proc(1, "in", true, false)).unwrap();
        list.upsert(proc(2, "out", false, true)).unwrap();
        list.upsert(proc(3, "idle", false, false)).unwrap();
        let input: Vec<i32> = list.running_input().map(|p| p.pid).collect();
        let output: Vec<i32> = list.running_output().map(|p| p.pid).collect();
        assert_eq!(input, vec![1]);
        assert_eq!(output, vec![2]);
        assert!(!list.get(3).unwrap().is_running());
    }
}
