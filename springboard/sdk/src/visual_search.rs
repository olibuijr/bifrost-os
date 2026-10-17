//! Visual search for apps: an app answers "what in my content matches this scene?" for a scene the user captured
//! with the camera or selected in a screenshot.
//!
//! The model is `bifrost_ui::visual_search` (the scene, the object selection, the label score and the merge of
//! matches). An app implements [`VisualSearchProvider`]; the shell calls [`search`] with the scene and draws the
//! merged list. Each provider's matches are stamped with the provider's own app id, so an app cannot answer for
//! another one.
//!
//! The wire path is an App Intent: a provider app declares [`intent`] and answers it with [`perform`], so whoever holds
//! a scene (the shell, another app, the control socket) asks it over `intent.visual-search` with the scene kind and
//! labels. The reply is a list of matches ([`matches_to_json`]), which the caller reads with [`matches_from_json`] and
//! merges across providers with [`collect`]. Pixels do not cross the intent socket: a provider that matches on pixels is
//! handed them by the app that holds the scene. The camera and screenshot capture, and the classifier that fills the
//! labels, belong to the system side and are not built yet; see docs/parity/visual-intelligence.md.

use crate::control::{RpcError, INVALID_PARAMS};
use crate::intents::{Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;

pub use bifrost_ui::visual_search::{label_score, merge, Match, PixelRect, Scene, SceneError, SceneKind, MAX_MATCHES, MAX_SIDE};

/// The id of the intent a provider app declares.
pub const INTENT_ID: &str = "visual-search";
/// The longest label list one request carries, in labels.
pub const MAX_LABELS: usize = 32;

/// An app that answers visual search requests with its matching content.
pub trait VisualSearchProvider {
    /// The app's bundle identifier. It is stamped on every match this provider returns.
    fn app_id(&self) -> &str;
    /// The matches for `scene`, in any order. The shell ranks them.
    fn search(&self, scene: &Scene) -> Vec<Match>;
}

/// Asks every provider about `scene` and returns the merged matches, at most `limit` of them.
pub fn search(providers: &[&dyn VisualSearchProvider], scene: &Scene, limit: usize) -> Vec<Match> {
    let mut all = Vec::new();
    for p in providers {
        for mut m in p.search(scene) {
            m.app = p.app_id().to_string();
            all.push(m);
        }
    }
    merge(all, limit)
}

/// The intent a provider app declares (add it to `App::intents`): `kind` (`screenshot`, `photo` or `stream`, `photo` when
/// missing) and `labels` (the scene labels, comma separated).
pub fn intent() -> Intent {
    Intent::new(INTENT_ID, "Visual Search", "Finds content in this app that matches a captured scene.")
        .param(Param::new("kind", "Scene", ParamType::Enum(["screenshot", "photo", "stream"].iter().map(|k| k.to_string()).collect())).optional())
        .param(Param::new("labels", "Labels", ParamType::String).optional())
}

/// The scene a request describes: its kind and labels, over a single transparent pixel (the intent carries no pixels).
pub fn scene_from_params(params: &Json) -> Result<Scene, RpcError> {
    let kind = match params.get("kind").and_then(Json::as_str) {
        None | Some("photo") => SceneKind::Photo,
        Some("screenshot") => SceneKind::Screenshot,
        Some("stream") => SceneKind::Stream,
        Some(other) => return Err(RpcError::new(INVALID_PARAMS, format!("scene kind {other} is not screenshot, photo or stream"))),
    };
    let labels: Vec<&str> = params.get("labels").and_then(Json::as_str).map(|l| l.split(',').collect()).unwrap_or_default();
    if labels.len() > MAX_LABELS {
        return Err(RpcError::new(INVALID_PARAMS, format!("at most {MAX_LABELS} labels")));
    }
    let scene = Scene::new(kind, 1, 1, vec![0, 0, 0, 0]).map_err(|e| RpcError::new(INVALID_PARAMS, format!("scene: {e:?}")))?;
    Ok(scene.with_labels(&labels))
}

/// Answers a `visual-search` request with `provider`'s matches, stamped with its app id and ranked (at most
/// [`MAX_MATCHES`]). The result's value is the list [`matches_to_json`] writes.
pub fn perform(provider: &dyn VisualSearchProvider, params: &Json) -> Result<IntentResult, RpcError> {
    let scene = scene_from_params(params)?;
    let found = search(&[provider], &scene, MAX_MATCHES);
    let dialog = match found.len() {
        0 => "No matches".to_string(),
        1 => "1 match".to_string(),
        n => format!("{n} matches"),
    };
    Ok(IntentResult::value(matches_to_json(&found)).with_dialog(&dialog))
}

/// The matches as the intent's value: an array of `{app, intent, title, subtitle, score}`.
pub fn matches_to_json(matches: &[Match]) -> Json {
    Json::Arr(
        matches
            .iter()
            .map(|m| {
                Json::Obj(vec![
                    ("app".into(), Json::Str(m.app.clone())),
                    ("intent".into(), Json::Str(m.intent.clone())),
                    ("title".into(), Json::Str(m.title.clone())),
                    ("subtitle".into(), Json::Str(m.subtitle.clone())),
                    ("score".into(), Json::Num(m.score as f64)),
                ])
            })
            .collect(),
    )
}

/// The matches in an intent value. A malformed entry (not an object, or without a title) is skipped; a score is clamped
/// to 0..=100.
pub fn matches_from_json(value: &Json) -> Vec<Match> {
    let Some(items) = value.as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(Json::as_str)?.to_string();
            let text = |key: &str| item.get(key).and_then(Json::as_str).unwrap_or("").to_string();
            let score = item.get("score").and_then(Json::number).filter(|n| n.is_finite()).map_or(0, |n| n.clamp(0.0, 100.0) as u32);
            Some(Match { app: text("app"), intent: text("intent"), title, subtitle: text("subtitle"), score })
        })
        .collect()
}

/// Merges the replies of several providers: each reply is the answering app's id and its intent value. Every match is
/// stamped with the app that sent it (an app cannot answer for another), and the list is ranked and cut to `limit`.
pub fn collect(replies: &[(&str, &Json)], limit: usize) -> Vec<Match> {
    let mut all = Vec::new();
    for (app, value) in replies {
        for mut m in matches_from_json(value) {
            m.app = app.to_string();
            all.push(m);
        }
    }
    merge(all, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        id: &'static str,
        answers: Vec<Match>,
    }

    impl VisualSearchProvider for Fixed {
        fn app_id(&self) -> &str {
            self.id
        }
        fn search(&self, _scene: &Scene) -> Vec<Match> {
            self.answers.clone()
        }
    }

    fn answer(app: &str, title: &str, score: u32) -> Match {
        Match { app: app.into(), intent: "open".into(), title: title.into(), subtitle: String::new(), score }
    }

    fn scene() -> Scene {
        Scene::new(SceneKind::Photo, 1, 1, vec![0, 0, 0, 255]).unwrap().with_labels(&["plant"])
    }

    #[test]
    fn the_shell_stamps_the_provider_app_id() {
        let liar = Fixed { id: "com.example.notes", answers: vec![answer("com.example.bank", "Receipt", 90)] };
        let out = search(&[&liar], &scene(), MAX_MATCHES);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].app, "com.example.notes");
    }

    #[test]
    fn matches_from_several_apps_are_merged_by_score() {
        let a = Fixed { id: "a.app", answers: vec![answer("x", "Fern guide", 60)] };
        let b = Fixed { id: "b.app", answers: vec![answer("x", "Monstera", 85), answer("x", "Fern guide", 20)] };
        let out = search(&[&a, &b], &scene(), MAX_MATCHES);
        let titles: Vec<&str> = out.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, vec!["Monstera", "Fern guide", "Fern guide"]);
        assert_eq!(out[0].app, "b.app");
    }

    #[test]
    fn the_limit_applies_to_the_merged_list() {
        let a = Fixed { id: "a.app", answers: vec![answer("x", "One", 10), answer("x", "Two", 20), answer("x", "Three", 30)] };
        let out = search(&[&a], &scene(), 2);
        assert_eq!(out.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["Three", "Two"]);
    }

    #[test]
    fn no_providers_give_no_matches() {
        assert!(search(&[], &scene(), MAX_MATCHES).is_empty());
    }

    /// A provider that matches on the scene labels it is given.
    struct Plants;

    impl VisualSearchProvider for Plants {
        fn app_id(&self) -> &str {
            "plants.app"
        }
        fn search(&self, scene: &Scene) -> Vec<Match> {
            let score = label_score(scene, &["plant", "leaf"]);
            if score == 0 {
                return Vec::new();
            }
            vec![Match { app: "someone.else".into(), intent: "open".into(), title: format!("Plant guide ({:?})", scene.kind), subtitle: "Care tips".into(), score }]
        }
    }

    fn args(kind: Option<&str>, labels: Option<&str>) -> Json {
        let mut fields = Vec::new();
        if let Some(k) = kind {
            fields.push(("kind".to_string(), Json::Str(k.into())));
        }
        if let Some(l) = labels {
            fields.push(("labels".to_string(), Json::Str(l.into())));
        }
        Json::Obj(fields)
    }

    #[test]
    fn the_intent_is_declared_with_two_optional_parameters() {
        let i = intent();
        assert_eq!(i.id, INTENT_ID);
        assert!(i.params.iter().all(|p| p.optional) && i.params.len() == 2);
        assert!(i.validate(&args(Some("screenshot"), Some("plant, leaf"))).is_ok());
        assert!(i.validate(&args(Some("video"), None)).is_err(), "the declaration refuses an unknown scene kind");
        assert!(i.validate(&Json::Obj(vec![])).is_ok());
    }

    #[test]
    fn a_request_becomes_a_scene_with_its_kind_and_labels() {
        let s = scene_from_params(&args(Some("screenshot"), Some(" Plant , leaf,plant ,, "))).unwrap();
        assert_eq!(s.kind, SceneKind::Screenshot);
        assert_eq!(s.labels, vec!["plant", "leaf"], "trimmed, lower case, no duplicates or blanks");
        assert_eq!(scene_from_params(&Json::Obj(vec![])).unwrap().kind, SceneKind::Photo);
        assert!(scene_from_params(&args(Some("video"), None)).is_err());
        let many = (0..MAX_LABELS + 1).map(|i| format!("l{i}")).collect::<Vec<_>>().join(",");
        assert_eq!(scene_from_params(&args(None, Some(&many))).unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn performing_the_intent_answers_with_stamped_ranked_matches() {
        let r = perform(&Plants, &args(Some("photo"), Some("plant"))).unwrap();
        assert_eq!(r.dialog.as_deref(), Some("1 match"));
        let found = matches_from_json(&r.value);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].app, "plants.app", "the provider cannot answer for another app");
        assert_eq!(found[0].title, "Plant guide (Photo)");
        assert_eq!(found[0].score, 50);
        let none = perform(&Plants, &args(None, Some("car"))).unwrap();
        assert_eq!(none.dialog.as_deref(), Some("No matches"));
        assert!(matches_from_json(&none.value).is_empty());
    }

    #[test]
    fn replies_from_several_providers_merge_under_their_own_app_ids() {
        let a = matches_to_json(&[Match { app: "liar".into(), intent: "open".into(), title: "Fern".into(), subtitle: String::new(), score: 40 }]);
        let b = matches_to_json(&[Match { app: "x".into(), intent: "open".into(), title: "Monstera".into(), subtitle: "House plant".into(), score: 90 }]);
        let out = collect(&[("a.app", &a), ("b.app", &b)], MAX_MATCHES);
        assert_eq!(out.iter().map(|m| (m.app.as_str(), m.title.as_str())).collect::<Vec<_>>(), vec![("b.app", "Monstera"), ("a.app", "Fern")]);
        assert_eq!(collect(&[("a.app", &a), ("b.app", &b)], 1).len(), 1);
    }

    #[test]
    fn a_malformed_reply_gives_no_matches_and_a_score_is_clamped() {
        assert!(matches_from_json(&Json::Null).is_empty());
        assert!(matches_from_json(&Json::Arr(vec![Json::Str("x".into()), Json::Obj(vec![("score".into(), Json::Num(5.0))])])).is_empty(), "no title, no match");
        let odd = Json::Arr(vec![Json::Obj(vec![("title".into(), Json::Str("T".into())), ("score".into(), Json::Num(900.0))])]);
        assert_eq!(matches_from_json(&odd)[0].score, 100);
        let negative = Json::Arr(vec![Json::Obj(vec![("title".into(), Json::Str("T".into())), ("score".into(), Json::Num(-4.0))])]);
        assert_eq!(matches_from_json(&negative)[0].score, 0);
    }

    #[test]
    fn a_provider_can_match_on_scene_labels() {
        let s = scene();
        assert_eq!(label_score(&s, &["plant", "leaf"]), 50);
        let p = Fixed { id: "plants.app", answers: vec![] };
        assert!(search(&[&p], &s, MAX_MATCHES).is_empty());
    }
}
