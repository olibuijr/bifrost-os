//! Client interaction and maintenance (`maintenance client` parity): a queue of check, format and mount requests that
//! an app or daemon submits for a named resource, with the phase of each request and a log of its messages. Pure
//! model: the queue runs no check, formats nothing and mounts nothing; a system runner drives the phases.
//!
//! Mirrors the operations a client can request (a check, a format and a single-volume mount that names the bundle
//! to mount with), the operation identifier, the task log messages and the task completion (succeeded or failed
//! with a reason). One resource has at most one active request, so a format cannot start while a check of the same
//! resource is queued or running.
//!
//! Gaps: no discovery of installed modules, no settings shortcut, no options dictionary, and no persistence; a
//! request lives only as long as the queue.

/// A request for work on a resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Check the resource for errors.
    Check,
    /// Format the resource.
    Format,
    /// Mount the resource as one volume with the module named by `bundle_id`.
    Mount { bundle_id: String },
}

/// The identifier of a request. Ids start at 1 and increase in submission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(u64);

impl OperationId {
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Where a request is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Queued,
    Running,
    Succeeded,
    /// Finished with a failure; the text is the reason.
    Failed(String),
    Cancelled,
}

impl Phase {
    /// True once the request has succeeded, failed or been cancelled.
    pub fn is_finished(&self) -> bool {
        matches!(self, Phase::Succeeded | Phase::Failed(_) | Phase::Cancelled)
    }
}

/// One request in the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: OperationId,
    pub resource: String,
    pub operation: Operation,
    pub phase: Phase,
    /// Messages the runner logged for this request, oldest first.
    pub messages: Vec<String>,
}

/// Why the queue refused a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceError {
    EmptyResource,
    EmptyBundleId,
    /// Another request for the same resource is still queued or running.
    Busy { resource: String, active: OperationId },
    UnknownOperation(OperationId),
    /// The request is not in a phase that allows this change.
    InvalidTransition,
}

/// The queue of requests, in submission order.
#[derive(Clone, Debug, Default)]
pub struct MaintenanceQueue {
    last_id: u64,
    jobs: Vec<Job>,
}

impl MaintenanceQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a request for `resource`. Fails when the resource name is empty, when a mount names no bundle, or
    /// when another request for the same resource is still active.
    pub fn submit(&mut self, resource: &str, operation: Operation) -> Result<OperationId, MaintenanceError> {
        if resource.is_empty() {
            return Err(MaintenanceError::EmptyResource);
        }
        if let Operation::Mount { bundle_id } = &operation {
            if bundle_id.is_empty() {
                return Err(MaintenanceError::EmptyBundleId);
            }
        }
        if let Some(active) = self.active_for(resource) {
            return Err(MaintenanceError::Busy {
                resource: resource.to_string(),
                active: active.id,
            });
        }
        self.last_id += 1;
        let id = OperationId(self.last_id);
        self.jobs.push(Job {
            id,
            resource: resource.to_string(),
            operation,
            phase: Phase::Queued,
            messages: Vec::new(),
        });
        Ok(id)
    }

    pub fn job(&self, id: OperationId) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// The queued or running request for `resource`, if any.
    pub fn active_for(&self, resource: &str) -> Option<&Job> {
        self.jobs
            .iter()
            .find(|j| j.resource == resource && !j.phase.is_finished())
    }

    /// The oldest request that is still queued. A runner takes requests in this order.
    pub fn next_queued(&self) -> Option<OperationId> {
        self.jobs
            .iter()
            .find(|j| j.phase == Phase::Queued)
            .map(|j| j.id)
    }

    /// Moves a queued request to running.
    pub fn start(&mut self, id: OperationId) -> Result<(), MaintenanceError> {
        let job = self.job_mut(id)?;
        if job.phase != Phase::Queued {
            return Err(MaintenanceError::InvalidTransition);
        }
        job.phase = Phase::Running;
        Ok(())
    }

    /// Completes a running request. `Ok(())` means it succeeded; `Err(reason)` records the reason.
    pub fn finish(&mut self, id: OperationId, outcome: Result<(), &str>) -> Result<(), MaintenanceError> {
        let job = self.job_mut(id)?;
        if job.phase != Phase::Running {
            return Err(MaintenanceError::InvalidTransition);
        }
        job.phase = match outcome {
            Ok(()) => Phase::Succeeded,
            Err(reason) => Phase::Failed(reason.to_string()),
        };
        Ok(())
    }

    /// Cancels a request that is queued or running.
    pub fn cancel(&mut self, id: OperationId) -> Result<(), MaintenanceError> {
        let job = self.job_mut(id)?;
        if job.phase.is_finished() {
            return Err(MaintenanceError::InvalidTransition);
        }
        job.phase = Phase::Cancelled;
        Ok(())
    }

    /// Appends a message to the request's log.
    pub fn log(&mut self, id: OperationId, message: &str) -> Result<(), MaintenanceError> {
        self.job_mut(id)?.messages.push(message.to_string());
        Ok(())
    }

    fn job_mut(&mut self, id: OperationId) -> Result<&mut Job, MaintenanceError> {
        self.jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or(MaintenanceError::UnknownOperation(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_start_at_one_and_increase() {
        let mut q = MaintenanceQueue::new();
        let a = q.submit("disk1", Operation::Check).unwrap();
        let b = q.submit("disk2", Operation::Format).unwrap();
        assert_eq!(a.get(), 1);
        assert_eq!(b.get(), 2);
        assert_eq!(q.job(b).unwrap().phase, Phase::Queued);
    }

    #[test]
    fn one_active_request_per_resource() {
        let mut q = MaintenanceQueue::new();
        let check = q.submit("disk1", Operation::Check).unwrap();
        assert_eq!(
            q.submit("disk1", Operation::Format),
            Err(MaintenanceError::Busy {
                resource: "disk1".to_string(),
                active: check,
            })
        );
        // Another resource is not blocked.
        assert!(q.submit("disk2", Operation::Format).is_ok());
    }

    #[test]
    fn invalid_submissions_are_refused() {
        let mut q = MaintenanceQueue::new();
        assert_eq!(q.submit("", Operation::Check), Err(MaintenanceError::EmptyResource));
        assert_eq!(
            q.submit(
                "disk1",
                Operation::Mount {
                    bundle_id: String::new()
                }
            ),
            Err(MaintenanceError::EmptyBundleId)
        );
    }

    #[test]
    fn success_path_and_no_second_finish() {
        let mut q = MaintenanceQueue::new();
        let id = q.submit("disk1", Operation::Check).unwrap();
        assert_eq!(q.finish(id, Ok(())), Err(MaintenanceError::InvalidTransition));
        q.start(id).unwrap();
        assert_eq!(q.start(id), Err(MaintenanceError::InvalidTransition));
        q.finish(id, Ok(())).unwrap();
        assert_eq!(q.job(id).unwrap().phase, Phase::Succeeded);
        assert_eq!(q.finish(id, Ok(())), Err(MaintenanceError::InvalidTransition));
        // A finished request no longer blocks its resource.
        assert!(q.active_for("disk1").is_none());
        assert!(q.submit("disk1", Operation::Format).is_ok());
    }

    #[test]
    fn failure_keeps_the_reason_and_the_log() {
        let mut q = MaintenanceQueue::new();
        let id = q.submit("disk1", Operation::Mount { bundle_id: "org.example.fs".into() }).unwrap();
        q.start(id).unwrap();
        q.log(id, "probing").unwrap();
        q.log(id, "superblock damaged").unwrap();
        q.finish(id, Err("bad magic")).unwrap();
        let job = q.job(id).unwrap();
        assert_eq!(job.phase, Phase::Failed("bad magic".to_string()));
        assert_eq!(job.messages, vec!["probing".to_string(), "superblock damaged".to_string()]);
        assert!(job.phase.is_finished());
    }

    #[test]
    fn cancel_applies_to_queued_and_not_to_finished() {
        let mut q = MaintenanceQueue::new();
        let id = q.submit("disk1", Operation::Check).unwrap();
        q.cancel(id).unwrap();
        assert_eq!(q.job(id).unwrap().phase, Phase::Cancelled);
        assert_eq!(q.cancel(id), Err(MaintenanceError::InvalidTransition));
        assert_eq!(q.cancel(OperationId(99)), Err(MaintenanceError::UnknownOperation(OperationId(99))));
    }

    #[test]
    fn next_queued_is_first_in_first_out() {
        let mut q = MaintenanceQueue::new();
        let a = q.submit("disk1", Operation::Check).unwrap();
        let b = q.submit("disk2", Operation::Check).unwrap();
        assert_eq!(q.next_queued(), Some(a));
        q.start(a).unwrap();
        assert_eq!(q.next_queued(), Some(b));
        q.cancel(b).unwrap();
        assert_eq!(q.next_queued(), None);
    }
}
