//! Data attribution for weather (`WeatherAttribution`): the name of the provider whose data is shown, the legal page
//! that carries its copyright notice, and the text an app must show when it cannot open that page. Mirrors the
//! attribution structure of the official weather framework. The shell shows this line wherever provider data appears.

/// Who supplied the weather data, and how to credit them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherAttribution {
    /// The provider name, for example the name of the data service.
    pub service_name: String,
    /// A link to the legal page with the copyright notice, when the provider has one.
    pub legal_page_url: Option<String>,
    /// Attribution text for places that cannot show the legal page. Empty when the provider gives none.
    pub legal_text: String,
}

impl WeatherAttribution {
    /// Builds the attribution. `None` when the provider name is blank. The legal page is kept only when it is an
    /// `https://` address; a plain `http://` or any other scheme is dropped.
    pub fn new(service_name: &str, legal_page_url: Option<&str>, legal_text: &str) -> Option<Self> {
        let service_name = service_name.trim();
        if service_name.is_empty() {
            return None;
        }
        let legal_page_url = legal_page_url
            .map(str::trim)
            .filter(|u| u.starts_with("https://"))
            .map(str::to_string);
        Some(WeatherAttribution {
            service_name: service_name.to_string(),
            legal_page_url,
            legal_text: legal_text.trim().to_string(),
        })
    }

    /// The line the shell prints under weather data: the provider's own legal text when it has one, otherwise a plain
    /// credit line with the provider name.
    pub fn line(&self) -> String {
        if self.legal_text.is_empty() {
            format!("Weather data from {}", self.service_name)
        } else {
            self.legal_text.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_provider_name_is_rejected() {
        assert!(WeatherAttribution::new("   ", None, "text").is_none());
        assert!(WeatherAttribution::new("", Some("https://example.org/legal"), "").is_none());
    }

    #[test]
    fn legal_page_must_be_https() {
        let secure = WeatherAttribution::new("Provider", Some(" https://example.org/legal "), "").unwrap();
        assert_eq!(secure.legal_page_url.as_deref(), Some("https://example.org/legal"));
        let plain = WeatherAttribution::new("Provider", Some("http://example.org/legal"), "").unwrap();
        assert_eq!(plain.legal_page_url, None);
        let script = WeatherAttribution::new("Provider", Some("javascript:void(0)"), "").unwrap();
        assert_eq!(script.legal_page_url, None);
    }

    #[test]
    fn line_falls_back_to_a_credit_when_there_is_no_legal_text() {
        let bare = WeatherAttribution::new(" Provider ", None, "").unwrap();
        assert_eq!(bare.service_name, "Provider");
        assert_eq!(bare.line(), "Weather data from Provider");
    }

    #[test]
    fn line_prefers_the_provider_legal_text() {
        let full = WeatherAttribution::new("Provider", None, " Copyright 2026 Provider. ").unwrap();
        assert_eq!(full.line(), "Copyright 2026 Provider.");
    }

    #[test]
    fn a_legal_page_alone_does_not_replace_the_credit_line() {
        let linked = WeatherAttribution::new("Provider", Some("https://example.org/legal"), "").unwrap();
        assert_eq!(linked.line(), "Weather data from Provider");
        assert_eq!(linked.legal_page_url.as_deref(), Some("https://example.org/legal"));
    }
}
