//! The partition layout of the Partition sheet: the sizes and names the user typed, checked against the size of the
//! disk, as the `PartitionSpec` list a `DiskRequest::PartitionDisk` carries, and as the segments of the layout bar the
//! sheet draws. Pure: the sheet (`forms.rs`) and the bar (`draw.rs`) read the same numbers, and the tests run on the
//! host. Sizes are written like image sizes (`512m`, `4g`); the last partition may be `rest`, which takes what is left.

use crate::images;
use crate::model::format_size;
use bifrost_core::disk_format::{PartitionSpec, Personality, MAX_PARTITIONS};

const MIB: u64 = 1 << 20;
/// The room the partition table takes at the start and end of the disk, in MiB.
pub const TABLE_MARGIN_MIB: u64 = 2;

/// One part of the layout bar: its share of the disk, its caption, and whether it is free space left over.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub fraction: f32,
    pub caption: String,
    pub free: bool,
}

/// The MiB a disk of `bytes` offers to partitions, past the table margin.
pub fn usable_mib(bytes: u64) -> u64 {
    (bytes / MIB).saturating_sub(TABLE_MARGIN_MIB)
}

/// The text of the Sizes field that splits a disk of `bytes` into `count` equal parts, the last being `rest`.
pub fn equal_sizes_text(bytes: u64, count: usize) -> String {
    let mut words: Vec<String> = crate::model::equal_parts_mib(bytes, count).into_iter().map(|m| if m == 0 { "rest".to_string() } else { format!("{m}m") }).collect();
    if words.is_empty() {
        words.push("rest".to_string());
    }
    words.join(", ")
}

/// A size word: `rest` (also `*`, or an empty word) is None, anything else is a size in MiB of at least one.
fn size_word(word: &str) -> Result<Option<u64>, String> {
    let word = word.trim();
    if word.is_empty() || word == "*" || word.eq_ignore_ascii_case("rest") {
        return Ok(None);
    }
    let bytes = images::parse_size(word)?;
    let mib = bytes / MIB;
    if mib == 0 {
        return Err(format!("{word} is too small: a partition is at least 1 MiB."));
    }
    Ok(Some(mib))
}

/// The partitions for the typed `sizes` (comma separated) and `names` (comma separated, optional) on a disk of
/// `bytes`, all formatted as `fs`. Only the last partition may be `rest`. The sizes must fit the disk, and a layout
/// that has `rest` must leave at least 1 MiB for it. A layout without `rest` leaves free space at the end.
pub fn specs(bytes: u64, sizes: &str, names: &str, fs: Personality) -> Result<Vec<PartitionSpec>, String> {
    let words: Vec<&str> = sizes.split(',').collect();
    if words.iter().all(|w| w.trim().is_empty()) {
        return Err("Type a size for each partition, such as 512m, 4g, rest.".to_string());
    }
    if words.len() > MAX_PARTITIONS {
        return Err(format!("A disk takes at most {MAX_PARTITIONS} partitions."));
    }
    let labels: Vec<&str> = if names.trim().is_empty() { Vec::new() } else { names.split(',').collect() };
    if labels.len() > words.len() {
        return Err("There are more names than partitions.".to_string());
    }
    let last = words.len() - 1;
    let mut out = Vec::with_capacity(words.len());
    let mut fixed = 0u64;
    for (i, word) in words.iter().enumerate() {
        let size = size_word(word)?;
        if size.is_none() && i != last {
            return Err("Only the last partition can take the rest of the disk.".to_string());
        }
        fixed = fixed.saturating_add(size.unwrap_or(0));
        let label = labels.get(i).map(|l| l.trim().to_string()).unwrap_or_default();
        out.push(PartitionSpec { size_mib: size.unwrap_or(0), fs, label });
    }
    let usable = usable_mib(bytes);
    let takes_rest = out[last].size_mib == 0;
    if fixed > usable || (takes_rest && fixed >= usable) {
        return Err(format!("The partitions need more than the {} the disk offers.", format_size(usable * MIB)));
    }
    Ok(out)
}

/// The segments of the layout bar for `specs` on a disk of `bytes`: one per partition, then the free space.
pub fn segments(bytes: u64, specs: &[PartitionSpec]) -> Vec<Segment> {
    let total = (bytes / MIB).max(1) as f32;
    let usable = usable_mib(bytes);
    let fixed: u64 = specs.iter().map(|s| s.size_mib).sum();
    let mut out = Vec::with_capacity(specs.len() + 1);
    for (i, spec) in specs.iter().enumerate() {
        let mib = if spec.size_mib == 0 { usable.saturating_sub(fixed) } else { spec.size_mib };
        let name = if spec.label.is_empty() { format!("Partition {}", i + 1) } else { spec.label.clone() };
        out.push(Segment { fraction: mib as f32 / total, caption: format!("{name} \u{b7} {}", format_size(mib * MIB)), free: false });
    }
    let used: f32 = out.iter().map(|s| s.fraction).sum();
    if used < 0.995 {
        out.push(Segment { fraction: 1.0 - used, caption: "Free".to_string(), free: true });
    }
    out
}

/// The segments for the typed fields, or none while they do not parse (the bar shows an empty track).
pub fn preview(bytes: u64, sizes: &str, names: &str, fs: Personality) -> Vec<Segment> {
    specs(bytes, sizes, names, fs).map(|s| segments(bytes, &s)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn sizes_become_partitions_in_order_with_names() {
        let got = specs(8 * GIB, "512m, 2g, rest", "BOOT, , DATA", Personality::Ext4).unwrap();
        assert_eq!(got.iter().map(|s| s.size_mib).collect::<Vec<_>>(), vec![512, 2048, 0]);
        assert_eq!(got.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), vec!["BOOT", "", "DATA"]);
        assert!(got.iter().all(|s| s.fs == Personality::Ext4));
    }

    #[test]
    fn rest_is_only_allowed_last_and_an_empty_word_means_rest() {
        assert!(specs(8 * GIB, "rest, 1g", "", Personality::Ext4).unwrap_err().contains("last"));
        assert_eq!(specs(8 * GIB, "1g, ", "", Personality::Ext4).unwrap()[1].size_mib, 0);
        assert_eq!(specs(8 * GIB, "1g, *", "", Personality::Ext4).unwrap()[1].size_mib, 0);
        assert_eq!(specs(8 * GIB, "rest", "", Personality::Ext4).unwrap().len(), 1);
    }

    #[test]
    fn nothing_typed_or_a_bad_size_is_refused_with_a_sentence() {
        assert!(specs(8 * GIB, "", "", Personality::Ext4).is_err());
        assert!(specs(8 * GIB, " , ", "", Personality::Ext4).is_err());
        assert!(specs(8 * GIB, "lots, rest", "", Personality::Ext4).unwrap_err().contains("not a number"));
        assert!(specs(8 * GIB, "512k, rest", "", Personality::Ext4).unwrap_err().contains("at least 1 MiB"));
    }

    #[test]
    fn the_partitions_must_fit_the_disk() {
        assert!(specs(GIB, "512m, 512m", "", Personality::Ext4).is_err(), "the table margin does not fit");
        assert!(specs(GIB, "512m, 400m", "", Personality::Ext4).is_ok());
        assert!(specs(GIB, "2g", "", Personality::Ext4).is_err());
        assert!(specs(GIB, "1022m, rest", "", Personality::Ext4).is_err(), "rest needs room");
        assert!(specs(GIB, "1021m, rest", "", Personality::Ext4).is_ok());
    }

    #[test]
    fn too_many_partitions_or_names_are_refused() {
        let many = vec!["1m"; MAX_PARTITIONS + 1].join(",");
        assert!(specs(8 * GIB, &many, "", Personality::Ext4).unwrap_err().contains("at most"));
        assert!(specs(8 * GIB, "1g", "a, b", Personality::Ext4).unwrap_err().contains("more names"));
    }

    #[test]
    fn hostile_text_never_panics() {
        for text in ["\u{0}", ",,,,", "999999999999999999999g", "-1g", "1g,\u{202e}rest", "rest,rest"] {
            let _ = specs(8 * GIB, text, text, Personality::Fat32);
        }
        assert!(specs(0, "1m", "", Personality::Ext4).is_err());
    }

    #[test]
    fn the_bar_shows_each_partition_and_the_free_space() {
        let layout = specs(10 * GIB, "1g, 2g", "A", Personality::Ext4).unwrap();
        let bar = segments(10 * GIB, &layout);
        assert_eq!(bar.len(), 3);
        assert!(!bar[0].free && bar[0].caption.starts_with("A"));
        assert!(bar[1].caption.starts_with("Partition 2"));
        assert!(bar[2].free);
        let sum: f32 = bar.iter().map(|s| s.fraction).sum();
        assert!((sum - 1.0).abs() < 0.01, "{sum}");
        assert!((bar[0].fraction - 0.1).abs() < 0.001);
    }

    #[test]
    fn rest_fills_the_bar_up_to_the_table_margin() {
        let layout = specs(4 * GIB, "1g, rest", "", Personality::Ext4).unwrap();
        let bar = segments(4 * GIB, &layout);
        assert_eq!(bar.len(), 2, "no free segment is left over");
        assert!(bar[1].fraction > 0.74 && bar[1].fraction < 0.75);
    }

    #[test]
    fn the_preview_is_empty_until_the_text_parses() {
        assert!(preview(8 * GIB, "x", "", Personality::Ext4).is_empty());
        assert_eq!(preview(8 * GIB, "1g, rest", "", Personality::Ext4).len(), 2);
    }

    #[test]
    fn the_equal_split_text_reads_back_as_that_split() {
        let text = equal_sizes_text(8 * GIB, 3);
        let layout = specs(8 * GIB, &text, "", Personality::Ext4).unwrap();
        assert_eq!(layout.len(), 3);
        assert_eq!(layout[2].size_mib, 0);
        assert_eq!(equal_sizes_text(1, 3), "rest", "a disk too small to split offers one part");
    }
}
