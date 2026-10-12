//! A recyclable pool of buffers. `acquire` hands out a free buffer when one is ready, and otherwise makes a new one,
//! up to an allocation threshold. `release` returns a buffer to the pool. `age` drops free buffers that have not
//! been used for longer than the maximum age, never taking the pool below its minimum count. `flush` drops every
//! free buffer. Time is a tick count the caller supplies (frames or milliseconds), so the pool holds no clock.
//! The pool is generic over the buffer type: the caller supplies the constructor.

/// Limits of a pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Buffers kept alive while idle. `prewarm` creates this many up front.
    pub min_count: usize,
    /// Free buffers unused for more than this many ticks are dropped. None: never dropped by age.
    pub max_age: Option<u64>,
    /// Most buffers alive at once, free or in use. None: no limit.
    pub threshold: Option<usize>,
}

/// Why `acquire` could not hand out a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolError {
    /// Every allowed buffer is in use and none is free.
    WouldExceedThreshold,
}

/// A pool of buffers of type `T`.
#[derive(Debug)]
pub struct Pool<T> {
    config: Config,
    /// Free buffers with the tick each one was released at. Oldest first, newest last.
    free: Vec<(T, u64)>,
    /// Buffers created and not dropped, free or in use.
    live: usize,
}

impl<T> Pool<T> {
    /// An empty pool with the given limits.
    pub fn new(config: Config) -> Self {
        Pool { config, free: Vec::new(), live: 0 }
    }

    /// Buffers created and not dropped, free or in use.
    pub fn live(&self) -> usize {
        self.live
    }

    /// Buffers in the pool that are free right now.
    pub fn free_count(&self) -> usize {
        self.free.len()
    }

    fn below_threshold(&self) -> bool {
        self.config.threshold.map_or(true, |t| self.live < t)
    }

    /// A buffer: the most recently released free one, or a new one from `make` while under the threshold.
    pub fn acquire(&mut self, make: impl FnOnce() -> T) -> Result<T, PoolError> {
        if let Some((buf, _)) = self.free.pop() {
            return Ok(buf);
        }
        if !self.below_threshold() {
            return Err(PoolError::WouldExceedThreshold);
        }
        self.live += 1;
        Ok(make())
    }

    /// Returns a buffer to the pool at tick `now`.
    pub fn release(&mut self, buf: T, now: u64) {
        self.free.push((buf, now));
    }

    /// Creates buffers until the pool holds `min_count`, stopping at the threshold. Returns how many were created.
    pub fn prewarm(&mut self, now: u64, mut make: impl FnMut() -> T) -> usize {
        let mut created = 0;
        while self.live < self.config.min_count && self.below_threshold() {
            self.live += 1;
            created += 1;
            self.free.push((make(), now));
        }
        created
    }

    /// Drops free buffers unused for more than `max_age` ticks at tick `now`, oldest first, while the pool stays at
    /// or above `min_count`. Returns how many were dropped.
    pub fn age(&mut self, now: u64) -> usize {
        let Some(max) = self.config.max_age else {
            return 0;
        };
        let mut dropped = 0;
        loop {
            let oldest = match self.free.first() {
                Some((_, t)) => *t,
                None => break,
            };
            if now.saturating_sub(oldest) <= max || self.live <= self.config.min_count {
                break;
            }
            self.free.remove(0);
            self.live -= 1;
            dropped += 1;
        }
        dropped
    }

    /// Drops every free buffer, whatever the minimum count. Buffers in use stay. Returns how many were dropped.
    pub fn flush(&mut self) -> usize {
        let dropped = self.free.len();
        self.free.clear();
        self.live -= dropped;
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(min: usize, max_age: Option<u64>, threshold: Option<usize>) -> Config {
        Config { min_count: min, max_age, threshold }
    }

    #[test]
    fn first_acquire_makes_a_buffer() {
        let mut pool: Pool<u32> = Pool::new(config(0, None, None));
        let mut next = 0u32;
        let buf = pool.acquire(|| {
            next += 1;
            next
        });
        assert_eq!(buf, Ok(1));
        assert_eq!(pool.live(), 1);
        assert_eq!(pool.free_count(), 0);
    }

    #[test]
    fn released_buffer_is_reused_without_making_a_new_one() {
        let mut pool: Pool<u32> = Pool::new(config(0, None, None));
        let mut made = 0u32;
        let a = pool.acquire(|| {
            made += 1;
            made
        })
        .unwrap();
        pool.release(a, 0);
        let b = pool.acquire(|| {
            made += 1;
            made
        })
        .unwrap();
        assert_eq!(b, 1);
        assert_eq!(made, 1);
        assert_eq!(pool.live(), 1);
    }

    #[test]
    fn threshold_refuses_a_buffer_past_the_limit_until_one_is_released() {
        let mut pool: Pool<u32> = Pool::new(config(0, None, Some(2)));
        let mut next = 0u32;
        let mut make = || {
            next += 1;
            next
        };
        let a = pool.acquire(&mut make).unwrap();
        let _b = pool.acquire(&mut make).unwrap();
        assert_eq!(pool.acquire(&mut make), Err(PoolError::WouldExceedThreshold));
        pool.release(a, 1);
        assert_eq!(pool.acquire(&mut make), Ok(a));
        assert_eq!(pool.live(), 2);
    }

    #[test]
    fn age_keeps_a_buffer_exactly_at_the_limit_and_drops_it_one_tick_later() {
        let mut pool: Pool<u32> = Pool::new(config(0, Some(10), None));
        let x = pool.acquire(|| 7).unwrap();
        pool.release(x, 0);
        assert_eq!(pool.age(10), 0);
        assert_eq!(pool.age(11), 1);
        assert_eq!(pool.live(), 0);
    }

    #[test]
    fn age_never_takes_the_pool_below_its_minimum() {
        let mut pool: Pool<u32> = Pool::new(config(1, Some(10), None));
        assert_eq!(pool.prewarm(0, || 7), 1);
        assert_eq!(pool.age(100), 0);
        assert_eq!(pool.live(), 1);
        assert_eq!(pool.free_count(), 1);
    }

    #[test]
    fn age_drops_the_oldest_free_buffer_and_acquire_takes_the_newest() {
        let mut pool: Pool<&str> = Pool::new(config(0, Some(10), None));
        let a = pool.acquire(|| "a").unwrap();
        let b = pool.acquire(|| "b").unwrap();
        pool.release(a, 0);
        pool.release(b, 5);
        // At tick 12: "a" has been idle 12 ticks (dropped), "b" 7 ticks (kept).
        assert_eq!(pool.age(12), 1);
        assert_eq!(pool.live(), 1);
        assert_eq!(pool.acquire(|| "new"), Ok("b"));
    }

    #[test]
    fn prewarm_stops_at_the_threshold() {
        let mut pool: Pool<u32> = Pool::new(config(3, None, Some(2)));
        assert_eq!(pool.prewarm(0, || 0), 2);
        assert_eq!(pool.live(), 2);
        assert_eq!(pool.free_count(), 2);
    }

    #[test]
    fn flush_drops_every_free_buffer_and_keeps_the_ones_in_use() {
        let mut pool: Pool<u32> = Pool::new(config(0, None, None));
        let mut next = 0u32;
        let mut make = || {
            next += 1;
            next
        };
        let in_use = pool.acquire(&mut make).unwrap();
        let b = pool.acquire(&mut make).unwrap();
        let c = pool.acquire(&mut make).unwrap();
        pool.release(b, 0);
        pool.release(c, 0);
        assert_eq!(pool.live(), 3);
        assert_eq!(pool.flush(), 2);
        assert_eq!(pool.live(), 1);
        assert_eq!(pool.free_count(), 0);
        pool.release(in_use, 1);
        assert_eq!(pool.free_count(), 1);
    }
}
