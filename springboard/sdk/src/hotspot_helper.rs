//! Hotspot helper for apps: an app registers for the Wi-Fi hotspot commands the system sends it, receives them one at a
//! time in order, and answers each with an outcome. This is the helper side of hotspot sign-in: the system asks the
//! helper to evaluate a network, to authenticate to it (for example a captive login page), to keep it alive, or to log
//! off.
//!
//! This is a pure model with no radio and no transport. The shell delivers each command with [`Helper::deliver`] and
//! takes it back with [`Helper::take_command`]. [`Helper::respond`] checks the answer against the command kind, so an
//! app cannot report a confidence for a command that does not carry one.

use std::collections::VecDeque;
use std::fmt;

/// The kind of hotspot command a helper can register for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// The system asks how likely the helper is to sign the device in to this network.
    Evaluate,
    /// The system asks the helper to sign the device in.
    Authenticate,
    /// The system asks the helper to keep an existing sign-in alive.
    Maintain,
    /// The system asks the helper to end the sign-in.
    Logoff,
}

/// How sure the helper is that it can sign in to a network, sent with an evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    None,
    Low,
    High,
}

/// The outcome an app reports for a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
    Unsupported,
}

/// One command from the system. `ssid` names the network the command is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub kind: CommandKind,
    pub ssid: String,
}

/// The helper's answer to one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub kind: CommandKind,
    pub ssid: String,
    pub outcome: Outcome,
    pub confidence: Confidence,
}

/// Why a registration, delivery or response was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `register` was given no command kinds.
    NoCommandKinds,
    /// `register` was given an empty name.
    EmptyName,
    /// The command names no network.
    EmptySsid,
    /// The command kind was not in the registration.
    NotRegistered,
    /// A confidence was given for a command that does not carry one, or for a failed evaluation.
    ConfidenceNotAllowed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::NoCommandKinds => "A hotspot helper must register for at least one command.",
            Error::EmptyName => "A hotspot helper needs a name.",
            Error::EmptySsid => "The hotspot command names no network.",
            Error::NotRegistered => "The helper did not register for this command.",
            Error::ConfidenceNotAllowed => "Only a successful evaluation carries a confidence.",
        })
    }
}

impl std::error::Error for Error {}

/// A registered hotspot helper with its queue of commands the app has not taken yet.
#[derive(Debug)]
pub struct Helper {
    name: String,
    kinds: Vec<CommandKind>,
    pending: VecDeque<Command>,
}

impl Helper {
    /// Registers a helper named `name` for `kinds`. A repeated kind is kept once.
    pub fn register(name: &str, kinds: &[CommandKind]) -> Result<Self, Error> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        let mut registered: Vec<CommandKind> = Vec::new();
        for &kind in kinds {
            if !registered.contains(&kind) {
                registered.push(kind);
            }
        }
        if registered.is_empty() {
            return Err(Error::NoCommandKinds);
        }
        Ok(Self { name: name.to_string(), kinds: registered, pending: VecDeque::new() })
    }

    /// The name the helper registered under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The command kinds the helper registered for.
    pub fn kinds(&self) -> &[CommandKind] {
        &self.kinds
    }

    /// Queues a command from the system. Only registered kinds are accepted, and the network name must not be empty.
    pub fn deliver(&mut self, command: Command) -> Result<(), Error> {
        if command.ssid.trim().is_empty() {
            return Err(Error::EmptySsid);
        }
        if !self.kinds.contains(&command.kind) {
            return Err(Error::NotRegistered);
        }
        self.pending.push_back(command);
        Ok(())
    }

    /// Takes the oldest queued command. Commands come back in the order the system sent them.
    pub fn take_command(&mut self) -> Option<Command> {
        self.pending.pop_front()
    }

    /// The number of commands waiting for the app.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Builds the answer to `command`. Confidence is only for a successful evaluation: any other command, or an
    /// evaluation that does not succeed, must use [`Confidence::None`].
    pub fn respond(&self, command: &Command, outcome: Outcome, confidence: Confidence) -> Result<Response, Error> {
        if !self.kinds.contains(&command.kind) {
            return Err(Error::NotRegistered);
        }
        let evaluated_ok = command.kind == CommandKind::Evaluate && outcome == Outcome::Success;
        if confidence != Confidence::None && !evaluated_ok {
            return Err(Error::ConfidenceNotAllowed);
        }
        Ok(Response { kind: command.kind, ssid: command.ssid.clone(), outcome, confidence })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(kind: CommandKind, ssid: &str) -> Command {
        Command { kind, ssid: ssid.to_string() }
    }

    #[test]
    fn registration_needs_a_name_and_at_least_one_kind() {
        assert_eq!(Helper::register("  ", &[CommandKind::Evaluate]).unwrap_err(), Error::EmptyName);
        assert_eq!(Helper::register("Login", &[]).unwrap_err(), Error::NoCommandKinds);
        let h = Helper::register(" Login ", &[CommandKind::Evaluate, CommandKind::Evaluate, CommandKind::Logoff]).unwrap();
        assert_eq!(h.name(), "Login");
        assert_eq!(h.kinds(), &[CommandKind::Evaluate, CommandKind::Logoff]);
    }

    #[test]
    fn commands_for_unregistered_kinds_or_empty_networks_are_refused() {
        let mut h = Helper::register("Login", &[CommandKind::Authenticate]).unwrap();
        assert_eq!(h.deliver(cmd(CommandKind::Maintain, "Cafe")), Err(Error::NotRegistered));
        assert_eq!(h.deliver(cmd(CommandKind::Authenticate, " ")), Err(Error::EmptySsid));
        assert_eq!(h.pending_count(), 0);
    }

    #[test]
    fn commands_come_back_in_the_order_they_were_sent() {
        let mut h = Helper::register("Login", &[CommandKind::Evaluate, CommandKind::Authenticate]).unwrap();
        h.deliver(cmd(CommandKind::Evaluate, "Cafe")).unwrap();
        h.deliver(cmd(CommandKind::Authenticate, "Hotel")).unwrap();
        assert_eq!(h.pending_count(), 2);
        assert_eq!(h.take_command(), Some(cmd(CommandKind::Evaluate, "Cafe")));
        assert_eq!(h.take_command(), Some(cmd(CommandKind::Authenticate, "Hotel")));
        assert_eq!(h.take_command(), None);
    }

    #[test]
    fn a_successful_evaluation_carries_its_confidence() {
        let h = Helper::register("Login", &[CommandKind::Evaluate]).unwrap();
        let r = h.respond(&cmd(CommandKind::Evaluate, "Cafe"), Outcome::Success, Confidence::High).unwrap();
        assert_eq!(r.confidence, Confidence::High);
        assert_eq!(r.ssid, "Cafe");
        assert_eq!(r.kind, CommandKind::Evaluate);
    }

    #[test]
    fn confidence_is_refused_for_other_commands_and_failed_evaluations() {
        let h = Helper::register("Login", &[CommandKind::Evaluate, CommandKind::Authenticate]).unwrap();
        let auth = cmd(CommandKind::Authenticate, "Cafe");
        assert_eq!(h.respond(&auth, Outcome::Success, Confidence::Low), Err(Error::ConfidenceNotAllowed));
        assert!(h.respond(&auth, Outcome::Success, Confidence::None).is_ok());
        let eval = cmd(CommandKind::Evaluate, "Cafe");
        assert_eq!(h.respond(&eval, Outcome::Failure, Confidence::High), Err(Error::ConfidenceNotAllowed));
        assert!(h.respond(&eval, Outcome::Failure, Confidence::None).is_ok());
    }

    #[test]
    fn a_response_for_an_unregistered_kind_is_refused() {
        let h = Helper::register("Login", &[CommandKind::Logoff]).unwrap();
        assert_eq!(h.respond(&cmd(CommandKind::Maintain, "Cafe"), Outcome::Success, Confidence::None), Err(Error::NotRegistered));
    }
}
