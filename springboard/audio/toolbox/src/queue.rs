//! The buffer queue (task 13): the buffer-based playback API. A client enqueues buffers of interleaved f32
//! frames; the output side pulls them with [`AudioQueue::render`], which is real-time safe (no allocation, no
//! locking, no syscalls: every container is sized when the queue is built). Finished buffers are reported by id
//! through [`AudioQueue::drain_completed`] so the owner can recycle or free them off the real-time thread.
//! The queue is a pure model: moving it between the client thread and the IO callback (a lock-free hand-off or the
//! shared ring) is the caller's job.

use bifrost_audio_types::AudioError;
use std::collections::VecDeque;

/// The most buffers pending (queued or finished but not yet collected) at once.
pub const MAX_BUFFERS: usize = 64;

/// Identifies an enqueued buffer.
pub type BufferId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    /// Nothing plays; buffers stay queued.
    Stopped,
    Running,
    /// Like stopped, but the position is kept for [`AudioQueue::start`] to resume.
    Paused,
}

struct Entry {
    id: BufferId,
    samples: Vec<f32>,
    /// Frames already played.
    cursor: usize,
}

pub struct AudioQueue {
    channels: usize,
    rate: u32,
    pending: VecDeque<Entry>,
    completed: Vec<BufferId>,
    state: QueueState,
    volume: f32,
    next_id: BufferId,
    played: u64,
}

impl AudioQueue {
    pub fn new(rate: u32, channels: u16) -> Result<AudioQueue, AudioError> {
        if rate == 0 || channels == 0 {
            return Err(AudioError::BadValue);
        }
        Ok(AudioQueue {
            channels: channels as usize,
            rate,
            pending: VecDeque::with_capacity(MAX_BUFFERS),
            completed: Vec::with_capacity(MAX_BUFFERS),
            state: QueueState::Stopped,
            volume: 1.0,
            next_id: 1,
            played: 0,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels as u16
    }

    pub fn state(&self) -> QueueState {
        self.state
    }

    /// Queues a buffer of interleaved frames. Refused when the samples are not whole frames, are empty, or
    /// [`MAX_BUFFERS`] are already pending.
    pub fn enqueue(&mut self, samples: Vec<f32>) -> Result<BufferId, AudioError> {
        if samples.is_empty() || samples.len() % self.channels != 0 || self.pending.len() + self.completed.len() >= MAX_BUFFERS {
            return Err(AudioError::BadValue);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.pending.push_back(Entry { id, samples, cursor: 0 });
        Ok(id)
    }

    pub fn start(&mut self) {
        self.state = QueueState::Running;
    }

    pub fn pause(&mut self) {
        if self.state == QueueState::Running {
            self.state = QueueState::Paused;
        }
    }

    /// Stops playback. With `flush` every queued buffer is dropped and reported as completed; without it the
    /// queue keeps them and a later [`AudioQueue::start`] resumes at the same frame.
    pub fn stop(&mut self, flush: bool) {
        self.state = QueueState::Stopped;
        if flush {
            self.flush();
        }
    }

    /// Drops every queued buffer, reporting each as completed.
    pub fn flush(&mut self) {
        while let Some(e) = self.pending.pop_front() {
            self.completed.push(e.id);
        }
    }

    /// Output gain, clamped to 0 through 1.
    pub fn set_volume(&mut self, v: f32) {
        self.volume = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Frames played since the queue was built (the sample time).
    pub fn sample_time(&self) -> u64 {
        self.played
    }

    /// Buffers still queued (including the one playing).
    pub fn queued(&self) -> usize {
        self.pending.len()
    }

    /// Fills `out` (interleaved, whole frames) from the queued buffers and returns the frames of audio written;
    /// the rest of `out` is silence. Real-time safe.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        out.fill(0.0);
        if self.state != QueueState::Running {
            return 0;
        }
        let ch = self.channels;
        let want = out.len() / ch;
        let mut done = 0;
        while done < want {
            let Some(e) = self.pending.front_mut() else { break };
            let total = e.samples.len() / ch;
            let n = (total - e.cursor).min(want - done);
            let src = &e.samples[e.cursor * ch..(e.cursor + n) * ch];
            for (o, s) in out[done * ch..(done + n) * ch].iter_mut().zip(src) {
                *o = s * self.volume;
            }
            e.cursor += n;
            done += n;
            if e.cursor == total {
                let id = e.id;
                self.pending.pop_front();
                self.completed.push(id);
            }
        }
        self.played += done as u64;
        done
    }

    /// Moves the ids of finished buffers into `out` (appending) and forgets them.
    pub fn drain_completed(&mut self, out: &mut Vec<BufferId>) {
        out.extend(self.completed.drain(..));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> AudioQueue {
        AudioQueue::new(48000, 2).unwrap()
    }

    #[test]
    fn a_stopped_queue_is_silent() {
        let mut q = q();
        q.enqueue(vec![1.0; 4]).unwrap();
        let mut out = [9.0; 4];
        assert_eq!(q.render(&mut out), 0);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(q.sample_time(), 0);
    }

    #[test]
    fn buffers_play_in_order_across_render_calls() {
        let mut q = q();
        let a = q.enqueue(vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0]).unwrap(); // 3 frames
        let b = q.enqueue(vec![4.0, 4.0]).unwrap(); // 1 frame
        q.start();
        let mut out = [0.0; 4]; // 2 frames
        assert_eq!(q.render(&mut out), 2);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);
        let mut done = Vec::new();
        q.drain_completed(&mut done);
        assert!(done.is_empty());
        let mut out = [0.0; 6]; // 3 frames wanted, 2 left
        assert_eq!(q.render(&mut out), 2);
        assert_eq!(out, [3.0, 3.0, 4.0, 4.0, 0.0, 0.0]);
        q.drain_completed(&mut done);
        assert_eq!(done, vec![a, b]);
        assert_eq!(q.sample_time(), 4);
        assert_eq!(q.queued(), 0);
    }

    #[test]
    fn pause_keeps_the_position() {
        let mut q = q();
        q.enqueue(vec![1.0, 1.0, 2.0, 2.0]).unwrap();
        q.start();
        let mut out = [0.0; 2];
        q.render(&mut out);
        q.pause();
        assert_eq!(q.state(), QueueState::Paused);
        assert_eq!(q.render(&mut out), 0);
        q.start();
        assert_eq!(q.render(&mut out), 1);
        assert_eq!(out, [2.0, 2.0]);
    }

    #[test]
    fn stop_with_flush_reports_everything() {
        let mut q = q();
        let a = q.enqueue(vec![1.0; 2]).unwrap();
        let b = q.enqueue(vec![1.0; 2]).unwrap();
        q.start();
        q.stop(true);
        let mut done = Vec::new();
        q.drain_completed(&mut done);
        assert_eq!(done, vec![a, b]);
        assert_eq!(q.state(), QueueState::Stopped);
        // stop without flush keeps the buffers
        let c = q.enqueue(vec![0.5; 2]).unwrap();
        q.stop(false);
        assert_eq!(q.queued(), 1);
        q.start();
        let mut out = [0.0; 2];
        q.render(&mut out);
        q.drain_completed(&mut done);
        assert_eq!(done.last(), Some(&c));
    }

    #[test]
    fn volume_scales_and_clamps() {
        let mut q = q();
        q.enqueue(vec![1.0, -1.0]).unwrap();
        q.set_volume(0.5);
        q.start();
        let mut out = [0.0; 2];
        q.render(&mut out);
        assert_eq!(out, [0.5, -0.5]);
        q.set_volume(3.0);
        assert_eq!(q.volume(), 1.0);
        q.set_volume(f32::NAN);
        assert_eq!(q.volume(), 0.0);
    }

    #[test]
    fn enqueue_validates() {
        let mut q = q();
        assert_eq!(q.enqueue(vec![]), Err(AudioError::BadValue));
        assert_eq!(q.enqueue(vec![0.0; 3]), Err(AudioError::BadValue));
        for _ in 0..MAX_BUFFERS {
            q.enqueue(vec![0.0; 2]).unwrap();
        }
        assert_eq!(q.enqueue(vec![0.0; 2]), Err(AudioError::BadValue));
        assert!(AudioQueue::new(0, 2).is_err());
        assert!(AudioQueue::new(48000, 0).is_err());
    }

    #[test]
    fn completed_ids_count_against_the_limit_until_drained() {
        let mut q = q();
        for _ in 0..MAX_BUFFERS {
            q.enqueue(vec![0.0; 2]).unwrap();
        }
        q.start();
        let mut out = vec![0.0; 2 * MAX_BUFFERS];
        assert_eq!(q.render(&mut out), MAX_BUFFERS);
        assert!(q.enqueue(vec![0.0; 2]).is_err());
        let mut done = Vec::new();
        q.drain_completed(&mut done);
        assert_eq!(done.len(), MAX_BUFFERS);
        assert!(q.enqueue(vec![0.0; 2]).is_ok());
    }
}
