//! Delivery of a visual search scene from the shell to the apps and of their matches back, over each app's control
//! socket (one JSON-RPC request line, one reply line; see [`crate::control`]). The encoding is `bifrost_ui::visual_search_wire`,
//! re-exported here.
//!
//! An app that answers visual searches declares `[[intent]] id = "visual-search"` in its manifest (so the shell knows to
//! ask it) and calls [`serve`] from its `rpc` handler, which decodes the scene, runs the app's [`VisualSearchProvider`]
//! and encodes the matches:
//!
//! ```ignore
//! fn rpc(&mut self, ..., method: &str, params: &Json) -> Result<Json, RpcError> {
//!     if let Some(answer) = visual_search_wire::serve(&self.search_provider(), method, params) { return answer; }
//!     /* the app's other methods */
//! }
//! ```
//!
//! The shell side builds a request with [`request_line`] and wraps each app in a [`RemoteProvider`] when it uses the SDK's
//! `visual_search::search` (the shell binary itself uses the same encoding through `bifrost_ui`). A provider whose app is
//! not running, refuses, or answers badly contributes nothing. Matches are stamped with the app that answered, so an app
//! cannot answer for another one.

pub use bifrost_ui::visual_search_wire::*;

use crate::control::{RpcError, INVALID_PARAMS};
use crate::visual_search::{Match, Scene, VisualSearchProvider};
use bifrost_core::json::Json;

/// The app side: answers [`METHOD`] with `provider`'s matches for the scene in `params`. `None` for any other method,
/// so an app's `rpc` handler can fall through to its own methods.
pub fn serve(provider: &dyn VisualSearchProvider, method: &str, params: &Json) -> Option<Result<Json, RpcError>> {
    if method != METHOD {
        return None;
    }
    Some(match scene_from_params(params) {
        Ok(scene) => Ok(matches_json(&provider.search(&scene))),
        Err(e) => Err(RpcError::new(INVALID_PARAMS, format!("visual_search: {e:?}"))),
    })
}

/// How a caller reaches an app: sends one request line to the app's control socket and returns the reply line.
pub trait Transport {
    fn exchange(&self, app_id: &str, line: &str) -> Result<String, String>;
}

/// An installed app as a search provider, reached through a [`Transport`]. An app that cannot be reached or that
/// answers badly gives no matches.
pub struct RemoteProvider<'a> {
    app_id: String,
    transport: &'a dyn Transport,
}

impl<'a> RemoteProvider<'a> {
    pub fn new(app_id: &str, transport: &'a dyn Transport) -> Self {
        RemoteProvider { app_id: app_id.to_string(), transport }
    }
}

impl VisualSearchProvider for RemoteProvider<'_> {
    fn app_id(&self) -> &str {
        &self.app_id
    }

    fn search(&self, scene: &Scene) -> Vec<Match> {
        self.transport
            .exchange(&self.app_id, &request_line(scene, 1))
            .ok()
            .and_then(|reply| matches_from_reply(&reply).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual_search::{label_score, search, SceneKind, MAX_MATCHES};

    fn scene(w: u32, h: u32) -> Scene {
        Scene::new(SceneKind::Screenshot, w, h, vec![9; w as usize * h as usize * 4]).unwrap().with_labels(&["Plant", "leaf"])
    }

    /// An app that matches on labels, as a real provider would.
    struct Plants;

    impl VisualSearchProvider for Plants {
        fn app_id(&self) -> &str {
            "is.x.plants"
        }
        fn search(&self, scene: &Scene) -> Vec<Match> {
            let score = label_score(scene, &["plant", "fern"]);
            if score == 0 {
                return Vec::new();
            }
            vec![Match { app: "is.y.liar".into(), intent: "open-guide".into(), title: format!("Guide {}x{}", scene.width, scene.height), subtitle: "Care".into(), score }]
        }
    }

    /// A transport that runs the request through the control line handler and the app's `serve`.
    struct InMemory;

    impl Transport for InMemory {
        fn exchange(&self, app_id: &str, line: &str) -> Result<String, String> {
            if app_id != "is.x.plants" {
                return Err("not running".to_string());
            }
            Ok(crate::control::handle_line(line, &mut |method, params| {
                serve(&Plants, method, params).unwrap_or_else(|| Err(RpcError::new(crate::control::METHOD_NOT_FOUND, "no method")))
            }))
        }
    }

    #[test]
    fn a_scene_reaches_an_app_and_the_matches_come_back_stamped_with_that_app() {
        let transport = InMemory;
        let remote = RemoteProvider::new("is.x.plants", &transport);
        let out = search(&[&remote], &scene(40, 30), MAX_MATCHES);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].app, "is.x.plants", "the shell stamps the app it asked, not the one the app claims");
        assert_eq!(out[0].title, "Guide 40x30", "the app saw the whole scene");
        assert_eq!(out[0].score, 50);
        assert_eq!(out[0].subtitle, "Care");
    }

    #[test]
    fn serve_ignores_other_methods_and_refuses_a_broken_scene() {
        assert!(serve(&Plants, "intent.new-note", &Json::Null).is_none());
        let err = serve(&Plants, METHOD, &Json::Obj(vec![])).unwrap().unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert!(serve(&Plants, METHOD, &Json::Null).unwrap().is_err());
    }

    #[test]
    fn an_app_that_is_gone_or_answers_badly_gives_nothing() {
        struct Garbage;
        impl Transport for Garbage {
            fn exchange(&self, _app: &str, _line: &str) -> Result<String, String> {
                Ok("not json".to_string())
            }
        }
        let gone = InMemory;
        let missing = RemoteProvider::new("is.z.absent", &gone);
        let garbage = Garbage;
        let broken = RemoteProvider::new("is.x.broken", &garbage);
        assert!(search(&[&missing, &broken], &scene(4, 4), MAX_MATCHES).is_empty());
    }
}
