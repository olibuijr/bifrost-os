//! Channel layouts. Owner: task 01 moves `springboard/ui/src/channel_layout.rs` here unchanged in behaviour (its
//! `Tag`, `Label`, `LayoutError` and `ChannelLayout`), and makes `bifrost-ui` re-export it so the ui tests keep
//! passing. Until then this placeholder keeps the name.

/// Placeholder: replaced by the moved `ChannelLayout` (task 01).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelLayout {
    pub channels: u32,
}
