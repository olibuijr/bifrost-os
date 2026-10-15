//! Offline media: download tasks and the storage budget that decides which finished items stay on the device.
//!
//! The model keeps one task per item. A task moves from queued to downloading, to paused or finished or failed, and
//! a paused or failed task resumes from the byte it had reached, so the caller can ask the server for that range.
//! The store holds at most `budget` bytes. When a new download needs room, the finished items that were played least
//! recently go first, and an item that is still downloading is never evicted. The model opens no connection and
//! writes no file: the caller reports the bytes that arrived and the bytes it stored.

/// The state of one download task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Downloading,
    Paused,
    Finished,
    Failed,
}

/// One item kept or being kept offline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    id: String,
    total: Option<u64>,
    received: u64,
    state: TaskState,
    last_played: Option<u64>,
}

impl Task {
    /// The item's identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The expected size in bytes, when the server gave one.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// The bytes stored so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// The task state.
    pub fn state(&self) -> TaskState {
        self.state
    }

    /// The fraction downloaded, when the total size is known. Never above 1.
    pub fn fraction(&self) -> Option<f64> {
        self.total.filter(|&t| t > 0).map(|t| (self.received as f64 / t as f64).min(1.0))
    }
}

/// Why a store operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// An item with this identifier is already in the store.
    Duplicate,
    /// No item with this identifier is in the store.
    Unknown,
    /// The item's state does not allow this operation.
    InvalidState,
    /// The bytes do not fit, even after every finished item that may be evicted has gone.
    NoRoom,
}

/// The downloads of one device and the storage budget they share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfflineStore {
    budget: u64,
    tasks: Vec<Task>,
    clock: u64,
}

impl OfflineStore {
    /// An empty store that may hold `budget` bytes.
    pub fn new(budget: u64) -> Self {
        OfflineStore { budget, tasks: Vec::new(), clock: 0 }
    }

    /// The storage budget in bytes.
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Every task, in the order they were queued.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The task for `id`, if any.
    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// The bytes held by every task, finished or not.
    pub fn bytes_held(&self) -> u64 {
        self.tasks.iter().map(|t| t.received).sum()
    }

    /// Queues a download for `id`, whose size is `total` when the server gave one.
    pub fn enqueue(&mut self, id: &str, total: Option<u64>) -> Result<(), StoreError> {
        if self.task(id).is_some() {
            return Err(StoreError::Duplicate);
        }
        self.tasks.push(Task { id: id.to_owned(), total, received: 0, state: TaskState::Queued, last_played: None });
        Ok(())
    }

    /// Starts or resumes a download. Returns the byte offset to request from the server: zero for a new download,
    /// or the bytes already stored when a paused or failed download resumes.
    pub fn start(&mut self, id: &str) -> Result<u64, StoreError> {
        let task = self.task_mut(id)?;
        match task.state {
            TaskState::Queued | TaskState::Paused | TaskState::Failed => {
                task.state = TaskState::Downloading;
                Ok(task.received)
            }
            TaskState::Downloading | TaskState::Finished => Err(StoreError::InvalidState),
        }
    }

    /// Stops a running download and keeps the bytes it has.
    pub fn pause(&mut self, id: &str) -> Result<(), StoreError> {
        let task = self.task_mut(id)?;
        if task.state != TaskState::Downloading {
            return Err(StoreError::InvalidState);
        }
        task.state = TaskState::Paused;
        Ok(())
    }

    /// Records `bytes` that arrived for a running download. The download finishes when it reaches its total size, and
    /// the stored bytes never pass that size. Returns the new state.
    pub fn receive(&mut self, id: &str, bytes: u64) -> Result<TaskState, StoreError> {
        let task = self.task_mut(id)?;
        if task.state != TaskState::Downloading {
            return Err(StoreError::InvalidState);
        }
        let mut received = task.received.saturating_add(bytes);
        if let Some(total) = task.total {
            received = received.min(total);
            if received == total {
                task.state = TaskState::Finished;
            }
        }
        task.received = received;
        Ok(task.state)
    }

    /// Marks a queued or running download as failed. Its bytes stay, so a later start resumes from them.
    pub fn fail(&mut self, id: &str) -> Result<(), StoreError> {
        let task = self.task_mut(id)?;
        match task.state {
            TaskState::Queued | TaskState::Downloading => {
                task.state = TaskState::Failed;
                Ok(())
            }
            TaskState::Paused | TaskState::Failed | TaskState::Finished => Err(StoreError::InvalidState),
        }
    }

    /// Marks a finished item as played now, so that it is evicted after the items played before it.
    pub fn played(&mut self, id: &str) -> Result<(), StoreError> {
        let next = self.clock + 1;
        let task = self.task_mut(id)?;
        if task.state != TaskState::Finished {
            return Err(StoreError::InvalidState);
        }
        task.last_played = Some(next);
        self.clock = next;
        Ok(())
    }

    /// Makes room for `bytes` more by evicting finished items, least recently played first (items never played go
    /// before any played item, and ties keep the queue order). Returns the identifiers evicted. Nothing is evicted
    /// when the bytes cannot fit even after every finished item has gone.
    pub fn make_room(&mut self, bytes: u64) -> Result<Vec<String>, StoreError> {
        let held = self.bytes_held();
        if held.saturating_add(bytes) <= self.budget {
            return Ok(Vec::new());
        }
        let mut candidates: Vec<usize> = (0..self.tasks.len()).filter(|&i| self.tasks[i].state == TaskState::Finished).collect();
        candidates.sort_by_key(|&i| self.tasks[i].last_played);
        let mut freed = 0u64;
        let mut chosen = Vec::new();
        for i in candidates {
            if held.saturating_sub(freed).saturating_add(bytes) <= self.budget {
                break;
            }
            freed += self.tasks[i].received;
            chosen.push(i);
        }
        if held.saturating_sub(freed).saturating_add(bytes) > self.budget {
            return Err(StoreError::NoRoom);
        }
        let mut evicted = Vec::new();
        chosen.sort_unstable();
        for i in chosen.into_iter().rev() {
            evicted.push(self.tasks.remove(i).id);
        }
        evicted.reverse();
        Ok(evicted)
    }

    /// Removes an item and its bytes. Returns whether it was there.
    pub fn remove(&mut self, id: &str) -> bool {
        match self.tasks.iter().position(|t| t.id == id) {
            Some(i) => {
                self.tasks.remove(i);
                true
            }
            None => false,
        }
    }

    fn task_mut(&mut self, id: &str) -> Result<&mut Task, StoreError> {
        self.tasks.iter_mut().find(|t| t.id == id).ok_or(StoreError::Unknown)
    }
}
