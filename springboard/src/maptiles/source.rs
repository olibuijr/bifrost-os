//! The raster tile source: an OpenStreetMap-compatible URL template, the user agent, and request pacing.
//! Follows the OpenStreetMap tile usage policy: an identifying user agent, cached tiles, and a low request rate.

use super::tiles::TileId;
use std::time::Duration;

pub const DEFAULT_TEMPLATE: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
/// The imagery layer of the Satellite map type.
pub const IMAGERY_TEMPLATE: &str = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
/// The place names and boundaries drawn over the imagery in the Hybrid map type (transparent tiles).
pub const LABELS_TEMPLATE: &str =
    "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Boundaries_and_Places/MapServer/tile/{z}/{y}/{x}";

/// The map types of the View menu. Each is a tile source preset with the credit its data requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapType {
    Standard,
    Satellite,
    /// the imagery with the place names drawn over it
    Hybrid,
}

impl MapType {
    pub const ALL: [MapType; 3] = [MapType::Standard, MapType::Satellite, MapType::Hybrid];

    pub fn label(self) -> &'static str {
        match self {
            MapType::Standard => "Map",
            MapType::Satellite => "Satellite",
            MapType::Hybrid => "Hybrid",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            MapType::Standard => "standard",
            MapType::Satellite => "satellite",
            MapType::Hybrid => "hybrid",
        }
    }

    /// An unknown or missing value is the standard map.
    pub fn from_key(key: &str) -> MapType {
        Self::ALL.into_iter().find(|m| m.key() == key).unwrap_or(MapType::Standard)
    }

    pub fn template(self) -> &'static str {
        match self {
            MapType::Standard => DEFAULT_TEMPLATE,
            MapType::Satellite | MapType::Hybrid => IMAGERY_TEMPLATE,
        }
    }

    /// The layer drawn over the base tiles, when the map type has one.
    pub fn labels_template(self) -> Option<&'static str> {
        match self {
            MapType::Hybrid => Some(LABELS_TEMPLATE),
            MapType::Standard | MapType::Satellite => None,
        }
    }

    /// The data credit the tile source asks to be shown in the corner of the map.
    pub fn attribution(self) -> &'static str {
        match self {
            MapType::Standard => "\u{a9} OpenStreetMap contributors",
            MapType::Satellite => "Imagery \u{a9} Esri, Maxar, Earthstar Geographics",
            MapType::Hybrid => "Imagery \u{a9} Esri, Maxar, Earthstar Geographics; labels \u{a9} Esri",
        }
    }

    pub fn source(self, app_version: &str) -> TileSource {
        TileSource::new(self.template(), &user_agent(app_version)).expect("the preset tile templates are valid")
    }

    /// The labels layer of the map type, read from its own cache folder.
    pub fn labels_source(self, app_version: &str) -> Option<TileSource> {
        self.labels_template().map(|t| TileSource::new(t, &user_agent(app_version)).expect("the preset labels template is valid"))
    }
}

/// Labels change rarely, so a cached label tile is kept for a week before it is fetched again.
pub const LABELS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Traffic tiles go stale quickly: a cached traffic tile older than this is fetched again.
pub const TRAFFIC_MAX_AGE: Duration = Duration::from_secs(5 * 60);

/// Pause between tile requests, so one view never bursts at the tile server.
pub const MIN_REQUEST_INTERVAL_MS: u64 = 500;
/// Most tile requests in flight at once.
pub const MAX_IN_FLIGHT: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct TileSource {
    template: String,
    user_agent: String,
}

/// The user agent for tile and geocoder requests. It names the app and version so the server can contact the operator.
pub fn user_agent(app_version: &str) -> String {
    format!("BifrOSt-Maps/{app_version} (BifrOSt desktop; +https://bifrost.olibuijr.com)")
}

impl TileSource {
    /// Checks the template: an http or https URL with `{z}`, `{x}` and `{y}`, no whitespace, and a non-empty user agent.
    pub fn new(template: &str, user_agent: &str) -> Result<Self, String> {
        let t = template.trim();
        if !(t.starts_with("https://") || t.starts_with("http://")) {
            return Err("the tile URL must start with http:// or https://".into());
        }
        if t.chars().any(char::is_whitespace) {
            return Err("the tile URL must not contain whitespace".into());
        }
        for key in ["{z}", "{x}", "{y}"] {
            if t.matches(key).count() != 1 {
                return Err(format!("the tile URL needs exactly one {key}"));
            }
        }
        let ua = user_agent.trim();
        if ua.is_empty() {
            return Err("a tile source needs a user agent".into());
        }
        Ok(Self { template: t.to_string(), user_agent: ua.to_string() })
    }

    pub fn default_source(app_version: &str) -> Self {
        Self::new(DEFAULT_TEMPLATE, &user_agent(app_version)).expect("the default tile template is valid")
    }

    pub fn url(&self, t: TileId) -> String {
        self.template
            .replace("{z}", &t.z.to_string())
            .replace("{x}", &t.x.to_string())
            .replace("{y}", &t.y.to_string())
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The cache folder of this source used as an overlay on the base map. It is kept apart from the base tiles, so
    /// an overlay served from the same host as the map never overwrites them.
    pub fn overlay_cache_key(&self) -> String {
        format!("overlay-{}", self.cache_key())
    }

    pub fn template(&self) -> &str {
        &self.template
    }

    /// A cache key for this source: the host name, so tiles from two servers never share a folder.
    pub fn cache_key(&self) -> String {
        let rest = self.template.split("://").nth(1).unwrap_or("");
        let host = rest.split('/').next().unwrap_or("tiles");
        host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
    }
}

/// Milliseconds to wait before the next tile request, given when the last one was sent.
pub fn delay_before_request(last_sent_ms: Option<u64>, now_ms: u64) -> u64 {
    match last_sent_ms {
        None => 0,
        Some(last) => (last + MIN_REQUEST_INTERVAL_MS).saturating_sub(now_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_template_fills_all_three_placeholders() {
        let src = TileSource::default_source("1.0.0");
        assert_eq!(src.url(TileId { z: 10, x: 449, y: 272 }), "https://tile.openstreetmap.org/10/449/272.png");
        assert!(src.user_agent().starts_with("BifrOSt-Maps/1.0.0 "));
        assert_eq!(src.cache_key(), "tile.openstreetmap.org");
    }

    #[test]
    fn bad_templates_are_refused() {
        assert!(TileSource::new("ftp://x/{z}/{x}/{y}.png", "ua").is_err());
        assert!(TileSource::new("https://x/{z}/{x}.png", "ua").is_err());
        assert!(TileSource::new("https://x/{z}/{x}/{y}/{z}.png", "ua").is_err());
        assert!(TileSource::new("https://x/{z}/{x}/ {y}.png", "ua").is_err());
        assert!(TileSource::new("https://x/{z}/{x}/{y}.png", "  ").is_err());
    }

    #[test]
    fn a_custom_server_gets_its_own_cache_folder() {
        let src = TileSource::new("http://maps.local:8080/t/{z}/{x}/{y}.png", "ua").unwrap();
        assert_eq!(src.cache_key(), "maps.local_8080");
    }

    #[test]
    fn every_map_type_has_a_valid_source_its_own_cache_folder_and_a_credit() {
        let keys: Vec<String> = MapType::ALL.iter().map(|m| m.source("1.0.0").cache_key()).collect();
        assert_eq!(keys, vec!["tile.openstreetmap.org", "server.arcgisonline.com", "server.arcgisonline.com"]);
        for m in MapType::ALL {
            assert_eq!(MapType::from_key(m.key()), m);
            assert!(!m.attribution().is_empty());
        }
        assert_eq!(MapType::from_key("?"), MapType::Standard);
        let url = MapType::Satellite.source("1.0.0").url(TileId { z: 3, x: 4, y: 2 });
        assert!(url.ends_with("/tile/3/2/4"), "{url}");
        assert!(MapType::Standard.attribution().contains("OpenStreetMap"));
    }

    #[test]
    fn an_overlay_never_shares_a_cache_folder_with_the_base_map() {
        let base = TileSource::default_source("1.0.0");
        let traffic = TileSource::new("https://tile.openstreetmap.org/traffic/{z}/{x}/{y}.png", "ua").unwrap();
        assert_eq!(base.cache_key(), traffic.cache_key());
        assert_ne!(base.cache_key(), traffic.overlay_cache_key());
        assert_eq!(traffic.overlay_cache_key(), "overlay-tile.openstreetmap.org");
    }

    #[test]
    fn hybrid_is_the_imagery_with_a_labels_layer_in_its_own_folder() {
        assert_eq!(MapType::Hybrid.template(), MapType::Satellite.template());
        assert_eq!(MapType::Hybrid.label(), "Hybrid");
        assert_eq!(MapType::from_key("hybrid"), MapType::Hybrid);
        assert_eq!(MapType::Standard.labels_template(), None);
        assert_eq!(MapType::Satellite.labels_template(), None);
        assert!(MapType::Satellite.labels_source("1.0.0").is_none());
        let labels = MapType::Hybrid.labels_source("1.0.0").expect("hybrid has labels");
        let url = labels.url(TileId { z: 3, x: 4, y: 2 });
        assert!(url.contains("World_Boundaries_and_Places") && url.ends_with("/tile/3/2/4"), "{url}");
        // the labels share the imagery's host, so only the overlay folder keeps them apart
        let imagery = MapType::Hybrid.source("1.0.0");
        assert_eq!(labels.cache_key(), imagery.cache_key());
        assert_ne!(labels.overlay_cache_key(), imagery.cache_key());
        assert!(MapType::Hybrid.attribution().contains("labels"));
    }

    #[test]
    fn requests_are_paced() {
        assert_eq!(delay_before_request(None, 1000), 0);
        assert_eq!(delay_before_request(Some(1000), 1100), 400);
        assert_eq!(delay_before_request(Some(1000), 2000), 0);
    }
}
