//! The OBEX object push client over BlueZ's obexd, on the session bus. obexd runs the OBEX protocol and the RFCOMM or
//! L2CAP link itself; this module asks it to open a session to a device, send one file, read the transfer's progress
//! and cancel it. The calls block, so only the worker thread uses them.
//!
//! The shell's own radio code (`springboard/src/bare/bluetooth.rs`) talks to the kernel's management socket, not to
//! BlueZ, so file transfer is the one part of this app that goes over D-Bus.

use crate::model::{parse_status, Status};
use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

/// The well-known name obexd registers on the session bus.
const SERVICE: &str = "org.bluez.obex";
const ROOT: &str = "/org/bluez/obex";
const CLIENT: &str = "org.bluez.obex.Client1";
const PUSH: &str = "org.bluez.obex.ObjectPush1";
const TRANSFER: &str = "org.bluez.obex.Transfer1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// A connection to obexd. Made once by the worker and kept for every transfer.
pub struct Obexd {
    bus: Connection,
}

impl Obexd {
    /// Connects to the session bus, where obexd lives.
    pub fn connect() -> Result<Obexd, String> {
        Connection::session().map(|bus| Obexd { bus }).map_err(|e| format!("No session bus: {e}"))
    }

    /// Opens an object push session to the device at `address` ("AA:BB:CC:DD:EE:FF"). Returns the session's path.
    pub fn create_session(&self, address: &str) -> Result<String, String> {
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("Target", Value::from("opp"));
        let reply = self
            .bus
            .call_method(Some(SERVICE), ROOT, Some(CLIENT), "CreateSession", &(address, options))
            .map_err(explain)?;
        let session: OwnedObjectPath = reply.body().deserialize().map_err(explain)?;
        Ok(session.to_string())
    }

    /// Asks obexd to send the file at the absolute path `file` on `session`. Returns the transfer's path.
    pub fn send_file(&self, session: &str, file: &str) -> Result<String, String> {
        let reply = self.bus.call_method(Some(SERVICE), session, Some(PUSH), "SendFile", &file).map_err(explain)?;
        let (transfer, _properties): (OwnedObjectPath, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(explain)?;
        Ok(transfer.to_string())
    }

    /// The bytes sent so far and the status of `transfer`.
    pub fn progress(&self, transfer: &str) -> Result<(u64, Status), String> {
        let sent: u64 = self.property(transfer, "Transferred")?;
        let status: String = self.property(transfer, "Status")?;
        Ok((sent, parse_status(&status)))
    }

    /// Stops `transfer`. obexd reports it as an error afterwards.
    pub fn cancel(&self, transfer: &str) -> Result<(), String> {
        self.bus.call_method(Some(SERVICE), transfer, Some(TRANSFER), "Cancel", &()).map(|_| ()).map_err(explain)
    }

    /// Closes `session` and everything in it.
    pub fn remove_session(&self, session: &str) -> Result<(), String> {
        let path = ObjectPath::try_from(session).map_err(explain)?;
        self.bus.call_method(Some(SERVICE), ROOT, Some(CLIENT), "RemoveSession", &path).map(|_| ()).map_err(explain)
    }

    /// One property of a transfer, read through `org.freedesktop.DBus.Properties.Get`.
    fn property<T>(&self, transfer: &str, name: &str) -> Result<T, String>
    where
        T: TryFrom<OwnedValue>,
        T::Error: std::fmt::Display,
    {
        let reply = self
            .bus
            .call_method(Some(SERVICE), transfer, Some(PROPERTIES), "Get", &(TRANSFER, name))
            .map_err(explain)?;
        let value: OwnedValue = reply.body().deserialize().map_err(explain)?;
        T::try_from(value).map_err(|e| format!("Bad {name} from the transfer: {e}"))
    }
}

/// The bus error as a sentence. A missing obexd is the common case, so it is named.
fn explain<E: std::fmt::Display>(error: E) -> String {
    let text = error.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        "The file transfer service (obexd) is not running".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_obexd_is_named_in_words() {
        assert_eq!(explain("org.freedesktop.DBus.Error.ServiceUnknown: name not activatable"), "The file transfer service (obexd) is not running");
        assert_eq!(explain("something else"), "something else");
    }
}
