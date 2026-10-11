//! Visual search for apps: an app answers "what in my content matches this scene?" for a scene the user captured
//! with the camera or selected in a screenshot.
//!
//! The model is `bifrost_ui::visual_search` (the scene, the object selection, the label score and the merge of
//! matches). An app implements [`VisualSearchProvider`]; the shell calls [`search`] with the scene and draws the
//! merged list. Each provider's matches are stamped with the provider's own app id, so an app cannot answer for
//! another one. The camera and screenshot delivery to the shell is not built yet; see docs/parity/visual-intelligence.md.

pub use bifrost_ui::visual_search::{label_score, merge, Match, PixelRect, Scene, SceneError, SceneKind, MAX_MATCHES, MAX_SIDE};

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

    #[test]
    fn a_provider_can_match_on_scene_labels() {
        let s = scene();
        assert_eq!(label_score(&s, &["plant", "leaf"]), 50);
        let p = Fixed { id: "plants.app", answers: vec![] };
        assert!(search(&[&p], &s, MAX_MATCHES).is_empty());
    }
}
