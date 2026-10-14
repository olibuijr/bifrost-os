//! Pointer lock: a scene can capture the pointer. While its lock is held and the scene is the key
//! scene, the cursor is hidden and motion is reported as relative deltas, so a game or a drawing surface can keep
//! moving past the screen edges. Pure model: the compositor reports which scene is key, the scene asks for and
//! releases the lock, and raw motion goes through `motion`, which returns the delta to deliver when the lock is held.

/// A change to the lock state that the owner of a scene has to hear about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The scene now holds the lock.
    Acquired(u64),
    /// The scene lost the lock: it released it, or it stopped being the key scene.
    Released(u64),
}

/// The pointer lock state of every scene. Only the key scene can hold the lock, and at most one scene holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerLock {
    key: Option<u64>,
    held: Option<u64>,
}

impl PointerLock {
    pub fn new() -> Self {
        Self::default()
    }

    /// The key scene changed. A lock held by a scene that is no longer key is released.
    pub fn set_key_scene(&mut self, scene: Option<u64>) -> Vec<Change> {
        self.key = scene;
        match self.held {
            Some(h) if Some(h) != scene => {
                self.held = None;
                vec![Change::Released(h)]
            }
            _ => Vec::new(),
        }
    }

    /// Asks for the lock for `scene`. Refused unless `scene` is the key scene. Asking again while the lock is held is a
    /// no-op that succeeds.
    pub fn request(&mut self, scene: u64) -> Result<Option<Change>, LockError> {
        if self.key != Some(scene) {
            return Err(LockError::NotKeyScene);
        }
        if self.held == Some(scene) {
            return Ok(None);
        }
        self.held = Some(scene);
        Ok(Some(Change::Acquired(scene)))
    }

    /// Releases the lock if `scene` holds it. Returns the change, or `None` when `scene` did not hold the lock.
    pub fn release(&mut self, scene: u64) -> Option<Change> {
        if self.held == Some(scene) {
            self.held = None;
            return Some(Change::Released(scene));
        }
        None
    }

    /// True when `scene` holds the lock.
    pub fn is_locked(&self, scene: u64) -> bool {
        self.held == Some(scene)
    }

    /// True when the cursor should be hidden: a lock is held by the key scene.
    pub fn cursor_hidden(&self) -> bool {
        self.held.is_some() && self.held == self.key
    }

    /// The motion to deliver for a raw pointer movement. While the lock is held the delta goes to the locked scene; without
    /// a lock the caller moves the cursor by absolute position and gets `None`.
    pub fn motion(&self, dx: f32, dy: f32) -> Option<(f32, f32)> {
        if self.cursor_hidden() { Some((dx, dy)) } else { None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockError {
    /// Only the key scene may hold the pointer.
    NotKeyScene,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_key_scene_can_lock() {
        let mut l = PointerLock::new();
        assert_eq!(l.request(1), Err(LockError::NotKeyScene));
        l.set_key_scene(Some(1));
        assert_eq!(l.request(2), Err(LockError::NotKeyScene));
        assert_eq!(l.request(1), Ok(Some(Change::Acquired(1))));
        assert!(l.is_locked(1));
    }

    #[test]
    fn requesting_twice_is_a_no_op() {
        let mut l = PointerLock::new();
        l.set_key_scene(Some(3));
        l.request(3).unwrap();
        assert_eq!(l.request(3), Ok(None));
    }

    #[test]
    fn losing_key_status_releases_the_lock() {
        let mut l = PointerLock::new();
        l.set_key_scene(Some(1));
        l.request(1).unwrap();
        assert_eq!(l.set_key_scene(Some(2)), vec![Change::Released(1)]);
        assert!(!l.is_locked(1));
        assert!(!l.cursor_hidden());
    }

    #[test]
    fn keeping_key_status_keeps_the_lock() {
        let mut l = PointerLock::new();
        l.set_key_scene(Some(1));
        l.request(1).unwrap();
        assert!(l.set_key_scene(Some(1)).is_empty());
        assert!(l.is_locked(1));
    }

    #[test]
    fn release_reports_only_a_held_lock() {
        let mut l = PointerLock::new();
        l.set_key_scene(Some(1));
        assert_eq!(l.release(1), None);
        l.request(1).unwrap();
        assert_eq!(l.release(1), Some(Change::Released(1)));
        assert_eq!(l.release(1), None);
    }

    #[test]
    fn motion_is_relative_only_while_locked() {
        let mut l = PointerLock::new();
        l.set_key_scene(Some(1));
        assert_eq!(l.motion(4.0, -2.0), None);
        l.request(1).unwrap();
        assert!(l.cursor_hidden());
        assert_eq!(l.motion(4.0, -2.0), Some((4.0, -2.0)));
    }
}
