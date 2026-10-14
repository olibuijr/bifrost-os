//! Scene sessions (`UIScene`, `UISceneSession`, `UISceneDelegate`): one app keeps several windows, and each window is a
//! scene with its own session, state, user info and restorable state.
//!
//! The app owns its sessions in a [`SceneSessions`] registry and implements [`SceneDelegate`] to receive the lifecycle
//! callbacks. A session moves through four states, and each move delivers the callbacks of the reference platform in
//! the same order ([`transition_events`]):
//!
//! - `Unattached`: the session does not exist yet or was destroyed.
//! - `ForegroundActive`: the scene is on screen and has focus.
//! - `ForegroundInactive`: the scene is on screen without focus.
//! - `Background`: the app is not in front, or the scene is hidden.
//!
//! The shell creates the windows. The app asks for one with `scenes.request` on the `scenes` service (see
//! [`request_params`] and [`SceneAnswer::decode`]), and the answer names the new scene's persistent identifier, its window
//! number and its frame. The registry is the app-side half of that exchange, and the shell's table is the other half
//! (`springboard/src/shell/services/scene_services.rs`).
//!
//! ```ignore
//! struct Notes { scenes: SceneSessions }
//! impl SceneDelegate for Notes {
//!     fn scene_event(&mut self, scene: &SceneSession, event: SceneEvent) {
//!         if event == SceneEvent::WillConnect { /* build the window's content for scene.user_info */ }
//!     }
//! }
//! // a new window: ask the shell, then register the scene with the answer
//! let id = service::call(ctx, scene_session::SERVICE, scene_session::METHOD_REQUEST, &scene_session::request_params(...));
//! ```
use bifrost_core::json::Json;
use bifrost_ui::wire_json::*;
use bifrost_ui::Rect;
use crate::restoration::RestorableState;
use std::fmt;

/// The shell service that opens, lists, focuses and closes the scene windows of an app.
pub const SERVICE: &str = "scenes";
/// `request {title, width, height, user_info}`: open a new window; answers [`SceneAnswer`].
pub const METHOD_REQUEST: &str = "request";
/// `close {scene}`: close one window; answers `{}`.
pub const METHOD_CLOSE: &str = "close";
/// `focus {scene}`: bring one window to the front; answers `{}`.
pub const METHOD_FOCUS: &str = "focus";
/// `list {}`: the windows of the app, front first; answers `{scenes: [SceneAnswer]}`.
pub const METHOD_LIST: &str = "list";
/// `reopen {scene, frame}`: put back a window that the app saved before a relaunch, with its identifier and frame.
pub const METHOD_REOPEN: &str = "reopen";
/// The role of a window scene, the only role a window of an app has here (`UIWindowSceneSessionRoleApplication`).
pub const ROLE_WINDOW_APPLICATION: &str = "windowApplication";
/// The most scenes one registry holds at once, unless the app sets another limit.
pub const DEFAULT_LIMIT: usize = 8;

/// The state of a scene session (`UISceneActivationState`, plus `Unattached` for a session that is gone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneState {
    Unattached,
    ForegroundActive,
    ForegroundInactive,
    Background,
}

/// A lifecycle callback of `UISceneDelegate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneEvent {
    WillConnect,
    WillEnterForeground,
    DidBecomeActive,
    WillResignActive,
    DidEnterBackground,
    DidDisconnect,
}

/// The callbacks a state change delivers, in the order the reference platform calls them. A change to the same state
/// delivers nothing. A scene always goes through `ForegroundInactive` on its way between `Background` and
/// `ForegroundActive`, and a scene that is connected in the background gets `WillConnect` and `DidEnterBackground`
/// without a foreground call.
pub fn transition_events(from: SceneState, to: SceneState) -> Vec<SceneEvent> {
    use SceneEvent as E;
    use SceneState as S;
    if from == to {
        return Vec::new();
    }
    if to == S::Unattached {
        let mut out = Vec::new();
        if from == S::ForegroundActive {
            out.push(E::WillResignActive);
        }
        if from != S::Background {
            out.push(E::DidEnterBackground);
        }
        out.push(E::DidDisconnect);
        return out;
    }
    if from == S::Unattached && to == S::Background {
        return vec![E::WillConnect, E::DidEnterBackground];
    }
    let mut out = Vec::new();
    let mut cur = from;
    match cur {
        S::Unattached => {
            out.push(E::WillConnect);
            out.push(E::WillEnterForeground);
            cur = S::ForegroundInactive;
        }
        S::ForegroundActive => {
            out.push(E::WillResignActive);
            cur = S::ForegroundInactive;
        }
        S::Background if to != S::Background => {
            out.push(E::WillEnterForeground);
            cur = S::ForegroundInactive;
        }
        _ => {}
    }
    if cur == S::ForegroundInactive {
        match to {
            S::Background => out.push(E::DidEnterBackground),
            S::ForegroundActive => out.push(E::DidBecomeActive),
            _ => {}
        }
    }
    out
}

/// One scene session of the app.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneSession {
    /// The identifier that survives a relaunch (`persistentIdentifier`), given by the shell's answer.
    pub persistent_id: String,
    /// The scene role; [`ROLE_WINDOW_APPLICATION`] for the windows of an app.
    pub role: String,
    pub state: SceneState,
    /// The information the app passed when it asked for the scene (`userInfo`), in order.
    pub user_info: Vec<(String, String)>,
    /// The state the app saved for this scene (`encodeRestorableState`). The app fills and reads it.
    pub restorable: RestorableState,
}

/// Receives the lifecycle callbacks of the scenes of one registry.
pub trait SceneDelegate {
    /// Called after the scene's state has changed to its new state, once per callback in order.
    fn scene_event(&mut self, scene: &SceneSession, event: SceneEvent);
}

/// Why a scene request or a scene change failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// No scene has this persistent identifier.
    UnknownScene(String),
    /// The registry already holds the most scenes it allows.
    TooManyScenes(usize),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::UnknownScene(id) => write!(f, "no scene named {id}"),
            SceneError::TooManyScenes(max) => write!(f, "an app holds at most {max} windows"),
        }
    }
}

/// The scenes of one app, in the order they were requested.
///
/// One scene at a time is the focused scene. While the app is in front, the focused scene is `ForegroundActive` and the
/// others are `ForegroundInactive`. While the app is in the background every scene is `Background`.
#[derive(Debug)]
pub struct SceneSessions {
    scenes: Vec<SceneSession>,
    focused: Option<String>,
    foreground: bool,
    limit: usize,
    next: u32,
}

impl Default for SceneSessions {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl SceneSessions {
    /// An empty registry that holds at most `limit` scenes.
    pub fn new(limit: usize) -> Self {
        Self { scenes: Vec::new(), focused: None, foreground: false, limit, next: 1 }
    }

    /// The scenes, in request order.
    pub fn list(&self) -> &[SceneSession] {
        &self.scenes
    }

    pub fn get(&self, persistent_id: &str) -> Option<&SceneSession> {
        self.scenes.iter().find(|s| s.persistent_id == persistent_id)
    }

    pub fn get_mut(&mut self, persistent_id: &str) -> Option<&mut SceneSession> {
        self.scenes.iter_mut().find(|s| s.persistent_id == persistent_id)
    }

    /// The scene that has focus, if any.
    pub fn focused(&self) -> Option<&SceneSession> {
        self.focused.as_deref().and_then(|id| self.get(id))
    }

    /// Creates a scene for a window the shell opened under `persistent_id`, or a new identifier when `None`.
    /// The scene connects, and then focuses when the app is in front. Returns the identifier.
    pub fn request(
        &mut self,
        delegate: &mut dyn SceneDelegate,
        persistent_id: Option<&str>,
        user_info: Vec<(String, String)>,
    ) -> Result<String, SceneError> {
        if self.scenes.len() >= self.limit {
            return Err(SceneError::TooManyScenes(self.limit));
        }
        let id = match persistent_id {
            Some(id) => id.to_string(),
            None => {
                let id = format!("scene-{}", self.next);
                self.next += 1;
                id
            }
        };
        self.scenes.push(SceneSession {
            persistent_id: id.clone(),
            role: ROLE_WINDOW_APPLICATION.to_string(),
            state: SceneState::Unattached,
            user_info,
            restorable: RestorableState::new(),
        });
        if self.foreground {
            self.focused = Some(id.clone());
        }
        self.settle(delegate);
        Ok(id)
    }

    /// Gives focus to a scene (`requestSceneSessionActivation`).
    pub fn focus(&mut self, delegate: &mut dyn SceneDelegate, persistent_id: &str) -> Result<(), SceneError> {
        if self.get(persistent_id).is_none() {
            return Err(SceneError::UnknownScene(persistent_id.to_string()));
        }
        self.focused = Some(persistent_id.to_string());
        self.settle(delegate);
        Ok(())
    }

    /// Records that the app came to the front or went to the background. A background app has no focused scene.
    pub fn set_foreground(&mut self, delegate: &mut dyn SceneDelegate, foreground: bool) {
        self.foreground = foreground;
        if foreground && self.focused.is_none() {
            self.focused = self.scenes.first().map(|s| s.persistent_id.clone());
        }
        self.settle(delegate);
    }

    /// Ends a scene (`UISceneSession` destruction): it disconnects and leaves the registry. Returns the scene.
    pub fn close(&mut self, delegate: &mut dyn SceneDelegate, persistent_id: &str) -> Result<SceneSession, SceneError> {
        let index = self
            .scenes
            .iter()
            .position(|s| s.persistent_id == persistent_id)
            .ok_or_else(|| SceneError::UnknownScene(persistent_id.to_string()))?;
        self.apply_state(delegate, index, SceneState::Unattached);
        let scene = self.scenes.remove(index);
        if self.focused.as_deref() == Some(persistent_id) {
            self.focused = self.scenes.last().map(|s| s.persistent_id.clone());
        }
        self.settle(delegate);
        Ok(scene)
    }

    /// The state every scene should have now, from whether the app is in front and which scene has focus.
    fn wanted(&self, scene: &SceneSession) -> SceneState {
        if !self.foreground {
            return SceneState::Background;
        }
        if self.focused.as_deref() == Some(scene.persistent_id.as_str()) {
            SceneState::ForegroundActive
        } else {
            SceneState::ForegroundInactive
        }
    }

    fn settle(&mut self, delegate: &mut dyn SceneDelegate) {
        for index in 0..self.scenes.len() {
            let wanted = self.wanted(&self.scenes[index]);
            self.apply_state(delegate, index, wanted);
        }
    }

    fn apply_state(&mut self, delegate: &mut dyn SceneDelegate, index: usize, to: SceneState) {
        let from = self.scenes[index].state;
        let events = transition_events(from, to);
        self.scenes[index].state = to;
        for event in events {
            delegate.scene_event(&self.scenes[index], event);
        }
    }
}

/// The parameters of `request` for the shell: the window title, its wanted size and the scene's user info.
pub fn request_params(title: &str, width: f32, height: f32, user_info: &[(String, String)]) -> Json {
    let info = user_info.iter().map(|(k, v)| (k.as_str(), text(v))).collect::<Vec<_>>();
    obj(vec![
        ("title", text(title)),
        ("width", num(width)),
        ("height", num(height)),
        ("user_info", obj(info)),
    ])
}

/// A window the shell opened or listed for the app.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneAnswer {
    /// The persistent identifier of the scene.
    pub scene: String,
    /// The window number, as the window list reports it.
    pub number: u32,
    /// The frame in points on the screen.
    pub frame: Rect,
    pub title: String,
}

impl SceneAnswer {
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("scene", text(&self.scene)),
            ("number", num(self.number)),
            ("frame", rect_json(self.frame)),
            ("title", text(&self.title)),
        ])
    }

    /// Reads one answer; `None` when a field is missing.
    pub fn from_json(j: &Json) -> Option<Self> {
        Some(Self {
            scene: str_of(j, "scene")?,
            number: u32_of(j, "number")?,
            frame: rect_of(j, "frame")?,
            title: str_of(j, "title").unwrap_or_default(),
        })
    }

    /// Decodes the answer of `request` or `reopen`.
    pub fn decode(result: &Result<Json, String>) -> Result<Self, String> {
        let body = result.as_ref().map_err(|e| e.clone())?;
        Self::from_json(body).ok_or_else(|| "scenes: unreadable window record".to_string())
    }

    /// Decodes the answer of `list`.
    pub fn decode_list(result: &Result<Json, String>) -> Result<Vec<Self>, String> {
        let body = result.as_ref().map_err(|e| e.clone())?;
        let items = body.get("scenes").and_then(Json::as_array).ok_or("scenes: the list is missing")?;
        items.iter().map(|j| Self::from_json(j).ok_or_else(|| "scenes: unreadable window record".to_string())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SceneEvent as E;
    use SceneState as S;

    #[derive(Default)]
    struct Log(Vec<(String, SceneEvent, SceneState)>);

    impl SceneDelegate for Log {
        fn scene_event(&mut self, scene: &SceneSession, event: SceneEvent) {
            self.0.push((scene.persistent_id.clone(), event, scene.state));
        }
    }

    fn events(log: &Log) -> Vec<SceneEvent> {
        log.0.iter().map(|(_, e, _)| *e).collect()
    }

    #[test]
    fn a_connection_in_the_foreground_runs_the_full_entry_sequence() {
        assert_eq!(transition_events(S::Unattached, S::ForegroundActive), vec![E::WillConnect, E::WillEnterForeground, E::DidBecomeActive]);
        assert_eq!(transition_events(S::Unattached, S::ForegroundInactive), vec![E::WillConnect, E::WillEnterForeground]);
    }

    #[test]
    fn a_connection_in_the_background_skips_the_foreground_callbacks() {
        assert_eq!(transition_events(S::Unattached, S::Background), vec![E::WillConnect, E::DidEnterBackground]);
    }

    #[test]
    fn the_foreground_and_background_moves_follow_the_platform_order() {
        assert_eq!(transition_events(S::ForegroundInactive, S::ForegroundActive), vec![E::DidBecomeActive]);
        assert_eq!(transition_events(S::ForegroundActive, S::ForegroundInactive), vec![E::WillResignActive]);
        assert_eq!(transition_events(S::Background, S::ForegroundInactive), vec![E::WillEnterForeground]);
        assert_eq!(transition_events(S::Background, S::ForegroundActive), vec![E::WillEnterForeground, E::DidBecomeActive]);
        assert_eq!(transition_events(S::ForegroundInactive, S::Background), vec![E::DidEnterBackground]);
        assert_eq!(transition_events(S::ForegroundActive, S::Background), vec![E::WillResignActive, E::DidEnterBackground]);
    }

    #[test]
    fn disconnecting_leaves_the_scene_resigned_and_backgrounded_first() {
        assert_eq!(transition_events(S::ForegroundActive, S::Unattached), vec![E::WillResignActive, E::DidEnterBackground, E::DidDisconnect]);
        assert_eq!(transition_events(S::ForegroundInactive, S::Unattached), vec![E::DidEnterBackground, E::DidDisconnect]);
        assert_eq!(transition_events(S::Background, S::Unattached), vec![E::DidDisconnect]);
    }

    #[test]
    fn no_change_delivers_nothing() {
        assert!(transition_events(S::Background, S::Background).is_empty());
        assert!(transition_events(S::Unattached, S::Unattached).is_empty());
    }

    #[test]
    fn requesting_scenes_while_in_front_focuses_the_newest_and_deactivates_the_rest() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        reg.set_foreground(&mut log, true);
        let a = reg.request(&mut log, None, vec![]).unwrap();
        assert_eq!(a, "scene-1");
        let b = reg.request(&mut log, Some("shell-9"), vec![("doc".into(), "x".into())]).unwrap();
        assert_eq!(b, "shell-9");
        assert_eq!(reg.focused().unwrap().persistent_id, "shell-9");
        assert_eq!(reg.get("scene-1").unwrap().state, S::ForegroundInactive);
        assert_eq!(reg.get("shell-9").unwrap().state, S::ForegroundActive);
        assert_eq!(reg.get("shell-9").unwrap().user_info, vec![("doc".to_string(), "x".to_string())]);
    }

    #[test]
    fn a_scene_requested_in_the_background_waits_there() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        let id = reg.request(&mut log, None, vec![]).unwrap();
        assert_eq!(reg.get(&id).unwrap().state, S::Background);
        assert_eq!(events(&log), vec![E::WillConnect, E::DidEnterBackground]);
        assert!(reg.focused().is_none());
    }

    #[test]
    fn going_to_the_background_and_back_reaches_the_focused_scene_only() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        reg.set_foreground(&mut log, true);
        reg.request(&mut log, None, vec![]).unwrap();
        reg.request(&mut log, None, vec![]).unwrap();
        log.0.clear();
        reg.set_foreground(&mut log, false);
        assert!(log.0.iter().all(|(_, _, state)| *state == S::Background));
        assert_eq!(log.0.len(), 3);
        log.0.clear();
        reg.set_foreground(&mut log, true);
        assert_eq!(reg.focused().unwrap().persistent_id, "scene-2");
        assert_eq!(reg.get("scene-2").unwrap().state, S::ForegroundActive);
        assert_eq!(reg.get("scene-1").unwrap().state, S::ForegroundInactive);
    }

    #[test]
    fn focusing_moves_the_active_scene_and_rejects_unknown_ids() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        reg.set_foreground(&mut log, true);
        let a = reg.request(&mut log, None, vec![]).unwrap();
        let b = reg.request(&mut log, None, vec![]).unwrap();
        reg.focus(&mut log, &a).unwrap();
        assert_eq!(reg.get(&a).unwrap().state, S::ForegroundActive);
        assert_eq!(reg.get(&b).unwrap().state, S::ForegroundInactive);
        assert_eq!(reg.focus(&mut log, "nope"), Err(SceneError::UnknownScene("nope".into())));
    }

    #[test]
    fn closing_disconnects_the_scene_and_moves_focus_to_the_last_one() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        reg.set_foreground(&mut log, true);
        let a = reg.request(&mut log, None, vec![]).unwrap();
        let b = reg.request(&mut log, None, vec![]).unwrap();
        reg.focus(&mut log, &b).unwrap();
        log.0.clear();
        let gone = reg.close(&mut log, &b).unwrap();
        assert_eq!(gone.persistent_id, b);
        assert_eq!(gone.state, S::Unattached);
        assert_eq!(events(&log), vec![E::WillResignActive, E::DidEnterBackground, E::DidDisconnect]);
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.focused().unwrap().persistent_id, a);
        assert_eq!(reg.get(&a).unwrap().state, S::ForegroundActive);
        assert!(matches!(reg.close(&mut log, &b), Err(SceneError::UnknownScene(_))));
    }

    #[test]
    fn the_registry_refuses_more_scenes_than_its_limit() {
        let mut reg = SceneSessions::new(2);
        let mut log = Log::default();
        reg.request(&mut log, None, vec![]).unwrap();
        reg.request(&mut log, None, vec![]).unwrap();
        assert_eq!(reg.request(&mut log, None, vec![]), Err(SceneError::TooManyScenes(2)));
        assert_eq!(SceneError::TooManyScenes(2).to_string(), "an app holds at most 2 windows");
    }

    #[test]
    fn a_scene_keeps_its_restorable_state() {
        let mut reg = SceneSessions::default();
        let mut log = Log::default();
        let id = reg.request(&mut log, None, vec![]).unwrap();
        reg.get_mut(&id).unwrap().restorable.encode_for("note", &"essay".to_string());
        assert_eq!(reg.get(&id).unwrap().restorable.decode_for::<String>("note").as_deref(), Some("essay"));
    }

    #[test]
    fn the_answer_round_trips_through_its_json() {
        let answer = SceneAnswer {
            scene: "scene-4".into(),
            number: 20_004,
            frame: Rect { x: 24.0, y: 64.0, w: 400.0, h: 300.0 },
            title: "Notes".into(),
        };
        assert_eq!(SceneAnswer::decode(&Ok(answer.to_json())).unwrap(), answer);
        let list = obj(vec![("scenes", Json::Arr(vec![answer.to_json(), answer.to_json()]))]);
        assert_eq!(SceneAnswer::decode_list(&Ok(list)).unwrap().len(), 2);
        assert!(SceneAnswer::decode(&Err("no".into())).is_err());
        assert!(SceneAnswer::decode(&Ok(obj(vec![("scene", text("x"))]))).is_err());
    }

    #[test]
    fn the_request_carries_the_title_size_and_user_info() {
        let j = request_params("Notes", 400.0, 300.0, &[("doc".into(), "a.txt".into())]);
        assert_eq!(j.get("title").and_then(Json::as_str), Some("Notes"));
        assert_eq!(j.get("width").and_then(Json::number), Some(400.0));
        assert_eq!(j.get("user_info").and_then(|u| u.get("doc")).and_then(Json::as_str), Some("a.txt"));
    }
}
