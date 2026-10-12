//! `bifrost-audio-policy` (owner: task 09): defaults per role, jack and hot-plug rules, ducking, the microphone
//! and capture permissions and the menu-bar indicator, hog grants, per-app volume. A client of `bifrost-audiod`
//! with policy rights (`Request::Hello { policy: true }`). The decisions live in the library modules; the
//! control-socket loop that feeds them from property notices is wired when the HAL client (task 06) lands.
use bifrost_audio_policy::{defaults::Defaults, log};

fn main() {
    let mut d = Defaults::new();
    if let Ok(t) = std::fs::read_to_string(prefs_path()) {
        d.load(&t);
    }
    log("policy process started; waiting for the HAL client (task 06) to attach to the daemon");
}

fn prefs_path() -> String {
    std::env::var("BIFROST_AUDIO_POLICY_PREFS").unwrap_or_else(|_| "/var/lib/bifrost/audio/policy.prefs".into())
}
