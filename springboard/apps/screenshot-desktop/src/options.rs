//! The toolbar's option state: what to capture, whether to record, where the file goes,
//! the timer, the pointer and the floating thumbnail.

use std::path::{Path, PathBuf};

/// What the toolbar captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Screen,
    Window,
    Selection,
}

/// Still capture or screen recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Screenshot,
    Recording,
}

/// Where a capture is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Desktop,
    Documents,
    Clipboard,
}

impl Destination {
    /// The folder under the home directory, or `None` for the clipboard.
    pub fn folder(&self) -> Option<&'static str> {
        match self {
            Destination::Desktop => Some("Desktop"),
            Destination::Documents => Some("Documents"),
            Destination::Clipboard => None,
        }
    }

    /// The full folder for this destination under `home`, or `None` for the clipboard.
    pub fn directory(&self, home: &Path) -> Option<PathBuf> {
        self.folder().map(|f| home.join(f))
    }
}

/// The delay before a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    Off,
    Seconds5,
    Seconds10,
}

impl Timer {
    /// Whole seconds to wait; zero when off.
    pub fn seconds(&self) -> u32 {
        match self {
            Timer::Off => 0,
            Timer::Seconds5 => 5,
            Timer::Seconds10 => 10,
        }
    }
}

/// The toolbar state. `Options::default()` is the state a fresh toolbar starts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub target: Target,
    pub mode: Mode,
    pub destination: Destination,
    pub timer: Timer,
    pub show_pointer: bool,
    pub show_thumbnail: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            target: Target::Screen,
            mode: Mode::Screenshot,
            destination: Destination::Desktop,
            timer: Timer::Off,
            show_pointer: false,
            show_thumbnail: true,
        }
    }
}

impl Options {
    /// Chooses what to capture. Recording keeps the target, so a recording can cover a selection.
    pub fn set_target(&mut self, target: Target) {
        self.target = target;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn set_destination(&mut self, destination: Destination) {
        self.destination = destination;
    }

    pub fn set_timer(&mut self, timer: Timer) {
        self.timer = timer;
    }

    /// Whether the floating thumbnail appears after a capture.
    pub fn thumbnail_shown(&self) -> bool {
        self.show_thumbnail
    }

    /// The timer delay for the next capture, in seconds.
    pub fn delay_seconds(&self) -> u32 {
        self.timer.seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_a_fresh_toolbar() {
        let o = Options::default();
        assert_eq!(o.target, Target::Screen);
        assert_eq!(o.mode, Mode::Screenshot);
        assert_eq!(o.destination, Destination::Desktop);
        assert_eq!(o.timer, Timer::Off);
        assert!(!o.show_pointer);
        assert!(o.show_thumbnail);
        assert_eq!(o.delay_seconds(), 0);
    }

    #[test]
    fn timer_seconds() {
        assert_eq!(Timer::Off.seconds(), 0);
        assert_eq!(Timer::Seconds5.seconds(), 5);
        assert_eq!(Timer::Seconds10.seconds(), 10);
    }

    #[test]
    fn setters_update_state() {
        let mut o = Options::default();
        o.set_target(Target::Selection);
        o.set_mode(Mode::Recording);
        o.set_destination(Destination::Clipboard);
        o.set_timer(Timer::Seconds10);
        assert_eq!(o.target, Target::Selection);
        assert_eq!(o.mode, Mode::Recording);
        assert_eq!(o.destination, Destination::Clipboard);
        assert_eq!(o.delay_seconds(), 10);
    }

    #[test]
    fn destination_folders() {
        let home = Path::new("/home/someone");
        assert_eq!(Destination::Desktop.directory(home), Some(PathBuf::from("/home/someone/Desktop")));
        assert_eq!(Destination::Documents.directory(home), Some(PathBuf::from("/home/someone/Documents")));
        assert_eq!(Destination::Clipboard.directory(home), None);
    }

    #[test]
    fn thumbnail_follows_option() {
        let mut o = Options::default();
        o.show_thumbnail = false;
        assert!(!o.thumbnail_shown());
    }
}
