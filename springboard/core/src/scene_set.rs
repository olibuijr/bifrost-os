//! Scene model: the scenes each app has open, in the order they were made, each with its own saved state.
//!
//! A scene is one window or content area an app hosts. An app can host several, and the shell keeps each scene's
//! state so it can be restored with the same id. Pure: no process, no window and no disk. The shell asks
//! [`SceneSet::scenes`] what an app has, and it stores the state an app hands back with [`SceneSet::set_state`].

use std::collections::BTreeMap;

/// The longest id a scene may have, in bytes.
pub const MAX_ID_LEN: usize = 128;

/// One scene of an app: its id (unique within the app), its title, and the state it last saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub id: String,
    pub title: String,
    pub state: Vec<u8>,
}

/// Why a scene could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// The id is empty or longer than [`MAX_ID_LEN`].
    BadId,
    /// The app already has a scene with this id.
    Duplicate,
}

/// The scenes of every app, keyed by app id. An app's scenes stay in the order they were made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneSet {
    apps: BTreeMap<String, Vec<Scene>>,
}

impl SceneSet {
    pub fn new() -> SceneSet {
        SceneSet::default()
    }

    /// Makes a scene for `app` with an empty state. Refused when the id is empty or too long, or when the app has
    /// a scene with the same id.
    pub fn create(&mut self, app: &str, id: &str, title: &str) -> Result<(), SceneError> {
        if id.is_empty() || id.len() > MAX_ID_LEN {
            return Err(SceneError::BadId);
        }
        let scenes = self.apps.entry(app.to_string()).or_default();
        if scenes.iter().any(|s| s.id == id) {
            return Err(SceneError::Duplicate);
        }
        scenes.push(Scene { id: id.to_string(), title: title.to_string(), state: Vec::new() });
        Ok(())
    }

    /// The scenes of `app` in the order they were made. Empty when the app has none.
    pub fn scenes(&self, app: &str) -> &[Scene] {
        self.apps.get(app).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The scene `id` of `app`, if it exists. This is the restore point: its state is what the app saved last.
    pub fn restore(&self, app: &str, id: &str) -> Option<&Scene> {
        self.scenes(app).iter().find(|s| s.id == id)
    }

    /// Replaces the saved state of scene `id` of `app`. Returns false when the scene does not exist.
    pub fn set_state(&mut self, app: &str, id: &str, state: &[u8]) -> bool {
        match self.apps.get_mut(app).and_then(|v| v.iter_mut().find(|s| s.id == id)) {
            Some(scene) => {
                scene.state = state.to_vec();
                true
            }
            None => false,
        }
    }

    /// Removes scene `id` of `app`. Returns false when it was not there.
    pub fn remove(&mut self, app: &str, id: &str) -> bool {
        let Some(scenes) = self.apps.get_mut(app) else { return false };
        let before = scenes.len();
        scenes.retain(|s| s.id != id);
        let removed = scenes.len() != before;
        if scenes.is_empty() {
            self.apps.remove(app);
        }
        removed
    }

    /// Drops every scene of `app`, for an app that was removed from the system. Returns how many it had.
    pub fn forget_app(&mut self, app: &str) -> usize {
        self.apps.remove(app).map(|v| v.len()).unwrap_or(0)
    }

    /// The apps that have at least one scene, in id order.
    pub fn apps(&self) -> Vec<&str> {
        self.apps.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_can_host_several_scenes_in_the_order_they_were_made() {
        let mut set = SceneSet::new();
        set.create("notes", "main", "Notes").unwrap();
        set.create("notes", "preview", "Preview").unwrap();
        let ids: Vec<&str> = set.scenes("notes").iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["main", "preview"]);
        assert!(set.scenes("other").is_empty());
    }

    #[test]
    fn a_scene_id_must_be_unique_within_its_app_and_not_empty() {
        let mut set = SceneSet::new();
        set.create("notes", "main", "Notes").unwrap();
        assert_eq!(set.create("notes", "main", "Again"), Err(SceneError::Duplicate));
        // the same id in another app is a different scene
        assert_eq!(set.create("mail", "main", "Mail"), Ok(()));
        assert_eq!(set.create("notes", "", "Empty"), Err(SceneError::BadId));
        let long = "x".repeat(MAX_ID_LEN + 1);
        assert_eq!(set.create("notes", &long, "Long"), Err(SceneError::BadId));
        assert_eq!(set.scenes("notes").len(), 1);
    }

    #[test]
    fn the_saved_state_is_kept_per_scene_and_restored_by_id() {
        let mut set = SceneSet::new();
        set.create("notes", "main", "Notes").unwrap();
        set.create("notes", "preview", "Preview").unwrap();
        assert!(set.set_state("notes", "main", b"cursor=4"));
        assert_eq!(set.restore("notes", "main").map(|s| s.state.clone()), Some(b"cursor=4".to_vec()));
        assert_eq!(set.restore("notes", "preview").map(|s| s.state.is_empty()), Some(true));
        assert!(!set.set_state("notes", "gone", b"x"), "a scene that does not exist keeps nothing");
        assert!(set.restore("notes", "gone").is_none());
    }

    #[test]
    fn removing_the_last_scene_drops_the_app_and_forgetting_an_app_drops_all_its_scenes() {
        let mut set = SceneSet::new();
        set.create("notes", "main", "Notes").unwrap();
        set.create("notes", "preview", "Preview").unwrap();
        set.create("mail", "main", "Mail").unwrap();
        assert!(set.remove("notes", "main"));
        assert!(!set.remove("notes", "main"), "a second remove finds nothing");
        assert_eq!(set.apps(), vec!["mail", "notes"]);
        assert!(set.remove("notes", "preview"));
        assert_eq!(set.apps(), vec!["mail"], "an app with no scenes is no longer listed");
        assert_eq!(set.forget_app("mail"), 1);
        assert_eq!(set.forget_app("mail"), 0);
        assert_eq!(set, SceneSet::new());
    }
}
