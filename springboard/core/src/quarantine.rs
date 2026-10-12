//! Quarantine record: where a downloaded or received item came from, kept with the item (std only).
//!
//! When a file arrives from the web, a mail attachment or another app, the agent that received it writes a record
//! with the kind of transfer, the agent's bundle id and name, the time, the item's own address and the page it came
//! from. The shell reads that record before it opens the item, so it can ask the user first. This module only builds,
//! validates, reads and writes the record as `key=value` lines. It attaches nothing to a file and checks no signature.
use std::fmt;

/// The reason an item was quarantined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuarantineKind {
    WebDownload,
    OtherDownload,
    EmailAttachment,
    MessageAttachment,
    CalendarAttachment,
    OtherAttachment,
}

impl QuarantineKind {
    /// The word stored in the record.
    pub fn label(self) -> &'static str {
        match self {
            QuarantineKind::WebDownload => "web-download",
            QuarantineKind::OtherDownload => "other-download",
            QuarantineKind::EmailAttachment => "email-attachment",
            QuarantineKind::MessageAttachment => "message-attachment",
            QuarantineKind::CalendarAttachment => "calendar-attachment",
            QuarantineKind::OtherAttachment => "other-attachment",
        }
    }

    pub fn parse(s: &str) -> Option<QuarantineKind> {
        match s.trim() {
            "web-download" => Some(QuarantineKind::WebDownload),
            "other-download" => Some(QuarantineKind::OtherDownload),
            "email-attachment" => Some(QuarantineKind::EmailAttachment),
            "message-attachment" => Some(QuarantineKind::MessageAttachment),
            "calendar-attachment" => Some(QuarantineKind::CalendarAttachment),
            "other-attachment" => Some(QuarantineKind::OtherAttachment),
            _ => None,
        }
    }
}

/// Why a record could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuarantineError {
    /// A required key is missing: `kind`, `agent`, `agent_name` or `time`.
    Missing(&'static str),
    /// A key has a value it cannot take: an unknown kind, or a time that is not a whole number of seconds.
    Invalid(&'static str),
    /// A value holds a line break, which the one-key-per-line form cannot hold.
    LineBreak,
}

impl fmt::Display for QuarantineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuarantineError::Missing(k) => write!(f, "quarantine record has no {k}"),
            QuarantineError::Invalid(k) => write!(f, "quarantine record has a bad {k}"),
            QuarantineError::LineBreak => write!(f, "quarantine value holds a line break"),
        }
    }
}

/// One quarantine record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quarantine {
    pub kind: QuarantineKind,
    /// The bundle identifier of the agent that quarantined the item.
    pub agent: String,
    /// The name of that agent, for the message shown to the user.
    pub agent_name: String,
    /// When the item was quarantined, in seconds since the Unix epoch.
    pub time: u64,
    /// The address the item was fetched from, if any.
    pub data_url: Option<String>,
    /// The page or sender the item came from, if any.
    pub origin_url: Option<String>,
}

impl Quarantine {
    pub fn new(kind: QuarantineKind, agent: &str, agent_name: &str, time: u64) -> Self {
        Quarantine { kind, agent: agent.into(), agent_name: agent_name.into(), time, data_url: None, origin_url: None }
    }

    /// True when the item came from the web (a page download), which the shell treats with the most caution.
    pub fn is_web_download(&self) -> bool {
        self.kind == QuarantineKind::WebDownload
    }

    /// The record as `key=value` lines. Optional addresses are written only when set.
    pub fn to_lines(&self) -> Result<String, QuarantineError> {
        let mut out = String::new();
        let fields = [
            ("kind", Some(self.kind.label().to_string())),
            ("agent", Some(self.agent.clone())),
            ("agent_name", Some(self.agent_name.clone())),
            ("time", Some(self.time.to_string())),
            ("data_url", self.data_url.clone()),
            ("origin_url", self.origin_url.clone()),
        ];
        for (key, value) in fields {
            if let Some(v) = value {
                if v.contains('\n') || v.contains('\r') {
                    return Err(QuarantineError::LineBreak);
                }
                out.push_str(&format!("{key}={v}\n"));
            }
        }
        Ok(out)
    }

    /// Reads `to_lines` output. Unknown keys are ignored, and a value may contain `=`.
    pub fn parse(text: &str) -> Result<Quarantine, QuarantineError> {
        let mut kind = None;
        let mut agent = None;
        let mut agent_name = None;
        let mut time = None;
        let mut data_url = None;
        let mut origin_url = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "kind" => kind = Some(QuarantineKind::parse(value).ok_or(QuarantineError::Invalid("kind"))?),
                "agent" => agent = Some(value.to_string()),
                "agent_name" => agent_name = Some(value.to_string()),
                "time" => time = Some(value.trim().parse::<u64>().map_err(|_| QuarantineError::Invalid("time"))?),
                "data_url" => data_url = Some(value.to_string()),
                "origin_url" => origin_url = Some(value.to_string()),
                _ => {}
            }
        }
        Ok(Quarantine {
            kind: kind.ok_or(QuarantineError::Missing("kind"))?,
            agent: agent.ok_or(QuarantineError::Missing("agent"))?,
            agent_name: agent_name.ok_or(QuarantineError::Missing("agent_name"))?,
            time: time.ok_or(QuarantineError::Missing("time"))?,
            data_url,
            origin_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Quarantine {
        let mut q = Quarantine::new(QuarantineKind::WebDownload, "org.example.browser", "Example Browser", 1_700_000_000);
        q.data_url = Some("https://files.example.com/a.zip?x=1".into());
        q.origin_url = Some("https://example.com/downloads".into());
        q
    }

    #[test]
    fn a_record_round_trips_with_its_optional_addresses() {
        let q = sample();
        let text = q.to_lines().unwrap();
        assert_eq!(
            text,
            "kind=web-download\nagent=org.example.browser\nagent_name=Example Browser\ntime=1700000000\n\
             data_url=https://files.example.com/a.zip?x=1\norigin_url=https://example.com/downloads\n"
        );
        assert_eq!(Quarantine::parse(&text), Ok(q));
    }

    #[test]
    fn optional_addresses_are_left_out_when_unset() {
        let q = Quarantine::new(QuarantineKind::EmailAttachment, "org.example.mail", "Mail", 42);
        assert_eq!(q.to_lines().unwrap(), "kind=email-attachment\nagent=org.example.mail\nagent_name=Mail\ntime=42\n");
        let back = Quarantine::parse("kind=email-attachment\nagent=org.example.mail\nagent_name=Mail\ntime=42\n").unwrap();
        assert_eq!(back.data_url, None);
        assert!(!back.is_web_download());
    }

    #[test]
    fn a_missing_key_is_named_in_the_error() {
        assert_eq!(Quarantine::parse("agent=a\nagent_name=b\ntime=1\n"), Err(QuarantineError::Missing("kind")));
        assert_eq!(Quarantine::parse("kind=web-download\nagent_name=b\ntime=1\n"), Err(QuarantineError::Missing("agent")));
        assert_eq!(Quarantine::parse("kind=web-download\nagent=a\nagent_name=b\n"), Err(QuarantineError::Missing("time")));
    }

    #[test]
    fn an_unknown_kind_or_a_bad_time_is_refused() {
        assert_eq!(
            Quarantine::parse("kind=sms\nagent=a\nagent_name=b\ntime=1\n"),
            Err(QuarantineError::Invalid("kind"))
        );
        assert_eq!(
            Quarantine::parse("kind=web-download\nagent=a\nagent_name=b\ntime=-5\n"),
            Err(QuarantineError::Invalid("time"))
        );
        assert_eq!(
            Quarantine::parse("kind=web-download\nagent=a\nagent_name=b\ntime=1.5\n"),
            Err(QuarantineError::Invalid("time"))
        );
    }

    #[test]
    fn a_value_with_a_line_break_cannot_be_written() {
        let mut q = sample();
        q.agent_name = "Two\nLines".into();
        assert_eq!(q.to_lines(), Err(QuarantineError::LineBreak));
        assert_eq!(QuarantineError::LineBreak.to_string(), "quarantine value holds a line break");
    }

    #[test]
    fn every_kind_has_a_label_that_parses_back() {
        let kinds = [
            QuarantineKind::WebDownload,
            QuarantineKind::OtherDownload,
            QuarantineKind::EmailAttachment,
            QuarantineKind::MessageAttachment,
            QuarantineKind::CalendarAttachment,
            QuarantineKind::OtherAttachment,
        ];
        for k in kinds {
            assert_eq!(QuarantineKind::parse(k.label()), Some(k));
        }
        assert!(sample().is_web_download());
    }
}
