//! What a disk or partition holds, read from the magic bytes of its first sectors, and whether this computer can read
//! it. The shell uses it to tell the person when a disk they plugged in has a format it cannot mount (the disk
//! prompt: leave it, erase it, or eject it). Nothing is written and nothing is mounted: the probe reads at most a few
//! hundred bytes at four fixed offsets.
//!
//! The offsets are the public on-disk layouts: ext2/3/4 (magic 0xEF53 at 1080), FAT (the type label at 54 or 82),
//! exFAT and NTFS (the OEM name at 3), XFS (`XFSB` at 0), btrfs (`_BHRfS_M` at 65600), F2FS (0xF2F52010 at 1024),
//! ISO 9660 (`CD001` at 32769), LUKS (`LUKS` and 0xBABE at 0) and Linux swap (`SWAPSPACE2` at 4086).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// A filesystem or container the probe knows by its magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fs {
    Ext,
    Fat,
    ExFat,
    Ntfs,
    Xfs,
    Btrfs,
    F2fs,
    Iso9660,
    Luks,
    Swap,
    /// An MBR or GPT partition table: the volumes inside are probed on their own.
    Table,
}

impl Fs {
    /// The name shown to the person.
    pub fn name(self) -> &'static str {
        match self {
            Fs::Ext => "ext4",
            Fs::Fat => "FAT",
            Fs::ExFat => "exFAT",
            Fs::Ntfs => "NTFS",
            Fs::Xfs => "XFS",
            Fs::Btrfs => "btrfs",
            Fs::F2fs => "F2FS",
            Fs::Iso9660 => "ISO 9660",
            Fs::Luks => "encrypted volume",
            Fs::Swap => "swap",
            Fs::Table => "partition table",
        }
    }
}

/// What the shell should do about a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A filesystem the system mounts.
    Readable(Fs),
    /// An encrypted volume: readable after it is unlocked, so it is not reported as broken.
    Encrypted,
    /// Not data the person put there to read (swap, a partition table): no prompt.
    Ignored,
    /// Nothing recognised: the prompt is shown.
    Unreadable,
}

/// How many bytes each probe read covers: the head holds the offsets up to 4096 (swap) and the layouts below it.
pub const HEAD_LEN: usize = 4096;
const ISO_AT: u64 = 32_769;
const BTRFS_AT: u64 = 65_600;

fn at(buf: &[u8], offset: usize, want: &[u8]) -> bool {
    buf.get(offset..offset + want.len()) == Some(want)
}

/// The filesystem in `head` (the first [`HEAD_LEN`] bytes), `iso` (5 bytes from 32769) and `btrfs` (8 bytes from
/// 65600). Pass empty slices for the later two to probe the head only.
pub fn detect(head: &[u8], iso: &[u8], btrfs: &[u8]) -> Option<Fs> {
    if at(head, 0, b"LUKS\xba\xbe") {
        Some(Fs::Luks)
    } else if at(head, 3, b"EXFAT   ") {
        Some(Fs::ExFat)
    } else if at(head, 3, b"NTFS    ") {
        Some(Fs::Ntfs)
    } else if at(head, 0, b"XFSB") {
        Some(Fs::Xfs)
    } else if (at(head, 82, b"FAT32   ") || at(head, 54, b"FAT12   ") || at(head, 54, b"FAT16   ")) && at(head, 510, &[0x55, 0xAA]) {
        Some(Fs::Fat)
    } else if at(head, 1080, &[0x53, 0xEF]) {
        Some(Fs::Ext)
    } else if at(head, 1024, &[0x10, 0x20, 0xF5, 0xF2]) {
        Some(Fs::F2fs)
    } else if at(head, 4086, b"SWAPSPACE2") {
        Some(Fs::Swap)
    } else if iso == b"CD001" {
        Some(Fs::Iso9660)
    } else if btrfs == b"_BHRfS_M" {
        Some(Fs::Btrfs)
    } else if at(head, 512, b"EFI PART") || at(head, 510, &[0x55, 0xAA]) {
        Some(Fs::Table)
    } else {
        None
    }
}

/// The verdict for what `detect` found.
pub fn verdict(found: Option<Fs>) -> Verdict {
    match found {
        Some(Fs::Luks) => Verdict::Encrypted,
        Some(Fs::Swap | Fs::Table) => Verdict::Ignored,
        Some(fs) => Verdict::Readable(fs),
        None => Verdict::Unreadable,
    }
}

fn read_at(f: &mut File, offset: u64, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    if f.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }
    let mut got = 0;
    while got < len {
        match f.read(&mut buf[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    buf.truncate(got);
    buf
}

/// Reads the device or image at `path` and gives its verdict. None when it cannot be opened or read (no permission,
/// no medium): nothing is reported for it.
pub fn classify(path: &Path) -> Option<Verdict> {
    let mut f = File::open(path).ok()?;
    let head = read_at(&mut f, 0, HEAD_LEN);
    if head.is_empty() {
        return None;
    }
    let mut found = detect(&head, &[], &[]);
    if matches!(found, None | Some(Fs::Table)) {
        let iso = read_at(&mut f, ISO_AT, 5);
        let btrfs = read_at(&mut f, BTRFS_AT, 8);
        found = detect(&head, &iso, &btrfs);
    }
    Some(verdict(found))
}

/// The partitions of the disk `disk` under `sys_block`: the sub-directories that carry a `partition` file, sorted.
pub fn partitions_of(sys_block: &Path, disk: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(sys_block.join(disk))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("partition").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    out.sort();
    out
}

/// The sentence for a disk with unreadable volumes, or None when every volume is fine.
pub fn unreadable_notice(label: &str, results: &[(String, Verdict)]) -> Option<String> {
    let bad: Vec<&str> = results.iter().filter(|(_, v)| *v == Verdict::Unreadable).map(|(n, _)| n.as_str()).collect();
    match bad.as_slice() {
        [] => None,
        [one] => Some(format!("{label} ({one}) uses a format this computer cannot read. Open Disk Utility to erase or repair it.")),
        many => Some(format!("{label} has {} volumes this computer cannot read. Open Disk Utility to erase or repair them.", many.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head_with(offset: usize, bytes: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; HEAD_LEN];
        h[offset..offset + bytes.len()].copy_from_slice(bytes);
        h
    }

    #[test]
    fn each_layout_is_recognised_by_its_magic() {
        assert_eq!(detect(&head_with(1080, &[0x53, 0xEF]), &[], &[]), Some(Fs::Ext));
        assert_eq!(detect(&head_with(3, b"NTFS    "), &[], &[]), Some(Fs::Ntfs));
        assert_eq!(detect(&head_with(3, b"EXFAT   "), &[], &[]), Some(Fs::ExFat));
        assert_eq!(detect(&head_with(0, b"XFSB"), &[], &[]), Some(Fs::Xfs));
        assert_eq!(detect(&head_with(1024, &[0x10, 0x20, 0xF5, 0xF2]), &[], &[]), Some(Fs::F2fs));
        assert_eq!(detect(&head_with(0, b"LUKS\xba\xbe"), &[], &[]), Some(Fs::Luks));
        assert_eq!(detect(&head_with(4086, b"SWAPSPACE2"), &[], &[]), Some(Fs::Swap));
        assert_eq!(detect(&vec![0; HEAD_LEN], b"CD001", &[]), Some(Fs::Iso9660));
        assert_eq!(detect(&vec![0; HEAD_LEN], &[], b"_BHRfS_M"), Some(Fs::Btrfs));
    }

    #[test]
    fn a_partition_table_is_ignored_and_a_hybrid_image_is_still_an_image() {
        assert_eq!(detect(&head_with(512, b"EFI PART"), &[], &[]), Some(Fs::Table));
        assert_eq!(detect(&head_with(510, &[0x55, 0xAA]), &[], &[]), Some(Fs::Table));
        assert_eq!(verdict(Some(Fs::Table)), Verdict::Ignored);
        assert_eq!(detect(&head_with(510, &[0x55, 0xAA]), b"CD001", &[]), Some(Fs::Iso9660), "isohybrid has both");
    }

    #[test]
    fn fat_needs_its_label_and_the_boot_signature() {
        let mut h = head_with(82, b"FAT32   ");
        assert_eq!(detect(&h, &[], &[]), None, "no boot signature: not FAT");
        h[510] = 0x55;
        h[511] = 0xAA;
        assert_eq!(detect(&h, &[], &[]), Some(Fs::Fat));
        let mut h16 = head_with(54, b"FAT16   ");
        h16[510] = 0x55;
        h16[511] = 0xAA;
        assert_eq!(detect(&h16, &[], &[]), Some(Fs::Fat));
    }

    #[test]
    fn blank_and_short_data_is_unreadable_and_the_verdicts_split() {
        assert_eq!(detect(&vec![0; HEAD_LEN], &[], &[]), None);
        assert_eq!(detect(&[1, 2, 3], &[], &[]), None, "a short read is not a panic");
        assert_eq!(verdict(None), Verdict::Unreadable);
        assert_eq!(verdict(Some(Fs::Luks)), Verdict::Encrypted);
        assert_eq!(verdict(Some(Fs::Swap)), Verdict::Ignored);
        assert_eq!(verdict(Some(Fs::Ntfs)), Verdict::Readable(Fs::Ntfs));
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("disk-magic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_device_file_is_read_at_the_far_offsets_too() {
        let dir = scratch("classify");
        let ext = dir.join("ext.img");
        std::fs::write(&ext, head_with(1080, &[0x53, 0xEF])).unwrap();
        assert_eq!(classify(&ext), Some(Verdict::Readable(Fs::Ext)));
        let mut big = vec![0u8; BTRFS_AT as usize + 8];
        big[BTRFS_AT as usize..].copy_from_slice(b"_BHRfS_M");
        let btrfs = dir.join("btrfs.img");
        std::fs::write(&btrfs, big).unwrap();
        assert_eq!(classify(&btrfs), Some(Verdict::Readable(Fs::Btrfs)));
        let blank = dir.join("blank.img");
        std::fs::write(&blank, vec![0u8; 70_000]).unwrap();
        assert_eq!(classify(&blank), Some(Verdict::Unreadable));
        let empty = dir.join("empty.img");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(classify(&empty), None, "no medium");
        assert_eq!(classify(&dir.join("missing")), None);
    }

    #[test]
    fn partitions_are_the_sub_directories_with_a_partition_file() {
        let dir = scratch("parts");
        for p in ["sdb1", "sdb2"] {
            std::fs::create_dir_all(dir.join("sdb").join(p)).unwrap();
            std::fs::write(dir.join("sdb").join(p).join("partition"), "1").unwrap();
        }
        std::fs::create_dir_all(dir.join("sdb").join("queue")).unwrap();
        assert_eq!(partitions_of(&dir, "sdb"), vec!["sdb1", "sdb2"]);
        assert!(partitions_of(&dir, "sdz").is_empty());
    }

    #[test]
    fn the_notice_names_one_volume_or_counts_several() {
        let ok = ("sdb1".to_string(), Verdict::Readable(Fs::Fat));
        let bad = |n: &str| (n.to_string(), Verdict::Unreadable);
        assert_eq!(unreadable_notice("Flash Disk", &[ok.clone(), ("sdb2".into(), Verdict::Encrypted)]), None);
        assert!(unreadable_notice("Flash Disk", &[ok.clone(), bad("sdb2")]).unwrap().contains("(sdb2)"));
        assert!(unreadable_notice("Flash Disk", &[bad("sdb1"), bad("sdb2")]).unwrap().contains("2 volumes"));
    }
}
