//! The device-owner authentication prompt, as a pure model (the sheet an app shows when it asks the user to confirm the
//! account password). The shell draws the sheet and collects the password; this module decides what each attempt does:
//! how many attempts remain, when the prompt locks out, and whether the fallback choice is offered.

/// Attempts allowed before the prompt refuses further input.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptError {
    /// The reason shown in the prompt is empty.
    EmptyReason,
    /// A prompt needs at least one attempt.
    NoAttempts,
    /// The prompt is no longer waiting for input.
    NotPrompting,
    /// The fallback option was not offered for this prompt.
    FallbackNotAllowed,
}

/// Where the prompt is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptPhase {
    /// Waiting for the user to enter the password.
    Prompting,
    /// The password was accepted.
    Succeeded,
    /// The user dismissed the prompt.
    Cancelled,
    /// The user chose the fallback action instead of the password.
    FallbackChosen,
    /// Every attempt failed.
    LockedOut,
}

/// A device-owner prompt for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthPrompt {
    reason: String,
    allow_fallback: bool,
    max_attempts: u32,
    failed: u32,
    phase: PromptPhase,
}

impl AuthPrompt {
    /// Creates a prompt that shows `reason`. The reason is trimmed and must not be empty.
    pub fn new(reason: &str, allow_fallback: bool, max_attempts: u32) -> Result<Self, PromptError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(PromptError::EmptyReason);
        }
        if max_attempts == 0 {
            return Err(PromptError::NoAttempts);
        }
        Ok(Self {
            reason: reason.to_string(),
            allow_fallback,
            max_attempts,
            failed: 0,
            phase: PromptPhase::Prompting,
        })
    }

    /// The text the sheet shows under its title.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Whether the sheet offers the fallback choice next to the password field.
    pub fn allows_fallback(&self) -> bool {
        self.allow_fallback
    }

    pub fn phase(&self) -> &PromptPhase {
        &self.phase
    }

    /// Attempts still available before the lockout.
    pub fn attempts_left(&self) -> u32 {
        self.max_attempts.saturating_sub(self.failed)
    }

    /// Records one password attempt. A correct password succeeds at once. A wrong one counts against the limit, and
    /// the last allowed wrong attempt locks the prompt out.
    pub fn submit(&mut self, password_ok: bool) -> Result<&PromptPhase, PromptError> {
        self.require_prompting()?;
        if password_ok {
            self.phase = PromptPhase::Succeeded;
        } else {
            self.failed += 1;
            if self.failed >= self.max_attempts {
                self.phase = PromptPhase::LockedOut;
            }
        }
        Ok(&self.phase)
    }

    /// The user dismissed the sheet.
    pub fn cancel(&mut self) -> Result<&PromptPhase, PromptError> {
        self.require_prompting()?;
        self.phase = PromptPhase::Cancelled;
        Ok(&self.phase)
    }

    /// The user picked the fallback action. Refused when the prompt did not offer it.
    pub fn choose_fallback(&mut self) -> Result<&PromptPhase, PromptError> {
        self.require_prompting()?;
        if !self.allow_fallback {
            return Err(PromptError::FallbackNotAllowed);
        }
        self.phase = PromptPhase::FallbackChosen;
        Ok(&self.phase)
    }

    fn require_prompting(&self) -> Result<(), PromptError> {
        if self.phase == PromptPhase::Prompting {
            Ok(())
        } else {
            Err(PromptError::NotPrompting)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_password_succeeds_on_first_try() {
        let mut p = AuthPrompt::new("Unlock the vault", false, DEFAULT_MAX_ATTEMPTS).unwrap();
        assert_eq!(p.submit(true), Ok(&PromptPhase::Succeeded));
        assert_eq!(p.attempts_left(), DEFAULT_MAX_ATTEMPTS);
    }

    #[test]
    fn wrong_passwords_count_down_then_lock_out() {
        let mut p = AuthPrompt::new("Unlock", false, 3).unwrap();
        p.submit(false).unwrap();
        assert_eq!(p.attempts_left(), 2);
        p.submit(false).unwrap();
        assert_eq!(p.phase(), &PromptPhase::Prompting);
        assert_eq!(p.submit(false), Ok(&PromptPhase::LockedOut));
        assert_eq!(p.attempts_left(), 0);
        assert_eq!(p.submit(true), Err(PromptError::NotPrompting));
    }

    #[test]
    fn cancel_ends_the_prompt() {
        let mut p = AuthPrompt::new("Unlock", false, 3).unwrap();
        assert_eq!(p.cancel(), Ok(&PromptPhase::Cancelled));
        assert_eq!(p.cancel(), Err(PromptError::NotPrompting));
    }

    #[test]
    fn fallback_only_when_offered() {
        let mut refused = AuthPrompt::new("Unlock", false, 3).unwrap();
        assert_eq!(refused.choose_fallback(), Err(PromptError::FallbackNotAllowed));
        assert_eq!(refused.phase(), &PromptPhase::Prompting);

        let mut offered = AuthPrompt::new("Unlock", true, 3).unwrap();
        assert!(offered.allows_fallback());
        assert_eq!(offered.choose_fallback(), Ok(&PromptPhase::FallbackChosen));
    }

    #[test]
    fn reason_is_trimmed_and_required() {
        assert_eq!(AuthPrompt::new("   ", false, 3), Err(PromptError::EmptyReason));
        assert_eq!(AuthPrompt::new("", false, 3), Err(PromptError::EmptyReason));
        assert_eq!(AuthPrompt::new(" Sign in ", false, 3).unwrap().reason(), "Sign in");
    }

    #[test]
    fn zero_attempts_is_refused() {
        assert_eq!(AuthPrompt::new("Unlock", false, 0), Err(PromptError::NoAttempts));
    }
}
