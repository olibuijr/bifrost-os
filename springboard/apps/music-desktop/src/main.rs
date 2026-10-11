//! Music as an installed BifrOSt app: the former in-shell Music, with playback handed to the audio service
//! (`is.olibuijr.audio`) instead of an in-process engine.

fn main() {
    bifrost_app::run(bifrost_music::app::MusicApp::new());
}
