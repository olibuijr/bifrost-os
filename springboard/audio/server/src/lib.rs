//! `bifrost-audiod`: the audio server (docs/audio-architecture.md). Module owners: `objects` (task 02), `io`,
//! `mixer`, `client`, `control` (task 05), `hog` and `latency` (task 10), `aggregate` (task 11), `tap` (task 17).
//! Each owner declares its module here with one line.
pub mod objects;

pub mod client;
pub mod control;
pub mod io;
pub mod mixer;

pub mod hog;
pub mod latency;
pub mod quantum;
pub mod aggregate;
pub mod apps;
pub mod dsp;
pub mod level;
pub mod tap;
pub mod rt;
