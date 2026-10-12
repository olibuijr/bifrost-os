//! `jack_ringbuffer_*`: the lock-free single-producer single-consumer byte ring from the public header. The struct
//! layout is the header's, so apps that read the fields directly keep working.

use std::os::raw::{c_char, c_int};

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

impl Ring {
    fn read_space(&self) -> usize {
        let (w, r) = (self.write_ptr, self.read_ptr);
        if w > r { w - r } else { w.wrapping_sub(r) & self.size_mask }
    }

    fn write_space(&self) -> usize {
        let (w, r) = (self.write_ptr, self.read_ptr);
        if w > r {
            (r.wrapping_sub(w) & self.size_mask) - 1
        } else if w < r {
            r - w - 1
        } else {
            self.size - 1
        }
    }
}

// SAFETY (all functions below): `rb` comes from `jack_ringbuffer_create`; one reader and one writer at a time, as
// the ring's contract says.
#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_create(sz: usize) -> *mut Ring {
    let size = sz.max(2).next_power_of_two();
    let buf = Box::into_raw(vec![0u8; size].into_boxed_slice()) as *mut u8 as *mut c_char;
    Box::into_raw(Box::new(Ring { buf, write_ptr: 0, read_ptr: 0, size, size_mask: size - 1, mlocked: 0 }))
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_free(rb: *mut Ring) {
    if rb.is_null() {
        return;
    }
    let r = Box::from_raw(rb);
    drop(Box::from_raw(std::slice::from_raw_parts_mut(r.buf as *mut u8, r.size)));
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_mlock(rb: *mut Ring) -> c_int {
    let r = &mut *rb;
    if libc::mlock(r.buf as *const libc::c_void, r.size) != 0 {
        return -1;
    }
    r.mlocked = 1;
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_reset(rb: *mut Ring) {
    (*rb).read_ptr = 0;
    (*rb).write_ptr = 0;
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_reset_size(rb: *mut Ring, sz: usize) {
    let r = &mut *rb;
    r.size = sz;
    r.size_mask = sz.wrapping_sub(1);
    r.read_ptr = 0;
    r.write_ptr = 0;
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_read_space(rb: *const Ring) -> usize {
    (*rb).read_space()
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write_space(rb: *const Ring) -> usize {
    (*rb).write_space()
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_peek(rb: *mut Ring, dest: *mut c_char, cnt: usize) -> usize {
    let r = &*rb;
    let n = cnt.min(r.read_space());
    for i in 0..n {
        *dest.add(i) = *r.buf.add((r.read_ptr + i) & r.size_mask);
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
    let r = &mut *rb;
    r.read_ptr = (r.read_ptr + cnt) & r.size_mask;
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write(rb: *mut Ring, src: *const c_char, cnt: usize) -> usize {
    let r = &mut *rb;
    let n = cnt.min(r.write_space());
    for i in 0..n {
        *r.buf.add((r.write_ptr + i) & r.size_mask) = *src.add(i);
    }
    r.write_ptr = (r.write_ptr + n) & r.size_mask;
    n
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_write_advance(rb: *mut Ring, cnt: usize) {
    let r = &mut *rb;
    r.write_ptr = (r.write_ptr + cnt) & r.size_mask;
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_get_read_vector(rb: *const Ring, vec: *mut RingData) {
    let r = &*rb;
    let free = r.read_space();
    let end = r.read_ptr + free;
    if end > r.size {
        *vec = RingData { buf: r.buf.add(r.read_ptr), len: r.size - r.read_ptr };
        *vec.add(1) = RingData { buf: r.buf, len: end & r.size_mask };
    } else {
        *vec = RingData { buf: r.buf.add(r.read_ptr), len: free };
        *vec.add(1) = RingData { buf: std::ptr::null_mut(), len: 0 };
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_ringbuffer_get_write_vector(rb: *const Ring, vec: *mut RingData) {
    let r = &*rb;
    let free = r.write_space();
    let end = r.write_ptr + free;
    if end > r.size {
        *vec = RingData { buf: r.buf.add(r.write_ptr), len: r.size - r.write_ptr };
        *vec.add(1) = RingData { buf: r.buf, len: end & r.size_mask };
    } else {
        *vec = RingData { buf: r.buf.add(r.write_ptr), len: free };
        *vec.add(1) = RingData { buf: std::ptr::null_mut(), len: 0 };
    }
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
}
