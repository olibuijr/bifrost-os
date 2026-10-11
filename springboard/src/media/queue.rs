//! The play queue: a list of items, a play order (shuffled or not), the
//! current position and the repeat mode. Pure logic, shared by the audio
//! thread and the UI.

/// What happens at the end of the queue (or of a song).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    /// The next mode when the repeat button is tapped: Off → All → One → Off.
    pub fn cycle(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Queue<T> {
    items: Vec<T>,
    /// indices into `items` in play order
    order: Vec<usize>,
    /// position in `order`; None when nothing is queued
    pos: Option<usize>,
    shuffle: bool,
    repeat: Repeat,
    /// xorshift state for shuffling
    rng: u64,
}

impl<T> Queue<T> {
    /// A queue over `items` starting at `start`, in list order.
    pub fn new(items: Vec<T>, start: usize) -> Self {
        let order: Vec<usize> = (0..items.len()).collect();
        let pos = (!items.is_empty()).then(|| start.min(items.len() - 1));
        Queue { items, order, pos, shuffle: false, repeat: Repeat::Off, rng: 0x9E37_79B9_7F4A_7C15 }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }

    /// Index into `items` of the current song.
    pub fn current_index(&self) -> Option<usize> {
        self.pos.map(|p| self.order[p])
    }

    pub fn current(&self) -> Option<&T> {
        self.current_index().map(|i| &self.items[i])
    }

    /// Jumps to the item at `index` (list order), keeping the shuffle order
    /// but moving it to the front of what is left.
    pub fn jump_to(&mut self, index: usize) {
        if let Some(p) = self.order.iter().position(|&i| i == index) {
            self.pos = Some(p);
        }
    }

    /// The items after the current one, in play order.
    pub fn upcoming(&self) -> Vec<usize> {
        match self.pos {
            Some(p) => self.order[p + 1..].to_vec(),
            None => Vec::new(),
        }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Fisher-Yates over `order[from..]`.
    fn shuffle_from(&mut self, from: usize) {
        for i in (from + 1..self.order.len()).rev() {
            let j = from + (self.rand() % (i - from + 1) as u64) as usize;
            self.order.swap(i, j);
        }
    }

    /// Turns shuffle on (the current song stays first, the rest is
    /// reordered with `seed`) or off (list order, same current song).
    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        let current = self.current_index();
        self.shuffle = on;
        self.rng = (seed ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0x2545_F491_4F6C_DD1D) | 1;
        self.order = (0..self.items.len()).collect();
        self.pos = current;
        if on {
            if let Some(c) = current {
                self.order.swap(0, c);
                self.shuffle_from(1);
                self.pos = Some(0);
            } else {
                self.shuffle_from(0);
            }
        }
    }

    /// What `next(manual)` would move to, without moving: the index into
    /// `items`, or None when playback would stop.
    pub fn following(&self, manual: bool) -> Option<usize> {
        let p = self.pos?;
        if self.repeat == Repeat::One && !manual {
            return Some(self.order[p]);
        }
        if p + 1 < self.order.len() {
            return Some(self.order[p + 1]);
        }
        match self.repeat {
            Repeat::Off if !manual => None,
            _ => Some(self.order[0]),
        }
    }

    /// Advances; `manual` is a tap on Next (repeat-one still advances, and
    /// the end wraps). Returns false when playback stops: the queue then
    /// rests on its first song.
    pub fn next(&mut self, manual: bool) -> bool {
        let Some(p) = self.pos else { return false };
        if self.repeat == Repeat::One && !manual {
            return true;
        }
        if p + 1 < self.order.len() {
            self.pos = Some(p + 1);
            return true;
        }
        if self.shuffle {
            // a new round, in a new order
            let seed = self.rand();
            self.pos = None;
            self.set_shuffle(true, seed);
        }
        self.pos = Some(0);
        self.repeat != Repeat::Off || manual
    }

    /// Goes back one song (the first stays first unless repeating all).
    pub fn prev(&mut self) {
        let Some(p) = self.pos else { return };
        self.pos = Some(if p > 0 {
            p - 1
        } else if self.repeat == Repeat::All {
            self.order.len() - 1
        } else {
            0
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(n: usize, start: usize) -> Queue<u32> {
        Queue::new((0..n as u32).collect(), start)
    }

    #[test]
    fn plays_in_list_order_and_stops_at_the_end() {
        let mut q = q(3, 1);
        assert_eq!(q.current(), Some(&1));
        assert_eq!(q.upcoming(), [2]);
        assert_eq!(q.following(false), Some(2));
        assert!(q.next(false));
        assert_eq!(q.following(false), None);
        assert!(!q.next(false), "end of the queue stops");
        assert_eq!(q.current(), Some(&0), "rests on the first song");
        assert!(q.next(true), "a manual Next at the end wraps");
        assert_eq!(q.current(), Some(&1));
    }

    #[test]
    fn repeat_modes() {
        assert_eq!(Repeat::Off.cycle(), Repeat::All);
        assert_eq!(Repeat::All.cycle(), Repeat::One);
        assert_eq!(Repeat::One.cycle(), Repeat::Off);
        let mut q = q(2, 1);
        q.set_repeat(Repeat::All);
        assert_eq!(q.following(false), Some(0));
        assert!(q.next(false));
        assert_eq!(q.current(), Some(&0));
        q.prev();
        assert_eq!(q.current(), Some(&1), "prev wraps under repeat all");
        q.set_repeat(Repeat::One);
        assert_eq!(q.following(false), Some(1));
        assert!(q.next(false));
        assert_eq!(q.current(), Some(&1), "repeat one stays");
        assert_eq!(q.following(true), Some(0));
        assert!(q.next(true));
        assert_eq!(q.current(), Some(&0), "a tap still advances");
        q.set_repeat(Repeat::Off);
        q.prev();
        assert_eq!(q.current(), Some(&0), "first stays first");
    }

    #[test]
    fn shuffle_keeps_the_current_song_first_and_covers_everything() {
        let mut q = q(20, 7);
        q.set_shuffle(true, 42);
        assert!(q.shuffle());
        assert_eq!(q.current(), Some(&7));
        let mut seen = vec![7];
        while q.next(false) {
            seen.push(*q.current().unwrap());
        }
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<u32>>(), "every song once: {seen:?}");
        assert_ne!(seen, (7..20).chain(0..7).collect::<Vec<u32>>(), "actually shuffled");
        let mut other = Queue::new((0..20u32).collect(), 7);
        other.set_shuffle(true, 43);
        let mut a = Vec::new();
        while other.next(false) {
            a.push(*other.current().unwrap());
        }
        assert_ne!(a[..19], seen[1..], "another seed, another order");
    }

    #[test]
    fn shuffle_off_returns_to_list_order_around_the_current_song() {
        let mut q = q(5, 0);
        q.set_shuffle(true, 9);
        q.next(false);
        let cur = *q.current().unwrap();
        q.set_shuffle(false, 0);
        assert_eq!(q.current(), Some(&cur));
        assert_eq!(q.upcoming(), (cur as usize + 1..5).collect::<Vec<_>>());
        q.jump_to(3);
        assert_eq!(q.current(), Some(&3));
    }

    #[test]
    fn repeat_all_with_shuffle_reshuffles_each_round() {
        let mut q = q(12, 0);
        q.set_shuffle(true, 5);
        q.set_repeat(Repeat::All);
        let first: Vec<u32> = (0..12).map(|_| {
            let c = *q.current().unwrap();
            q.next(false);
            c
        }).collect();
        let second: Vec<u32> = (0..12).map(|_| {
            let c = *q.current().unwrap();
            q.next(false);
            c
        }).collect();
        let (mut a, mut b) = (first.clone(), second.clone());
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_ne!(first, second);
    }

    #[test]
    fn empty_queue_is_inert() {
        let mut q: Queue<u32> = Queue::new(vec![], 0);
        assert!(q.is_empty() && q.current().is_none() && q.following(true).is_none());
        assert!(!q.next(true));
        q.prev();
        q.set_shuffle(true, 1);
        assert!(q.current().is_none() && q.upcoming().is_empty());
        assert_eq!(Queue::new(vec![1u32], 99).current(), Some(&1), "start is clamped");
        assert_eq!(q.len(), 0);
    }
}
