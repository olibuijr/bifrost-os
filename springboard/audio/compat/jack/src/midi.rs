//! The MIDI port buffer: the layout behind `jack_midi_*`. Apps only reach it through those functions, so the
//! layout is ours. `[count u32][lost u32][data_used u32][capacity u32]`, event data from byte 16 upwards, and an
//! event table growing downwards from the end of the buffer (12 bytes per event: time, data offset, size).

const HEADER: usize = 16;
const ENTRY: usize = 12;

fn rd(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn wr(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_ne_bytes());
}

/// Empties the buffer (start of a cycle for an output port).
pub fn clear(buf: &mut [u8]) {
    wr(buf, 0, 0);
    wr(buf, 4, 0);
    wr(buf, 8, 0);
    wr(buf, 12, buf.len() as u32);
}

pub fn count(buf: &[u8]) -> u32 {
    rd(buf, 0)
}

pub fn lost(buf: &[u8]) -> u32 {
    rd(buf, 4)
}

/// The largest event that still fits.
pub fn max_event_size(buf: &[u8]) -> usize {
    let used = HEADER + rd(buf, 8) as usize + ENTRY * (rd(buf, 0) as usize + 1);
    (rd(buf, 12) as usize).saturating_sub(used)
}

/// Event `index` as (time, bytes).
pub fn get(buf: &[u8], index: u32) -> Option<(u32, &[u8])> {
    if index >= rd(buf, 0) {
        return None;
    }
    let cap = rd(buf, 12) as usize;
    let e = cap - ENTRY * (index as usize + 1);
    let (time, off, size) = (rd(buf, e), rd(buf, e + 4) as usize, rd(buf, e + 8) as usize);
    Some((time, &buf[HEADER + off..HEADER + off + size]))
}

/// Reserves `size` bytes for an event at `time`; events must be written in time order. Returns the offset of the
/// data in `buf`, or None (and counts a lost event) when it does not fit or is out of order.
pub fn reserve(buf: &mut [u8], time: u32, size: usize, frames: u32) -> Option<usize> {
    let n = rd(buf, 0);
    let in_order = time < frames && (n == 0 || get(buf, n - 1).map_or(true, |(t, _)| t <= time));
    if size == 0 || !in_order || size > max_event_size(buf) {
        let l = rd(buf, 4);
        wr(buf, 4, l + 1);
        return None;
    }
    let used = rd(buf, 8) as usize;
    let cap = rd(buf, 12) as usize;
    let e = cap - ENTRY * (n as usize + 1);
    wr(buf, e, time);
    wr(buf, e + 4, used as u32);
    wr(buf, e + 8, size as u32);
    wr(buf, 0, n + 1);
    wr(buf, 8, (used + size) as u32);
    Some(HEADER + used)
}

pub fn write(buf: &mut [u8], time: u32, data: &[u8], frames: u32) -> bool {
    match reserve(buf, time, data.len(), frames) {
        Some(at) => {
            buf[at..at + data.len()].copy_from_slice(data);
            true
        }
        None => false,
    }
}

/// Merges the events of `sources` into `dst` ordered by time (earlier sources first on equal times). At most 32
/// sources are merged. No allocation.
pub fn merge(dst: &mut [u8], sources: &[&[u8]], frames: u32) {
    clear(dst);
    let mut cur = [0u32; 32];
    let n = sources.len().min(32);
    loop {
        let mut best: Option<(usize, u32)> = None;
        for s in 0..n {
            if let Some((t, _)) = get(sources[s], cur[s]) {
                if best.map_or(true, |(_, bt)| t < bt) {
                    best = Some((s, t));
                }
            }
        }
        let Some((s, _)) = best else { break };
        let (t, data) = get(sources[s], cur[s]).unwrap();
        cur[s] += 1;
        write(dst, t, data, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Vec<u8> {
        let mut v = vec![0u8; 256];
        clear(&mut v);
        v
    }

    #[test]
    fn write_and_read_back() {
        let mut b = fresh();
        assert!(write(&mut b, 3, &[0x90, 60, 100], 64));
        assert!(write(&mut b, 10, &[0x80, 60, 0], 64));
        assert_eq!(count(&b), 2);
        assert_eq!(get(&b, 0), Some((3, &[0x90u8, 60, 100][..])));
        assert_eq!(get(&b, 1), Some((10, &[0x80u8, 60, 0][..])));
        assert_eq!(get(&b, 2), None);
    }

    #[test]
    fn out_of_order_and_overflow_count_as_lost() {
        let mut b = fresh();
        assert!(write(&mut b, 5, &[1], 64));
        assert!(!write(&mut b, 4, &[2], 64)); // earlier than the last
        assert!(!write(&mut b, 64, &[2], 64)); // time >= frames
        assert!(!write(&mut b, 6, &[0; 300], 64)); // too big
        assert_eq!(lost(&b), 3);
        assert_eq!(count(&b), 1);
    }

    #[test]
    fn max_event_size_shrinks() {
        let mut b = fresh();
        // 256 - header 16 - one table entry 12 = 228 before any event.
        assert_eq!(max_event_size(&b), 228);
        write(&mut b, 0, &[1, 2, 3], 64);
        // minus 3 data bytes and one more table entry: 228 - 3 - 12.
        assert_eq!(max_event_size(&b), 213);
    }

    #[test]
    fn merge_orders_by_time() {
        let mut a = fresh();
        let mut b = fresh();
        write(&mut a, 1, &[1], 64);
        write(&mut a, 9, &[9], 64);
        write(&mut b, 5, &[5], 64);
        write(&mut b, 9, &[10], 64);
        let mut d = fresh();
        merge(&mut d, &[&a, &b], 64);
        let got: Vec<(u32, u8)> = (0..count(&d)).map(|i| { let (t, x) = get(&d, i).unwrap(); (t, x[0]) }).collect();
        assert_eq!(got, vec![(1, 1), (5, 5), (9, 9), (9, 10)]);
    }
}
