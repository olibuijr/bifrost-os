//! The photo picker for apps (the PhotoKit `PHPicker` surface): `PHPickerConfiguration` with its filters and selection
//! limit, and the `PHPickerResult` list a picker returns. The shell owns the sheet and the library, so an app only
//! names what it wants and reads back the identifiers the person chose.
//!
//! ```ignore
//! let mut config = PHPickerConfiguration::default();
//! config.filters = vec![PHPickerFilter::Images];
//! config.selection_limit = 3;
//! let id = PHPickerViewController::present(ctx, &config);
//! // App::service_reply: let results = PHPickerResult::decode_reply(&reply)?;
//! ```
//!
//! Gaps: the shell does not yet answer the `picker` call; until it does, the reply is an error and apps should fall
//! back to their own picker. Live Photos and video assets are filtered by name only, as the library has no such assets.

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// `PHPickerFilter`: the kinds of asset a picker shows. A configuration keeps a list, and an asset passes when it
/// matches any entry; an empty list shows everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PHPickerFilter {
    Images,
    Videos,
    LivePhotos,
}

impl PHPickerFilter {
    pub fn name(self) -> &'static str {
        match self {
            PHPickerFilter::Images => "images",
            PHPickerFilter::Videos => "videos",
            PHPickerFilter::LivePhotos => "live_photos",
        }
    }
    pub fn from_name(name: &str) -> Option<PHPickerFilter> {
        Some(match name {
            "images" => PHPickerFilter::Images,
            "videos" => PHPickerFilter::Videos,
            "live_photos" => PHPickerFilter::LivePhotos,
            _ => return None,
        })
    }
}

/// `PHPickerConfiguration`. `selection_limit` 0 means no limit, as in PhotoKit; the default is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PHPickerConfiguration {
    pub filters: Vec<PHPickerFilter>,
    pub selection_limit: usize,
}

impl Default for PHPickerConfiguration {
    fn default() -> Self {
        PHPickerConfiguration { filters: Vec::new(), selection_limit: 1 }
    }
}

impl PHPickerConfiguration {
    /// The wire params of a `picker` call.
    pub fn to_params(&self) -> Json {
        Json::Obj(vec![
            ("filters".into(), Json::Arr(self.filters.iter().map(|f| Json::Str(f.name().into())).collect())),
            ("limit".into(), Json::Num(self.selection_limit as f64)),
        ])
    }
}

/// `PHPickerResult`: one chosen asset. `asset_identifier` is `None` when the library gave no identifier, as in PhotoKit
/// for assets outside the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PHPickerResult {
    pub asset_identifier: Option<String>,
}

/// `PHPickerViewController`.
pub struct PHPickerViewController;

impl PHPickerViewController {
    /// Asks the shell to show a picker. The reply decodes with [`PHPickerResult::decode_reply`].
    pub fn present(ctx: &Ctx, config: &PHPickerConfiguration) -> u32 {
        service::call(ctx, crate::photo_library::SERVICE, "picker", &config.to_params())
    }
}

impl PHPickerResult {
    /// The chosen results in a `picker` reply, in the order the person picked them. `None` for a reply of another
    /// shape or a result whose identifier is not a string. A selection above the limit is refused by
    /// [`PHPickerResult::check_limit`].
    pub fn decode_reply(reply: &Json) -> Option<Vec<PHPickerResult>> {
        reply
            .get("results")?
            .as_array()?
            .iter()
            .map(|r| {
                let asset_identifier = match r.get("id") {
                    None | Some(Json::Null) => None,
                    Some(v) => Some(v.as_str()?.to_string()),
                };
                Some(PHPickerResult { asset_identifier })
            })
            .collect()
    }

    /// True when the selection fits `config.selection_limit` (0 means no limit).
    pub fn check_limit(results: &[PHPickerResult], config: &PHPickerConfiguration) -> bool {
        config.selection_limit == 0 || results.len() <= config.selection_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_round_trip_their_names() {
        for f in [PHPickerFilter::Images, PHPickerFilter::Videos, PHPickerFilter::LivePhotos] {
            assert_eq!(PHPickerFilter::from_name(f.name()), Some(f));
        }
        assert_eq!(PHPickerFilter::from_name("holograms"), None);
    }

    #[test]
    fn configuration_becomes_wire_params() {
        let config = PHPickerConfiguration { filters: vec![PHPickerFilter::Images, PHPickerFilter::Videos], selection_limit: 3 };
        let p = config.to_params();
        assert_eq!(p.get("limit").and_then(Json::as_u64), Some(3));
        let names: Vec<&str> = p.get("filters").unwrap().as_array().unwrap().iter().map(|j| j.as_str().unwrap()).collect();
        assert_eq!(names, vec!["images", "videos"]);
        assert_eq!(PHPickerConfiguration::default().selection_limit, 1);
    }

    #[test]
    fn a_reply_decodes_in_pick_order() {
        let reply = Json::Obj(vec![(
            "results".into(),
            Json::Arr(vec![Json::Obj(vec![("id".into(), Json::Str("b".into()))]), Json::Obj(vec![("id".into(), Json::Null)])]),
        )]);
        let results = PHPickerResult::decode_reply(&reply).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].asset_identifier.as_deref(), Some("b"));
        assert_eq!(results[1].asset_identifier, None);
    }

    #[test]
    fn a_malformed_reply_is_refused() {
        assert!(PHPickerResult::decode_reply(&Json::Null).is_none());
        let bad = Json::Obj(vec![("results".into(), Json::Arr(vec![Json::Obj(vec![("id".into(), Json::Num(4.0))])]))]);
        assert!(PHPickerResult::decode_reply(&bad).is_none());
    }

    #[test]
    fn the_selection_limit_is_enforced() {
        let two = vec![PHPickerResult { asset_identifier: None }, PHPickerResult { asset_identifier: None }];
        assert!(!PHPickerResult::check_limit(&two, &PHPickerConfiguration::default()));
        assert!(PHPickerResult::check_limit(&two, &PHPickerConfiguration { filters: vec![], selection_limit: 2 }));
        assert!(PHPickerResult::check_limit(&two, &PHPickerConfiguration { filters: vec![], selection_limit: 0 }));
    }
}
