//! Media intents for apps: the playback request a person speaks or types, the catalogue entries an app returns for it,
//! and the play queue the app keeps. The models come from `bifrost_ui`; the shared audio service plays the items
//! (`bifrost_app::audio`). Nothing here starts playback or listens for speech.

pub use bifrost_ui::media_queue::PlaybackQueue;
pub use bifrost_ui::media_request::{MediaVerb, PlaybackAction, PlaybackRequest};
pub use bifrost_ui::media_search::{resolve, MediaCriteria, MediaFilter, MediaItem, MediaKind, MediaSearch, Resolution};

/// Answers one request against the app's catalogue: plans it, applies the plan to `queue` and returns the dialog
/// that answers the person. At most `limit` items are considered.
pub fn handle_request(queue: &mut PlaybackQueue, request: &PlaybackRequest, catalogue: &[MediaItem], limit: usize) -> String {
    let action = request.plan(catalogue, limit);
    queue.apply(&action);
    request.dialog(&action)
}
