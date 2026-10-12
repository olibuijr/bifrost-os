//! Stream errors: the codes a capture can end with, grouped the way the official reference groups them, with
//! a short text for each. Pure: the capture pipeline maps its own failures onto these codes.

use std::fmt;

/// Why a capture stopped or could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    UserStopped,
    UserDeclined,
    MissingEntitlements,
    AttemptToStartStreamState,
    AttemptToStopStreamState,
    AttemptToUpdateFilterState,
    AttemptToConfigState,
    FailedToStart,
    RemovingStream,
    FailedToStartAudioCapture,
    FailedToStopAudioCapture,
    FailedToStartMicrophoneCapture,
    SystemStoppedStream,
    InternalError,
    NoCaptureSource,
    NoDisplayList,
    NoWindowList,
    FailedApplicationConnectionInvalid,
    FailedApplicationConnectionInterrupted,
    FailedNoMatchingApplicationContext,
    InvalidParameter,
}

impl StreamError {
    /// Every code, in the order of the groups below.
    pub const ALL: [StreamError; 21] = [
        StreamError::UserStopped,
        StreamError::UserDeclined,
        StreamError::MissingEntitlements,
        StreamError::AttemptToStartStreamState,
        StreamError::AttemptToStopStreamState,
        StreamError::AttemptToUpdateFilterState,
        StreamError::AttemptToConfigState,
        StreamError::FailedToStart,
        StreamError::RemovingStream,
        StreamError::FailedToStartAudioCapture,
        StreamError::FailedToStopAudioCapture,
        StreamError::FailedToStartMicrophoneCapture,
        StreamError::SystemStoppedStream,
        StreamError::InternalError,
        StreamError::NoCaptureSource,
        StreamError::NoDisplayList,
        StreamError::NoWindowList,
        StreamError::FailedApplicationConnectionInvalid,
        StreamError::FailedApplicationConnectionInterrupted,
        StreamError::FailedNoMatchingApplicationContext,
        StreamError::InvalidParameter,
    ];

    /// The group name the reference uses for this code.
    pub fn group(self) -> &'static str {
        match self {
            StreamError::UserStopped => "User cancellation",
            StreamError::UserDeclined | StreamError::MissingEntitlements => "Privacy and entitlements",
            StreamError::AttemptToStartStreamState
            | StreamError::AttemptToStopStreamState
            | StreamError::AttemptToUpdateFilterState
            | StreamError::AttemptToConfigState
            | StreamError::FailedToStart
            | StreamError::RemovingStream
            | StreamError::FailedToStartAudioCapture
            | StreamError::FailedToStopAudioCapture
            | StreamError::FailedToStartMicrophoneCapture
            | StreamError::SystemStoppedStream
            | StreamError::InternalError => "Stream management",
            StreamError::NoCaptureSource
            | StreamError::NoDisplayList
            | StreamError::NoWindowList
            | StreamError::FailedApplicationConnectionInvalid
            | StreamError::FailedApplicationConnectionInterrupted
            | StreamError::FailedNoMatchingApplicationContext => "Shareable content",
            StreamError::InvalidParameter => "Invalid parameters",
        }
    }

    /// True for the code that means the person stopped the capture.
    pub fn is_user_cancellation(self) -> bool {
        self == StreamError::UserStopped
    }

    /// True for the codes about permission or app signing, which the person or the app must fix.
    pub fn is_privacy(self) -> bool {
        self.group() == "Privacy and entitlements"
    }

    /// A short description of the failure.
    pub fn describe(self) -> &'static str {
        match self {
            StreamError::UserStopped => "The person stopped the capture.",
            StreamError::UserDeclined => "Screen recording permission was not granted to the app.",
            StreamError::MissingEntitlements => "The app is missing a required entitlement.",
            StreamError::AttemptToStartStreamState => "The stream is already running or does not exist.",
            StreamError::AttemptToStopStreamState => "The stream is already stopped or does not exist.",
            StreamError::AttemptToUpdateFilterState => "The stream could not change its content filter.",
            StreamError::AttemptToConfigState => "The stream could not change its configuration.",
            StreamError::FailedToStart => "The stream failed to start.",
            StreamError::RemovingStream => "The stream could not be removed.",
            StreamError::FailedToStartAudioCapture => "The audio part of the stream failed to start.",
            StreamError::FailedToStopAudioCapture => "The audio part of the stream failed to stop.",
            StreamError::FailedToStartMicrophoneCapture => "Microphone capture failed to start.",
            StreamError::SystemStoppedStream => "The system stopped the stream.",
            StreamError::InternalError => "The capture system hit an internal failure.",
            StreamError::NoCaptureSource => "The stream has nothing to capture.",
            StreamError::NoDisplayList => "No displays are available to the stream.",
            StreamError::NoWindowList => "No windows are available to the stream.",
            StreamError::FailedApplicationConnectionInvalid => "The stream lost its connection to an app.",
            StreamError::FailedApplicationConnectionInterrupted => "The connection to an app was interrupted.",
            StreamError::FailedNoMatchingApplicationContext => "No matching app context exists for the stream.",
            StreamError::InvalidParameter => "A parameter value is invalid.",
        }
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_code_once() {
        assert_eq!(StreamError::ALL.len(), 21);
        for (i, a) in StreamError::ALL.iter().enumerate() {
            for b in &StreamError::ALL[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn groups_match_the_reference_sections() {
        assert_eq!(StreamError::UserStopped.group(), "User cancellation");
        assert_eq!(StreamError::MissingEntitlements.group(), "Privacy and entitlements");
        assert_eq!(StreamError::InternalError.group(), "Stream management");
        assert_eq!(StreamError::NoWindowList.group(), "Shareable content");
        assert_eq!(StreamError::InvalidParameter.group(), "Invalid parameters");
    }

    #[test]
    fn only_the_stop_code_is_a_user_cancellation() {
        let cancels: Vec<StreamError> =
            StreamError::ALL.iter().copied().filter(|e| e.is_user_cancellation()).collect();
        assert_eq!(cancels, vec![StreamError::UserStopped]);
    }

    #[test]
    fn privacy_codes_are_the_two_in_their_group() {
        let privacy: Vec<StreamError> = StreamError::ALL.iter().copied().filter(|e| e.is_privacy()).collect();
        assert_eq!(privacy, vec![StreamError::UserDeclined, StreamError::MissingEntitlements]);
    }

    #[test]
    fn every_code_has_its_own_non_empty_text() {
        for (i, e) in StreamError::ALL.iter().enumerate() {
            assert!(!e.describe().is_empty());
            for other in &StreamError::ALL[i + 1..] {
                assert_ne!(e.describe(), other.describe());
            }
        }
        assert_eq!(StreamError::InvalidParameter.to_string(), "A parameter value is invalid.");
    }
}
