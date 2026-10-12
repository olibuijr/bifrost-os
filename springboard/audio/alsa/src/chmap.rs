//! PCM channel maps: the kernel's `Playback Channel Map` and `Capture Channel Map` controls
//! (`include/uapi/sound/asound.h` `SNDRV_CHMAP_*`, `include/uapi/sound/tlv.h` `SNDRV_CTL_TLVT_CHMAP_*`), turned into
//! the device's channel layout (left, right, centre, LFE, surrounds).
//!
//! The control sits on interface PCM with `device` = the PCM device and `index` = the substream. Its value is the
//! map in use, one integer per channel, once the stream has hw params (all zero before). Its TLV lists every map the
//! device can take: a container `[TLVT_CONTAINER, bytes, entry...]`, each entry
//! `[TLVT_CHMAP_FIXED | VAR | PAIRED, bytes, position...]` with `bytes = 4 * channels`. A position's low 16 bits
//! name the speaker; bit 16 marks a phase inversion and bit 17 a driver-specific value.
//!
//! The TLV comes from a driver, so the parser trusts none of its lengths: each is checked against the bytes
//! present, the walk never panics, and it allocates no more than the input holds.

use bifrost_audio_types::layout::MAX_CHANNELS;
use bifrost_audio_types::{ChannelLayout, Label, LayoutError};

/// SNDRV_CTL_TLVT_CONTAINER
pub const TLVT_CONTAINER: u32 = 0;
/// SNDRV_CTL_TLVT_CHMAP_FIXED: the channels sit where the map says, nothing can move.
pub const TLVT_CHMAP_FIXED: u32 = 0x101;
/// SNDRV_CTL_TLVT_CHMAP_VAR: any channel can be swapped.
pub const TLVT_CHMAP_VAR: u32 = 0x102;
/// SNDRV_CTL_TLVT_CHMAP_PAIRED: pairs of channels can be swapped.
pub const TLVT_CHMAP_PAIRED: u32 = 0x103;

pub const CHMAP_POSITION_MASK: u32 = 0xffff;
pub const CHMAP_PHASE_INVERSE: u32 = 0x1 << 16;
pub const CHMAP_DRIVER_SPEC: u32 = 0x2 << 16;

// enum snd_pcm_chmap_position (the ones a layout names; the rest map to `Label::Unknown`)
pub const CHMAP_UNKNOWN: u32 = 0;
/// N/A: a silent channel.
pub const CHMAP_NA: u32 = 1;
pub const CHMAP_MONO: u32 = 2;
pub const CHMAP_FL: u32 = 3;
pub const CHMAP_FR: u32 = 4;
pub const CHMAP_RL: u32 = 5;
pub const CHMAP_RR: u32 = 6;
pub const CHMAP_FC: u32 = 7;
pub const CHMAP_LFE: u32 = 8;
pub const CHMAP_SL: u32 = 9;
pub const CHMAP_SR: u32 = 10;
pub const CHMAP_RC: u32 = 11;
/// SNDRV_CHMAP_LAST (= BRC)
pub const CHMAP_LAST: u32 = 36;

/// The element names of the two chmap controls.
pub const CHMAP_PLAYBACK_NAME: &str = "Playback Channel Map";
pub const CHMAP_CAPTURE_NAME: &str = "Capture Channel Map";

/// The most channels one map may carry here: the layout model's limit.
pub const MAX_MAP_CHANNELS: usize = MAX_CHANNELS;

/// How a map's channels may be rearranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChmapKind {
    Fixed,
    Var,
    Paired,
}

impl ChmapKind {
    pub fn from_tlv(t: u32) -> Option<ChmapKind> {
        match t {
            TLVT_CHMAP_FIXED => Some(ChmapKind::Fixed),
            TLVT_CHMAP_VAR => Some(ChmapKind::Var),
            TLVT_CHMAP_PAIRED => Some(ChmapKind::Paired),
            _ => None,
        }
    }
}

/// One channel map: a raw position (flags included) per channel, in channel order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chmap {
    pub kind: ChmapKind,
    pub positions: Vec<u32>,
}

impl Chmap {
    pub fn channels(&self) -> usize {
        self.positions.len()
    }

    /// One label per channel. The rear pair (RL, RR) is the surround pair of quad and 5.1 unless the map also has
    /// a side pair (SL, SR), as 7.1 does; then the side pair is the surround pair and the rear pair the back one.
    pub fn labels(&self) -> Vec<Label> {
        let side_pair = self.positions.iter().any(|&p| p & CHMAP_DRIVER_SPEC == 0 && matches!(p & CHMAP_POSITION_MASK, CHMAP_SL | CHMAP_SR));
        self.positions.iter().map(|&p| label_of(p, side_pair)).collect()
    }

    /// The channel layout this map describes (tagged as a standard arrangement when its order is one).
    pub fn layout(&self) -> Result<ChannelLayout, LayoutError> {
        ChannelLayout::from_labels(&self.labels())
    }
}

/// The speaker of one raw position. Phase inversion does not change the speaker; a driver-specific value names no
/// standard one. `side_pair`: see `Chmap::labels`.
pub fn label_of(position: u32, side_pair: bool) -> Label {
    if position & CHMAP_DRIVER_SPEC != 0 {
        return Label::Unknown;
    }
    match position & CHMAP_POSITION_MASK {
        CHMAP_FL => Label::Left,
        CHMAP_FR => Label::Right,
        CHMAP_FC | CHMAP_MONO => Label::Center,
        CHMAP_LFE => Label::Lfe,
        CHMAP_SL => Label::LeftSurround,
        CHMAP_SR => Label::RightSurround,
        CHMAP_RL if side_pair => Label::LeftRear,
        CHMAP_RR if side_pair => Label::RightRear,
        CHMAP_RL => Label::LeftSurround,
        CHMAP_RR => Label::RightSurround,
        _ => Label::Unknown,
    }
}

/// The TLV bytes as native-endian words (the kernel writes them in the CPU's order); a trailing partial word is
/// dropped.
pub fn tlv_words(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Every map in a chmap TLV: the container the kernel's PCM core and the HDA drivers write, or a single bare entry.
/// A container length past the end is cut to the words present (and to whole words); an entry whose length is not
/// whole words, or runs past the container, ends the walk with the maps read so far; entries of another type, with
/// no channels or with more than `MAX_MAP_CHANNELS`, are skipped.
pub fn parse_chmap_tlv(bytes: &[u8]) -> Vec<Chmap> {
    let w = tlv_words(bytes);
    if w.len() < 2 {
        return Vec::new();
    }
    if w[0] == TLVT_CONTAINER {
        let len = (w[1] / 4) as usize;
        let end = len.saturating_add(2).min(w.len());
        parse_entries(&w[2..end])
    } else {
        parse_entries(&w)
    }
}

fn parse_entries(mut w: &[u32]) -> Vec<Chmap> {
    let mut out = Vec::new();
    while w.len() >= 2 {
        let (t, bytes) = (w[0], w[1]);
        if bytes % 4 != 0 {
            break;
        }
        let n = (bytes / 4) as usize;
        let Some(body) = n.checked_add(2).and_then(|e| w.get(2..e)) else { break };
        if let Some(kind) = ChmapKind::from_tlv(t) {
            if (1..=MAX_MAP_CHANNELS).contains(&n) {
                out.push(Chmap { kind, positions: body.to_vec() });
            }
        }
        w = &w[2 + n..];
    }
    out
}

/// The map in use from the control's values (one integer per channel, zero-filled past the stream's channels).
/// `None` before hw params (the kernel answers all zeros), for no channels, or when fewer values than `channels`
/// came back. A value outside `u32` is an unknown position.
pub fn current_map(values: &[i64], channels: usize) -> Option<Chmap> {
    if channels == 0 || channels > MAX_MAP_CHANNELS {
        return None;
    }
    let v = values.get(..channels)?;
    if v.iter().all(|&x| x == 0) {
        return None;
    }
    Some(Chmap { kind: ChmapKind::Fixed, positions: v.iter().map(|&x| u32::try_from(x).unwrap_or(CHMAP_UNKNOWN)).collect() })
}

/// The map a device offers for `channels`: the first entry with that many.
pub fn map_for(maps: &[Chmap], channels: usize) -> Option<&Chmap> {
    maps.iter().find(|m| m.channels() == channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Tag;

    /// Native-endian bytes of TLV words, as the kernel hands them over.
    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_ne_bytes()).collect()
    }

    #[test]
    fn a_container_of_stereo_and_5_1_maps() {
        // stereo: 2 header words + 2 positions = 16 bytes; 5.1: 2 + 6 = 32 bytes; container body 48 bytes
        let tlv = bytes(&[0, 48, 0x101, 8, 3, 4, 0x101, 24, 3, 4, 5, 6, 7, 8]);
        let maps = parse_chmap_tlv(&tlv);
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0], Chmap { kind: ChmapKind::Fixed, positions: vec![CHMAP_FL, CHMAP_FR] });
        assert_eq!(maps[0].layout().unwrap().tag(), Tag::Stereo);
        // ALSA's 5.1 order FL FR RL RR FC LFE; with no side pair the rear pair is the surround pair
        assert_eq!(
            maps[1].labels(),
            vec![Label::Left, Label::Right, Label::LeftSurround, Label::RightSurround, Label::Center, Label::Lfe]
        );
        assert_eq!(maps[1].layout().unwrap().tag(), Tag::Descriptions, "not the standard 5.1 order");
        assert_eq!(map_for(&maps, 6), Some(&maps[1]));
        assert_eq!(map_for(&maps, 2), Some(&maps[0]));
        assert_eq!(map_for(&maps, 4), None);
    }

    #[test]
    fn seven_one_puts_the_rear_pair_behind_the_sides() {
        let tlv = bytes(&[0, 40, 0x102, 32, 3, 4, 5, 6, 7, 8, 9, 10]);
        let maps = parse_chmap_tlv(&tlv);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].kind, ChmapKind::Var);
        assert_eq!(
            maps[0].labels(),
            vec![
                Label::Left,
                Label::Right,
                Label::LeftRear,
                Label::RightRear,
                Label::Center,
                Label::Lfe,
                Label::LeftSurround,
                Label::RightSurround
            ]
        );
    }

    #[test]
    fn standard_order_maps_get_the_standard_tag() {
        // FL FR FC LFE SL SR is the standard 5.1 order
        let tlv = bytes(&[0, 32, 0x103, 24, 3, 4, 7, 8, 9, 10]);
        let maps = parse_chmap_tlv(&tlv);
        assert_eq!(maps[0].kind, ChmapKind::Paired);
        assert_eq!(maps[0].layout().unwrap().tag(), Tag::Surround51);
        // FL FR RL RR: quad
        let quad = parse_chmap_tlv(&bytes(&[0, 24, 0x101, 16, 3, 4, 5, 6]));
        assert_eq!(quad[0].layout().unwrap().tag(), Tag::Quadraphonic);
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn a_hand_written_mono_container() {
        // [CONTAINER, 12, CHMAP_FIXED, 4, MONO], little-endian
        let tlv: [u8; 20] = [0, 0, 0, 0, 12, 0, 0, 0, 0x01, 0x01, 0, 0, 4, 0, 0, 0, 2, 0, 0, 0];
        let maps = parse_chmap_tlv(&tlv);
        assert_eq!(maps, vec![Chmap { kind: ChmapKind::Fixed, positions: vec![CHMAP_MONO] }]);
        assert_eq!(maps[0].layout().unwrap().tag(), Tag::Mono);
        // a trailing partial word is ignored
        let mut odd = tlv.to_vec();
        odd.extend_from_slice(&[9, 9, 9]);
        assert_eq!(parse_chmap_tlv(&odd), maps);
    }

    #[test]
    fn a_bare_entry_without_a_container() {
        let maps = parse_chmap_tlv(&bytes(&[0x102, 8, 4, 3]));
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].labels(), vec![Label::Right, Label::Left]);
    }

    #[test]
    fn hostile_lengths_are_cut_or_stop_the_walk() {
        assert!(parse_chmap_tlv(&[]).is_empty());
        assert!(parse_chmap_tlv(&[0, 0, 0]).is_empty());
        assert!(parse_chmap_tlv(&bytes(&[0])).is_empty());
        // a container that claims more than is there: cut to the words present
        assert_eq!(parse_chmap_tlv(&bytes(&[0, 1000, 0x101, 8, 3, 4])).len(), 1);
        // an entry running past the container
        assert!(parse_chmap_tlv(&bytes(&[0, 16, 0x101, 400, 3, 4])).is_empty());
        // an entry length that is not whole words
        assert!(parse_chmap_tlv(&bytes(&[0, 16, 0x101, 6, 3, 4])).is_empty());
        // the largest lengths do not overflow
        assert!(parse_chmap_tlv(&bytes(&[0, u32::MAX, 0x101, 0xffff_fffc, 3])).is_empty());
        assert!(parse_chmap_tlv(&bytes(&[0x101, 0xffff_fffc, 3])).is_empty());
        // a good entry before a broken one survives
        let maps = parse_chmap_tlv(&bytes(&[0, 32, 0x101, 8, 3, 4, 0x101, 400, 3, 4]));
        assert_eq!(maps.len(), 1);
        // the container length bounds the walk: the second entry lies past its 16 bytes
        assert_eq!(parse_chmap_tlv(&bytes(&[0, 16, 0x101, 8, 3, 4, 0x101, 8, 4, 3])).len(), 1);
    }

    #[test]
    fn unknown_empty_and_oversized_entries_are_skipped() {
        // an entry of another type (one word of body), then stereo: 3 + 4 words = 28 bytes
        let maps = parse_chmap_tlv(&bytes(&[0, 28, 0x999, 4, 7, 0x101, 8, 3, 4]));
        assert_eq!(maps, vec![Chmap { kind: ChmapKind::Fixed, positions: vec![3, 4] }]);
        // a zero-channel entry, then stereo: 2 + 4 words = 24 bytes
        let maps = parse_chmap_tlv(&bytes(&[0, 24, 0x101, 0, 0x101, 8, 3, 4]));
        assert_eq!(maps.len(), 1);
        // 65 channels is over the limit, 64 is not
        for (n, want) in [(65usize, 0usize), (64, 1)] {
            let mut w = vec![0, (8 + 4 * n) as u32, 0x101, (4 * n) as u32];
            w.extend(std::iter::repeat(3).take(n));
            assert_eq!(parse_chmap_tlv(&bytes(&w)).len(), want, "{n} channels");
        }
    }

    #[test]
    fn position_flags_and_odd_positions() {
        assert_eq!(label_of(CHMAP_FL | CHMAP_PHASE_INVERSE, false), Label::Left);
        assert_eq!(label_of(CHMAP_RL | CHMAP_DRIVER_SPEC, false), Label::Unknown);
        assert_eq!(label_of(CHMAP_NA, false), Label::Unknown);
        assert_eq!(label_of(CHMAP_RC, false), Label::Unknown);
        assert_eq!(label_of(CHMAP_UNKNOWN, false), Label::Unknown);
        assert_eq!(label_of(CHMAP_LAST, false), Label::Unknown);
        assert_eq!(label_of(0xffff, true), Label::Unknown);
        assert_eq!(label_of(CHMAP_MONO, false), Label::Center);
        // a driver-specific SL does not turn the rear pair into the back pair
        let m = Chmap { kind: ChmapKind::Fixed, positions: vec![CHMAP_RL, CHMAP_SL | CHMAP_DRIVER_SPEC] };
        assert_eq!(m.labels(), vec![Label::LeftSurround, Label::Unknown]);
    }

    #[test]
    fn the_current_map_comes_from_the_control_values() {
        assert_eq!(current_map(&[3, 4, 0, 0, 0, 0, 0, 0], 2), Some(Chmap { kind: ChmapKind::Fixed, positions: vec![3, 4] }));
        assert_eq!(current_map(&[0; 8], 2), None, "before hw params");
        assert_eq!(current_map(&[3, 4], 0), None);
        assert_eq!(current_map(&[3], 2), None, "fewer values than channels");
        assert_eq!(current_map(&[-1, 4], 2).unwrap().positions, vec![CHMAP_UNKNOWN, CHMAP_FR]);
        assert_eq!(current_map(&[1 << 40, 4], 2).unwrap().positions, vec![CHMAP_UNKNOWN, CHMAP_FR]);
        assert_eq!(current_map(&[3; 65], 65), None, "over the layout limit");
    }
}
