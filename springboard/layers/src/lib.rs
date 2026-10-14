//! `bifrost-layers`: the layer tree, transactions and animations shared by the SDK (client) and the window server's
//! render server (docs/graphics-architecture.md, "bifrost-layers").
//!
//! An app builds a tree of layers and changes it only through `Transaction`s, applied atomically. Animations are
//! described once (`Animation`) and evaluated by whoever renders the tree, at the display's frame times, with
//! `Tree::presentation(t)`. The window server is that renderer, so an animation keeps running at full rate when the
//! app is busy.
//!
//! Std only, no unsafe, host-tested. Module owners (wave G1): `layer` + `tree` + `transaction` (`layers-model`),
//! `animation` + `timing` (`layers-animation`), `codec` (`layers-codec`).

pub mod animation;
pub mod codec;
pub mod geometry;
pub mod glass_group;
pub mod layer;
pub mod timing;
pub mod transaction;
pub mod tree;

pub use animation::{Animation, AnimationId, FillMode, Keyframes, MediaTiming, Spring, Value};
pub use geometry::{Affine, Color, Point, Rect, Size};
pub use layer::{Border, ContentsGravity, ContentsId, Layer, LayerContents, LayerId, Material, Property, Shadow};
pub use timing::TimingFunction;
pub use transaction::{Change, Transaction};
pub use tree::Tree;

/// Wayland bindings for `bifrost_layer_v1` (owner: wave G1 agent `layers-protocol`).
#[cfg(any(feature = "client", feature = "server"))]
pub mod protocol;
