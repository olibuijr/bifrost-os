//! HID report descriptors: parses the short items of a report descriptor into fields (elements) and a collection tree,
//! and reads or writes one field's value inside a report payload.
//!
//! Pure: the caller reads the descriptor bytes from the device and sends the report buffers. Field and collection
//! values follow the HID standard's field and collection model, the same model the platform's HID framework exposes
//! to apps. Input, output and feature items are parsed. Long items
//! are rejected; string and designator items are ignored; delimiter sets are not modelled.

use std::collections::BTreeMap;

/// The direction of a report (the HID standard's report type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReportKind {
    /// Device to host.
    Input,
    /// Host to device, for example an LED.
    Output,
    /// Bidirectional configuration data.
    Feature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The bytes end inside an item.
    Truncated,
    /// A long item (prefix 0xFE) is not supported.
    LongItem,
    /// An end-collection item at the top level.
    UnbalancedEnd,
    /// A collection is still open at the end of the descriptor.
    UnclosedCollection,
    /// A pop of the global state without a matching push.
    PopWithoutPush,
}

/// One field of a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub kind: ReportKind,
    /// Zero when the descriptor declares no report ID.
    pub report_id: u8,
    pub usage_page: u16,
    pub usage: u16,
    /// The Input, Output or Feature flags. Bit 0 is constant, bit 1 variable, bit 2 relative.
    pub flags: u32,
    /// Offset of the field's first bit, counted from the first data byte after the report ID.
    pub bit_offset: u32,
    /// `reportSize`: bits per field.
    pub bit_size: u32,
    pub logical_min: i32,
    pub logical_max: i32,
    pub physical_min: i32,
    pub physical_max: i32,
    pub unit_exponent: i32,
    pub unit: u32,
    /// Index into `ReportDescriptor::collections` of the collection that contains this field.
    pub parent: usize,
}

impl Element {
    pub fn is_constant(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn is_variable(&self) -> bool {
        self.flags & 2 != 0
    }

    pub fn is_relative(&self) -> bool {
        self.flags & 4 != 0
    }

    /// The field's logical value, read from `payload` (the report bytes after the report ID). A field whose logical
    /// minimum is negative is read as two's complement. `None` if the payload is too short.
    pub fn value(&self, payload: &[u8]) -> Option<i64> {
        let size = self.bit_size;
        if size == 0 || size > 32 {
            return None;
        }
        let mut raw: u64 = 0;
        for k in 0..size {
            let bit = self.bit_offset + k;
            let byte = *payload.get((bit / 8) as usize)?;
            raw |= (u64::from(byte >> (bit % 8)) & 1) << k;
        }
        if self.logical_min < 0 && raw & (1u64 << (size - 1)) != 0 {
            Some(raw as i64 - (1i64 << size))
        } else {
            Some(raw as i64)
        }
    }

    /// Writes a logical value into `payload`. Returns `false` (and writes nothing) for a constant field, a value
    /// outside the logical range, or a payload too short to hold the field.
    pub fn set_value(&self, payload: &mut [u8], value: i64) -> bool {
        let size = self.bit_size;
        if self.is_constant() || size == 0 || size > 32 {
            return false;
        }
        if value < i64::from(self.logical_min) || value > i64::from(self.logical_max) {
            return false;
        }
        if payload.len() * 8 < (self.bit_offset + size) as usize {
            return false;
        }
        let mask: u64 = (1u64 << size) - 1;
        let raw = (value as u64) & mask;
        for k in 0..size {
            let bit = self.bit_offset + k;
            let byte = &mut payload[(bit / 8) as usize];
            if (raw >> k) & 1 == 1 {
                *byte |= 1 << (bit % 8);
            } else {
                *byte &= !(1 << (bit % 8));
            }
        }
        true
    }

    /// Converts a logical value to its physical value: the logical range is mapped onto the physical range, then
    /// multiplied by 10^`unit_exponent`. With both physical bounds zero the logical value is used as it is.
    pub fn physical_value(&self, raw: i64) -> f64 {
        let scale = 10f64.powi(self.unit_exponent);
        let phys = if self.physical_min == 0 && self.physical_max == 0 {
            raw as f64
        } else if self.logical_max == self.logical_min {
            f64::from(self.physical_min)
        } else {
            let span = i64::from(self.logical_max) - i64::from(self.logical_min);
            let t = (raw - i64::from(self.logical_min)) as f64 / span as f64;
            f64::from(self.physical_min) + t * f64::from(self.physical_max - self.physical_min)
        };
        phys * scale
    }
}

/// A node of the collection tree. Index 0 is the top level, which holds the items declared
/// outside any collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    /// The collection type byte: 0 physical, 1 application, 2 logical, 3 report, and so on.
    pub kind: u8,
    pub usage_page: u16,
    pub usage: u16,
    pub parent: Option<usize>,
    pub child_collections: Vec<usize>,
    pub child_elements: Vec<usize>,
}

/// A parsed report descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportDescriptor {
    pub elements: Vec<Element>,
    pub collections: Vec<Collection>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Globals {
    usage_page: u16,
    logical_min: i32,
    logical_max: i32,
    physical_min: i32,
    physical_max: i32,
    unit_exponent: i32,
    unit: u32,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

/// Reads up to four little-endian bytes as an unsigned value.
fn unsigned(data: &[u8]) -> u32 {
    data.iter().rev().fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
}

/// Reads up to four little-endian bytes as a signed value of the same width.
fn signed(data: &[u8]) -> i32 {
    match data.len() {
        0 => 0,
        1 => i32::from(data[0] as i8),
        2 => i32::from(i16::from_le_bytes([data[0], data[1]])),
        _ => i32::from_le_bytes([data[0], data[1], data[2], data[3]]),
    }
}

/// A usage item: a 32-bit value carries the usage page in its high half, otherwise the current page applies.
fn usage_of(value: u32, page: u16) -> (u16, u16) {
    if value > 0xffff {
        ((value >> 16) as u16, (value & 0xffff) as u16)
    } else {
        (page, value as u16)
    }
}

impl ReportDescriptor {
    /// The report IDs of one kind, in ascending order. Zero is listed when the descriptor declares no report ID.
    pub fn report_ids(&self, kind: ReportKind) -> Vec<u8> {
        let mut ids: Vec<u8> = self.elements.iter().filter(|e| e.kind == kind).map(|e| e.report_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The fields of one report, in declaration order.
    pub fn fields(&self, kind: ReportKind, report_id: u8) -> Vec<&Element> {
        self.elements.iter().filter(|e| e.kind == kind && e.report_id == report_id).collect()
    }

    /// The payload length in bytes of one report, not counting the report ID byte.
    pub fn payload_len(&self, kind: ReportKind, report_id: u8) -> usize {
        let bits = self
            .elements
            .iter()
            .filter(|e| e.kind == kind && e.report_id == report_id)
            .map(|e| e.bit_offset.saturating_add(e.bit_size))
            .max()
            .unwrap_or(0);
        ((bits + 7) / 8) as usize
    }

    /// Parses a report descriptor.
    pub fn parse(bytes: &[u8]) -> Result<ReportDescriptor, ParseError> {
        let mut g = Globals::default();
        let mut pushed: Vec<Globals> = Vec::new();
        let mut usages: Vec<(u16, u16)> = Vec::new();
        let mut umin: Option<(u16, u16)> = None;
        let mut umax: Option<(u16, u16)> = None;
        let mut collections = vec![Collection {
            kind: 0,
            usage_page: 0,
            usage: 0,
            parent: None,
            child_collections: Vec::new(),
            child_elements: Vec::new(),
        }];
        let mut open: Vec<usize> = vec![0];
        let mut elements: Vec<Element> = Vec::new();
        let mut offsets: BTreeMap<(ReportKind, u8), u32> = BTreeMap::new();

        let mut i = 0;
        while i < bytes.len() {
            let prefix = bytes[i];
            if prefix == 0xfe {
                return Err(ParseError::LongItem);
            }
            let size = match prefix & 3 {
                0 => 0usize,
                1 => 1,
                2 => 2,
                _ => 4,
            };
            if i + 1 + size > bytes.len() {
                return Err(ParseError::Truncated);
            }
            let data = &bytes[i + 1..i + 1 + size];
            i += 1 + size;
            let tag = prefix >> 4;
            let typ = (prefix >> 2) & 3;
            let raw = unsigned(data);
            match typ {
                1 => match tag {
                    0 => g.usage_page = raw as u16,
                    1 => g.logical_min = signed(data),
                    2 => g.logical_max = signed(data),
                    3 => g.physical_min = signed(data),
                    4 => g.physical_max = signed(data),
                    5 => g.unit_exponent = ((raw as i32 & 0x0f) ^ 0x08) - 0x08,
                    6 => g.unit = raw,
                    7 => g.report_size = raw,
                    8 => g.report_id = raw as u8,
                    9 => g.report_count = raw,
                    10 => pushed.push(g),
                    11 => g = pushed.pop().ok_or(ParseError::PopWithoutPush)?,
                    _ => {}
                },
                2 => match tag {
                    0 => usages.push(usage_of(raw, g.usage_page)),
                    1 => umin = Some(usage_of(raw, g.usage_page)),
                    2 => umax = Some(usage_of(raw, g.usage_page)),
                    _ => {}
                },
                0 => match tag {
                    8 | 9 | 11 => {
                        let kind = match tag {
                            8 => ReportKind::Input,
                            9 => ReportKind::Output,
                            _ => ReportKind::Feature,
                        };
                        let mut list = usages.clone();
                        if let (Some(lo), Some(hi)) = (umin, umax) {
                            if hi.1 >= lo.1 {
                                list.extend((lo.1..=hi.1).map(|u| (lo.0, u)));
                            }
                        }
                        let parent = *open.last().unwrap_or(&0);
                        let offset = offsets.entry((kind, g.report_id)).or_insert(0);
                        for n in 0..g.report_count as usize {
                            let (page, usage) = list
                                .get(n)
                                .or(list.last())
                                .copied()
                                .unwrap_or((g.usage_page, 0));
                            elements.push(Element {
                                kind,
                                report_id: g.report_id,
                                usage_page: page,
                                usage,
                                flags: raw,
                                bit_offset: *offset,
                                bit_size: g.report_size,
                                logical_min: g.logical_min,
                                logical_max: g.logical_max,
                                physical_min: g.physical_min,
                                physical_max: g.physical_max,
                                unit_exponent: g.unit_exponent,
                                unit: g.unit,
                                parent,
                            });
                            *offset = offset.saturating_add(g.report_size);
                            collections[parent].child_elements.push(elements.len() - 1);
                        }
                        usages.clear();
                        umin = None;
                        umax = None;
                    }
                    10 => {
                        let parent = *open.last().unwrap_or(&0);
                        let (page, usage) = usages.first().copied().unwrap_or((g.usage_page, 0));
                        let idx = collections.len();
                        collections.push(Collection {
                            kind: raw as u8,
                            usage_page: page,
                            usage,
                            parent: Some(parent),
                            child_collections: Vec::new(),
                            child_elements: Vec::new(),
                        });
                        collections[parent].child_collections.push(idx);
                        open.push(idx);
                        usages.clear();
                        umin = None;
                        umax = None;
                    }
                    12 => {
                        if open.len() == 1 {
                            return Err(ParseError::UnbalancedEnd);
                        }
                        open.pop();
                        usages.clear();
                        umin = None;
                        umax = None;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if open.len() != 1 {
            return Err(ParseError::UnclosedCollection);
        }
        Ok(ReportDescriptor { elements, collections })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-button mouse with a 5-bit pad and X and Y axes, in one application collection.
    fn mouse() -> Vec<u8> {
        vec![
            0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, // Generic Desktop, Mouse, Application collection
            0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, // buttons
            0x95, 0x01, 0x75, 0x05, 0x81, 0x01, // 5-bit constant pad
            0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x02, 0x81, 0x06, // X, Y
            0xc0, // end collection
        ]
    }

    #[test]
    fn parses_fields_with_bit_offsets_and_usages() {
        let d = ReportDescriptor::parse(&mouse()).unwrap();
        assert_eq!(d.elements.len(), 6);
        let offsets: Vec<u32> = d.elements.iter().map(|e| e.bit_offset).collect();
        assert_eq!(offsets, vec![0, 1, 2, 3, 8, 16]);
        assert_eq!((d.elements[0].usage_page, d.elements[0].usage), (9, 1));
        assert_eq!((d.elements[2].usage_page, d.elements[2].usage), (9, 3));
        assert_eq!((d.elements[4].usage_page, d.elements[4].usage), (1, 0x30));
        assert_eq!(d.elements[4].logical_min, -127);
        assert_eq!(d.elements[4].logical_max, 127);
        assert!(d.elements[3].is_constant());
        assert!(!d.elements[0].is_constant());
        assert!(d.elements[5].is_relative());
        assert_eq!(d.payload_len(ReportKind::Input, 0), 3);
        assert_eq!(d.report_ids(ReportKind::Input), vec![0]);
    }

    #[test]
    fn builds_the_collection_tree() {
        let d = ReportDescriptor::parse(&mouse()).unwrap();
        assert_eq!(d.collections.len(), 2);
        assert_eq!(d.collections[0].child_collections, vec![1]);
        let app = &d.collections[1];
        assert_eq!((app.kind, app.usage_page, app.usage), (1, 1, 2));
        assert_eq!(app.parent, Some(0));
        assert_eq!(app.child_elements, vec![0, 1, 2, 3, 4, 5]);
        assert!(d.elements.iter().all(|e| e.parent == 1));
    }

    #[test]
    fn reads_signed_and_unsigned_field_values_from_a_payload() {
        let d = ReportDescriptor::parse(&mouse()).unwrap();
        // Buttons 1 and 3 pressed (bits 0 and 2), X = -1, Y = 2.
        let payload = [0b0000_0101, 0xff, 0x02];
        assert_eq!(d.elements[0].value(&payload), Some(1));
        assert_eq!(d.elements[1].value(&payload), Some(0));
        assert_eq!(d.elements[2].value(&payload), Some(1));
        assert_eq!(d.elements[4].value(&payload), Some(-1));
        assert_eq!(d.elements[5].value(&payload), Some(2));
        assert_eq!(d.elements[4].value(&payload[..1]), None);
    }

    #[test]
    fn writes_a_value_and_rejects_out_of_range_or_constant_fields() {
        let d = ReportDescriptor::parse(&mouse()).unwrap();
        let mut payload = [0u8; 3];
        assert!(d.elements[4].set_value(&mut payload, -5));
        assert_eq!(payload[1], 0xfb);
        assert_eq!(d.elements[4].value(&payload), Some(-5));
        assert!(!d.elements[4].set_value(&mut payload, 128));
        assert!(d.elements[0].set_value(&mut payload, 1));
        assert_eq!(payload[0] & 1, 1);
        assert!(!d.elements[0].set_value(&mut payload, 2));
        assert!(!d.elements[3].set_value(&mut payload, 1));
        assert_eq!(payload[0] & 0b1111_1000, 0);
    }

    #[test]
    fn scales_logical_values_to_physical_values_with_the_unit_exponent() {
        let mut e = ReportDescriptor::parse(&mouse()).unwrap().elements[4].clone();
        e.logical_min = 0;
        e.logical_max = 255;
        e.physical_min = 0;
        e.physical_max = 510;
        e.unit_exponent = 0;
        assert_eq!(e.physical_value(255), 510.0);
        assert_eq!(e.physical_value(0), 0.0);
        e.unit_exponent = -1;
        assert!((e.physical_value(255) - 51.0).abs() < 1e-9);
        e.physical_min = 0;
        e.physical_max = 0;
        e.unit_exponent = 0;
        assert_eq!(e.physical_value(7), 7.0);
    }

    #[test]
    fn keeps_report_ids_apart_and_restores_globals_on_pop() {
        let report_id = [
            0x85, 0x01, 0x05, 0x01, 0x09, 0x30, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02,
        ];
        let d = ReportDescriptor::parse(&report_id).unwrap();
        assert_eq!(d.report_ids(ReportKind::Input), vec![1]);
        assert_eq!(d.payload_len(ReportKind::Input, 1), 1);
        assert!(d.fields(ReportKind::Input, 0).is_empty());
        assert_eq!(d.elements[0].value(&[200]), Some(200));

        // A push saves the page, the pop restores it for the field that follows.
        let push_pop = [
            0x05, 0x01, 0xa4, 0x05, 0x09, 0xb4, 0x09, 0x30, 0x95, 0x01, 0x75, 0x08, 0x81, 0x02,
        ];
        let d = ReportDescriptor::parse(&push_pop).unwrap();
        assert_eq!((d.elements[0].usage_page, d.elements[0].usage), (1, 0x30));
    }

    #[test]
    fn rejects_malformed_descriptors() {
        assert_eq!(ReportDescriptor::parse(&[0x05]), Err(ParseError::Truncated));
        assert_eq!(ReportDescriptor::parse(&[0xc0]), Err(ParseError::UnbalancedEnd));
        assert_eq!(ReportDescriptor::parse(&[0xa1, 0x01]), Err(ParseError::UnclosedCollection));
        assert_eq!(ReportDescriptor::parse(&[0xb4]), Err(ParseError::PopWithoutPush));
        assert_eq!(ReportDescriptor::parse(&[0xfe, 0x02, 0x00, 0x00]), Err(ParseError::LongItem));
    }
}
