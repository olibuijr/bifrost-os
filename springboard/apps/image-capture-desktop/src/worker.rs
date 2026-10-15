//! Image Capture's disk and scanner work. A serial utility queue runs requests, so a slow scanner or a large copy never
//! stalls the window; replies return on the main queue and wake it without a polling worker.

use crate::model::{self, Reply};
use bifrost_ui::scan_job::ScanSettings;
use bifrost_ui::scanner;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use springboard::shell::camera::devices;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A request for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// List the cameras and SD cards, and the scanners.
    Refresh,
    /// List the pictures of the camera `volume`.
    Contents { volume: String },
    /// Read what the scanner `device` can do.
    Capabilities { device: String },
    /// Copy `paths` into Pictures under `home`.
    Import { paths: Vec<PathBuf>, home: PathBuf },
    /// Scan `device` with `settings`: a preview into the cache, or a page into the scan folder under `home`.
    Scan { device: String, settings: ScanSettings, preview: bool, home: PathBuf },
}

/// The window's end of the channels. Dropping it ends the worker thread.
pub struct Link {
    queue: DispatchQueue,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    enabled: bool,
}

impl Link {
    /// Starts the serial utility queue for disk and scanner requests.
    pub fn start() -> Link {
        Link {
            queue: DispatchQueue::serial_qos("is.olibuijr.imagecapture.work", Qos::Utility),
            replies: Arc::new(Mutex::new(VecDeque::new())),
            enabled: true,
        }
    }

    /// A link with no worker behind it: requests are dropped and no reply comes. For tests.
    #[cfg(test)]
    pub fn detached() -> Link {
        Link {
            queue: DispatchQueue::serial_qos("is.olibuijr.imagecapture.detached", Qos::Utility),
            replies: Arc::new(Mutex::new(VecDeque::new())),
            enabled: false,
        }
    }

    pub fn send(&self, request: Request) {
        if !self.enabled { return; }
        let replies = self.replies.clone();
        self.queue.async_(move || {
            let reply = handle(request);
            DispatchQueue::main().async_(move || {
                if let Ok(mut queue) = replies.lock() { queue.push_back(reply); }
            });
        });
    }

    /// The replies that arrived since the last call.
    pub fn drain(&self) -> Vec<Reply> {
        self.replies.lock().map(|mut queue| queue.drain(..).collect()).unwrap_or_default()
    }
}

/// The folder the signed-in person keeps their files in, from $HOME.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Answers one request.
pub fn handle(request: Request) -> Reply {
    match request {
        Request::Refresh => Reply::Listed {
            volumes: devices::volume_devices(),
            scanners: scanner::list().map_err(|e| e.to_string()),
        },
        // volume_path only strips the id prefix, so a camera that was unplugged still has a path; its mount must exist.
        Request::Contents { volume } => match devices::volume_path(&volume).filter(|root| root.is_dir()) {
            Some(root) => {
                let contents = devices::contents_of(&root);
                Reply::Contents { files: model::camera_files(&root, &contents), volume }
            }
            None => Reply::Failed("That camera is no longer connected".into()),
        },
        Request::Capabilities { device } => {
            let caps = scanner::capabilities_of(&device);
            Reply::Capabilities { device, caps }
        }
        Request::Import { paths, home } => {
            let dest = model::pictures_dir(&home);
            Reply::Imported(devices::import_files(&paths, &dest))
        }
        Request::Scan { device, settings, preview, home } => {
            let path = if preview {
                model::preview_path(&home)
            } else {
                let name = scanner::scan_file_name(&settings.document_name, &model::stamp(now_secs()), settings.format);
                model::scans_dir(&home, &settings.downloads_directory).join(name)
            };
            match prepare(&path).and_then(|_| scanner::scan_to(&device, &settings, preview, &path).map_err(|e| e.to_string())) {
                Ok(()) => Reply::Scanned { preview, path },
                Err(e) => Reply::ScanFailed(e),
            }
        }
    }
}

/// Makes the folder a file goes in. Its error is the one the scan reports when the folder cannot be made.
fn prepare(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(dir) => std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display())),
        None => Ok(()),
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_camera_that_is_gone_says_so() {
        match handle(Request::Contents { volume: "volume:/no/such/card".into() }) {
            Reply::Failed(text) => assert!(text.contains("no longer connected")),
            other => panic!("unexpected reply {other:?}"),
        }
    }

    #[test]
    fn the_clock_is_after_2020() {
        assert!(now_secs() > 1_577_836_800);
    }

    #[test]
    fn a_file_is_prepared_by_making_its_folder() {
        let dir = std::env::temp_dir().join(format!("image-capture-test-{}", now_secs()));
        let path = dir.join("nested").join("x.png");
        assert!(prepare(&path).is_ok());
        assert!(dir.join("nested").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
