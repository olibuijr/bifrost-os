//! Foundation formatters in one place. The locale-aware core (`NumberFormatter` decimal, percent and
//! currency, `DateFormatter`, `ByteCountFormatter`, `MeasurementFormatter`, `ListFormatter`) lives in
//! `locale.rs` and is re-exported here; this module adds the styles that file lacks: spell-out and ordinal
//! numbers (`number`), the named `RelativeDateTimeFormatter` style (`relative`), and
//! `DateComponentsFormatter` (`date_components`).

pub mod date_components;
pub mod number;
pub mod relative;

pub use crate::locale::{byte_count, list, measurement, relative_date_time, DateFormatter, DateStyle, Locale, Measurement, NumberFormatter, NumberStyle, Unit};
