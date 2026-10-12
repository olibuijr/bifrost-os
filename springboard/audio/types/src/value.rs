//! Typed property values. Owner: task 01 (the type), task 02 (its use in the property table and the codec).

use crate::address::ObjectId;
use crate::format::StreamFormat;
use alloc::string::String;
use alloc::vec::Vec;

/// A property's value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    U32(u32),
    I32(i32),
    F32(f32),
    F64(f64),
    String(String),
    Object(ObjectId),
    Objects(Vec<ObjectId>),
    /// A (min, max) range, such as the buffer frame size range.
    RangeU32(u32, u32),
    /// Available sample rates as (min, max) ranges.
    Rates(Vec<(f64, f64)>),
    Format(StreamFormat),
    Formats(Vec<StreamFormat>),
    /// Preferred stereo channels (1-based).
    ChannelPair(u32, u32),
    /// A process id (0 = none), for the hog owner.
    Pid(i32),
    /// Raw bytes (a layout, a workgroup token).
    Bytes(Vec<u8>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_compare_and_clone() {
        let v = PropertyValue::Rates(alloc::vec![(44100.0, 48000.0)]);
        assert_eq!(v.clone(), v);
        assert_ne!(PropertyValue::U32(1), PropertyValue::I32(1));
        let f = PropertyValue::Format(StreamFormat::float32(48000.0, 2));
        assert_eq!(f.clone(), f);
    }
}
