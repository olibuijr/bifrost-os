//! Capture sessions: the model behind a camera session. A session holds one video input, a frame size and a running
//! state. The input is checked with `VideoDevice::is_usable`, and `start` plans the stream with `video_device::plan_stream`.
//! The input and the size are locked while the session runs, so an app changes them only between `stop` and `start`.
//! Pure: no device is opened here. The system layer opens the node named in the returned `StreamPlan`.

use crate::video_device::{plan_stream, StreamError, StreamPlan, VideoDevice};

/// Whether a session delivers frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionState {
    #[default]
    Stopped,
    Running,
}

/// Why a session refused a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// The session is running, so its input and its size are locked.
    Running,
    /// The session is already running.
    AlreadyRunning,
    /// The session has no input to stream from.
    NoInput,
    /// The device cannot stream (see `VideoDevice::is_usable`).
    UnusableInput { path: String },
    /// The width or the height is zero.
    InvalidSize,
    /// The input offers no format that can carry a frame of the session size.
    Stream(StreamError),
}

/// A camera session: one video input, the frame size it asks for, and whether it is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSession {
    input: Option<VideoDevice>,
    width: u32,
    height: u32,
    state: SessionState,
}

impl CaptureSession {
    /// A stopped session with no input that asks for frames of `width` by `height`.
    pub fn new(width: u32, height: u32) -> Result<Self, CaptureError> {
        check_size(width, height)?;
        Ok(Self { input: None, width, height, state: SessionState::Stopped })
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn input(&self) -> Option<&VideoDevice> {
        self.input.as_ref()
    }

    /// The frame size the session asks for, as (width, height).
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Makes `device` the input, replacing any earlier one.
    pub fn set_input(&mut self, device: VideoDevice) -> Result<(), CaptureError> {
        self.ensure_stopped()?;
        if !device.is_usable() {
            return Err(CaptureError::UnusableInput { path: device.path });
        }
        self.input = Some(device);
        Ok(())
    }

    pub fn remove_input(&mut self) -> Result<(), CaptureError> {
        self.ensure_stopped()?;
        self.input = None;
        Ok(())
    }

    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), CaptureError> {
        self.ensure_stopped()?;
        check_size(width, height)?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Starts the session. Plans the stream of the input at the session size, and returns the plan the system
    /// layer opens the device with.
    pub fn start(&mut self) -> Result<StreamPlan, CaptureError> {
        if self.state == SessionState::Running {
            return Err(CaptureError::AlreadyRunning);
        }
        let input = self.input.as_ref().ok_or(CaptureError::NoInput)?;
        let plan = plan_stream(std::slice::from_ref(input), self.width, self.height).map_err(CaptureError::Stream)?;
        self.state = SessionState::Running;
        Ok(plan)
    }

    /// Stops the session and unlocks its configuration. Stopping a stopped session does nothing.
    pub fn stop(&mut self) {
        self.state = SessionState::Stopped;
    }

    fn ensure_stopped(&self) -> Result<(), CaptureError> {
        match self.state {
            SessionState::Stopped => Ok(()),
            SessionState::Running => Err(CaptureError::Running),
        }
    }
}

fn check_size(width: u32, height: u32) -> Result<(), CaptureError> {
    if width == 0 || height == 0 {
        Err(CaptureError::InvalidSize)
    } else {
        Ok(())
    }
}
