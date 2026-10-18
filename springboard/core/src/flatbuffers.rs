//! A builder and a checking reader for the FlatBuffers binary format: a serialisation whose buffers are read in
//! place, with no parsing step. This is the public wire layout (little-endian scalars, tables with a vtable of
//! field offsets, strings and vectors with a 32-bit length, offsets relative to the place that holds them), so a
//! buffer written here is readable by any implementation of the format and the other way round.
//!
//! The builder fills the buffer from the end, as the format needs. Children (strings, vectors, tables) are built
//! first and passed to their parent by [`Ref`]; a table is started, its fields added in any order, and ended.
//! A field equal to its default is not written. The reader checks every offset and length against the buffer,
//! so a damaged or hostile buffer gives an [`Error`] and never reads outside the slice. Vtables are not shared
//! between tables when building; readers do not need that.

use std::fmt;

/// Why a read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An offset, length or index points outside the buffer.
    OutOfBounds,
    /// A string is not valid UTF-8.
    BadUtf8,
    /// A table's vtable position is before the start of the buffer.
    BadVtable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::OutOfBounds => "an offset points outside the buffer",
            Error::BadUtf8 => "a string is not valid UTF-8",
            Error::BadVtable => "a table has an invalid vtable",
        })
    }
}

impl std::error::Error for Error {}

/// A fixed-size little-endian value that can be a table field or a vector element.
pub trait Scalar: Copy + PartialEq {
    const SIZE: usize;
    fn read_le(bytes: &[u8]) -> Self;
    fn write_le(self, out: &mut [u8]);
}

macro_rules! scalar {
    ($($t:ty),*) => {$(
        impl Scalar for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn read_le(bytes: &[u8]) -> Self {
                let mut a = [0u8; std::mem::size_of::<$t>()];
                a.copy_from_slice(&bytes[..std::mem::size_of::<$t>()]);
                <$t>::from_le_bytes(a)
            }
            fn write_le(self, out: &mut [u8]) {
                out[..std::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }
        }
    )*};
}
scalar!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

/// A finished child object (table, string or vector), by its distance from the end of the buffer under construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ref(usize);

/// Builds one buffer.
#[derive(Debug)]
pub struct Builder {
    buf: Vec<u8>,
    head: usize,
    min_align: usize,
    vtable: Vec<usize>,
    object_start: usize,
    in_table: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Builder {
        Builder { buf: vec![0; 64], head: 64, min_align: 1, vtable: Vec::new(), object_start: 0, in_table: false }
    }

    /// Bytes written so far.
    fn offset(&self) -> usize {
        self.buf.len() - self.head
    }

    fn grow(&mut self) {
        let old = self.buf.len();
        let new = old * 2;
        let mut bigger = vec![0u8; new];
        bigger[new - old..].copy_from_slice(&self.buf);
        self.buf = bigger;
        self.head += new - old;
    }

    /// Makes room for `size` bytes plus `additional`, aligned so that the `size`-byte value ends up on a multiple of
    /// `size` from the end of the buffer once `additional` bytes follow it.
    fn prep(&mut self, size: usize, additional: usize) {
        if size > self.min_align {
            self.min_align = size;
        }
        let align = (!(self.offset() + additional)).wrapping_add(1) & (size - 1);
        while self.head <= align + size + additional {
            self.grow();
        }
        for _ in 0..align {
            self.head -= 1;
            self.buf[self.head] = 0;
        }
    }

    fn prepend<T: Scalar>(&mut self, v: T) {
        self.prep(T::SIZE, 0);
        self.head -= T::SIZE;
        v.write_le(&mut self.buf[self.head..]);
    }

    fn prepend_uoffset(&mut self, target: Ref) {
        self.prep(4, 0);
        debug_assert!(target.0 <= self.offset(), "a child must be built before its parent");
        let relative = (self.offset() - target.0 + 4) as u32;
        self.prepend(relative);
    }

    fn assert_not_in_table(&self) {
        assert!(!self.in_table, "strings, vectors and tables are built before the table that holds them is started");
    }

    /// A string: a 32-bit length, the UTF-8 bytes and a zero byte.
    pub fn create_string(&mut self, s: &str) -> Ref {
        self.assert_not_in_table();
        self.prep(4, s.len() + 1);
        self.prepend(0u8);
        self.create_bytes_inner(s.as_bytes(), 1)
    }

    /// A `[ubyte]` vector.
    pub fn create_bytes(&mut self, bytes: &[u8]) -> Ref {
        self.assert_not_in_table();
        self.create_bytes_inner(bytes, 1)
    }

    fn create_bytes_inner(&mut self, bytes: &[u8], align: usize) -> Ref {
        self.prep(4, bytes.len());
        self.prep(align, bytes.len());
        while self.head <= bytes.len() {
            self.grow();
        }
        self.head -= bytes.len();
        self.buf[self.head..self.head + bytes.len()].copy_from_slice(bytes);
        self.prepend(bytes.len() as u32);
        Ref(self.offset())
    }

    /// A vector of scalars of one type.
    pub fn create_vector<T: Scalar>(&mut self, items: &[T]) -> Ref {
        self.assert_not_in_table();
        self.prep(4, T::SIZE * items.len());
        self.prep(T::SIZE, T::SIZE * items.len());
        for v in items.iter().rev() {
            self.prepend(*v);
        }
        self.prepend(items.len() as u32);
        Ref(self.offset())
    }

    /// A vector of offsets to tables or strings built earlier.
    pub fn create_ref_vector(&mut self, items: &[Ref]) -> Ref {
        self.assert_not_in_table();
        self.prep(4, 4 * items.len());
        for r in items.iter().rev() {
            self.prepend_uoffset(*r);
        }
        self.prepend(items.len() as u32);
        Ref(self.offset())
    }

    /// Starts a table with `fields` field slots (slot numbers `0..fields`).
    pub fn start_table(&mut self, fields: usize) {
        self.assert_not_in_table();
        self.in_table = true;
        self.vtable.clear();
        self.vtable.resize(fields, 0);
        self.object_start = self.offset();
    }

    fn slot(&mut self, slot: usize) {
        assert!(self.in_table, "a field is added between start_table and end_table");
        self.vtable[slot] = self.offset();
    }

    /// Adds a scalar field unless it equals its default.
    pub fn add<T: Scalar>(&mut self, slot: usize, value: T, default: T) {
        assert!(self.in_table, "a field is added between start_table and end_table");
        if value != default {
            self.prepend(value);
            self.slot(slot);
        }
    }

    pub fn add_bool(&mut self, slot: usize, value: bool, default: bool) {
        self.add(slot, value as u8, default as u8);
    }

    /// Adds a field that points at a string, vector or table built earlier.
    pub fn add_ref(&mut self, slot: usize, target: Ref) {
        assert!(self.in_table, "a field is added between start_table and end_table");
        self.prepend_uoffset(target);
        self.slot(slot);
    }

    /// Ends the table: writes its vtable and returns the table.
    pub fn end_table(&mut self) -> Ref {
        assert!(self.in_table, "end_table without start_table");
        self.prepend(0i32);
        let object_offset = self.offset();
        while self.vtable.last() == Some(&0) {
            self.vtable.pop();
        }
        let vtable = std::mem::take(&mut self.vtable);
        for &slot in vtable.iter().rev() {
            let field = if slot != 0 { (object_offset - slot) as u16 } else { 0 };
            self.prepend(field);
        }
        self.prepend((object_offset - self.object_start) as u16);
        self.prepend(((vtable.len() + 2) * 2) as u16);
        let at = self.buf.len() - object_offset;
        let soffset = (self.offset() - object_offset) as i32;
        soffset.write_le(&mut self.buf[at..]);
        self.in_table = false;
        Ref(object_offset)
    }

    /// Finishes the buffer with `root` as its root table and an optional four-byte file identifier.
    pub fn finish(mut self, root: Ref, identifier: Option<[u8; 4]>) -> Vec<u8> {
        self.assert_not_in_table();
        let extra = if identifier.is_some() { 4 } else { 0 };
        let align = self.min_align;
        self.prep(align, 4 + extra);
        if let Some(id) = identifier {
            for b in id.iter().rev() {
                self.prepend(*b);
            }
        }
        self.prepend_uoffset(root);
        self.buf[self.head..].to_vec()
    }
}

fn slice(buf: &[u8], at: usize, len: usize) -> Result<&[u8], Error> {
    let end = at.checked_add(len).ok_or(Error::OutOfBounds)?;
    buf.get(at..end).ok_or(Error::OutOfBounds)
}

fn read<T: Scalar>(buf: &[u8], at: usize) -> Result<T, Error> {
    Ok(T::read_le(slice(buf, at, T::SIZE)?))
}

/// Follows the offset stored at `at`: its target is `at` plus the stored value.
fn follow(buf: &[u8], at: usize) -> Result<usize, Error> {
    let rel = read::<u32>(buf, at)? as usize;
    let target = at.checked_add(rel).ok_or(Error::OutOfBounds)?;
    if target > buf.len() {
        return Err(Error::OutOfBounds);
    }
    Ok(target)
}

/// The four-byte file identifier, if the buffer is long enough to hold one.
pub fn file_identifier(buf: &[u8]) -> Option<[u8; 4]> {
    buf.get(4..8).map(|b| [b[0], b[1], b[2], b[3]])
}

/// The root table of a buffer.
pub fn root(buf: &[u8]) -> Result<Table<'_>, Error> {
    let pos = follow(buf, 0)?;
    Table::at(buf, pos)
}

/// A table inside a buffer.
#[derive(Clone, Copy, Debug)]
pub struct Table<'a> {
    buf: &'a [u8],
    pos: usize,
    vtable: usize,
    vtable_len: usize,
}

impl<'a> Table<'a> {
    fn at(buf: &'a [u8], pos: usize) -> Result<Table<'a>, Error> {
        let soffset = read::<i32>(buf, pos)? as i64;
        let vtable = pos as i64 - soffset;
        if vtable < 0 {
            return Err(Error::BadVtable);
        }
        let vtable = vtable as usize;
        let vtable_len = read::<u16>(buf, vtable)? as usize;
        slice(buf, vtable, vtable_len.max(4))?;
        Ok(Table { buf, pos, vtable, vtable_len })
    }

    /// Where the field of `slot` is stored, or `None` when it was left at its default.
    fn field(&self, slot: usize) -> Result<Option<usize>, Error> {
        let entry = 4 + 2 * slot;
        if entry + 2 > self.vtable_len {
            return Ok(None);
        }
        let off = read::<u16>(self.buf, self.vtable + entry)? as usize;
        Ok(if off == 0 { None } else { Some(self.pos + off) })
    }

    /// Whether the field of `slot` is present (written, not defaulted).
    pub fn has(&self, slot: usize) -> bool {
        matches!(self.field(slot), Ok(Some(_)))
    }

    /// A scalar field, or `default` when it was not written.
    pub fn get<T: Scalar>(&self, slot: usize, default: T) -> Result<T, Error> {
        match self.field(slot)? {
            Some(at) => read(self.buf, at),
            None => Ok(default),
        }
    }

    pub fn get_bool(&self, slot: usize, default: bool) -> Result<bool, Error> {
        Ok(self.get::<u8>(slot, default as u8)? != 0)
    }

    /// A nested table.
    pub fn table(&self, slot: usize) -> Result<Option<Table<'a>>, Error> {
        match self.field(slot)? {
            Some(at) => Ok(Some(Table::at(self.buf, follow(self.buf, at)?)?)),
            None => Ok(None),
        }
    }

    /// A string field.
    pub fn string(&self, slot: usize) -> Result<Option<&'a str>, Error> {
        match self.field(slot)? {
            Some(at) => string_at(self.buf, follow(self.buf, at)?).map(Some),
            None => Ok(None),
        }
    }

    /// A vector field.
    pub fn vector(&self, slot: usize) -> Result<Option<Vector<'a>>, Error> {
        match self.field(slot)? {
            Some(at) => Vector::at(self.buf, follow(self.buf, at)?).map(Some),
            None => Ok(None),
        }
    }
}

fn string_at(buf: &[u8], pos: usize) -> Result<&str, Error> {
    let len = read::<u32>(buf, pos)? as usize;
    let bytes = slice(buf, pos + 4, len)?;
    std::str::from_utf8(bytes).map_err(|_| Error::BadUtf8)
}

/// A vector inside a buffer.
#[derive(Clone, Copy, Debug)]
pub struct Vector<'a> {
    buf: &'a [u8],
    start: usize,
    len: usize,
}

impl<'a> Vector<'a> {
    fn at(buf: &'a [u8], pos: usize) -> Result<Vector<'a>, Error> {
        let len = read::<u32>(buf, pos)? as usize;
        Ok(Vector { buf, start: pos + 4, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Checks that `len` elements of `size` bytes fit in the buffer, then reads element `i`.
    pub fn get<T: Scalar>(&self, i: usize) -> Result<T, Error> {
        if i >= self.len {
            return Err(Error::OutOfBounds);
        }
        read(self.buf, self.start + i * T::SIZE)
    }

    /// The bytes of a `[ubyte]` vector.
    pub fn bytes(&self) -> Result<&'a [u8], Error> {
        slice(self.buf, self.start, self.len)
    }

    /// Element `i` of a vector of tables.
    pub fn table(&self, i: usize) -> Result<Table<'a>, Error> {
        if i >= self.len {
            return Err(Error::OutOfBounds);
        }
        Table::at(self.buf, follow(self.buf, self.start + i * 4)?)
    }

    /// Element `i` of a vector of strings.
    pub fn string(&self, i: usize) -> Result<&'a str, Error> {
        if i >= self.len {
            return Err(Error::OutOfBounds);
        }
        string_at(self.buf, follow(self.buf, self.start + i * 4)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_with_one_field_has_the_documented_layout() {
        let mut b = Builder::new();
        b.start_table(1);
        b.add(0, 5i32, 0);
        let t = b.end_table();
        let bytes = b.finish(t, None);
        let expect: Vec<u8> = vec![12, 0, 0, 0, 0, 0, 6, 0, 8, 0, 4, 0, 6, 0, 0, 0, 5, 0, 0, 0];
        assert_eq!(bytes, expect);
        assert_eq!(root(&bytes).unwrap().get::<i32>(0, -1).unwrap(), 5);
    }

    #[test]
    fn defaults_are_not_written_and_read_back() {
        let mut b = Builder::new();
        b.start_table(3);
        b.add(0, 0i32, 0);
        b.add(1, 7u16, 7);
        b.add_bool(2, true, false);
        let t = b.end_table();
        let bytes = b.finish(t, None);
        let r = root(&bytes).unwrap();
        assert!(!r.has(0) && !r.has(1) && r.has(2));
        assert_eq!(r.get::<i32>(0, 9).unwrap(), 9);
        assert_eq!(r.get::<u16>(1, 7).unwrap(), 7);
        assert!(r.get_bool(2, false).unwrap());
        // a slot beyond the vtable is a default too
        assert_eq!(r.get::<u8>(40, 3).unwrap(), 3);
    }

    #[test]
    fn strings_vectors_and_nested_tables_round_trip() {
        let mut b = Builder::new();
        let name = b.create_string("Bifröst");
        let nums = b.create_vector(&[1i32, -2, 3, 40]);
        let blob = b.create_bytes(&[9, 8, 7]);
        b.start_table(1);
        b.add(0, 1.5f64, 0.0);
        let child = b.end_table();
        let a = b.create_string("a");
        let c = b.create_string("cc");
        let strs = b.create_ref_vector(&[a, c]);
        let kids = b.create_ref_vector(&[child, child]);
        b.start_table(7);
        b.add_ref(0, name);
        b.add_ref(1, nums);
        b.add_ref(2, child);
        b.add(3, -4i64, 0);
        b.add_ref(4, blob);
        b.add_ref(5, strs);
        b.add_ref(6, kids);
        let t = b.end_table();
        let bytes = b.finish(t, Some(*b"BIFR"));
        assert_eq!(file_identifier(&bytes), Some(*b"BIFR"));
        let r = root(&bytes).unwrap();
        assert_eq!(r.string(0).unwrap(), Some("Bifröst"));
        let v = r.vector(1).unwrap().unwrap();
        assert_eq!(v.len(), 4);
        assert_eq!((0..4).map(|i| v.get::<i32>(i).unwrap()).collect::<Vec<_>>(), vec![1, -2, 3, 40]);
        assert_eq!(v.get::<i32>(4), Err(Error::OutOfBounds));
        assert_eq!(r.table(2).unwrap().unwrap().get::<f64>(0, 0.0).unwrap(), 1.5);
        assert_eq!(r.get::<i64>(3, 0).unwrap(), -4);
        assert_eq!(r.vector(4).unwrap().unwrap().bytes().unwrap(), &[9, 8, 7]);
        let s = r.vector(5).unwrap().unwrap();
        assert_eq!((s.string(0).unwrap(), s.string(1).unwrap()), ("a", "cc"));
        let k = r.vector(6).unwrap().unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(k.table(1).unwrap().get::<f64>(0, 0.0).unwrap(), 1.5);
    }

    #[test]
    fn many_tables_grow_the_buffer_and_stay_aligned() {
        let mut b = Builder::new();
        let mut kids = Vec::new();
        for i in 0..200u32 {
            let s = b.create_string(&format!("item {i}"));
            b.start_table(2);
            b.add(0, i, 0);
            b.add_ref(1, s);
            kids.push(b.end_table());
        }
        let list = b.create_ref_vector(&kids);
        b.start_table(1);
        b.add_ref(0, list);
        let t = b.end_table();
        let bytes = b.finish(t, None);
        let v = root(&bytes).unwrap().vector(0).unwrap().unwrap();
        assert_eq!(v.len(), 200);
        for i in 0..200usize {
            let t = v.table(i).unwrap();
            assert_eq!(t.get::<u32>(0, 0).unwrap() as usize, i);
            assert_eq!(t.string(1).unwrap().unwrap(), format!("item {i}"));
        }
    }

    #[test]
    fn damaged_buffers_are_errors_and_never_panic() {
        assert_eq!(root(&[]).err(), Some(Error::OutOfBounds));
        assert_eq!(root(&[255, 255, 255, 255]).err(), Some(Error::OutOfBounds));
        let mut b = Builder::new();
        let s = b.create_string("hello");
        b.start_table(1);
        b.add_ref(0, s);
        let t = b.end_table();
        let good = b.finish(t, None);
        // every truncation and every single-byte change must give a value or an error, not a panic
        for cut in 0..good.len() {
            if let Ok(r) = root(&good[..cut]) {
                let _ = r.string(0);
                let _ = r.get::<u64>(0, 0);
            }
        }
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] = bad[i].wrapping_add(0x80);
            if let Ok(r) = root(&bad) {
                let _ = r.string(0);
                let _ = r.vector(0).map(|v| v.map(|v| v.bytes()));
            }
        }
    }

    #[test]
    fn invalid_utf8_is_refused() {
        let mut b = Builder::new();
        let s = b.create_bytes(&[0xff, 0xfe]);
        b.start_table(1);
        b.add_ref(0, s);
        let t = b.end_table();
        let bytes = b.finish(t, None);
        assert_eq!(root(&bytes).unwrap().string(0), Err(Error::BadUtf8));
    }
}
