//! Bonjour advertising through Avahi, for the service a `network_listener::Listener` names.
//!
//! The installer enables `avahi-daemon`, which answers mDNS for the whole machine. `Publication::start`
//! runs `avahi-publish-service` with the service's instance name, type and TXT record, and the port the
//! listener is bound to. The publisher keeps the record alive while it runs; dropping the `Publication`
//! stops it, which withdraws the service from the network. The arguments are built by [`publish_args`],
//! which is pure, so the wire form is tested without the daemon.

use crate::network_listener::Service;
use bifrost_core::txt_record::Presence;
use std::io;
use std::process::{Child, Command, Stdio};

/// The program that publishes a service through the Avahi daemon.
pub const PUBLISHER: &str = "avahi-publish-service";

/// The arguments for `avahi-publish-service`: the instance name, the type, the port, then one argument per TXT
/// entry (`key=value`, a bare `key` for an entry with no `=`, and `key=` for an empty value). The domain is
/// not passed: Avahi publishes in `.local`, the only domain a Bonjour listener on this machine serves.
pub fn publish_args(service: &Service, port: u16) -> Vec<String> {
    let mut args = vec![service.name.clone(), service.kind.clone(), port.to_string()];
    for index in 0..service.txt.len() {
        let Some((key, presence)) = service.txt.item_at(index) else { continue };
        match presence {
            Presence::Absent => {}
            Presence::NoValue => args.push(key.to_string()),
            Presence::Empty => args.push(format!("{key}=")),
            Presence::Value(bytes) => args.push(format!("{key}={}", String::from_utf8_lossy(bytes))),
        }
    }
    args
}

/// A running publication. Dropping it stops the publisher and withdraws the service.
#[derive(Debug)]
pub struct Publication {
    child: Child,
}

impl Publication {
    /// Starts publishing `service` on `port` through the Avahi daemon.
    pub fn start(service: &Service, port: u16) -> io::Result<Publication> {
        spawn(PUBLISHER, &publish_args(service, port))
    }

    /// True while the publisher process is still running. A publisher that exited (the daemon is not running,
    /// or the name is taken) reads as false.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stops the publisher now. Same as dropping the publication.
    pub fn stop(mut self) {
        self.kill();
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Runs `program` with `args` and no terminal input or output.
fn spawn(program: &str, args: &[String]) -> io::Result<Publication> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Publication { child })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::txt_record::TxtRecord;

    #[test]
    fn args_carry_the_name_type_port_and_txt_entries() {
        let mut txt = TxtRecord::new();
        txt.set("path", Some(b"/share")).unwrap();
        txt.set("flag", None).unwrap();
        txt.set("empty", Some(b"")).unwrap();
        let svc = Service::new("Den Share", "_http._tcp", "", txt);
        let args = publish_args(&svc, 8080);
        assert_eq!(args[..3], ["Den Share", "_http._tcp", "8080"]);
        assert!(args.contains(&"path=/share".to_string()));
        assert!(args.contains(&"flag".to_string()));
        assert!(args.contains(&"empty=".to_string()));
        assert_eq!(args.len(), 6);
    }

    #[test]
    fn a_service_without_txt_has_three_arguments() {
        let svc = Service::new("Den", "_ssh._tcp", "local", TxtRecord::new());
        assert_eq!(publish_args(&svc, 22), vec!["Den", "_ssh._tcp", "22"]);
    }

    #[test]
    fn a_missing_publisher_is_an_error_not_a_panic() {
        assert!(spawn("/nonexistent/avahi-publish-service", &[]).is_err());
    }

    #[test]
    fn a_publisher_that_exits_reads_as_stopped() {
        let mut p = spawn("true", &[]).unwrap();
        p.child.wait().unwrap();
        assert!(!p.is_running());
    }
}
