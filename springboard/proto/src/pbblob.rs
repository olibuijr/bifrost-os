//! Pasteboard payloads that do not fit a packet travel out of band: the bytes of every item are written one after the
//! other into a memfd, and the message (`ToShell::PasteboardBlob`, `ToApp::PasteboardBlob`) carries the descriptor
//! (SCM_RIGHTS) and each item's type and length. The reader checks the sizes against the message and the file, then
//! reads with `pread`, so a sender that shortens the file afterwards cannot fault the reader.
//!
//! A payload over the limits here is refused (`Err`), never cut: no receiver sees a shortened payload under the full
//! type. What was copied is never logged.

use crate::sys::{self, Owned};
use std::fs::File;
use std::io;
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;
use std::os::unix::fs::FileExt;

/// The most items one pasteboard write carries.
pub const MAX_ITEMS: usize = 16;
/// The largest single item, in bytes (the same bound the shell puts on a type it imports from another program).
pub const MAX_ITEM: usize = 32 << 20;
/// The most all items of one write weigh together, in bytes.
pub const MAX_TOTAL: usize = 64 << 20;

/// Whether `items` ride in one packet (`ToApp::Pasteboard`, `ToShell::PasteboardSet`) without leaving anything out.
pub fn fits_inline(items: &[(String, Vec<u8>)]) -> bool {
    items.len() <= MAX_ITEMS && items.iter().all(|(_, d)| d.len() <= crate::inline_item_limit(items.len()))
}

/// The type and length of each item: the message's description of the descriptor's contents.
pub type Layout = Vec<(String, u32)>;

/// Checks a layout the same way for the writer and the reader: at most [`MAX_ITEMS`] items, each within [`MAX_ITEM`], all
/// within [`MAX_TOTAL`]. Returns the total.
pub fn check(types: &[(String, u32)]) -> io::Result<usize> {
    if types.len() > MAX_ITEMS {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many pasteboard items"));
    }
    let mut total = 0usize;
    for (_, n) in types {
        let n = *n as usize;
        if n > MAX_ITEM {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a pasteboard item is too large"));
        }
        total += n;
    }
    if total > MAX_TOTAL {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the pasteboard payload is too large"));
    }
    Ok(total)
}

/// Writes `items` into a new memfd and describes them. Fails for a payload over the limits.
///
/// An item whose type has a control character is left out, as the inline message leaves it out: the reader refuses a
/// layout with one, and that would end the app.
pub fn write(items: &[(String, Vec<u8>)]) -> io::Result<(Owned, Layout)> {
    let items: Vec<&(String, Vec<u8>)> = items.iter().filter(|(t, _)| crate::valid_pb_type(t)).collect();
    let layout: Layout = items
        .iter()
        .map(|(t, d)| u32::try_from(d.len()).map(|n| (crate::cut_type(t).to_string(), n)).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a pasteboard item is too large")))
        .collect::<io::Result<_>>()?;
    let total = check(&layout)?;
    let fd = sys::shm_create("pasteboard", total)?;
    // SAFETY: the descriptor stays owned by `fd`; `ManuallyDrop` keeps the `File` from closing it.
    let file = ManuallyDrop::new(unsafe { File::from_raw_fd(fd.0) });
    let mut at = 0u64;
    for (_, data) in &items {
        file.write_all_at(data, at)?;
        at += data.len() as u64;
    }
    Ok((fd, layout))
}

/// Reads the items `layout` describes from `fd`. Fails for a layout over the limits, a descriptor that is not a seekable
/// file, or one shorter than the layout claims.
pub fn read(fd: &Owned, layout: &[(String, u32)]) -> io::Result<Vec<(String, Vec<u8>)>> {
    let total = check(layout)?;
    if sys::shm_len(fd)? < total {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the pasteboard descriptor is shorter than its layout"));
    }
    // SAFETY: the descriptor stays owned by `fd`; `ManuallyDrop` keeps the `File` from closing it.
    let file = ManuallyDrop::new(unsafe { File::from_raw_fd(fd.0) });
    let mut at = 0u64;
    let mut out = Vec::with_capacity(layout.len());
    for (t, n) in layout {
        let mut data = vec![0u8; *n as usize];
        file.read_exact_at(&mut data, at)?;
        at += *n as u64;
        out.push((t.clone(), data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(String, Vec<u8>)> {
        vec![("public.png".into(), vec![7; 100_000]), ("public.utf8-plain-text".into(), b"hi".to_vec()), ("x.empty".into(), Vec::new())]
    }

    #[test]
    fn items_round_trip_through_a_memfd_in_order() {
        let (fd, layout) = write(&items()).unwrap();
        assert_eq!(layout, vec![("public.png".to_string(), 100_000), ("public.utf8-plain-text".to_string(), 2), ("x.empty".to_string(), 0)]);
        assert_eq!(read(&fd, &layout).unwrap(), items());
    }

    #[test]
    fn a_layout_over_the_limits_is_refused_both_ways() {
        assert!(check(&vec![("t".to_string(), 1); MAX_ITEMS + 1]).is_err());
        assert!(check(&[("t".to_string(), (MAX_ITEM + 1) as u32)]).is_err());
        let heavy = vec![("a".to_string(), MAX_ITEM as u32), ("b".to_string(), MAX_ITEM as u32), ("c".to_string(), 1)];
        assert!(check(&heavy).is_err());
        let (fd, _) = write(&items()).unwrap();
        assert!(read(&fd, &heavy).is_err(), "the reader checks the layout it was sent");
    }

    #[test]
    fn a_descriptor_shorter_than_the_layout_is_refused() {
        let (fd, _) = write(&[("t".to_string(), vec![1, 2, 3])]).unwrap();
        assert!(read(&fd, &[("t".to_string(), 10)]).is_err());
    }

    #[test]
    fn an_item_with_a_control_character_in_its_type_is_left_out_of_the_blob() {
        let mixed = vec![("bad\0type".to_string(), vec![1; 10]), ("public.png".to_string(), vec![2; 5])];
        let (fd, layout) = write(&mixed).unwrap();
        assert_eq!(layout, vec![("public.png".to_string(), 5)]);
        assert_eq!(read(&fd, &layout).unwrap(), vec![("public.png".to_string(), vec![2; 5])]);
    }

    #[test]
    fn only_small_items_ride_inline() {
        assert!(fits_inline(&[("public.utf8-plain-text".into(), vec![0; 100])]));
        assert!(!fits_inline(&[("public.png".into(), vec![0; 50_000])]));
        assert!(fits_inline(&[]));
    }
}
