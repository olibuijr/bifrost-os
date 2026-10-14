//! The capture view (`AVCaptureView`) as pure state: which capture session the view shows, which previews it draws, the
//! controls style and the video gravity, and the delegate event an app receives when the user asks to start a recording.
//! The view does not own the session or the file output; it refers to them by `SessionId`. Nothing is drawn, captured or
//! written here. The owner draws the preview and performs the recording it is asked for.

/// Identifies a capture session owned by the capture service. The view only holds the id.
pub type SessionId = u64;

/// The controls the capture view shows around the preview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlsStyle {
    /// The view's default controls style.
    #[default]
    Default,
    /// Controls drawn inside the view, below the preview.
    Inline,
    /// Controls that float over the preview, as a movie player's capture window does.
    Floating,
    /// Inline controls that also let the viewer choose the capture device.
    InlineDeviceSelection,
}

/// How the video fills the view's bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoGravity {
    /// The whole picture fits inside the bounds, with bars where the aspect ratio differs.
    #[default]
    ResizeAspect,
    /// The picture fills the bounds and is cropped to keep its aspect ratio.
    ResizeAspectFill,
    /// The picture is stretched to the bounds.
    Resize,
}

impl VideoGravity {
    /// The string value the video gravity takes in the capture view's interface.
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::ResizeAspect => "AVLayerVideoGravityResizeAspect",
            Self::ResizeAspectFill => "AVLayerVideoGravityResizeAspectFill",
            Self::Resize => "AVLayerVideoGravityResize",
        }
    }
}

/// The session the view is bound to, with the previews it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub session: SessionId,
    pub video_preview: bool,
    pub audio_preview: bool,
}

/// What the view reports to its delegate. The delegate is the app's own object, so events are returned rather than sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user asked to start a new recording into `path`.
    StartRecording { path: String },
}

/// Why a recording request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No session is bound, so there is nothing to record.
    NoSession,
    /// The destination path is empty.
    EmptyPath,
}

/// The state of one capture view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureView {
    binding: Option<Binding>,
    controls_style: ControlsStyle,
    video_gravity: VideoGravity,
}

impl CaptureView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds the view to `session`, replacing any earlier binding. The previews are drawn only when their flags are set.
    pub fn set_session(&mut self, session: SessionId, show_video_preview: bool, show_audio_preview: bool) {
        self.binding = Some(Binding {
            session,
            video_preview: show_video_preview,
            audio_preview: show_audio_preview,
        });
    }

    /// Removes the binding. Nothing is previewed and nothing can be recorded until a session is bound again.
    pub fn clear_session(&mut self) {
        self.binding = None;
    }

    pub fn session(&self) -> Option<SessionId> {
        self.binding.map(|b| b.session)
    }

    /// True when the view draws the video preview. Always false without a session.
    pub fn shows_video_preview(&self) -> bool {
        self.binding.is_some_and(|b| b.video_preview)
    }

    /// True when the view draws the audio level preview. Always false without a session.
    pub fn shows_audio_preview(&self) -> bool {
        self.binding.is_some_and(|b| b.audio_preview)
    }

    pub fn controls_style(&self) -> ControlsStyle {
        self.controls_style
    }

    pub fn set_controls_style(&mut self, style: ControlsStyle) {
        self.controls_style = style;
    }

    pub fn video_gravity(&self) -> VideoGravity {
        self.video_gravity
    }

    pub fn set_video_gravity(&mut self, gravity: VideoGravity) {
        self.video_gravity = gravity;
    }

    /// The user taps the record control. The request becomes a delegate event when a session is bound and the path is
    /// not empty. Otherwise the request is refused and nothing is sent.
    pub fn request_recording(&self, path: &str) -> Result<Event, Refusal> {
        if self.binding.is_none() {
            return Err(Refusal::NoSession);
        }
        if path.trim().is_empty() {
            return Err(Refusal::EmptyPath);
        }
        Ok(Event::StartRecording { path: path.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_view_has_no_session_and_no_previews() {
        let v = CaptureView::new();
        assert_eq!(v.session(), None);
        assert!(!v.shows_video_preview());
        assert!(!v.shows_audio_preview());
        assert_eq!(v.controls_style(), ControlsStyle::Default);
        assert_eq!(v.video_gravity(), VideoGravity::ResizeAspect);
    }

    #[test]
    fn binding_a_session_sets_the_previews_from_the_flags() {
        let mut v = CaptureView::new();
        v.set_session(7, true, false);
        assert_eq!(v.session(), Some(7));
        assert!(v.shows_video_preview());
        assert!(!v.shows_audio_preview());
    }

    #[test]
    fn rebinding_replaces_the_earlier_session() {
        let mut v = CaptureView::new();
        v.set_session(1, true, true);
        v.set_session(2, false, true);
        assert_eq!(v.session(), Some(2));
        assert!(!v.shows_video_preview());
        assert!(v.shows_audio_preview());
    }

    #[test]
    fn clearing_the_session_hides_both_previews() {
        let mut v = CaptureView::new();
        v.set_session(3, true, true);
        v.clear_session();
        assert_eq!(v.session(), None);
        assert!(!v.shows_video_preview());
        assert!(!v.shows_audio_preview());
    }

    #[test]
    fn style_and_gravity_are_stored_as_set() {
        let mut v = CaptureView::new();
        v.set_controls_style(ControlsStyle::Floating);
        v.set_video_gravity(VideoGravity::ResizeAspectFill);
        assert_eq!(v.controls_style(), ControlsStyle::Floating);
        assert_eq!(v.video_gravity(), VideoGravity::ResizeAspectFill);
    }

    #[test]
    fn gravity_raw_values_match_the_layer_gravity_names() {
        assert_eq!(VideoGravity::ResizeAspect.raw_value(), "AVLayerVideoGravityResizeAspect");
        assert_eq!(VideoGravity::ResizeAspectFill.raw_value(), "AVLayerVideoGravityResizeAspectFill");
        assert_eq!(VideoGravity::Resize.raw_value(), "AVLayerVideoGravityResize");
    }

    #[test]
    fn recording_needs_a_session() {
        let v = CaptureView::new();
        assert_eq!(v.request_recording("/data/clip.mov"), Err(Refusal::NoSession));
    }

    #[test]
    fn recording_needs_a_destination_path() {
        let mut v = CaptureView::new();
        v.set_session(1, true, true);
        assert_eq!(v.request_recording("   "), Err(Refusal::EmptyPath));
        assert_eq!(v.request_recording(""), Err(Refusal::EmptyPath));
    }

    #[test]
    fn a_bound_view_turns_a_recording_request_into_a_delegate_event() {
        let mut v = CaptureView::new();
        v.set_session(1, true, false);
        assert_eq!(
            v.request_recording("/data/clip.mov"),
            Ok(Event::StartRecording { path: "/data/clip.mov".to_string() })
        );
    }
}
