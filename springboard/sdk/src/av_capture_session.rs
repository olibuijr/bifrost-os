//! Capture sessions for apps (the AVCaptureSession shape): one video input, the frame size the session asks for, and a
//! start that plans the stream the system layer opens. The model is `bifrost_ui::capture_session`, built on the
//! Linux V4L2 device list; an app gets the same checks the Camera app runs, with no device opened by this module.
//!
//! ```ignore
//! let mut session = CaptureSession::new(1280, 720)?;
//! session.set_input(device)?;   // a VideoDevice the system layer found
//! let plan = session.start()?;  // the StreamPlan names the node to open
//! ```

pub use bifrost_ui::capture_session::*;
pub use bifrost_ui::video_device::{StreamPlan, VideoDevice};

#[cfg(test)]
mod tests {
    use super::*;

    fn unusable(path: &str) -> VideoDevice {
        VideoDevice { path: path.to_string(), card: "Test".to_string(), capabilities: 0, device_caps: 0, formats: Vec::new() }
    }

    #[test]
    fn a_new_session_is_stopped_with_no_input_and_the_requested_size() {
        let session = CaptureSession::new(640, 480).unwrap();
        assert_eq!(session.state(), SessionState::Stopped);
        assert!(session.input().is_none());
        assert_eq!(session.size(), (640, 480));
    }

    #[test]
    fn a_zero_width_or_height_is_refused() {
        assert_eq!(CaptureSession::new(0, 480).unwrap_err(), CaptureError::InvalidSize);
        let mut session = CaptureSession::new(640, 480).unwrap();
        assert_eq!(session.set_size(640, 0), Err(CaptureError::InvalidSize));
        assert_eq!(session.size(), (640, 480));
    }

    #[test]
    fn a_session_with_no_input_cannot_start() {
        let mut session = CaptureSession::new(640, 480).unwrap();
        assert_eq!(session.start().unwrap_err(), CaptureError::NoInput);
        assert_eq!(session.state(), SessionState::Stopped);
    }

    #[test]
    fn a_device_that_cannot_stream_is_refused_as_input() {
        let mut session = CaptureSession::new(640, 480).unwrap();
        let err = session.set_input(unusable("/dev/video9")).unwrap_err();
        assert_eq!(err, CaptureError::UnusableInput { path: "/dev/video9".to_string() });
        assert!(session.input().is_none());
    }

    #[test]
    fn stopping_a_stopped_session_does_nothing() {
        let mut session = CaptureSession::new(640, 480).unwrap();
        session.stop();
        session.stop();
        assert_eq!(session.state(), SessionState::Stopped);
        assert!(session.remove_input().is_ok());
    }
}
