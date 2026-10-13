//! GPU presenter health: the pure part of "a GPU error falls back to the CPU path instead of freezing".
//!
//! The presenter records the first failure it sees (a GL error flag after a pass, a context that will not
//! become current, a pass that does not finish in time). Once set the failure stays: a lost context does not
//! heal by itself, and drawing into it again would only repeat the stall. The shell reads the failure after
//! each draw and moves to the SHM path. Everything here is plain data, so it runs on the host.

/// `EGL_CONDITION_SATISFIED_KHR`: the fence signalled.
pub const FENCE_SATISFIED: i32 = 0x30F6;
/// `EGL_TIMEOUT_EXPIRED_KHR`: the fence did not signal within the wait.
pub const FENCE_TIMEOUT: i32 = 0x30F5;

/// `GL_OUT_OF_MEMORY`: the next pass would fail the same way, so the presenter gives up.
pub const GL_OUT_OF_MEMORY: u32 = 0x505;
/// `GL_CONTEXT_LOST` (KHR_robustness): the context is gone.
pub const GL_CONTEXT_LOST: u32 = 0x0507;

/// The first failure the presenter saw, if any, and how many other GL error flags it has skipped past.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    failure: Option<String>,
    glitches: u32,
}

impl Health {
    /// True while no failure has been recorded.
    pub fn is_ok(&self) -> bool {
        self.failure.is_none()
    }

    /// The reason the presenter gave up, once it did.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Records a failure. The first one wins: later ones are symptoms of the same loss.
    pub fn fail(&mut self, why: impl FnOnce() -> String) {
        if self.failure.is_none() {
            self.failure = Some(why());
        }
    }

    /// Checks a GL error flag read after a pass (`0` is `GL_NO_ERROR`). Out of memory and a lost context end the
    /// presenter. Any other flag is a fault in one pass: the presenter keeps drawing, and the first such flag is
    /// returned so the caller can log it (later ones only count, so a flag that repeats every frame logs once).
    pub fn gl_flag(&mut self, what: &str, code: u32) -> Option<String> {
        if code == 0 {
            return None;
        }
        let msg = format!("{what}: GL error 0x{code:x}");
        if code == GL_OUT_OF_MEMORY || code == GL_CONTEXT_LOST {
            self.fail(|| msg);
            return None;
        }
        self.glitches += 1;
        (self.glitches == 1).then_some(msg)
    }

    /// GL error flags that did not end the presenter.
    pub fn glitches(&self) -> u32 {
        self.glitches
    }

    /// Judges the result of a client wait on a fence: `Ok` when the pass finished, `Err` for a timeout or an
    /// unexpected return. A timed-out pass means the GPU is not making progress, which is what a freeze looks like.
    pub fn fence(&mut self, what: &str, result: i32) -> bool {
        match result {
            FENCE_SATISFIED => true,
            FENCE_TIMEOUT => {
                self.fail(|| format!("{what}: GPU did not finish the pass in time"));
                false
            }
            other => {
                self.fail(|| format!("{what}: fence wait returned 0x{other:x}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_healthy() {
        let h = Health::default();
        assert!(h.is_ok());
        assert_eq!(h.failure(), None);
        assert_eq!(h.glitches(), 0);
    }

    #[test]
    fn no_gl_error_keeps_health() {
        let mut h = Health::default();
        assert_eq!(h.gl_flag("draw", 0), None);
        assert!(h.is_ok());
        assert_eq!(h.glitches(), 0);
    }

    #[test]
    fn out_of_memory_fails_the_presenter() {
        let mut h = Health::default();
        assert_eq!(h.gl_flag("draw", GL_OUT_OF_MEMORY), None);
        assert!(!h.is_ok());
        assert_eq!(h.failure(), Some("draw: GL error 0x505"));
    }

    #[test]
    fn a_lost_context_fails_the_presenter() {
        let mut h = Health::default();
        h.gl_flag("draw", GL_CONTEXT_LOST);
        assert_eq!(h.failure(), Some("draw: GL error 0x507"));
    }

    #[test]
    fn other_gl_errors_are_logged_once_and_drawing_goes_on() {
        let mut h = Health::default();
        assert_eq!(h.gl_flag("draw", 0x502), Some("draw: GL error 0x502".to_string()));
        assert_eq!(h.gl_flag("draw", 0x502), None, "a repeated flag is counted, not logged again");
        assert_eq!(h.gl_flag("blur", 0x501), None);
        assert!(h.is_ok());
        assert_eq!(h.glitches(), 3);
    }

    #[test]
    fn first_failure_wins() {
        let mut h = Health::default();
        h.gl_flag("draw", GL_OUT_OF_MEMORY);
        h.gl_flag("blur", GL_CONTEXT_LOST);
        h.fail(|| "make current".to_string());
        assert_eq!(h.failure(), Some("draw: GL error 0x505"));
    }

    #[test]
    fn a_satisfied_fence_is_healthy() {
        let mut h = Health::default();
        assert!(h.fence("draw", FENCE_SATISFIED));
        assert!(h.is_ok());
    }

    #[test]
    fn a_timed_out_fence_fails_the_presenter() {
        let mut h = Health::default();
        assert!(!h.fence("draw", FENCE_TIMEOUT));
        assert_eq!(h.failure(), Some("draw: GPU did not finish the pass in time"));
    }

    #[test]
    fn an_odd_fence_result_fails_the_presenter() {
        let mut h = Health::default();
        assert!(!h.fence("draw", 0));
        assert_eq!(h.failure(), Some("draw: fence wait returned 0x0"));
    }

    #[test]
    fn the_failure_closure_only_runs_for_the_first_failure() {
        let mut h = Health::default();
        let mut calls = 0;
        h.fail(|| {
            calls += 1;
            "first".to_string()
        });
        h.fail(|| {
            calls += 1;
            "second".to_string()
        });
        assert_eq!(calls, 1);
        assert_eq!(h.failure(), Some("first"));
    }
}
