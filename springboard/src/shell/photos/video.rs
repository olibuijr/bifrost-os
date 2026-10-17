//! Videos in Photos: a poster frame for the grid and playback in the viewer, both through the shell's media player.
//! A poster is decoded without sound; the viewer's playback goes to the speaker.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::gfx::Image;
use crate::media::audio_out::NullOut;
use crate::media::player::{Player, State};

/// How long a poster frame may take to decode before its thumbnail gives up.
const POSTER_WAIT: Duration = Duration::from_secs(3);

/// True for the movie files in the library.
pub fn is_video(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::library::is_video_name(&name)
}

/// The first decoded frame of a movie, for its thumbnail. None when it cannot be opened or decoded in time.
pub fn poster(path: &Path) -> Option<Image> {
    let mut player = Player::open_audio(path, Box::new(NullOut::new())).ok()?;
    let deadline = Instant::now() + POSTER_WAIT;
    loop {
        if let Some(img) = player.take_frame() {
            return Some(img);
        }
        if matches!(player.state(), State::Error(_) | State::Ended) || Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Opens a movie for playback in the viewer, with sound. It starts playing.
pub fn open(path: &Path) -> Option<Player> {
    Player::open(path).ok()
}

/// Whether the player is showing motion (the play/pause control shows the pause bars then).
pub fn is_playing(player: &Player) -> bool {
    matches!(player.state(), State::Playing | State::Buffering { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movies_are_recognised_by_extension_in_any_case() {
        assert!(is_video(Path::new("/p/IMG_1.MOV")));
        assert!(is_video(Path::new("/p/clip.mp4")));
        assert!(is_video(Path::new("/p/clip.m4v")));
        assert!(!is_video(Path::new("/p/photo.jpg")));
        assert!(!is_video(Path::new("/p/movie.mp4.txt")));
    }

    #[test]
    fn a_missing_movie_has_no_poster() {
        assert!(poster(Path::new("/nonexistent/clip.mp4")).is_none());
        assert!(open(Path::new("/nonexistent/clip.mp4")).is_none());
    }
}
