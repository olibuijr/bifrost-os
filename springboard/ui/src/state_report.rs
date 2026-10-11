//! State reporting: a named domain records one active state at a time. A state is identified by its label and its
//! stable metadata. Volatile metadata is context inside that state and is replaced when the next transition begins.
//! Reports are queued in memory and drained by whoever delivers them; this module does no I/O.

use std::collections::BTreeMap;

/// A dictionary of reportable values, keyed by metadata name.
pub type Dictionary = BTreeMap<String, MetadataValue>;

/// A value in a reportable-metadata dictionary.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataValue {
    /// Seconds since the Unix epoch.
    Date(f64),
    FloatingPoint(f64),
    Integer(i128),
    String(String),
}

impl MetadataValue {
    pub fn date(seconds_since_epoch: f64) -> Self {
        MetadataValue::Date(seconds_since_epoch)
    }
}

impl From<&str> for MetadataValue {
    fn from(v: &str) -> Self {
        MetadataValue::String(v.to_string())
    }
}

impl From<String> for MetadataValue {
    fn from(v: String) -> Self {
        MetadataValue::String(v)
    }
}

impl From<f32> for MetadataValue {
    fn from(v: f32) -> Self {
        MetadataValue::FloatingPoint(v as f64)
    }
}

impl From<f64> for MetadataValue {
    fn from(v: f64) -> Self {
        MetadataValue::FloatingPoint(v)
    }
}

macro_rules! integer_values {
    ($($t:ty)*) => {$(
        impl From<$t> for MetadataValue {
            fn from(v: $t) -> Self {
                MetadataValue::Integer(v as i128)
            }
        }
    )*};
}
integer_values!(i8 i16 i32 i64 i128 isize u8 u16 u32 u64 usize);

/// Metadata that describes itself as a dictionary of reportable values.
pub trait ReportableMetadata {
    fn metadata_dictionary(&self) -> Dictionary;
}

fn dictionary_of(metadata: Option<&dyn ReportableMetadata>) -> Dictionary {
    metadata.map(|m| m.metadata_dictionary()).unwrap_or_default()
}

/// A domain name in reverse-DNS form: at least two labels, each non-empty, at most 63 bytes, made of ASCII
/// letters, digits or hyphens, and not starting or ending with a hyphen.
pub fn valid_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// One record in the outgoing queue.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    /// A new state began. `label` is None when the feature reports that no state is active.
    Transition { domain: String, label: Option<String>, stable: Dictionary, volatile: Dictionary },
    /// Volatile metadata changed inside the current state without a new transition.
    VolatileUpdate { domain: String, volatile: Dictionary },
}

/// The reporter of one domain. Create one per domain and keep it for the life of the feature.
#[derive(Debug)]
pub struct StateReporter {
    domain: String,
    label: Option<String>,
    stable: Dictionary,
    volatile: Dictionary,
    pending: Vec<Report>,
}

impl StateReporter {
    /// Returns None when the domain is not a valid reverse-DNS name.
    pub fn new(domain: &str) -> Option<Self> {
        if !valid_domain(domain) {
            return None;
        }
        Some(StateReporter {
            domain: domain.to_string(),
            label: None,
            stable: Dictionary::new(),
            volatile: Dictionary::new(),
            pending: Vec::new(),
        })
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn current_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn current_volatile(&self) -> &Dictionary {
        &self.volatile
    }

    /// Reports a transition to `to`. The same label and stable metadata as the current state is a no-op and
    /// returns false; otherwise the new state is queued and becomes current, with its own volatile metadata.
    pub fn report_transition(
        &mut self,
        to: Option<&str>,
        stable: Option<&dyn ReportableMetadata>,
        volatile: Option<&dyn ReportableMetadata>,
    ) -> bool {
        let label = to.map(str::to_string);
        let stable = dictionary_of(stable);
        if label == self.label && stable == self.stable {
            return false;
        }
        let volatile = dictionary_of(volatile);
        self.label = label.clone();
        self.stable = stable.clone();
        self.volatile = volatile.clone();
        self.pending.push(Report::Transition { domain: self.domain.clone(), label, stable, volatile });
        true
    }

    /// Replaces the volatile metadata of the current state. This never starts a transition.
    pub fn report_volatile_update(&mut self, volatile: Option<&dyn ReportableMetadata>) {
        let volatile = dictionary_of(volatile);
        self.volatile = volatile.clone();
        self.pending.push(Report::VolatileUpdate { domain: self.domain.clone(), volatile });
    }

    /// Takes every queued report, oldest first.
    pub fn drain(&mut self) -> Vec<Report> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Media {
        title: String,
        seconds: f64,
    }

    impl ReportableMetadata for Media {
        fn metadata_dictionary(&self) -> Dictionary {
            let mut d = Dictionary::new();
            d.insert("title".into(), self.title.as_str().into());
            d.insert("seconds".into(), self.seconds.into());
            d
        }
    }

    fn media(title: &str, seconds: f64) -> Media {
        Media { title: title.into(), seconds }
    }

    #[test]
    fn transition_only_when_label_or_stable_metadata_changes() {
        let mut r = StateReporter::new("is.olibuijr.player").unwrap();
        let a = media("a", 1.0);
        assert!(r.report_transition(Some("playing"), Some(&a), None));
        assert!(!r.report_transition(Some("playing"), Some(&a), None));
        let b = media("b", 1.0);
        assert!(r.report_transition(Some("playing"), Some(&b), None));
        assert!(r.report_transition(Some("paused"), Some(&b), None));
        assert_eq!(r.drain().len(), 3);
    }

    #[test]
    fn volatile_update_is_queued_without_a_transition() {
        let mut r = StateReporter::new("is.olibuijr.player").unwrap();
        let a = media("a", 1.0);
        r.report_transition(Some("playing"), Some(&a), None);
        let v = media("v", 3.0);
        r.report_volatile_update(Some(&v));
        assert_eq!(r.current_volatile().get("seconds"), Some(&MetadataValue::FloatingPoint(3.0)));
        // the state is unchanged, so the same transition is still a no-op
        assert!(!r.report_transition(Some("playing"), Some(&a), None));
        let reports = r.drain();
        assert_eq!(reports.len(), 2);
        assert!(matches!(reports[1], Report::VolatileUpdate { .. }));
    }

    #[test]
    fn a_new_transition_replaces_volatile_metadata() {
        let mut r = StateReporter::new("is.olibuijr.player").unwrap();
        let a = media("a", 1.0);
        r.report_transition(Some("playing"), Some(&a), None);
        r.report_volatile_update(Some(&media("v", 9.0)));
        assert!(!r.current_volatile().is_empty());
        assert!(r.report_transition(Some("paused"), Some(&a), None));
        assert!(r.current_volatile().is_empty());
    }

    #[test]
    fn transition_to_no_state_is_reported() {
        let mut r = StateReporter::new("is.olibuijr.player").unwrap();
        r.report_transition(Some("playing"), None, None);
        assert!(r.report_transition(None, None, None));
        assert_eq!(r.current_label(), None);
        assert!(!r.report_transition(None, None, None));
    }

    #[test]
    fn rejects_domains_that_are_not_reverse_dns() {
        assert!(StateReporter::new("nodots").is_none());
        assert!(StateReporter::new("a..b").is_none());
        assert!(StateReporter::new("-a.b").is_none());
        assert!(StateReporter::new("a_b.c").is_none());
        assert!(StateReporter::new("is.olibuijr.app").is_some());
    }

    #[test]
    fn metadata_values_convert_from_plain_types() {
        assert_eq!(MetadataValue::from(42u8), MetadataValue::Integer(42));
        assert_eq!(MetadataValue::from(-3i32), MetadataValue::Integer(-3));
        assert_eq!(MetadataValue::from(1.5f32), MetadataValue::FloatingPoint(1.5));
        assert_eq!(MetadataValue::from("x"), MetadataValue::String("x".into()));
        assert_eq!(MetadataValue::date(10.0), MetadataValue::Date(10.0));
    }
}
