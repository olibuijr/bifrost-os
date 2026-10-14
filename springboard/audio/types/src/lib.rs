//! `bifrost-audio-types`: the vocabulary every audio crate shares (docs/audio-architecture.md, "Crates"): stream
//! formats, timestamps, buffer lists, channel layouts, the object model's addresses and values, the property table,
//! the control-socket protocol between clients and the server, and the error codes.
//!
//! `no_std` plus `alloc`, no `unsafe`, no dependencies. Module owners (audio team): `format`, `time`, `buffer`,
//! `layout`, `address`, `value`, `error` (task 01); `props` and `control` (task 02).

#![no_std]
extern crate alloc;

pub mod address;
pub mod buffer;
pub mod codec_id;
pub mod control;
pub mod error;
pub mod format;
pub mod layout;
pub mod props;
pub mod time;
pub mod value;

pub use address::{Class, Element, FourCC, ObjectId, PropertyAddress, Scope, SYSTEM_OBJECT};
pub use buffer::{Buffer, BufferList};
pub use codec_id::CodecId;
pub use error::AudioError;
pub use format::{FormatFlags, FormatId, StreamFormat};
pub use layout::{ChannelLayout, Label, LayoutError, Tag};
pub use time::{TimeStamp, TimeStampFlags};
pub use value::PropertyValue;
