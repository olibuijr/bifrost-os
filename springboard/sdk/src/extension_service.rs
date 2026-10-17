//! The extension side of the extension host (`bifrost_app::extension` declares and discovers extensions; the shell's
//! `extension_host` launches and calls them).
//!
//! The host starts the package's own app binary in extension mode: it sets `BIFROST_EXTENSION_POINT` to the point the
//! extension serves and `BIFROST_CONTROL` to the socket it will talk to. The binary checks [`requested`] before it
//! starts its window and, in extension mode, runs [`serve`] instead of `bifrost_app::run`:
//!
//! ```ignore
//! fn main() {
//!     if bifrost_app::extension_service::requested().is_some() {
//!         let mut provider = FileProviderService(MyDrive::open());
//!         bifrost_app::extension_service::serve(&mut provider).ok();
//!     } else {
//!         bifrost_app::run(MyApp::new())
//!     }
//! }
//! ```
//!
//! [`serve`] answers one JSON-RPC request per connection on the control socket until the host sends
//! `extension.shutdown` or the host process is gone. [`FileProviderService`] is the [`Service`] of the
//! `is.bifrost.file-provider` point: it decodes `provider.enumerate` and `provider.fetch`
//! (`bifrost_ui::file_provider_wire`) and calls a [`Provider`]. No window is made and no shell channel is opened in
//! extension mode, so an extension cannot draw; presenting an extension's scene is `remote_view`, which this host does
//! not use yet.

use crate::control::{Control, RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::file_provider::{ItemIdentifier, ProvidedItem, ProviderError};
use bifrost_core::json::Json;
pub use bifrost_ui::file_provider_wire as wire;
use std::io;
use std::path::Path;
use std::time::Duration;

/// Set by the host: the extension point this process serves. Its presence is what makes the binary an extension.
pub const ENV_POINT: &str = "BIFROST_EXTENSION_POINT";
/// The socket to serve on (the same variable app control uses).
pub const ENV_CONTROL: &str = "BIFROST_CONTROL";
/// Answered by every extension: `{point}` is not needed, the reply only proves the process is up.
pub const PING: &str = "extension.ping";
/// The host is done with this process: [`serve`] returns after replying.
pub const SHUTDOWN: &str = "extension.shutdown";
/// The server error code of a provider failure; the message is the provider error code name.
pub const PROVIDER_ERROR: i64 = -32010;

/// The extension point this process was started for, or None when it is the app itself.
pub fn requested() -> Option<String> {
    std::env::var(ENV_POINT).ok().filter(|p| !p.is_empty())
}

/// What an extension answers beyond ping and shutdown.
pub trait Service {
    fn call(&mut self, method: &str, params: &Json) -> Result<Json, RpcError>;
}

/// Serves `service` on `path` until `extension.shutdown`. With `watch_parent` the loop also ends when the process that
/// started it has gone, so an extension never outlives a host that crashed.
pub fn serve_at(path: &Path, service: &mut dyn Service, watch_parent: bool) -> io::Result<()> {
    let mut control = Control::bind(path)?;
    let parent = std::os::unix::process::parent_id();
    let mut quit = false;
    while !quit {
        control.service(&mut |method: &str, params: &Json| match method {
            PING => Ok(Json::Bool(true)),
            SHUTDOWN => {
                quit = true;
                Ok(Json::Bool(true))
            }
            _ => service.call(method, params),
        });
        if watch_parent && std::os::unix::process::parent_id() != parent {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// [`serve_at`] on the socket the host named in `BIFROST_CONTROL`, watching the host.
pub fn serve(service: &mut dyn Service) -> io::Result<()> {
    let path = std::env::var_os(ENV_CONTROL).ok_or_else(|| io::Error::other("BIFROST_CONTROL is not set; extensions are started by a host"))?;
    serve_at(Path::new(&path), service, true)
}

/// A file provider's storage: what a domain holds and how to get one item's bytes.
pub trait Provider {
    /// The items directly inside `container` (`ItemIdentifier::root_container()` for the top).
    fn enumerate(&mut self, container: &ItemIdentifier) -> Result<Vec<ProvidedItem>, ProviderError>;
    /// Writes the bytes of `item` to the file `dest`.
    fn fetch(&mut self, item: &ItemIdentifier, dest: &Path) -> Result<(), ProviderError>;
}

fn rpc_error(e: ProviderError) -> RpcError {
    RpcError::new(PROVIDER_ERROR, e.code.as_str()).with_reason(e.to_string())
}

/// The [`Service`] of a file provider extension.
pub struct FileProviderService<P: Provider>(pub P);

impl<P: Provider> Service for FileProviderService<P> {
    fn call(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        match method {
            wire::ENUMERATE => {
                let container = wire::container_from(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "container is required"))?;
                self.0.enumerate(&container).map(|items| wire::items_result(&items)).map_err(rpc_error)
            }
            wire::FETCH => {
                let (item, dest) = wire::fetch_from(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "id and dest are required"))?;
                self.0.fetch(&item, Path::new(&dest)).map(|()| wire::fetch_result(&dest)).map_err(rpc_error)
            }
            _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("unknown method {method}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::handle_line;
    use crate::file_provider::ErrorCode;
    use bifrost_core::json::parse;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    /// Two items under the root: a folder and a placeholder document of 5 bytes.
    struct Memory;

    impl Provider for Memory {
        fn enumerate(&mut self, container: &ItemIdentifier) -> Result<Vec<ProvidedItem>, ProviderError> {
            let root = ItemIdentifier::root_container();
            if container == &root {
                let folder = wire::as_folder(ProvidedItem::new(ItemIdentifier::new("f1").unwrap(), root.clone(), "Reports").unwrap());
                let doc = ProvidedItem::new(ItemIdentifier::new("d1").unwrap(), root, "Plan.txt")
                    .unwrap()
                    .with_document_size(5)
                    .with_capabilities(crate::file_provider::ItemCapabilities::READING)
                    .as_placeholder();
                Ok(vec![folder, doc])
            } else if container.as_str() == "f1" {
                Ok(Vec::new())
            } else {
                Err(ProviderError::for_item(ErrorCode::NoSuchItem, container.clone()))
            }
        }
        fn fetch(&mut self, item: &ItemIdentifier, dest: &Path) -> Result<(), ProviderError> {
            if item.as_str() != "d1" {
                return Err(ProviderError::for_item(ErrorCode::NoSuchItem, item.clone()));
            }
            std::fs::write(dest, b"hello").map_err(|_| ProviderError::new(ErrorCode::CannotSynchronize))
        }
    }

    fn ask(service: &mut dyn Service, method: &str, params: Json) -> Json {
        let line = wire::request_line(method, params);
        parse(&handle_line(line.trim(), &mut |m, p| service.call(m, p))).unwrap()
    }

    #[test]
    fn enumerate_lists_a_container_and_names_a_missing_one() {
        let mut service = FileProviderService(Memory);
        let reply = ask(&mut service, wire::ENUMERATE, wire::enumerate_params(&ItemIdentifier::root_container()));
        let items = wire::items_from(reply.get("result").unwrap()).unwrap();
        assert_eq!(items.len(), 2);
        assert!(wire::is_folder(&items[0]));
        assert_eq!(items[1].filename(), "Plan.txt");

        let missing = ask(&mut service, wire::ENUMERATE, wire::enumerate_params(&ItemIdentifier::new("zzz").unwrap()));
        let error = missing.get("error").unwrap();
        assert_eq!(error.get("code").and_then(Json::number), Some(PROVIDER_ERROR as f64));
        assert_eq!(error.str_field("message").as_deref(), Some("noSuchItem"));
    }

    #[test]
    fn fetch_writes_the_bytes_where_the_host_said() {
        let dir = std::env::temp_dir().join(format!("ext-svc-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("Plan.txt");
        let mut service = FileProviderService(Memory);
        let reply = ask(&mut service, wire::FETCH, wire::fetch_params(&ItemIdentifier::new("d1").unwrap(), dest.to_str().unwrap()));
        assert_eq!(wire::path_from(reply.get("result").unwrap()).as_deref(), dest.to_str());
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_params_and_unknown_methods_are_refused() {
        let mut service = FileProviderService(Memory);
        let bad = ask(&mut service, wire::ENUMERATE, Json::Obj(vec![]));
        assert_eq!(bad.get("error").and_then(|e| e.get("code")).and_then(Json::number), Some(INVALID_PARAMS as f64));
        let unknown = ask(&mut service, "provider.delete", Json::Obj(vec![]));
        assert_eq!(unknown.get("error").and_then(|e| e.get("code")).and_then(Json::number), Some(METHOD_NOT_FOUND as f64));
    }

    #[test]
    fn serve_answers_over_the_socket_until_shutdown() {
        let dir = std::env::temp_dir().join(format!("ext-svc-sock-{}", std::process::id()));
        let path = dir.join("e.sock");
        let server_path = path.clone();
        let server = std::thread::spawn(move || serve_at(&server_path, &mut FileProviderService(Memory), false));

        let call = |line: String| -> String {
            let mut stream = loop {
                match UnixStream::connect(&path) {
                    Ok(s) => break s,
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            };
            stream.write_all(line.as_bytes()).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            reply
        };
        let ping = parse(call(wire::request_line(PING, Json::Obj(vec![]))).trim()).unwrap();
        assert_eq!(ping.get("result"), Some(&Json::Bool(true)));
        let listed = parse(call(wire::request_line(wire::ENUMERATE, wire::enumerate_params(&ItemIdentifier::root_container()))).trim()).unwrap();
        assert_eq!(wire::items_from(listed.get("result").unwrap()).unwrap().len(), 2);
        call(wire::request_line(SHUTDOWN, Json::Obj(vec![])));
        server.join().unwrap().unwrap();
        assert!(!path.exists(), "the socket is removed when the extension stops");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
