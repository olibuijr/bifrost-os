//! Scheduled asset downloads (a download manager queue): essential downloads run before first launch, non-essential
//! ones after it. Size budgets apply to what is queued, and the queue can be written to lines and read back, so a
//! queue survives a restart. Pure state: the transfer itself runs on the URL session's download task.

/// Download priority. Essential sorts first, so `min` picks the next essential download.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Essential,
    NonEssential,
}

/// One queued download. `received` is the byte count reported by the transfer so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub id: String,
    pub url: String,
    pub bytes: u64,
    pub received: u64,
    pub priority: Priority,
}

/// Limits on what may be queued at once: an essential maximum and a total maximum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Largest total of queued essential downloads, in bytes. `None` means no limit.
    pub essential_max_bytes: Option<u64>,
    /// Largest total of all queued downloads, in bytes. `None` means no limit.
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueError {
    Duplicate(String),
    NotQueued(String),
    OverBudget { limit: u64, needed: u64 },
    OverEssentialBudget { limit: u64, needed: u64 },
    Malformed(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadQueue {
    items: Vec<Download>,
    budget: Budget,
}

impl DownloadQueue {
    pub fn new(budget: Budget) -> Self {
        Self { items: Vec::new(), budget }
    }

    /// Sum of queued bytes: (essential, all).
    fn totals(&self) -> (u64, u64) {
        self.items.iter().fold((0u64, 0u64), |(essential, all), d| {
            let essential = if d.priority == Priority::Essential { essential.saturating_add(d.bytes) } else { essential };
            (essential, all.saturating_add(d.bytes))
        })
    }

    /// `scheduleDownload`: queues a download. Fails when the id is already queued or when the queue would exceed
    /// the budget. The budget counts queued downloads only, so a finished download frees its share.
    pub fn schedule(&mut self, id: &str, url: &str, bytes: u64, priority: Priority) -> Result<(), QueueError> {
        if self.items.iter().any(|d| d.id == id) {
            return Err(QueueError::Duplicate(id.to_string()));
        }
        let (essential_now, all_now) = self.totals();
        let all = all_now.saturating_add(bytes);
        if let Some(limit) = self.budget.max_bytes {
            if all > limit {
                return Err(QueueError::OverBudget { limit, needed: all });
            }
        }
        if priority == Priority::Essential {
            let essential = essential_now.saturating_add(bytes);
            if let Some(limit) = self.budget.essential_max_bytes {
                if essential > limit {
                    return Err(QueueError::OverEssentialBudget { limit, needed: essential });
                }
            }
        }
        self.items.push(Download { id: id.to_string(), url: url.to_string(), bytes, received: 0, priority });
        Ok(())
    }

    /// The download to run next: essential before non-essential, and scheduling order within a priority.
    pub fn next(&self) -> Option<&Download> {
        self.items.iter().min_by_key(|d| d.priority)
    }

    /// `fetchCurrentDownloads`: every queued download in scheduling order.
    pub fn current(&self) -> &[Download] {
        &self.items
    }

    /// Records transfer progress. `received` is clamped to the download's size.
    pub fn record_progress(&mut self, id: &str, received: u64) -> Result<(), QueueError> {
        let index = self.position(id)?;
        let item = &mut self.items[index];
        item.received = received.min(item.bytes);
        Ok(())
    }

    /// Bytes still to transfer across the queue.
    pub fn remaining_bytes(&self) -> u64 {
        self.items.iter().map(|d| d.bytes - d.received).sum()
    }

    /// The transfer finished: the download leaves the queue and is returned.
    pub fn complete(&mut self, id: &str) -> Result<Download, QueueError> {
        let index = self.position(id)?;
        Ok(self.items.remove(index))
    }

    /// `cancel(_:)`: the download leaves the queue without a result.
    pub fn cancel(&mut self, id: &str) -> Result<Download, QueueError> {
        let index = self.position(id)?;
        Ok(self.items.remove(index))
    }

    fn position(&self, id: &str) -> Result<usize, QueueError> {
        self.items.iter().position(|d| d.id == id).ok_or_else(|| QueueError::NotQueued(id.to_string()))
    }

    /// Writes the queue as one tab-separated line per download: id, url, bytes, received, `E` or `N`. Ids and URLs
    /// must not contain tabs or newlines.
    pub fn to_lines(&self) -> String {
        let mut out = String::new();
        for d in &self.items {
            let priority = match d.priority {
                Priority::Essential => 'E',
                Priority::NonEssential => 'N',
            };
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\n", d.id, d.url, d.bytes, d.received, priority));
        }
        out
    }

    /// Reads a queue written by `to_lines`. Saved entries are restored as they were; the budget is only checked by
    /// `schedule`.
    pub fn from_lines(text: &str, budget: Budget) -> Result<Self, QueueError> {
        let mut queue = Self::new(budget);
        for (n, line) in text.lines().enumerate() {
            let line_no = n + 1;
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 5 {
                return Err(QueueError::Malformed(line_no));
            }
            let bytes: u64 = fields[2].parse().map_err(|_| QueueError::Malformed(line_no))?;
            let received: u64 = fields[3].parse().map_err(|_| QueueError::Malformed(line_no))?;
            let priority = match fields[4] {
                "E" => Priority::Essential,
                "N" => Priority::NonEssential,
                _ => return Err(QueueError::Malformed(line_no)),
            };
            if queue.items.iter().any(|d| d.id == fields[0]) {
                return Err(QueueError::Duplicate(fields[0].to_string()));
            }
            queue.items.push(Download {
                id: fields[0].to_string(),
                url: fields[1].to_string(),
                bytes,
                received: received.min(bytes),
                priority,
            });
        }
        Ok(queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> DownloadQueue {
        DownloadQueue::new(Budget::default())
    }

    #[test]
    fn essential_runs_before_non_essential_and_keeps_scheduling_order() {
        let mut q = queue();
        q.schedule("skin", "https://h/skin", 10, Priority::NonEssential).unwrap();
        q.schedule("e1", "https://h/e1", 10, Priority::Essential).unwrap();
        q.schedule("e2", "https://h/e2", 10, Priority::Essential).unwrap();
        assert_eq!(q.next().map(|d| d.id.as_str()), Some("e1"));
        q.complete("e1").unwrap();
        assert_eq!(q.next().map(|d| d.id.as_str()), Some("e2"));
        q.complete("e2").unwrap();
        assert_eq!(q.next().map(|d| d.id.as_str()), Some("skin"));
        q.complete("skin").unwrap();
        assert!(q.next().is_none());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut q = queue();
        q.schedule("pack", "https://h/p", 1, Priority::Essential).unwrap();
        assert_eq!(
            q.schedule("pack", "https://h/other", 2, Priority::NonEssential),
            Err(QueueError::Duplicate("pack".into()))
        );
        assert_eq!(q.current().len(), 1);
    }

    #[test]
    fn total_budget_accepts_the_limit_and_rejects_past_it() {
        let mut q = DownloadQueue::new(Budget { essential_max_bytes: None, max_bytes: Some(100) });
        q.schedule("a", "u", 60, Priority::NonEssential).unwrap();
        q.schedule("b", "u", 40, Priority::NonEssential).unwrap();
        assert_eq!(
            q.schedule("c", "u", 1, Priority::NonEssential),
            Err(QueueError::OverBudget { limit: 100, needed: 101 })
        );
        // Completing a download frees its share of the budget.
        q.complete("a").unwrap();
        q.schedule("c", "u", 60, Priority::NonEssential).unwrap();
    }

    #[test]
    fn essential_budget_limits_only_essential_downloads() {
        let mut q = DownloadQueue::new(Budget { essential_max_bytes: Some(50), max_bytes: None });
        q.schedule("e1", "u", 30, Priority::Essential).unwrap();
        assert_eq!(
            q.schedule("e2", "u", 30, Priority::Essential),
            Err(QueueError::OverEssentialBudget { limit: 50, needed: 60 })
        );
        // Non-essential downloads do not count against the essential budget.
        q.schedule("n1", "u", 500, Priority::NonEssential).unwrap();
    }

    #[test]
    fn progress_is_clamped_and_remaining_bytes_follow_it() {
        let mut q = queue();
        q.schedule("a", "u", 100, Priority::Essential).unwrap();
        q.schedule("b", "u", 50, Priority::NonEssential).unwrap();
        q.record_progress("a", 40).unwrap();
        assert_eq!(q.remaining_bytes(), 60 + 50);
        q.record_progress("a", 900).unwrap();
        assert_eq!(q.current()[0].received, 100);
        assert_eq!(q.remaining_bytes(), 50);
        assert_eq!(q.record_progress("zzz", 1), Err(QueueError::NotQueued("zzz".into())));
    }

    #[test]
    fn cancel_removes_the_download_without_completing_it() {
        let mut q = queue();
        q.schedule("a", "u", 5, Priority::Essential).unwrap();
        let gone = q.cancel("a").unwrap();
        assert_eq!(gone.id, "a");
        assert!(q.current().is_empty());
        assert_eq!(q.cancel("a"), Err(QueueError::NotQueued("a".into())));
    }

    #[test]
    fn lines_round_trip_restores_order_priority_and_progress() {
        let mut q = queue();
        q.schedule("e", "https://h/e", 80, Priority::Essential).unwrap();
        q.schedule("n", "https://h/n", 20, Priority::NonEssential).unwrap();
        q.record_progress("e", 30).unwrap();
        let text = q.to_lines();
        let back = DownloadQueue::from_lines(&text, Budget::default()).unwrap();
        assert_eq!(back.current(), q.current());
        assert_eq!(back.next().map(|d| d.received), Some(30));
    }

    #[test]
    fn malformed_saved_queue_is_rejected_with_its_line_number() {
        let budget = Budget::default();
        assert_eq!(DownloadQueue::from_lines("a\tu\t10\t0\tE\nbad line\n", budget), Err(QueueError::Malformed(2)));
        assert_eq!(DownloadQueue::from_lines("a\tu\tten\t0\tE\n", budget), Err(QueueError::Malformed(1)));
        assert_eq!(DownloadQueue::from_lines("a\tu\t10\t0\tX\n", budget), Err(QueueError::Malformed(1)));
        assert_eq!(
            DownloadQueue::from_lines("a\tu\t10\t0\tE\na\tv\t5\t0\tN\n", budget),
            Err(QueueError::Duplicate("a".into()))
        );
    }
}
