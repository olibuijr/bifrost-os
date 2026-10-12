//! `jack_ringbuffer_*`: the lock-free single-producer single-consumer byte ring from the public header. The struct
//! layout is the header's, so apps that read the fields directly keep working.

use std::os::raw::{c_char, c_int};
use std::ptr::{addr_of, addr_of_mut};
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(C)]
pub struct RingData {
    pub buf: *mut c_char,
    pub len: usize,
}

#[repr(C)]
pub struct Ring {
    pub buf: *mut c_char,
    pub write_ptr: usize,
    pub read_ptr: usize,
    pub size: usize,
    pub size_mask: usize,
    pub mlocked: c_int,
}

// The reader and writer run on different threads, so `write_ptr` and `read_ptr` are only touched atomically and
// through raw pointers (a `&Ring` would assert they do not change). The header's `size_t` fields have the layout of
// `AtomicUsize`. The writer publishes data with a Release store of `write_ptr` and the reader sees it with an
// Acquire load, and the same the other way for `read_ptr`. `buf`, `size` and `size_mask` change only in
// `create`, `reset` and `reset_size`, which the contract says run with no reader or writer active.
unsafe fn ptr_of(p: *mut usize) -> &'static AtomicUsize {
    AtomicUsize::from_ptr(p)
}

unsafe fn load_w(rb: *const Ring) -> usize {
    ptr_of(addr_of!((*rb).write_ptr) as *mut usize).load(Ordering::Acquire)
}

unsafe fn load_r(rb: *const Ring) -> usize {
    ptr_of(addr_of!((*rb).read_ptr) as *mut usize).load(Ordering::Acquire)
}

unsafe fn store_w(rb: *mut Ring, v: usize) {
    ptr_of(addr_of_mut!((*rb).write_ptr)).store(v, Ordering::Release)
}

unsafe fn store_r(rb: *mut Ring, v: usize) {
    ptr_of(addr_of_mut!((*rb).read_ptr)).store(v, Ordering::Release)
}

unsafe fn read_space(rb: *const Ring) -> usize {
    let (w, r) = (load_w(rb), load_r(rb));
    if w > r { w - r } else { w.wrapping_sub(r) & (*rb).size_mask }
}

unsafe fn write_space(rb: *const Ring) -> usize {
    let (w, r) = (load_w(rb), load_r(rb));
    if w > r {
        (r.wrapping_sub(w) & (*rb).size_mask) - 1
    } else if w < r {
        r - w - 1
    } else {
        (*rb).size - 1
    }
}

// SAFETY (all functions below): `rb` comes from `jack_ringbuffer_create`; one reader and one writer at a time, as
// the ring's contract says.
#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_create(sz: usize) -> *mut Ring {
    let Some(size) = sz.max(2).checked_next_power_of_two() else { return std::ptr::null_mut() };
    // malloc, like the reference library, so `reset_size` can shrink `size` and `free` still releases it all.
    let buf = libc::calloc(size, 1) as *mut c_char;
    if buf.is_null() {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(Ring { buf, write_ptr: 0, read_ptr: 0, size, size_mask: size - 1, mlocked: 0 }))
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_free(rb: *mut Ring) {
    if rb.is_null() {
        return;
    }
    let r = Box::from_raw(rb);
    if r.mlocked != 0 {
        libc::munlock(r.buf as *const libc::c_void, r.size);
    }
    libc::free(r.buf as *mut libc::c_void);
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_mlock(rb: *mut Ring) -> c_int {
    if libc::mlock((*rb).buf as *const libc::c_void, (*rb).size) != 0 {
        return -1;
    }
    (*rb).mlocked = 1;
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_reset(rb: *mut Ring) {
    store_r(rb, 0);
    store_w(rb, 0);
}

/// Sets the usable size; `sz` must be a power of two no larger than the size the ring was created with.
#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_reset_size(rb: *mut Ring, sz: usize) {
    if sz == 0 || !sz.is_power_of_two() || sz > (*rb).size {
        return;
    }
    (*rb).size = sz;
    (*rb).size_mask = sz - 1;
    store_r(rb, 0);
    store_w(rb, 0);
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_read_space(rb: *const Ring) -> usize {
    read_space(rb)
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write_space(rb: *const Ring) -> usize {
    write_space(rb)
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_peek(rb: *mut Ring, dest: *mut c_char, cnt: usize) -> usize {
    let n = cnt.min(read_space(rb));
    let (buf, mask, rp) = ((*rb).buf, (*rb).size_mask, load_r(rb));
    for i in 0..n {
        *dest.add(i) = *buf.add((rp + i) & mask);
    }
    n
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_read(rb: *mut Ring, dest: *mut c_char, cnt: usize) -> usize {
    let n = jack_ringbuffer_peek(rb, dest, cnt);
    jack_ringbuffer_read_advance(rb, n);
    n
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_read_advance(rb: *mut Ring, cnt: usize) {
    store_r(rb, (load_r(rb) + cnt) & (*rb).size_mask);
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write(rb: *mut Ring, src: *const c_char, cnt: usize) -> usize {
    let n = cnt.min(write_space(rb));
    let (buf, mask, wp) = ((*rb).buf, (*rb).size_mask, load_w(rb));
    for i in 0..n {
        *buf.add((wp + i) & mask) = *src.add(i);
    }
    jack_ringbuffer_write_advance(rb, n);
    n
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write_advance(rb: *mut Ring, cnt: usize) {
    store_w(rb, (load_w(rb) + cnt) & (*rb).size_mask);
}

unsafe fn vector(rb: *const Ring, start: usize, free: usize, vec: *mut RingData) {
    let (buf, size, mask) = ((*rb).buf, (*rb).size, (*rb).size_mask);
    let end = start + free;
    if end > size {
        *vec = RingData { buf: buf.add(start), len: size - start };
        *vec.add(1) = RingData { buf, len: end & mask };
    } else {
        *vec = RingData { buf: buf.add(start), len: free };
        *vec.add(1) = RingData { buf: std::ptr::null_mut(), len: 0 };
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_get_read_vector(rb: *const Ring, vec: *mut RingData) {
    vector(rb, load_r(rb), read_space(rb), vec)
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_get_write_vector(rb: *const Ring, vec: *mut RingData) {
    vector(rb, load_w(rb), write_space(rb), vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_rounds_up_and_usable_space_is_size_minus_one() {
        unsafe {
            let rb = jack_ringbuffer_create(10);
            assert_eq!((*rb).size, 16);
            assert_eq!(jack_ringbuffer_write_space(rb), 15);
            assert_eq!(jack_ringbuffer_read_space(rb), 0);
            jack_ringbuffer_free(rb);
        }
    }

    #[test]
    fn write_wraps_and_reads_back_in_order() {
        unsafe {
            let rb = jack_ringbuffer_create(8);
            let a = [1 as c_char, 2, 3, 4, 5, 6];
            assert_eq!(jack_ringbuffer_write(rb, a.as_ptr(), 6), 6);
            let mut out = [0 as c_char; 4];
            assert_eq!(jack_ringbuffer_read(rb, out.as_mut_ptr(), 4), 4);
            assert_eq!(out, [1, 2, 3, 4]);
            // write pointer 6, read pointer 4: the space is 4 - 6 + 8 - 1 = 5, so five of six bytes fit and wrap.
            let b = [7 as c_char, 8, 9, 10, 11, 12];
            assert_eq!(jack_ringbuffer_write(rb, b.as_ptr(), 6), 5);
            let mut v = [RingData { buf: std::ptr::null_mut(), len: 0 }, RingData { buf: std::ptr::null_mut(), len: 0 }];
            jack_ringbuffer_get_read_vector(rb, v.as_mut_ptr());
            // 7 bytes to read from index 4 of 8: 4 up to the end, 3 from the start.
            assert_eq!((v[0].len, v[1].len), (4, 3));
            let mut all = [0 as c_char; 8];
            assert_eq!(jack_ringbuffer_read(rb, all.as_mut_ptr(), 8), 7);
            assert_eq!(&all[..7], &[5, 6, 7, 8, 9, 10, 11]);
            jack_ringbuffer_free(rb);
        }
    }

    #[test]
    fn write_vector_covers_the_free_space_and_advance_commits_it() {
        unsafe {
            let rb = jack_ringbuffer_create(8);
            let mut v = [RingData { buf: std::ptr::null_mut(), len: 0 }, RingData { buf: std::ptr::null_mut(), len: 0 }];
            jack_ringbuffer_get_write_vector(rb, v.as_mut_ptr());
            assert_eq!((v[0].len, v[1].len), (7, 0));
            for i in 0..3 {
                *v[0].buf.add(i) = (i + 1) as c_char;
            }
            jack_ringbuffer_write_advance(rb, 3);
            assert_eq!(jack_ringbuffer_read_space(rb), 3);
            let mut two = [0 as c_char; 2];
            assert_eq!(jack_ringbuffer_peek(rb, two.as_mut_ptr(), 2), 2);
            assert_eq!(two, [1, 2]);
            assert_eq!(jack_ringbuffer_read_space(rb), 3, "peek does not consume");
            jack_ringbuffer_read_advance(rb, 3);
            // write 3, read 3: 7 free, from index 3 to the end (5) and 2 from the start
            jack_ringbuffer_get_write_vector(rb, v.as_mut_ptr());
            assert_eq!((v[0].len, v[1].len), (5, 2));
            jack_ringbuffer_free(rb);
        }
    }

    #[test]
    fn a_full_ring_refuses_more_and_reset_empties_it() {
        unsafe {
            let rb = jack_ringbuffer_create(4);
            let a = [9 as c_char; 8];
            assert_eq!(jack_ringbuffer_write(rb, a.as_ptr(), 8), 3);
            assert_eq!(jack_ringbuffer_write_space(rb), 0);
            assert_eq!(jack_ringbuffer_write(rb, a.as_ptr(), 1), 0);
            jack_ringbuffer_reset(rb);
            assert_eq!((jack_ringbuffer_read_space(rb), jack_ringbuffer_write_space(rb)), (0, 3));
            jack_ringbuffer_free(rb);
        }
    }

    #[test]
    fn reset_size_only_shrinks_to_a_power_of_two_and_free_still_works() {
        unsafe {
            let rb = jack_ringbuffer_create(64);
            jack_ringbuffer_reset_size(rb, 100);
            assert_eq!((*rb).size, 64);
            jack_ringbuffer_reset_size(rb, 12);
            assert_eq!((*rb).size, 64);
            jack_ringbuffer_reset_size(rb, 16);
            assert_eq!(((*rb).size, (*rb).size_mask, jack_ringbuffer_write_space(rb)), (16, 15, 15));
            jack_ringbuffer_free(rb);
            jack_ringbuffer_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn one_writer_and_one_reader_on_two_threads_see_every_byte_in_order() {
        struct P(*mut Ring);
        unsafe impl Send for P {}
        const N: usize = 200_000;
        let rb = unsafe { jack_ringbuffer_create(64) };
        let w = P(rb);
        let writer = std::thread::spawn(move || {
            let w = w;
            let mut next = 0usize;
            while next < N {
                let chunk: Vec<c_char> = (next..(next + 13).min(N)).map(|i| (i % 251) as c_char).collect();
                let n = unsafe { jack_ringbuffer_write(w.0, chunk.as_ptr(), chunk.len()) };
                next += n;
                if n == 0 {
                    std::thread::yield_now();
                }
            }
        });
        let mut got = 0usize;
        let mut buf = [0 as c_char; 17];
        while got < N {
            let n = unsafe { jack_ringbuffer_read(rb, buf.as_mut_ptr(), buf.len()) };
            for (k, b) in buf[..n].iter().enumerate() {
                assert_eq!(*b, ((got + k) % 251) as c_char, "byte {}", got + k);
            }
            got += n;
            if n == 0 {
                std::thread::yield_now();
            }
        }
        writer.join().unwrap();
        unsafe { jack_ringbuffer_free(rb) };
    }
}
