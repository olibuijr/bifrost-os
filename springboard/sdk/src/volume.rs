//! Volume capacity for apps: the total, free and available bytes of the filesystem that holds a path.
//!
//! ```ignore
//! let c = volume::capacity(Path::new(&document_path))?;
//! if c.available < needed { /* tell the user the disk is full */ }
//! ```
//! The model and the call are in `bifrost_core::volume`; the shell's disk readings use the same call. Reads only:
//! nothing is opened for writing and no quota is set. A path that does not exist is an error, not a zero reading.
pub use bifrost_core::volume::{capacity, Capacity};
