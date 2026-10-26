//! Screenshot toolbar model: what a capture is, where it goes and how it is named, and the session that runs the
//! Capture button and the timer. The window (`main.rs`) and the capture calls are added on top of this; this part has no
//! dependencies so the naming, the option state and the countdown can be tested alone.

pub mod naming;
pub mod options;
pub mod session;

pub use naming::{file_name, Stamp};
pub use options::{Destination, Mode, Options, Target, Timer};
pub use session::{Session, Step};
