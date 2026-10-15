//! udev database for apps: the properties (such as `ID_FS_TYPE` or `ID_INPUT_KEYBOARD`), device-node symlinks and tags that
//! udev recorded for one device. Read-only: nothing here runs udev or changes a device.
//!
//! The sandbox binds the database read-only at `/run/udev` (try-bind, native apps only), so an app that is not bound
//! to it, or a system without udev, sees `UdevError::Missing`. The model and the parser live in
//! `bifrost_core::udev_db`; this module names the database directory and re-exports the types apps use.

use std::path::Path;

pub use bifrost_core::udev_db::{parse_record, read_in, DbId, NodeKind, Record, UdevError};

/// The udev data directory, as the sandbox mounts it for native apps.
pub const DATA_ROOT: &str = "/run/udev/data";

/// The record of `id` from the system udev database.
pub fn read(id: &DbId) -> Result<Record, UdevError> {
    read_in(Path::new(DATA_ROOT), id)
}
