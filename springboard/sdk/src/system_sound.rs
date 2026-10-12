//! System sounds: a short sound an app creates once from a file, then plays by its id. The model keeps the ids and
//! plays one sound at a time: a new play replaces the request still waiting, and the audio service stops whatever
//! it plays when it takes the next request. Sounds are for short effects only (the official pages say 30 seconds
//! or less), with no level, position, looping or timing control, and no vibration. This is a pure model. It opens
//! no device; the audio service calls `take_request` and plays what it returns.

use std::fmt;

/// The id of a created sound. Ids start at 1 and are never reused after `dispose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundId(u32);

impl SoundId {
    /// The number behind the id.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// What a play asks for: an ordinary system sound, or an alert, which a device may also answer with a vibration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundKind {
    System,
    Alert,
}

/// The default-device role a sound plays on. System sounds and alerts always use the sound-effects device,
/// never the one music plays on, so they follow the user's sound-effects choice and level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputRole {
    Output,
    SoundEffects,
}

impl SoundKind {
    /// The device role the audio service opens to play this kind.
    pub const fn role(self) -> OutputRole {
        match self {
            SoundKind::System | SoundKind::Alert => OutputRole::SoundEffects,
        }
    }
}

impl PlayRequest {
    pub fn role(&self) -> OutputRole {
        self.kind.role()
    }
}

/// One play, as the audio service receives it. `completion` says the app asked to hear when the sound ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayRequest {
    pub id: SoundId,
    pub path: String,
    pub kind: SoundKind,
    pub completion: bool,
}

/// Failures of the sound model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundError {
    /// The path is empty or is not absolute.
    InvalidPath,
    /// The id was never created, or was disposed.
    UnknownSound,
    /// No id is left to hand out.
    Exhausted,
}

impl fmt::Display for SoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SoundError::InvalidPath => f.write_str("the sound path is empty or not absolute"),
            SoundError::UnknownSound => f.write_str("no sound has that id"),
            SoundError::Exhausted => f.write_str("no sound id is left"),
        }
    }
}

/// The sounds of one app: created sounds by id, the request waiting for the audio service, and the sound it has
/// taken.
#[derive(Debug, Default)]
pub struct SystemSounds {
    last: u32,
    sounds: Vec<(SoundId, String)>,
    pending: Option<PlayRequest>,
    active: Option<PlayRequest>,
}

impl SystemSounds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a sound from a file. The path must be absolute and not empty. The same path may be created twice,
    /// and each creation gets a new id.
    pub fn create(&mut self, path: &str) -> Result<SoundId, SoundError> {
        if path.is_empty() || !path.starts_with('/') {
            return Err(SoundError::InvalidPath);
        }
        self.last = self.last.checked_add(1).ok_or(SoundError::Exhausted)?;
        let id = SoundId(self.last);
        self.sounds.push((id, path.to_string()));
        Ok(id)
    }

    /// Frees a sound. A play of it that is still waiting or sounding is dropped.
    pub fn dispose(&mut self, id: SoundId) -> Result<(), SoundError> {
        let index = self.sounds.iter().position(|(sid, _)| *sid == id).ok_or(SoundError::UnknownSound)?;
        self.sounds.remove(index);
        if self.pending.as_ref().map_or(false, |r| r.id == id) {
            self.pending = None;
        }
        if self.active.as_ref().map_or(false, |r| r.id == id) {
            self.active = None;
        }
        Ok(())
    }

    /// The file a sound was created from, or `None` once it is disposed.
    pub fn path(&self, id: SoundId) -> Option<&str> {
        self.sounds.iter().find(|(sid, _)| *sid == id).map(|(_, path)| path.as_str())
    }

    /// Asks for a play. It replaces any request still waiting, so only the newest play is heard.
    pub fn play(&mut self, id: SoundId, kind: SoundKind, completion: bool) -> Result<(), SoundError> {
        let path = self.path(id).ok_or(SoundError::UnknownSound)?.to_string();
        self.pending = Some(PlayRequest { id, path, kind, completion });
        Ok(())
    }

    /// The waiting play, for the audio service to start. It becomes the active sound. Returns `None` when nothing
    /// is waiting.
    pub fn take_request(&mut self) -> Option<PlayRequest> {
        let request = self.pending.take()?;
        self.active = Some(request.clone());
        Some(request)
    }

    /// The sound that was taken and has not finished.
    pub fn active(&self) -> Option<SoundId> {
        self.active.as_ref().map(|r| r.id)
    }

    /// Reports that a sound has ended. Returns whether the app asked for a completion for it. A sound that is not
    /// the active one ends without effect and returns `false`.
    pub fn finish(&mut self, id: SoundId) -> Result<bool, SoundError> {
        if self.path(id).is_none() {
            return Err(SoundError::UnknownSound);
        }
        let completion = match self.active.as_ref() {
            Some(request) if request.id == id => Some(request.completion),
            _ => None,
        };
        match completion {
            Some(asked) => {
                self.active = None;
                Ok(asked)
            }
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_need_absolute_paths_and_get_new_ids() {
        let mut sounds = SystemSounds::new();
        assert_eq!(sounds.create(""), Err(SoundError::InvalidPath));
        assert_eq!(sounds.create("tink.wav"), Err(SoundError::InvalidPath));
        let a = sounds.create("/sounds/tink.wav").unwrap();
        let b = sounds.create("/sounds/tink.wav").unwrap();
        assert_eq!(a.raw(), 1);
        assert_eq!(b.raw(), 2);
        assert_eq!(sounds.path(b), Some("/sounds/tink.wav"));
    }

    #[test]
    fn playing_an_unknown_sound_is_an_error() {
        let mut sounds = SystemSounds::new();
        let a = sounds.create("/sounds/a.caf").unwrap();
        sounds.dispose(a).unwrap();
        assert_eq!(sounds.play(a, SoundKind::System, false), Err(SoundError::UnknownSound));
        assert_eq!(sounds.dispose(a), Err(SoundError::UnknownSound));
    }

    #[test]
    fn every_kind_plays_on_the_sound_effects_role() {
        assert_eq!(SoundKind::System.role(), OutputRole::SoundEffects);
        assert_eq!(SoundKind::Alert.role(), OutputRole::SoundEffects);
        let r = PlayRequest { id: SoundId(1), path: "/a.wav".into(), kind: SoundKind::Alert, completion: false };
        assert_eq!(r.role(), OutputRole::SoundEffects);
        assert_ne!(r.role(), OutputRole::Output);
    }

    #[test]
    fn only_the_newest_play_is_waiting() {
        let mut sounds = SystemSounds::new();
        let a = sounds.create("/sounds/a.caf").unwrap();
        let b = sounds.create("/sounds/b.caf").unwrap();
        sounds.play(a, SoundKind::System, false).unwrap();
        sounds.play(b, SoundKind::Alert, true).unwrap();
        let request = sounds.take_request().unwrap();
        assert_eq!(request.id, b);
        assert_eq!(request.path, "/sounds/b.caf");
        assert_eq!(request.kind, SoundKind::Alert);
        assert!(request.completion);
        assert!(sounds.take_request().is_none());
        assert_eq!(sounds.active(), Some(b));
    }

    #[test]
    fn finishing_reports_the_completion_request_once() {
        let mut sounds = SystemSounds::new();
        let a = sounds.create("/sounds/a.caf").unwrap();
        let b = sounds.create("/sounds/b.caf").unwrap();
        sounds.play(a, SoundKind::System, true).unwrap();
        sounds.take_request().unwrap();
        // A sound that is not the active one ends without effect.
        assert_eq!(sounds.finish(b), Ok(false));
        assert_eq!(sounds.active(), Some(a));
        assert_eq!(sounds.finish(a), Ok(true));
        assert_eq!(sounds.active(), None);
        assert_eq!(sounds.finish(a), Ok(false));
    }

    #[test]
    fn disposing_drops_a_waiting_play_and_ids_are_not_reused() {
        let mut sounds = SystemSounds::new();
        let a = sounds.create("/sounds/a.caf").unwrap();
        sounds.play(a, SoundKind::System, false).unwrap();
        sounds.dispose(a).unwrap();
        assert!(sounds.take_request().is_none());
        let c = sounds.create("/sounds/c.caf").unwrap();
        assert_eq!(c.raw(), 2);
        assert_eq!(sounds.path(a), None);
    }

    #[test]
    fn disposing_the_active_sound_clears_it() {
        let mut sounds = SystemSounds::new();
        let a = sounds.create("/sounds/a.caf").unwrap();
        sounds.play(a, SoundKind::System, false).unwrap();
        sounds.take_request().unwrap();
        assert_eq!(sounds.active(), Some(a));
        sounds.dispose(a).unwrap();
        assert_eq!(sounds.active(), None);
    }
}
