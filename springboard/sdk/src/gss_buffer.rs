//! GSS-API buffer management (`gss_buffer_desc`, `gss_buffer_set_desc`, `gss_create_empty_buffer_set`,
//! `gss_add_buffer_set_member`, `gss_release_buffer`, `gss_release_buffer_set`): octet streams exchanged with the
//! GSS-API functions, and sets that hold several of them.
//!
//! A buffer owns its bytes. Adding a buffer to a set copies the bytes in, as `gss_add_buffer_set_member` does, so the
//! caller may reuse its own storage afterwards. The release calls are replaced by drop: a buffer or a set frees its
//! memory when it goes out of scope. The IOV buffer forms (`gss_iov_buffer_desc`) are not modelled, because the page
//! names them without their type constants.

/// `gss_buffer_desc`: an owned octet stream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buffer {
    bytes: Vec<u8>,
}

impl Buffer {
    /// Takes ownership of the bytes.
    pub fn new(bytes: Vec<u8>) -> Buffer {
        Buffer { bytes }
    }

    /// Copies the bytes in.
    pub fn from_slice(bytes: &[u8]) -> Buffer {
        Buffer { bytes: bytes.to_vec() }
    }

    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// The `length` field: the number of bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True when the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// `gss_buffer_set_desc`: an array of buffers that a call returns or accepts as one unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BufferSet {
    members: Vec<Buffer>,
}

impl BufferSet {
    /// `gss_create_empty_buffer_set`: a set with no members.
    pub fn create_empty() -> BufferSet {
        BufferSet::default()
    }

    /// `gss_add_buffer_set_member`: appends a copy of the bytes. An empty slice is a valid member.
    pub fn add_member(&mut self, bytes: &[u8]) {
        self.members.push(Buffer::from_slice(bytes));
    }

    /// The member at `index`, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<&Buffer> {
        self.members.get(index)
    }

    /// The number of members (the `count` field).
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// True when the set has no members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The total number of bytes across all members, saturating at `usize::MAX`.
    pub fn total_len(&self) -> usize {
        self.members.iter().fold(0usize, |sum, b| sum.saturating_add(b.len()))
    }

    /// The members in the order they were added.
    pub fn iter(&self) -> std::slice::Iter<'_, Buffer> {
        self.members.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_reports_the_bytes_it_was_given() {
        let b = Buffer::from_slice(&[0x47, 0x53, 0x53]);
        assert_eq!(b.len(), 3);
        assert_eq!(b.as_slice(), &[0x47, 0x53, 0x53]);
        assert!(!b.is_empty());
        assert!(Buffer::default().is_empty());
    }

    #[test]
    fn an_empty_set_has_no_members() {
        let set = BufferSet::create_empty();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.get(0), None);
        assert_eq!(set.total_len(), 0);
    }

    #[test]
    fn adding_a_member_copies_the_bytes() {
        // The caller's storage changes after the add; the member keeps the value it had when it was added.
        let mut set = BufferSet::create_empty();
        let mut src = vec![9u8, 8];
        set.add_member(&src);
        src[0] = 1;
        assert_eq!(set.get(0).unwrap().as_slice(), &[9, 8]);
        assert_eq!(src, vec![1, 8]);
    }

    #[test]
    fn an_empty_member_is_kept_and_counted() {
        let mut set = BufferSet::create_empty();
        set.add_member(&[1, 2]);
        set.add_member(&[]);
        set.add_member(&[4]);
        assert_eq!(set.len(), 3);
        assert!(set.get(1).unwrap().is_empty());
    }

    #[test]
    fn total_length_sums_the_members() {
        // Members of length 3, 0 and 1 total 4 bytes.
        let mut set = BufferSet::create_empty();
        set.add_member(&[1, 2, 3]);
        set.add_member(&[]);
        set.add_member(&[4]);
        assert_eq!(set.total_len(), 4);
        assert_eq!(set.iter().count(), 3);
    }

    #[test]
    fn an_index_past_the_end_is_none() {
        let mut set = BufferSet::create_empty();
        set.add_member(&[7]);
        assert_eq!(set.get(1), None);
    }
}
