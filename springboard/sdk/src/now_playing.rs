//! Now Playing for apps: local sessions, playback commands and artwork sizes.
//!
//! [`MediaSession`] is the app's session: set its [`MediaContent`] and playback with `set_content` and `set_playback`,
//! declare each [`RemoteCommand`] as `Unsupported`, `Disabled` or `Enabled` with `set_command`, and check a command from
//! the system with `dispatch` before running the handler. The pure model is in `bifrost_ui::media_session`. Publishing the
//! session to the lock screen is not wired yet: the audio service carries no app session.
//!
//! [`Artwork`] is an identifier with the image sizes the item can supply. [`Artwork::choose`] picks the size to load for
//! a target, [`aspect_fit`] sizes it inside a box and [`request_pixels`] turns points and the display scale into the
//! pixel size passed to the artwork provider. The model is in `bifrost_ui::media_artwork`.

pub use bifrost_ui::media_artwork::{aspect_fit, request_pixels, Artwork, ArtworkError, ArtworkSize, MAX_SIDE};
pub use bifrost_ui::media_content::{MediaContent, MediaDuration, MediaType};
pub use bifrost_ui::media_session::{
    CommandState, MediaPlaybackSnapshot, MediaSession, MediaSessionError, PlaybackState, RemoteCommand,
};
