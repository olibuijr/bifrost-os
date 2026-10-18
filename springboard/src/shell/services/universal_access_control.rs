//! The `universal_access_control` service: a per-app Universal Access policy. An app can be denied some accessibility
//! features (for example Zoom or Screen Reader) without changing them for the whole system. The policy is stored in the
//! `a11y_app_denials` pref as `app=feature,feature;app=feature`. An app with no entry may use every feature, which is
//! the default. Only the global settings (`accessibility::AccessibilitySettings`) decide whether a feature is on at all.
//!
//! Methods: `allowed({"feature": NAME})` returns `true` or `false` for the calling app; `denied()` returns the array of
//! feature names denied to the calling app. Feature names are `zoom`, `screen_reader`, `keyboard` and `pointer`. An unknown
//! feature name is an error. The service has no event: the policy is read on each call.
use super::{Shell, SystemService};
use crate::shell::prefs::Prefs;
use bifrost_core::json::Json;
use std::collections::BTreeMap;

/// The service name apps call it by.
pub const SERVICE: &str = "universal_access_control";
/// The method that answers whether the calling app may use a feature.
pub const METHOD_ALLOWED: &str = "allowed";
/// The method that lists the features denied to the calling app.
pub const METHOD_DENIED: &str = "denied";
/// The pref that holds the policy.
pub const PREF: &str = "a11y_app_denials";

/// The accessibility features an app can be denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    /// Display Zoom.
    Zoom,
    /// Screen Reader speech and the keyboard cursor.
    ScreenReader,
    /// Sticky Keys, Slow Keys and Mouse Keys.
    Keyboard,
    /// Dwell Control and Cursor Size.
    Pointer,
}

impl Feature {
    pub const ALL: [Feature; 4] = [Feature::Zoom, Feature::ScreenReader, Feature::Keyboard, Feature::Pointer];

    /// The name apps and the pref use.
    pub fn name(self) -> &'static str {
        match self {
            Feature::Zoom => "zoom",
            Feature::ScreenReader => "screen_reader",
            Feature::Keyboard => "keyboard",
            Feature::Pointer => "pointer",
        }
    }

    pub fn parse(name: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// The denied features of each app. Apps without an entry may use everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    denied: BTreeMap<String, Vec<Feature>>,
}

/// An app id is usable in the pref when it is not empty and has no separator characters.
fn app_id_ok(app: &str) -> bool {
    !app.is_empty() && !app.chars().any(|c| c.is_whitespace() || matches!(c, '=' | ',' | ';'))
}

impl Policy {
    /// Parses the pref. Entries with a bad app id or no known feature are dropped, so a damaged pref never blocks everything.
    pub fn parse(s: &str) -> Policy {
        let mut denied: BTreeMap<String, Vec<Feature>> = BTreeMap::new();
        for entry in s.split(';') {
            let Some((app, list)) = entry.split_once('=') else { continue };
            let app = app.trim();
            if !app_id_ok(app) {
                continue;
            }
            let mut features: Vec<Feature> = list.split(',').filter_map(|n| Feature::parse(n.trim())).collect();
            features.sort();
            features.dedup();
            if !features.is_empty() {
                denied.entry(app.to_string()).or_default().extend(features);
            }
        }
        for list in denied.values_mut() {
            list.sort();
            list.dedup();
        }
        Policy { denied }
    }

    /// The pref text for this policy: entries in app-id order, features in declaration order.
    pub fn encode(&self) -> String {
        self.denied
            .iter()
            .filter(|(_, list)| !list.is_empty())
            .map(|(app, list)| format!("{app}={}", list.iter().map(|f| f.name()).collect::<Vec<_>>().join(",")))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Reads the policy from the prefs.
    pub fn load(p: &Prefs) -> Policy {
        Policy::parse(p.get(PREF).unwrap_or(""))
    }

    /// Whether `app` may use `feature`.
    pub fn allows(&self, app: &str, feature: Feature) -> bool {
        self.denied.get(app).map_or(true, |list| !list.contains(&feature))
    }

    /// Denies `feature` to `app`. Ignored when the app id is not usable in the pref.
    pub fn deny(&mut self, app: &str, feature: Feature) {
        if !app_id_ok(app) {
            return;
        }
        let list = self.denied.entry(app.to_string()).or_default();
        if !list.contains(&feature) {
            list.push(feature);
            list.sort();
        }
    }

    /// Allows `feature` again for `app`.
    pub fn allow(&mut self, app: &str, feature: Feature) {
        if let Some(list) = self.denied.get_mut(app) {
            list.retain(|f| *f != feature);
            if list.is_empty() {
                self.denied.remove(app);
            }
        }
    }

    /// The features denied to `app`, in declaration order.
    pub fn denied_for(&self, app: &str) -> Vec<Feature> {
        self.denied.get(app).cloned().unwrap_or_default()
    }
}

/// Answers one service call for the app `app_id` against `policy`.
pub fn answer(policy: &Policy, app_id: &str, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        METHOD_ALLOWED => {
            let name = params.get("feature").and_then(Json::as_str).ok_or("allowed needs a feature name")?;
            let feature = Feature::parse(name).ok_or_else(|| format!("unknown feature {name}"))?;
            Ok(Json::Bool(policy.allows(app_id, feature)))
        }
        METHOD_DENIED => Ok(Json::Arr(policy.denied_for(app_id).into_iter().map(|f| Json::Str(f.name().to_string())).collect())),
        _ => Err(format!("universal_access_control has no method {method}")),
    }
}

pub struct UniversalAccessControl;

impl SystemService for UniversalAccessControl {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(&Policy::load(&shell.prefs), app_id, method, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(name: &str) -> Json {
        Json::Obj(vec![("feature".to_string(), Json::Str(name.to_string()))])
    }

    #[test]
    fn every_app_may_use_everything_by_default() {
        let p = Policy::parse("");
        for f in Feature::ALL {
            assert!(p.allows("com.example.notes", f));
        }
        assert!(p.denied_for("com.example.notes").is_empty());
    }

    #[test]
    fn a_pref_round_trips_in_a_stable_order() {
        let p = Policy::parse("com.b.app=pointer,zoom;com.a.app=keyboard");
        assert_eq!(p.encode(), "com.a.app=keyboard;com.b.app=zoom,pointer");
        assert_eq!(Policy::parse(&p.encode()), p);
    }

    #[test]
    fn a_damaged_entry_is_dropped_and_the_rest_kept() {
        let p = Policy::parse("bad app=zoom;com.ok=nonsense,screen_reader;=zoom;com.empty=");
        assert!(!p.allows("com.ok", Feature::ScreenReader));
        assert!(p.allows("com.ok", Feature::Zoom));
        assert!(p.denied_for("com.empty").is_empty());
        assert_eq!(p.encode(), "com.ok=screen_reader");
    }

    #[test]
    fn deny_and_allow_change_one_app_only() {
        let mut p = Policy::default();
        p.deny("com.a.app", Feature::Zoom);
        p.deny("com.a.app", Feature::Zoom);
        assert!(!p.allows("com.a.app", Feature::Zoom));
        assert!(p.allows("com.b.app", Feature::Zoom));
        assert_eq!(p.denied_for("com.a.app"), vec![Feature::Zoom]);
        p.allow("com.a.app", Feature::Zoom);
        assert!(p.allows("com.a.app", Feature::Zoom));
        assert_eq!(p.encode(), "");
    }

    #[test]
    fn an_app_id_with_separators_is_refused() {
        let mut p = Policy::default();
        p.deny("com.a;com.b", Feature::Zoom);
        p.deny("", Feature::Zoom);
        assert_eq!(p.encode(), "");
    }

    #[test]
    fn allowed_answers_for_the_calling_app() {
        let p = Policy::parse("com.a.app=screen_reader");
        assert_eq!(answer(&p, "com.a.app", METHOD_ALLOWED, &obj("screen_reader")), Ok(Json::Bool(false)));
        assert_eq!(answer(&p, "com.a.app", METHOD_ALLOWED, &obj("zoom")), Ok(Json::Bool(true)));
        assert_eq!(answer(&p, "com.b.app", METHOD_ALLOWED, &obj("screen_reader")), Ok(Json::Bool(true)));
    }

    #[test]
    fn allowed_rejects_a_missing_or_unknown_feature() {
        let p = Policy::default();
        assert!(answer(&p, "com.a.app", METHOD_ALLOWED, &Json::Null).is_err());
        assert!(answer(&p, "com.a.app", METHOD_ALLOWED, &obj("telepathy")).is_err());
    }

    #[test]
    fn denied_lists_the_calling_apps_features() {
        let p = Policy::parse("com.a.app=pointer,zoom");
        assert_eq!(answer(&p, "com.a.app", METHOD_DENIED, &Json::Null), Ok(Json::Arr(vec![Json::Str("zoom".into()), Json::Str("pointer".into())])));
        assert_eq!(answer(&p, "com.b.app", METHOD_DENIED, &Json::Null), Ok(Json::Arr(vec![])));
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(UniversalAccessControl.name(), "universal_access_control");
        assert_eq!(SERVICE, "universal_access_control");
        assert_eq!(PREF, "a11y_app_denials");
    }
}
