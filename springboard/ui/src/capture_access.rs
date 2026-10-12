//! Capture permission for one app: whether it may use the camera, the microphone, or the screen contents (screen
//! recording and screenshots of other windows). Each media type has its own status, as the system keeps them. The person answers the prompt once per media type; after that
//! the answer changes only through Settings (`set_by_user`). A system restriction, such as one from parental controls,
//! blocks the prompt and every change until the system lifts it (`set_restricted`).
//! Pure state: the shell shows the prompt, calls `request` with the answer, and asks `can_capture` before it opens a
//! device. Nothing here opens a device or shows a dialog.

/// A kind of capture the app can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    /// Camera frames.
    Video,
    /// Microphone samples.
    Audio,
    /// Screen contents: screen recording and frames of content the app did not draw itself.
    Screen,
}

/// What the person, or the system, has decided for one media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The prompt has not been answered yet.
    NotDetermined,
    /// The system blocks this media type for the app; no answer changes it.
    Restricted,
    /// The person refused it.
    Denied,
    /// The person allowed it.
    Authorized,
}

/// Why a change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The system restricts this media type, so the person cannot change it.
    Restricted,
}

/// The capture permissions of one app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureAccess {
    video: Status,
    audio: Status,
    screen: Status,
}

impl Default for CaptureAccess {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureAccess {
    /// A new app: nothing is answered yet.
    pub fn new() -> Self {
        CaptureAccess { video: Status::NotDetermined, audio: Status::NotDetermined, screen: Status::NotDetermined }
    }

    fn slot(&self, media: Media) -> Status {
        match media {
            Media::Video => self.video,
            Media::Audio => self.audio,
            Media::Screen => self.screen,
        }
    }

    fn slot_mut(&mut self, media: Media) -> &mut Status {
        match media {
            Media::Video => &mut self.video,
            Media::Audio => &mut self.audio,
            Media::Screen => &mut self.screen,
        }
    }

    /// The status of one media type.
    pub fn status(&self, media: Media) -> Status {
        self.slot(media)
    }

    /// Answers the prompt for one media type. The prompt is shown once: when the status is already decided, or the
    /// system restricts it, the answer changes nothing and the current status is returned.
    pub fn request(&mut self, media: Media, granted: bool) -> Status {
        let slot = self.slot_mut(media);
        if *slot == Status::NotDetermined {
            *slot = if granted { Status::Authorized } else { Status::Denied };
        }
        *slot
    }

    /// A change the person makes in Settings. Refused while the system restricts the media type.
    pub fn set_by_user(&mut self, media: Media, allowed: bool) -> Result<Status, Error> {
        let slot = self.slot_mut(media);
        if *slot == Status::Restricted {
            return Err(Error::Restricted);
        }
        *slot = if allowed { Status::Authorized } else { Status::Denied };
        Ok(*slot)
    }

    /// Sets or lifts a system restriction on one media type. Lifting it returns the type to not determined, so the
    /// prompt can be shown again. Other media types are not touched.
    pub fn set_restricted(&mut self, media: Media, restricted: bool) -> Status {
        let slot = self.slot_mut(media);
        if restricted {
            *slot = Status::Restricted;
        } else if *slot == Status::Restricted {
            *slot = Status::NotDetermined;
        }
        *slot
    }

    /// Whether the app may open a device for this media type now.
    pub fn can_capture(&self, media: Media) -> bool {
        self.slot(media) == Status::Authorized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_app_has_no_capture_until_it_is_asked() {
        let a = CaptureAccess::new();
        assert_eq!(a.status(Media::Video), Status::NotDetermined);
        assert_eq!(a.status(Media::Audio), Status::NotDetermined);
        assert!(!a.can_capture(Media::Video));
        assert!(!a.can_capture(Media::Audio));
    }

    #[test]
    fn an_answer_covers_only_its_media_type() {
        let mut a = CaptureAccess::new();
        assert_eq!(a.request(Media::Video, true), Status::Authorized);
        assert!(a.can_capture(Media::Video));
        assert_eq!(a.status(Media::Audio), Status::NotDetermined);
        assert!(!a.can_capture(Media::Audio));
    }

    #[test]
    fn the_prompt_is_shown_once() {
        let mut a = CaptureAccess::new();
        assert_eq!(a.request(Media::Audio, false), Status::Denied);
        // A later answer does not change a decided status.
        assert_eq!(a.request(Media::Audio, true), Status::Denied);
        assert!(!a.can_capture(Media::Audio));
    }

    #[test]
    fn a_system_restriction_blocks_the_prompt_and_settings() {
        let mut a = CaptureAccess::new();
        assert_eq!(a.set_restricted(Media::Video, true), Status::Restricted);
        assert_eq!(a.request(Media::Video, true), Status::Restricted);
        assert_eq!(a.set_by_user(Media::Video, true), Err(Error::Restricted));
        assert!(!a.can_capture(Media::Video));
    }

    #[test]
    fn lifting_a_restriction_shows_the_prompt_again() {
        let mut a = CaptureAccess::new();
        a.set_restricted(Media::Audio, true);
        assert_eq!(a.set_restricted(Media::Audio, false), Status::NotDetermined);
        assert_eq!(a.request(Media::Audio, true), Status::Authorized);
        assert!(a.can_capture(Media::Audio));
    }

    #[test]
    fn settings_can_change_a_refused_answer() {
        let mut a = CaptureAccess::new();
        a.request(Media::Video, false);
        assert_eq!(a.set_by_user(Media::Video, true), Ok(Status::Authorized));
        assert!(a.can_capture(Media::Video));
        assert_eq!(a.set_by_user(Media::Video, false), Ok(Status::Denied));
        assert!(!a.can_capture(Media::Video));
    }

    #[test]
    fn a_restriction_on_one_type_leaves_the_other_alone() {
        let mut a = CaptureAccess::new();
        a.set_restricted(Media::Video, true);
        assert_eq!(a.request(Media::Audio, true), Status::Authorized);
        assert_eq!(a.status(Media::Video), Status::Restricted);
    }

    #[test]
    fn screen_recording_starts_undetermined_and_is_asked_once() {
        let mut a = CaptureAccess::new();
        assert_eq!(a.status(Media::Screen), Status::NotDetermined);
        assert!(!a.can_capture(Media::Screen));
        assert_eq!(a.request(Media::Screen, true), Status::Authorized);
        assert_eq!(a.request(Media::Screen, false), Status::Authorized);
        assert!(a.can_capture(Media::Screen));
    }

    #[test]
    fn a_screen_answer_does_not_grant_the_camera_or_microphone() {
        let mut a = CaptureAccess::new();
        a.request(Media::Screen, true);
        assert_eq!(a.status(Media::Video), Status::NotDetermined);
        assert_eq!(a.status(Media::Audio), Status::NotDetermined);
        assert!(!a.can_capture(Media::Video));
    }

    #[test]
    fn a_system_restriction_blocks_screen_recording_settings_changes() {
        let mut a = CaptureAccess::new();
        a.request(Media::Screen, false);
        assert_eq!(a.set_restricted(Media::Screen, true), Status::Restricted);
        assert_eq!(a.set_by_user(Media::Screen, true), Err(Error::Restricted));
        assert_eq!(a.set_restricted(Media::Screen, false), Status::NotDetermined);
        assert_eq!(a.set_by_user(Media::Screen, true), Ok(Status::Authorized));
        assert!(a.can_capture(Media::Screen));
    }
}
