//! Scenes for apps that host several windows or content areas: each scene has an id, a title and the state it saved,
//! so the shell can restore it. The model and its tests are `bifrost_core::scene_set`. The shell keeps the set; an
//! app reads its own scenes and hands back the state of each one.
pub use bifrost_core::scene_set::{Scene, SceneError, SceneSet, MAX_ID_LEN};
