//! Sending a message: one SMTP submission (RFC 5321, with AUTH PLAIN from RFC 4616 and STARTTLS when the account asks for
//! it), over the shell's TLS transport. The reply rules and command lines come from [`crate::smtp`]; this module runs
//! the dialogue and stops at the first reply that is not the expected one.

use bifrost_app::controls::internet_accounts::{Security, Server};
use crate::line::Lines;
use crate::smtp;
use crate::transport;
use springboard::deildu::http::{Stream, Transport};
use std::io::{self, Read, Write};

/// How many read timeouts the wait for the server's answer to the end of the data outlasts. The transport's read timeout
/// is 25 seconds, so this is about five minutes: a server may scan a large attachment or hold a first message back
/// (greylisting) before it says 250.
const DATA_PATIENCE: u32 = 12;

/// What an error starts with when the message was handed over but the server never confirmed it. The message may have
/// been delivered, so it is not sent again by itself.
pub const UNCONFIRMED: &str = "The server did not confirm the message";

/// True when `error` (from [`send`]) says the outcome is unknown: the message may have been delivered.
pub fn is_unconfirmed(error: &str) -> bool {
    error.starts_with(UNCONFIRMED)
}

/// Sends `message` (CRLF lines, as [`crate::message::Draft::render`] makes it) from `from` to `recipients`.
/// `login` and `password` are the account's SMTP credentials.
pub fn send(
    transport: &dyn Transport,
    server: &Server,
    login: &str,
    password: &str,
    from: &str,
    recipients: &[String],
    message: &str,
) -> Result<(), String> {
    let domain = from.rsplit_once('@').map_or("localhost", |(_, domain)| domain);
    let mut lines = greet(transport, server, domain)?;
    converse(&mut lines, login, password, from, recipients, message)
}

/// Opens the connection, reads the greeting, says EHLO, and upgrades to TLS when the account asks for STARTTLS.
fn greet(transport: &dyn Transport, server: &Server, domain: &str) -> Result<Lines<Box<dyn Stream>>, String> {
    let stream = transport::open(transport, server).map_err(io_text)?;
    let mut lines = Lines::new(stream);
    expect(&mut lines, &[220])?;
    put(&mut lines, &smtp::ehlo(domain))?;
    expect(&mut lines, &[250])?;
    if server.security == Security::StartTls {
        put(&mut lines, "STARTTLS\r\n")?;
        expect(&mut lines, &[220])?;
        let secure = transport.upgrade(&server.host, lines.into_inner()).map_err(io_text)?;
        let mut secure = Lines::new(secure);
        put(&mut secure, &smtp::ehlo(domain))?;
        expect(&mut secure, &[250])?;
        Ok(secure)
    } else {
        Ok(lines)
    }
}

/// Finds out whether the outgoing server takes the login (Mailbox > Check Connection): connects, signs in, and quits.
/// Nothing is sent. `domain` is what EHLO announces.
pub fn check(transport: &dyn Transport, server: &Server, login: &str, password: &str, domain: &str) -> Result<(), String> {
    let mut lines = greet(transport, server, domain)?;
    authenticate(&mut lines, login, password)?;
    let _ = put(&mut lines, &smtp::quit());
    Ok(())
}

/// AUTH PLAIN, and the server's answer to it.
fn authenticate<S: Read + Write>(lines: &mut Lines<S>, login: &str, password: &str) -> Result<(), String> {
    put(lines, &smtp::auth_plain(login, password))?;
    expect(lines, &[235]).map(|_| ())
}

/// The commands after the greeting and EHLO: authentication, the envelope, the data, and QUIT.
pub fn converse<S: Read + Write>(
    lines: &mut Lines<S>,
    login: &str,
    password: &str,
    from: &str,
    recipients: &[String],
    message: &str,
) -> Result<(), String> {
    authenticate(lines, login, password)?;
    put(lines, &smtp::mail_from(from))?;
    expect(lines, &[250])?;
    for to in recipients {
        put(lines, &smtp::rcpt_to(to))?;
        expect(lines, &[250, 251])?;
    }
    put(lines, &smtp::data())?;
    expect(lines, &[354])?;
    put(lines, &smtp::data_payload(message))?;
    // From here the server has the whole message. A timeout is not a refusal: the outcome is unknown.
    let (code, text) = reply_within(lines, DATA_PATIENCE).map_err(|e| format!("{UNCONFIRMED} ({e}). It may have been sent; check Sent Items before sending it again"))?;
    if code != 250 {
        return Err(format!("{code} {text}"));
    }
    let _ = put(lines, &smtp::quit());
    Ok(())
}

fn put<S: Read + Write>(lines: &mut Lines<S>, text: &str) -> Result<(), String> {
    lines.send(text).map_err(io_text)
}

/// Reads one complete reply (all its lines) and checks its code against `codes`. Returns the reply text on success.
fn expect<S: Read + Write>(lines: &mut Lines<S>, codes: &[u16]) -> Result<String, String> {
    let (code, text) = reply(lines)?;
    if codes.contains(&code) {
        Ok(text)
    } else {
        Err(format!("{code} {text}"))
    }
}

fn reply<S: Read + Write>(lines: &mut Lines<S>) -> Result<(u16, String), String> {
    reply_within(lines, 0)
}

/// Like [`reply`], and a read timeout is waited out up to `patience` times before it is an error.
fn reply_within<S: Read + Write>(lines: &mut Lines<S>, patience: u32) -> Result<(u16, String), String> {
    let mut stalls = 0;
    loop {
        let line = match lines.read_line() {
            Ok(line) => line,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) && stalls < patience => {
                stalls += 1;
                continue;
            }
            Err(e) => return Err(io_text(e)),
        };
        let parsed = smtp::parse_reply_line(&line).ok_or_else(|| format!("unexpected server line: {line}"))?;
        if !parsed.more {
            return Ok((parsed.code, parsed.text));
        }
    }
}

fn io_text(e: io::Error) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::script::{Script, Stalling};

    fn recipients() -> Vec<String> {
        vec!["c@d.is".to_string()]
    }

    #[test]
    fn a_message_goes_through_auth_envelope_and_data() {
        let script = "235 authenticated\r\n250 sender ok\r\n250 recipient ok\r\n354 go ahead\r\n250 queued\r\n221 bye\r\n";
        let mut lines = Lines::new(Script::new(script));
        converse(&mut lines, "user", "pw", "a@b.is", &recipients(), "Subject: x\r\n\r\n.dot\r\n").unwrap();
        let sent = lines.into_inner().sent();
        assert!(sent.starts_with("AUTH PLAIN AHVzZXIAcHc=\r\nMAIL FROM:<a@b.is>\r\nRCPT TO:<c@d.is>\r\nDATA\r\n"), "{sent}");
        assert!(sent.ends_with("\r\n..dot\r\n.\r\nQUIT\r\n"), "{sent}");
    }

    #[test]
    fn a_slow_acceptance_after_the_data_is_waited_for() {
        let script = vec![Some("235 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n"), None, None, None, Some("250 queued\r\n")];
        let mut lines = Lines::new(Stalling::new(script));
        converse(&mut lines, "u", "p", "a@b.is", &recipients(), "x\r\n").unwrap();
    }

    #[test]
    fn no_answer_after_the_data_is_unconfirmed_not_failed() {
        let mut script = vec![Some("235 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n")];
        script.extend(std::iter::repeat(None).take(DATA_PATIENCE as usize + 1));
        let mut lines = Lines::new(Stalling::new(script));
        let err = converse(&mut lines, "u", "p", "a@b.is", &recipients(), "x\r\n").unwrap_err();
        assert!(is_unconfirmed(&err), "{err}");
        let mut refused = Lines::new(Script::new("235 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n554 rejected\r\n"));
        let err = converse(&mut refused, "u", "p", "a@b.is", &recipients(), "x\r\n").unwrap_err();
        assert_eq!(err, "554 rejected");
        assert!(!is_unconfirmed(&err));
    }

    #[test]
    fn a_connection_check_signs_in_and_quits_without_sending() {
        let mut lines = Lines::new(Script::new("235 authenticated\r\n"));
        authenticate(&mut lines, "user", "pw").unwrap();
        let sent = lines.into_inner().sent();
        assert_eq!(sent, "AUTH PLAIN AHVzZXIAcHc=\r\n");
        let mut refused = Lines::new(Script::new("535 5.7.8 bad credentials\r\n"));
        assert_eq!(authenticate(&mut refused, "user", "x").unwrap_err(), "535 5.7.8 bad credentials");
    }

    #[test]
    fn a_refused_login_stops_the_dialogue() {
        let mut lines = Lines::new(Script::new("535 5.7.8 bad credentials\r\n"));
        let err = converse(&mut lines, "user", "pw", "a@b.is", &recipients(), "x\r\n").unwrap_err();
        assert_eq!(err, "535 5.7.8 bad credentials");
        assert!(!lines.into_inner().sent().contains("MAIL FROM"));
    }

    #[test]
    fn a_refused_recipient_is_reported_with_its_code() {
        let script = "235 ok\r\n250 ok\r\n550 no such user\r\n";
        let mut lines = Lines::new(Script::new(script));
        assert_eq!(converse(&mut lines, "u", "p", "a@b.is", &recipients(), "x\r\n").unwrap_err(), "550 no such user");
    }

    #[test]
    fn multi_line_replies_are_read_to_the_last_line() {
        let mut lines = Lines::new(Script::new("250-mail.example\r\n250-SIZE 100\r\n250 AUTH PLAIN\r\n"));
        assert_eq!(expect(&mut lines, &[250]).unwrap(), "AUTH PLAIN");
    }
}
