//! Weather alerts (`WeatherAlert`): warnings issued for a place by a governmental authority, with their severity
//! (`WeatherSeverity`), the data availability for a location (`WeatherAvailability`) and the ordering the shell uses to
//! show the most serious warning first. These mirror the alert structures of the official weather framework. Alerts are
//! served raw by their source, so a missing region or details link is normal and is kept as `None`, not invented.

/// How serious a weather event is. Ordered from least to most serious. `Unknown` sorts below `Minor`, because a
/// warning with no known threat level should not be shown above one that is known to be minor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Unknown,
    Minor,
    Moderate,
    Severe,
    Extreme,
}

impl Severity {
    /// The threat level in words, as the official framework gives it.
    pub fn description(self) -> &'static str {
        match self {
            Severity::Minor => "Minimal or no known threat",
            Severity::Moderate => "Possible threat",
            Severity::Severe => "Significant threat",
            Severity::Extreme => "Extraordinary threat",
            Severity::Unknown => "Unknown threat",
        }
    }

    /// True for `Severe` and `Extreme`, the levels the shell shows with a strong treatment. Not every alert is severe.
    pub fn is_significant(self) -> bool {
        self >= Severity::Severe
    }
}

/// A warning for a place. `summary` is the event type; `source` names the issuing authority.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherAlert {
    pub summary: String,
    pub region: Option<String>,
    pub severity: Severity,
    pub source: String,
    pub details_url: Option<String>,
}

impl WeatherAlert {
    /// Builds an alert. Blank text becomes `None` for the region, and a details link is kept only when it is an
    /// `http://` or `https://` address, so the shell never opens a link it cannot trust.
    pub fn new(
        summary: &str,
        region: Option<&str>,
        severity: Severity,
        source: &str,
        details_url: Option<&str>,
    ) -> Self {
        let region = region.map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
        let details_url = details_url
            .map(str::trim)
            .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            .map(str::to_string);
        WeatherAlert {
            summary: summary.trim().to_string(),
            region,
            severity,
            source: source.trim().to_string(),
            details_url,
        }
    }
}

/// Orders alerts for display: the most severe first, and within one severity by summary text, ignoring case.
pub fn sorted(mut alerts: Vec<WeatherAlert>) -> Vec<WeatherAlert> {
    alerts.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.summary.to_lowercase().cmp(&b.summary.to_lowercase()))
    });
    alerts
}

/// The most serious alert, or `None` when there are none. On a tie the first alert in the slice wins.
pub fn most_severe(alerts: &[WeatherAlert]) -> Option<&WeatherAlert> {
    alerts.iter().fold(None, |best: Option<&WeatherAlert>, a| match best {
        Some(b) if b.severity >= a.severity => Some(b),
        _ => Some(a),
    })
}

/// Whether one kind of data is served for a location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable,
}

/// The data a location has: whether alerts and the minute forecast are served there (`WeatherAvailability`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherAvailability {
    pub alerts: Availability,
    pub minute: Availability,
}

impl WeatherAvailability {
    /// Builds the availability from two flags.
    pub fn new(alerts: bool, minute: bool) -> Self {
        let kind = |on: bool| if on { Availability::Available } else { Availability::Unavailable };
        WeatherAvailability { alerts: kind(alerts), minute: kind(minute) }
    }

    /// True when the minute forecast can be shown for this location.
    pub fn has_minute_forecast(&self) -> bool {
        self.minute == Availability::Available
    }

    /// True when alerts can be fetched for this location. Without it the shell says so instead of showing "no alerts".
    pub fn has_alerts(&self) -> bool {
        self.alerts == Availability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(summary: &str, severity: Severity) -> WeatherAlert {
        WeatherAlert::new(summary, None, severity, "Met office", None)
    }

    #[test]
    fn severity_orders_from_unknown_to_extreme() {
        assert!(Severity::Unknown < Severity::Minor);
        assert!(Severity::Moderate < Severity::Severe && Severity::Severe < Severity::Extreme);
        assert!(Severity::Extreme.is_significant() && !Severity::Moderate.is_significant());
        assert_eq!(Severity::Severe.description(), "Significant threat");
    }

    #[test]
    fn alert_keeps_only_safe_links_and_drops_blank_regions() {
        let a = WeatherAlert::new(" Gale ", Some("  "), Severity::Severe, " Authority ", Some("javascript:alert(1)"));
        assert_eq!(a.summary, "Gale");
        assert_eq!(a.region, None);
        assert_eq!(a.source, "Authority");
        assert_eq!(a.details_url, None);
        let b = WeatherAlert::new("Gale", Some(" Reykjanes "), Severity::Minor, "Authority", Some(" https://example.org/a "));
        assert_eq!(b.region.as_deref(), Some("Reykjanes"));
        assert_eq!(b.details_url.as_deref(), Some("https://example.org/a"));
    }

    #[test]
    fn sorting_puts_the_most_severe_first_then_by_name() {
        let sorted = sorted(vec![
            alert("wind", Severity::Moderate),
            alert("Ice", Severity::Extreme),
            alert("avalanche", Severity::Moderate),
            alert("unknown event", Severity::Unknown),
        ]);
        let names: Vec<&str> = sorted.iter().map(|a| a.summary.as_str()).collect();
        assert_eq!(names, ["Ice", "avalanche", "wind", "unknown event"]);
    }

    #[test]
    fn most_severe_picks_the_top_alert_and_keeps_the_first_on_a_tie() {
        assert!(most_severe(&[]).is_none());
        let list = vec![alert("first", Severity::Severe), alert("second", Severity::Severe), alert("low", Severity::Minor)];
        assert_eq!(most_severe(&list).unwrap().summary, "first");
        let list = vec![alert("low", Severity::Minor), alert("high", Severity::Extreme)];
        assert_eq!(most_severe(&list).unwrap().summary, "high");
    }

    #[test]
    fn availability_reports_each_data_kind() {
        let on = WeatherAvailability::new(true, false);
        assert!(on.has_alerts());
        assert!(!on.has_minute_forecast());
        assert_eq!(WeatherAvailability::new(false, true).minute, Availability::Available);
    }
}
