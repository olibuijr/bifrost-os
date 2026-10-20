//! Fingerprint sensor support for local authentication. The Linux fingerprint stack is `libfprint` behind the
//! `fprintd` service; this module drives it through its command-line tools (`fprintd-list`, `fprintd-verify`) so no
//! bus library is needed, and turns what they print into the facts the biometry model (`crate::biometry`) decides
//! with. Everything but [`probe`] and [`verify`] is a pure parser over text, so it is tested without a sensor. The
//! fingerprint data never leaves `fprintd`: this code sees only finger names and a match answer.

use crate::biometry::{BiometryType, Device};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// One enrolled finger, by the name `fprintd` gives it (`right-index-finger`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finger(pub String);

impl Finger {
    /// The finger as a person says it: `right-index-finger` becomes "Right index finger".
    pub fn label(&self) -> String {
        let spaced = self.0.replace('-', " ");
        let mut chars = spaced.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}

/// How a sensor takes a print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Press,
    Swipe,
}

/// What `fprintd-list` found for one user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sensor {
    /// A fingerprint reader is present.
    pub present: bool,
    pub name: String,
    pub scan: Option<Scan>,
    pub enrolled: Vec<Finger>,
}

/// Reads the output of `fprintd-list <user>`.
pub fn parse_list(output: &str) -> Sensor {
    let mut sensor = Sensor::default();
    for line in output.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("No devices") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Fingerprints for user ") {
            // "<user> on <device name> (<press|swipe>):"
            sensor.present = true;
            if let Some((_, tail)) = rest.split_once(" on ") {
                let tail = tail.trim_end_matches(':');
                if let Some((name, kind)) = tail.rsplit_once(" (") {
                    sensor.name = name.to_string();
                    sensor.scan = match kind.trim_end_matches(')') {
                        "press" => Some(Scan::Press),
                        "swipe" => Some(Scan::Swipe),
                        _ => None,
                    };
                } else {
                    sensor.name = tail.to_string();
                }
            }
        } else if line.starts_with("User ") && line.contains(" has no fingers enrolled for ") {
            sensor.present = true;
            if let Some((_, name)) = line.split_once(" has no fingers enrolled for ") {
                sensor.name = name.trim_end_matches('.').to_string();
            }
        } else if let Some(rest) = line.strip_prefix("- #") {
            if let Some((_, name)) = rest.split_once(':') {
                let name = name.trim();
                if !name.is_empty() {
                    sensor.enrolled.push(Finger(name.to_string()));
                }
            }
        }
    }
    sensor
}

/// The biometry model's device facts for this sensor. A fingerprint reader is the `Touch` type.
pub fn device_facts(sensor: &Sensor, passcode_set: bool, failures: u32, max_failures: u32) -> Device {
    Device {
        biometry: if sensor.present { BiometryType::Touch } else { BiometryType::None },
        enrolled_sets: sensor.enrolled.iter().map(|f| f.0.clone()).collect(),
        passcode_set,
        failures,
        max_failures,
    }
}

/// One status line of `fprintd-verify`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Match,
    NoMatch,
    /// The scan was poor: touch again.
    RetryScan,
    SwipeTooShort,
    FingerNotCentered,
    RemoveAndRetry,
    Disconnected,
    UnknownError,
}

impl Status {
    /// Whether the verification ends with this status.
    pub fn is_final(self) -> bool {
        matches!(self, Status::Match | Status::NoMatch | Status::Disconnected | Status::UnknownError)
    }

    /// The prompt line shown to the person for this status.
    pub fn prompt(self) -> &'static str {
        match self {
            Status::Match => "Fingerprint recognised",
            Status::NoMatch => "Fingerprint not recognised. Try again or use your password",
            Status::RetryScan => "Could not read the fingerprint. Touch the sensor again",
            Status::SwipeTooShort => "Swipe was too short. Swipe the whole finger over the sensor",
            Status::FingerNotCentered => "Center your finger on the sensor",
            Status::RemoveAndRetry => "Lift your finger, then touch the sensor again",
            Status::Disconnected => "The fingerprint reader was disconnected. Use your password",
            Status::UnknownError => "The fingerprint reader failed. Use your password",
        }
    }
}

/// Reads one `fprintd-verify` line (`Verify result: verify-match (done)`), None for any other line.
pub fn parse_verify_line(line: &str) -> Option<Status> {
    let word = line.trim().strip_prefix("Verify result:")?.split_whitespace().next()?;
    Some(match word {
        "verify-match" => Status::Match,
        "verify-no-match" => Status::NoMatch,
        "verify-retry-scan" => Status::RetryScan,
        "verify-swipe-too-short" => Status::SwipeTooShort,
        "verify-finger-not-centered" => Status::FingerNotCentered,
        "verify-remove-and-retry" => Status::RemoveAndRetry,
        "verify-disconnected" => Status::Disconnected,
        _ => Status::UnknownError,
    })
}

/// The outcome of a whole verification: the first final status in `lines`, or None when the tool ended without one.
pub fn outcome<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<Status> {
    lines.into_iter().filter_map(parse_verify_line).find(|s| s.is_final())
}

/// Whether a user name is safe to hand to the tools: letters, digits, dot, dash and underscore, not starting with a dash.
pub fn valid_user(user: &str) -> bool {
    !user.is_empty() && user.len() <= 32 && !user.starts_with('-') && user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Asks `fprintd` about the reader and `user`'s enrolled fingers. An absent tool or service reads as no sensor.
pub fn probe(user: &str) -> Sensor {
    if !valid_user(user) {
        return Sensor::default();
    }
    match Command::new("fprintd-list").arg(user).output() {
        Ok(out) if out.status.success() => parse_list(&String::from_utf8_lossy(&out.stdout)),
        _ => Sensor::default(),
    }
}

/// Runs `fprintd-verify` for `user`, calling `on_status` for each status line (so a sheet can show the prompt), and
/// returns the final status. The tool is killed when the final status arrives.
pub fn verify(user: &str, mut on_status: impl FnMut(Status)) -> Status {
    if !valid_user(user) {
        return Status::UnknownError;
    }
    let Ok(mut child) = Command::new("fprintd-verify").arg(user).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return Status::UnknownError;
    };
    let mut result = Status::UnknownError;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(status) = parse_verify_line(&line) {
                on_status(status);
                if status.is_final() {
                    result = status;
                    break;
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometry::{resolve, Failure, Method, Policy};

    const LISTED: &str = "found 1 devices\nDevice at /net/reactivated/Fprint/Device/0\nUsing device /net/reactivated/Fprint/Device/0\nFingerprints for user olibuijr on Goodix MOC Fingerprint Sensor (press):\n - #0: right-index-finger\n - #1: left-thumb\n";

    #[test]
    fn a_listing_gives_the_sensor_and_the_enrolled_fingers() {
        let s = parse_list(LISTED);
        assert!(s.present);
        assert_eq!(s.name, "Goodix MOC Fingerprint Sensor");
        assert_eq!(s.scan, Some(Scan::Press));
        assert_eq!(s.enrolled, [Finger("right-index-finger".into()), Finger("left-thumb".into())]);
        assert_eq!(s.enrolled[0].label(), "Right index finger");
    }

    #[test]
    fn a_reader_with_no_fingers_is_present_but_not_enrolled() {
        let s = parse_list("Using device /net/reactivated/Fprint/Device/0\nUser olibuijr has no fingers enrolled for Synaptics Sensors.\n");
        assert!(s.present && s.enrolled.is_empty());
        assert_eq!(s.name, "Synaptics Sensors");
    }

    #[test]
    fn no_reader_means_no_sensor() {
        assert_eq!(parse_list("No devices available\n"), Sensor::default());
        assert_eq!(parse_list(""), Sensor::default());
    }

    #[test]
    fn a_swipe_reader_is_told_from_a_press_reader() {
        let s = parse_list("Fingerprints for user a on AuthenTec (swipe):\n - #0: left-index-finger\n");
        assert_eq!(s.scan, Some(Scan::Swipe));
    }

    #[test]
    fn verify_lines_map_to_statuses() {
        assert_eq!(parse_verify_line("Verify result: verify-match (done)"), Some(Status::Match));
        assert_eq!(parse_verify_line("Verify result: verify-no-match (done)"), Some(Status::NoMatch));
        assert_eq!(parse_verify_line("Verify result: verify-retry-scan (not done)"), Some(Status::RetryScan));
        assert_eq!(parse_verify_line("Verify result: verify-swipe-too-short (not done)"), Some(Status::SwipeTooShort));
        assert_eq!(parse_verify_line("Verify result: verify-something-new"), Some(Status::UnknownError));
        assert_eq!(parse_verify_line("Verify started!"), None);
        assert!(!Status::RetryScan.is_final());
        assert!(Status::Match.is_final());
    }

    #[test]
    fn the_outcome_is_the_first_final_status_after_retries() {
        let lines = ["Verify started!", "Verify result: verify-retry-scan (not done)", "Verify result: verify-match (done)"];
        assert_eq!(outcome(lines), Some(Status::Match));
        assert_eq!(outcome(["Verify result: verify-retry-scan (not done)"]), None);
        assert_eq!(outcome([]), None);
    }

    #[test]
    fn every_status_has_a_prompt_line() {
        for s in [Status::Match, Status::NoMatch, Status::RetryScan, Status::SwipeTooShort, Status::FingerNotCentered, Status::RemoveAndRetry, Status::Disconnected, Status::UnknownError] {
            assert!(!s.prompt().is_empty());
        }
    }

    #[test]
    fn the_sensor_feeds_the_biometry_model() {
        let owner = Policy::DeviceOwner.requirement();
        let only = Policy::BiometricsOnly.requirement();
        let dev = device_facts(&parse_list(LISTED), true, 0, 3);
        assert_eq!(dev.biometry, BiometryType::Touch);
        assert_eq!(dev.enrolled_sets.len(), 2);
        assert_eq!(resolve(owner, &dev, None), Ok(Method::Biometry));
        let none = device_facts(&Sensor::default(), true, 0, 3);
        assert_eq!(resolve(owner, &none, None), Ok(Method::Passcode));
        let empty = device_facts(&parse_list("User a has no fingers enrolled for X.\n"), true, 0, 3);
        assert_eq!(resolve(only, &empty, None), Err(Failure::BiometryNotEnrolled));
        let locked = device_facts(&parse_list(LISTED), true, 3, 3);
        assert_eq!(resolve(owner, &locked, None), Ok(Method::Passcode));
    }

    #[test]
    fn user_names_that_could_be_options_are_refused() {
        assert!(valid_user("olibuijr"));
        assert!(valid_user("a.b-c_d"));
        assert!(!valid_user("-x"));
        assert!(!valid_user(""));
        assert!(!valid_user("a b"));
        assert_eq!(probe("--help"), Sensor::default());
        assert_eq!(verify("-x", |_| {}), Status::UnknownError);
    }
}
