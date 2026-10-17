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
    use bifrost_ui::video_device::{PixelFormat, StreamFormat, BUFFER_COUNT, CAP_STREAMING, CAP_VIDEO_CAPTURE};

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

    fn streaming_device(path: &str) -> VideoDevice {
        VideoDevice {
            path: path.to_string(),
            card: "Test".to_string(),
            capabilities: CAP_VIDEO_CAPTURE | CAP_STREAMING,
            device_caps: 0,
            formats: vec![StreamFormat { format: PixelFormat::Yuyv, width: 640, height: 480 }],
        }
    }

    #[test]
    fn a_running_session_exposes_the_format_and_buffers_it_opened() {
        let mut session = CaptureSession::new(640, 480).unwrap();
        assert!(session.running_plan().is_none());
        session.set_input(streaming_device("/dev/video0")).unwrap();
        let started = session.start().unwrap();
        let plan = session.running_plan().expect("a running session has a plan");
        assert_eq!(plan, &started);
        assert_eq!(plan.path, "/dev/video0");
        assert_eq!(plan.format, StreamFormat { format: PixelFormat::Yuyv, width: 640, height: 480 });
        assert_eq!(plan.buffers, BUFFER_COUNT);
    }

    #[test]
    fn stopping_forgets_the_stream_plan_and_unlocks_the_input() {
        let mut session = CaptureSession::new(640, 480).unwrap();
        session.set_input(streaming_device("/dev/video0")).unwrap();
        session.start().unwrap();
        assert_eq!(session.set_size(320, 240), Err(CaptureError::Running));
        session.stop();
        assert!(session.running_plan().is_none());
        assert!(session.set_size(320, 240).is_ok());
    }
}
