//! Session events read from authentication log lines (parity row "Login, logout, su, sudo, SSH and screen-sharing
//! events"). The journal already records these lines through the system's sshd, sudo, login and su PAM stacks. This
//! module turns the text of one line into a typed record so a future event feed can publish it. It reads no file,
//! journal or socket itself, and it reports nothing for a line it does not recognise.

/// The PAM service that opened or closed a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Login,
    Su,
}

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::Login => "login",
            Service::Su => "su",
        }
    }
}

/// One session event. Every variant names the user the event concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// A command that ran through sudo. `target` is the user it ran as.
    Sudo { user: String, target: String, tty: String, command: String },
    /// An SSH authentication attempt, accepted or failed.
    SshAuth { user: String, from: String, method: String, accepted: bool },
    /// A login or su session that opened or closed.
    Session { service: Service, user: String, opened: bool },
}

impl SessionEvent {
    pub fn user(&self) -> &str {
        match self {
            SessionEvent::Sudo { user, .. }
            | SessionEvent::SshAuth { user, .. }
            | SessionEvent::Session { user, .. } => user.as_str(),
        }
    }
}

/// Parses one log line, with or without the journal's timestamp and host prefix. Returns `None` for other
/// processes, for sudo's failed password attempts, and for lines that do not match the expected shape.
pub fn parse(line: &str) -> Option<SessionEvent> {
    let (head, body) = line.split_once(": ")?;
    let tag = head.rsplit(' ').next()?;
    let process = tag.split('[').next()?;
    match process {
        "sshd" => parse_ssh(body),
        "sudo" => parse_sudo(body),
        "login" => parse_session(body, Service::Login),
        "su" => parse_session(body, Service::Su),
        _ => None,
    }
}

/// `alice : TTY=pts/0 ; PWD=/home/alice ; USER=root ; COMMAND=/usr/bin/ls -la`. A failed password attempt has no
/// `TTY=` field right after the user, so it is not a command event.
fn parse_sudo(body: &str) -> Option<SessionEvent> {
    let mut parts = body.split(" ; ");
    let (user, first) = parts.next()?.split_once(" : ")?;
    let tty = first.strip_prefix("TTY=")?;
    let mut target = None;
    let mut command = None;
    for field in parts {
        if let Some(v) = field.strip_prefix("USER=") {
            target = Some(v.to_string());
        } else if let Some(v) = field.strip_prefix("COMMAND=") {
            command = Some(v.to_string());
        }
    }
    Some(SessionEvent::Sudo {
        user: user.to_string(),
        target: target?,
        tty: tty.to_string(),
        command: command?,
    })
}

/// `Accepted publickey for alice from 10.0.0.5 port 51234 ssh2: ...` or
/// `Failed password for invalid user bob from 192.168.1.9 port 22 ssh2`.
fn parse_ssh(body: &str) -> Option<SessionEvent> {
    let (accepted, rest) = if let Some(r) = body.strip_prefix("Accepted ") {
        (true, r)
    } else {
        (false, body.strip_prefix("Failed ")?)
    };
    let (method, rest) = rest.split_once(" for ")?;
    let rest = rest.strip_prefix("invalid user ").unwrap_or(rest);
    let (user, rest) = rest.split_once(" from ")?;
    let from = rest.split_whitespace().next()?;
    if user.is_empty() {
        return None;
    }
    Some(SessionEvent::SshAuth {
        user: user.to_string(),
        from: from.to_string(),
        method: method.to_string(),
        accepted,
    })
}

/// `pam_unix(login:session): session opened for user alice(uid=1000) by LOGIN(uid=0)`.
fn parse_session(body: &str, service: Service) -> Option<SessionEvent> {
    let prefix = format!("pam_unix({}:session): ", service.name());
    let rest = body.strip_prefix(prefix.as_str())?;
    let (opened, rest) = if let Some(r) = rest.strip_prefix("session opened for user ") {
        (true, r)
    } else {
        (false, rest.strip_prefix("session closed for user ")?)
    };
    let user = rest.split(['(', ' ']).next().unwrap_or("");
    if user.is_empty() {
        return None;
    }
    Some(SessionEvent::Session { service, user: user.to_string(), opened })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sudo_command_line_gives_the_user_target_tty_and_command() {
        let line = "Oct 12 10:00:00 host sudo: alice : TTY=pts/0 ; PWD=/home/alice ; USER=root ; COMMAND=/usr/bin/ls -la";
        assert_eq!(
            parse(line),
            Some(SessionEvent::Sudo {
                user: "alice".into(),
                target: "root".into(),
                tty: "pts/0".into(),
                command: "/usr/bin/ls -la".into(),
            })
        );
    }

    #[test]
    fn a_failed_sudo_password_attempt_is_not_a_command_event() {
        let line = "sudo: alice : 3 incorrect password attempts ; TTY=pts/0 ; PWD=/x ; USER=root ; COMMAND=/bin/sh";
        assert_eq!(parse(line), None);
    }

    #[test]
    fn an_accepted_ssh_login_gives_the_method_and_address() {
        let line = "sshd[1234]: Accepted publickey for alice from 10.0.0.5 port 51234 ssh2: ED25519 SHA256:abc";
        assert_eq!(
            parse(line),
            Some(SessionEvent::SshAuth {
                user: "alice".into(),
                from: "10.0.0.5".into(),
                method: "publickey".into(),
                accepted: true,
            })
        );
    }

    #[test]
    fn a_failed_ssh_login_for_an_unknown_user_strips_the_invalid_user_words() {
        let line = "sshd[99]: Failed password for invalid user bob from 192.168.1.9 port 22 ssh2";
        let event = parse(line).unwrap();
        assert_eq!(event.user(), "bob");
        assert_eq!(
            event,
            SessionEvent::SshAuth {
                user: "bob".into(),
                from: "192.168.1.9".into(),
                method: "password".into(),
                accepted: false,
            }
        );
    }

    #[test]
    fn login_and_su_sessions_open_and_close() {
        let open = "login[812]: pam_unix(login:session): session opened for user alice(uid=1000) by LOGIN(uid=0)";
        assert_eq!(
            parse(open),
            Some(SessionEvent::Session { service: Service::Login, user: "alice".into(), opened: true })
        );
        let close = "su[77]: pam_unix(su:session): session closed for user root";
        assert_eq!(
            parse(close),
            Some(SessionEvent::Session { service: Service::Su, user: "root".into(), opened: false })
        );
        assert_eq!(Service::Su.name(), "su");
    }

    #[test]
    fn unrelated_lines_and_mismatched_services_are_ignored() {
        assert_eq!(parse("sshd[1]: Server listening on 0.0.0.0 port 22."), None);
        assert_eq!(parse("cron[4]: (root) CMD (run-parts /etc/cron.hourly)"), None);
        assert_eq!(parse("su[1]: pam_unix(login:session): session opened for user alice(uid=1000)"), None);
        assert_eq!(parse("login[1]: pam_unix(login:session): session opened for user (uid=0)"), None);
        assert_eq!(parse("no separator here"), None);
    }
}
