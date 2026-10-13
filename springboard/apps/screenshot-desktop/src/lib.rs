//! Screenshot toolbar model: what a capture is, where it goes and how it is named.
//! The window and the capture calls are added on top of this; this part has no
//! dependencies so the naming and the option state can be tested alone.

pub mod naming;
pub mod options;

pub use naming::{file_name, Stamp};
pub use options::{Destination, Mode, Options, Target, Timer};
