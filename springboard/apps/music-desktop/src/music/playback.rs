//! Playback settings: the crossfade length between songs, and Sound Check, which plays each song at one loudness.
//! The settings, their text form and the gain maths are pure. The audio service has no crossfade or gain command
//! yet, so nothing sends these values to the engine.

use std::f32::consts::FRAC_PI_2;

/// The loudness Sound Check levels every song to, in dB.
pub const SOUND_CHECK_TARGET_DB: f32 = -16.0;
/// The largest change Sound Check makes to one song, in dB either way.
pub const SOUND_CHECK_LIMIT_DB: f32 = 9.0;
/// The loudness a ReplayGain tag is measured against, in dB: a song tagged `g` dB has measured `REPLAYGAIN_REFERENCE_DB - g`.
pub const REPLAYGAIN_REFERENCE_DB: f32 = -18.0;
/// The longest crossfade offered, in seconds.
pub const CROSSFADE_MAX_SECS: u32 = 12;

/// The playback settings Music keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playback {
    /// seconds in which one song fades out as the next fades in; 0 is off
    pub crossfade_secs: u32,
    pub sound_check: bool,
}

impl Playback {
    /// The settings with the crossfade set to `secs`, at most `CROSSFADE_MAX_SECS`.
    pub fn with_crossfade(self, secs: u32) -> Self {
        Playback { crossfade_secs: secs.min(CROSSFADE_MAX_SECS), ..self }
    }

    /// The linear gain Sound Check applies to a song measured at `loudness_db`. A song with no measurement, a
    /// measurement that is not a number, and every song while Sound Check is off play at unity gain.
    pub fn gain(&self, loudness_db: Option<f32>) -> f32 {
        match loudness_db {
            Some(db) if self.sound_check && db.is_finite() => {
                let change = (SOUND_CHECK_TARGET_DB - db).clamp(-SOUND_CHECK_LIMIT_DB, SOUND_CHECK_LIMIT_DB);
                10f32.powf(change / 20.0)
            }
            _ => 1.0,
        }
    }

    /// Reads the settings file: a `crossfade <seconds>` line and a `sound_check <0|1>` line. Other lines, and values
    /// that do not parse, keep the default.
    pub fn parse(text: &str) -> Self {
        let mut settings = Playback::default();
        for line in text.lines() {
            let mut cols = line.split_whitespace();
            match (cols.next(), cols.next()) {
                (Some("crossfade"), Some(value)) => {
                    if let Ok(secs) = value.parse::<u32>() {
                        settings = settings.with_crossfade(secs);
                    }
                }
                (Some("sound_check"), Some("1")) => settings.sound_check = true,
                (Some("sound_check"), Some("0")) => settings.sound_check = false,
                _ => {}
            }
        }
        settings
    }

    /// The settings file text that `parse` reads back.
    pub fn to_text(&self) -> String {
        format!("crossfade {}\nsound_check {}\n", self.crossfade_secs, u8::from(self.sound_check))
    }
}

/// The loudness, in dB, that a song with a ReplayGain track tag of `gain_db` measures at. Sound Check levels that
/// measurement with `Playback::gain`.
pub fn loudness_from_replaygain(gain_db: f32) -> f32 {
    REPLAYGAIN_REFERENCE_DB - gain_db
}

/// The gains of the song that ends and the song that starts at `t` through the crossfade (0 at its start, 1 at its
/// end). The curve is equal power: the squares of the two gains add up to 1 at every point, so the level holds
/// through the middle of the overlap.
pub fn crossfade_gains(t: f32) -> (f32, f32) {
    let angle = t.clamp(0.0, 1.0) * FRAC_PI_2;
    (angle.cos(), angle.sin())
}
