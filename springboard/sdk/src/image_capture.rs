//! Image capture for apps: a connected still camera (or phone) as the device model of `bifrost_ui::camera_device`,
//! filled in and driven over the Picture Transfer Protocol (`bifrost_core::ptp`).
//!
//! `ImageCapture::open` starts a session on a transport, reads what the camera says about itself, turns that into the
//! capabilities of the model, lists the camera's storage as a folder and file tree, and reads the battery. After that an
//! app downloads a file by its entry index, deletes files (one result per file), takes a picture while tethering is on
//! and sets the camera's clock. The rules (locked camera, missing capability, folders) are the model's; this module
//! only sends the operation the rule allowed.
//!
//! Over USB the transport is `UsbTransport` (a claimed still-image interface). `MemoryCamera` is the same protocol in
//! memory, for tests and demonstrations.

pub use bifrost_core::ptp::{
    find_interface, format_date, is_write_protection, media_kind, parse_date, DeviceInfo, MediaKind, MemoryCamera, ObjectInfo,
    PtpError, Session, Transport, UsbTransport,
};
use bifrost_core::ptp::{format, op, prop};
use bifrost_ui::camera_device::{CameraContents, CameraDevice, CameraError, Capabilities, DeleteError, Media};
use std::collections::HashMap;

/// Why a camera request failed.
#[derive(Debug)]
pub enum CaptureError {
    /// The model refused it (missing capability, not tethered).
    Camera(CameraError),
    /// The camera or the transport failed.
    Ptp(PtpError),
    /// No live file has that entry index.
    NoSuchFile,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Camera(CameraError::NotSupported) => f.write_str("The camera cannot do that"),
            CaptureError::Camera(CameraError::NotTethered) => f.write_str("Tethered capture is off"),
            CaptureError::Ptp(e) => write!(f, "{e}"),
            CaptureError::NoSuchFile => f.write_str("No such file on the camera"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<PtpError> for CaptureError {
    fn from(e: PtpError) -> CaptureError {
        CaptureError::Ptp(e)
    }
}

impl From<CameraError> for CaptureError {
    fn from(e: CameraError) -> CaptureError {
        CaptureError::Camera(e)
    }
}

/// The capabilities the model gets from the operations and properties a camera lists.
pub fn capabilities_of(info: &DeviceInfo) -> Capabilities {
    let mut caps = Capabilities::PROTOCOL_COMMANDS;
    if info.supports_operation(op::INITIATE_CAPTURE) {
        caps = caps.union(Capabilities::TAKE_PICTURE);
    }
    if info.supports_operation(op::DELETE_OBJECT) {
        caps = caps.union(Capabilities::DELETE_ONE);
    }
    if info.supports_operation(op::SEND_OBJECT_INFO) && info.supports_operation(op::SEND_OBJECT) {
        caps = caps.union(Capabilities::RECEIVE_FILE);
    }
    if info.supports_property(prop::DATE_TIME) {
        caps = caps.union(Capabilities::SYNC_CLOCK);
    }
    caps
}

fn media_of(kind: MediaKind) -> Media {
    match kind {
        MediaKind::Image => Media::Image,
        MediaKind::Raw => Media::Raw,
        MediaKind::Movie => Media::Movie,
        MediaKind::Audio => Media::Audio,
    }
}

/// A camera on a transport, with its model.
pub struct ImageCapture<T: Transport> {
    session: Session<T>,
    info: DeviceInfo,
    device: CameraDevice,
    handles: HashMap<usize, u32>,
}

impl<T: Transport> ImageCapture<T> {
    /// Opens a session, reads the camera's description, lists its storage and reads the battery.
    pub fn open(transport: T) -> Result<ImageCapture<T>, PtpError> {
        let mut session = Session::new(transport);
        session.open_session(1)?;
        let info = session.device_info()?;
        let device = CameraDevice::new(capabilities_of(&info));
        let mut capture = ImageCapture { session, info, device, handles: HashMap::new() };
        capture.refresh()?;
        Ok(capture)
    }

    /// What the camera says about itself.
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// The model: contents, capabilities, tethering, battery, clock.
    pub fn device(&self) -> &CameraDevice {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut CameraDevice {
        &mut self.device
    }

    /// The camera's own handle for an entry of the listing.
    pub fn handle_of(&self, index: usize) -> Option<u32> {
        self.handles.get(&index).copied()
    }

    /// The session, for operations this module has no call for (`Capabilities::PROTOCOL_COMMANDS`).
    pub fn session(&mut self) -> &mut Session<T> {
        &mut self.session
    }

    fn add_entry(contents: &mut CameraContents, parent: Option<usize>, info: &ObjectInfo, handle: u32) -> Option<usize> {
        let base = if info.filename.trim().is_empty() { format!("Object {handle}") } else { info.filename.clone() };
        let media = media_of(media_kind(info.format, &info.filename));
        let mut add = |name: &str| {
            if info.is_folder() {
                contents.add_folder(parent, name)
            } else {
                contents.add_file(parent, name, u64::from(info.size), media)
            }
        };
        add(&base).or_else(|| add(&format!("{base} ({handle})")))
    }

    /// Reads the camera's storage again and replaces the listing. Entry indices are assigned again, so an app that
    /// holds indices calls this only when it wants a fresh start. Tethering and the locked state are kept.
    pub fn refresh(&mut self) -> Result<(), PtpError> {
        let handles = self.session.object_handles(0xFFFF_FFFF, 0, 0xFFFF_FFFF)?;
        let mut pending: Vec<(u32, ObjectInfo)> = Vec::with_capacity(handles.len());
        for handle in handles {
            pending.push((handle, self.session.object_info(handle)?));
        }
        *self.device.contents_mut() = CameraContents::new();
        self.handles.clear();
        let mut index_of: HashMap<u32, usize> = HashMap::new();
        // a folder may be listed after the files in it, so place what can be placed until nothing more can
        loop {
            let before = pending.len();
            let mut deferred = Vec::new();
            for (handle, info) in pending {
                let parent = if info.parent == 0 { Some(None) } else { index_of.get(&info.parent).map(|&i| Some(i)) };
                match parent {
                    Some(parent) => {
                        if let Some(index) = Self::add_entry(self.device.contents_mut(), parent, &info, handle) {
                            index_of.insert(handle, index);
                            self.handles.insert(index, handle);
                        }
                    }
                    None => deferred.push((handle, info)),
                }
            }
            pending = deferred;
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        // objects whose folder never appeared go to the top level
        for (handle, info) in pending {
            if let Some(index) = Self::add_entry(self.device.contents_mut(), None, &info, handle) {
                self.handles.insert(index, handle);
            }
        }
        if self.info.supports_property(prop::BATTERY_LEVEL) {
            let level = self.session.battery_level().ok();
            self.device.set_battery_level(level);
        }
        Ok(())
    }

    /// The whole file at an entry index.
    pub fn download(&mut self, index: usize) -> Result<Vec<u8>, CaptureError> {
        let handle = self.file_handle(index)?;
        Ok(self.session.get_object(handle)?)
    }

    /// The camera's thumbnail of the file at an entry index.
    pub fn thumbnail(&mut self, index: usize) -> Result<Vec<u8>, CaptureError> {
        let handle = self.file_handle(index)?;
        Ok(self.session.get_thumb(handle)?)
    }

    fn file_handle(&self, index: usize) -> Result<u32, CaptureError> {
        use bifrost_ui::camera_device::EntryKind;
        match self.device.contents().entry(index).map(|e| e.kind) {
            Some(EntryKind::File { .. }) => self.handle_of(index).ok_or(CaptureError::NoSuchFile),
            _ => Err(CaptureError::NoSuchFile),
        }
    }

    /// Deletes files, one result per index in the order given. The model decides first (locked camera, no delete
    /// capability, folders); the camera is asked only for what it allowed. A card that refuses with a write-protection
    /// answer reports `Locked`. After a transport failure the listing is read again so it matches the camera.
    pub fn delete(&mut self, indices: &[usize]) -> Vec<(usize, Result<(), DeleteError>)> {
        let mut results = Vec::with_capacity(indices.len());
        let mut resync = false;
        for &index in indices {
            let handle = self.handle_of(index);
            let (_, outcome) = self.device.request_delete(&[index]).remove(0);
            let outcome = match (outcome, handle) {
                (Ok(()), Some(handle)) => match self.session.delete_object(handle) {
                    Ok(()) => {
                        self.handles.remove(&index);
                        Ok(())
                    }
                    Err(PtpError::Response(code)) if is_write_protection(code) => {
                        resync = true;
                        Err(DeleteError::Locked)
                    }
                    Err(_) => {
                        resync = true;
                        Err(DeleteError::NotSupported)
                    }
                },
                (Ok(()), None) => Err(DeleteError::NotFound),
                (Err(e), _) => Err(e),
            };
            results.push((index, outcome));
        }
        if resync {
            let _ = self.refresh();
        }
        results
    }

    /// Takes a picture. Needs the capability and tethering on; the new file appears after `refresh`.
    pub fn take_picture(&mut self) -> Result<(), CaptureError> {
        self.device.request_take_picture()?;
        self.session.initiate_capture(0, 0)?;
        Ok(())
    }

    /// Sets the camera's clock to `host_time` (seconds in the terms of `parse_date`). Returns the offset the camera
    /// had before (camera minus host).
    pub fn sync_clock(&mut self, host_time: i64) -> Result<i64, CaptureError> {
        if !self.device.capabilities().contains(Capabilities::SYNC_CLOCK) {
            return Err(CameraError::NotSupported.into());
        }
        let camera_time = self.session.camera_time()?;
        let offset = self.device.request_sync_clock(camera_time, host_time)?;
        self.session.set_camera_time(host_time)?;
        Ok(offset)
    }

    /// Ends the session and returns the transport.
    pub fn close(mut self) -> Result<T, PtpError> {
        self.session.close_session()?;
        Ok(self.session.into_transport())
    }
}

/// The format codes a camera lists as capture formats are the ones `take_picture` can produce.
pub fn capture_formats(info: &DeviceInfo) -> Vec<u16> {
    info.capture_formats.iter().copied().filter(|&f| f != format::UNDEFINED).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::camera_device::EntryKind;

    fn camera() -> MemoryCamera {
        let mut cam = MemoryCamera::new("Shutter 1");
        let dcim = cam.add_folder(0, "DCIM");
        let sub = cam.add_folder(dcim, "100TEST");
        let photo: Vec<u8> = (0..1500u32).map(|i| (i % 251) as u8).collect();
        cam.add_file(sub, "IMG_0001.JPG", format::EXIF_JPEG, photo, "20240131T101530");
        cam.add_file(sub, "IMG_0002.CR2", format::UNDEFINED, vec![9; 9], "20240131T101600");
        cam.add_file(dcim, "MOV_0003.MP4", format::MP4, vec![1; 4], "20240131T101700");
        cam
    }

    #[test]
    fn open_lists_the_storage_as_a_tree() {
        let c = ImageCapture::open(camera()).unwrap();
        assert_eq!(c.info().model, "Shutter 1");
        let contents = c.device().contents();
        let top = contents.children(None);
        assert_eq!(top.len(), 1);
        assert_eq!(contents.entry(top[0]).unwrap().name, "DCIM");
        let paths: Vec<String> = (0..6).filter_map(|i| contents.path(i)).collect();
        assert!(paths.contains(&"DCIM/100TEST/IMG_0001.JPG".to_string()));
        assert!(paths.contains(&"DCIM/MOV_0003.MP4".to_string()));
        assert_eq!(contents.files_of(Media::Raw).len(), 1);
        assert_eq!(contents.files_of(Media::Movie).len(), 1);
        assert_eq!(c.device().battery_level(), Some(77));
    }

    #[test]
    fn capabilities_follow_the_operations_the_camera_lists() {
        let c = ImageCapture::open(camera()).unwrap();
        let caps = c.device().capabilities();
        assert!(caps.contains(Capabilities::TAKE_PICTURE));
        assert!(caps.contains(Capabilities::DELETE_ONE));
        assert!(caps.contains(Capabilities::SYNC_CLOCK));
        assert!(caps.contains(Capabilities::PROTOCOL_COMMANDS));
        assert!(!caps.contains(Capabilities::RECEIVE_FILE));
    }

    #[test]
    fn a_file_is_downloaded_by_entry_index() {
        let mut c = ImageCapture::open(camera()).unwrap();
        let index = c.device().contents().files_of(Media::Image)[0];
        let bytes = c.download(index).unwrap();
        assert_eq!(bytes.len(), 1500);
        assert_eq!(bytes[3], 3);
        assert!(c.thumbnail(index).is_ok());
    }

    #[test]
    fn a_folder_or_a_missing_entry_cannot_be_downloaded() {
        let mut c = ImageCapture::open(camera()).unwrap();
        let folder = c.device().contents().children(None)[0];
        assert!(matches!(c.download(folder), Err(CaptureError::NoSuchFile)));
        assert!(matches!(c.download(99), Err(CaptureError::NoSuchFile)));
    }

    #[test]
    fn delete_asks_the_camera_only_for_files_the_model_allowed() {
        let mut c = ImageCapture::open(camera()).unwrap();
        let file = c.device().contents().files_of(Media::Raw)[0];
        let folder = c.device().contents().children(None)[0];
        let handle = c.handle_of(file).unwrap();
        let results = c.delete(&[file, folder, 99]);
        assert_eq!(results[0], (file, Ok(())));
        assert_eq!(results[1], (folder, Err(DeleteError::NotAFile)));
        assert_eq!(results[2], (99, Err(DeleteError::NotFound)));
        assert!(c.device().contents().entry(file).is_none());
        assert_eq!(c.session().transport().deleted(), &[handle]);
    }

    #[test]
    fn a_locked_camera_is_never_asked() {
        let mut c = ImageCapture::open(camera()).unwrap();
        c.device_mut().set_locked(true);
        let file = c.device().contents().files_of(Media::Image)[0];
        let results = c.delete(&[file]);
        assert_eq!(results[0].1, Err(DeleteError::Locked));
        assert!(c.session().transport().deleted().is_empty());
    }

    #[test]
    fn a_write_protected_card_reports_locked_and_the_listing_is_restored() {
        let mut c = ImageCapture::open(camera()).unwrap();
        let before = c.device().contents().files_of(Media::Image).len();
        c.session().transport_mut().set_write_protected(true);
        let file = c.device().contents().files_of(Media::Image)[0];
        let results = c.delete(&[file]);
        assert_eq!(results[0].1, Err(DeleteError::Locked));
        assert_eq!(c.device().contents().files_of(Media::Image).len(), before);
    }

    #[test]
    fn taking_a_picture_needs_tethering() {
        let mut c = ImageCapture::open(camera()).unwrap();
        assert!(matches!(c.take_picture(), Err(CaptureError::Camera(CameraError::NotTethered))));
        c.device_mut().request_enable_tethering().unwrap();
        c.take_picture().unwrap();
        assert_eq!(c.session().transport().captures(), 1);
    }

    #[test]
    fn syncing_the_clock_reports_the_old_offset_and_sets_the_camera() {
        let mut c = ImageCapture::open(camera()).unwrap();
        let host = parse_date("20240131T102530").unwrap();
        let offset = c.sync_clock(host).unwrap();
        assert_eq!(offset, -600);
        assert_eq!(c.session().transport().clock(), "20240131T102530");
        assert_eq!(c.device().time_offset_seconds(), Some(0));
    }

    #[test]
    fn a_blank_or_repeated_name_still_gets_an_entry() {
        let mut cam = MemoryCamera::new("Shutter 1");
        cam.add_file(0, "", format::EXIF_JPEG, vec![1], "");
        cam.add_file(0, "A.JPG", format::EXIF_JPEG, vec![2], "");
        cam.add_file(0, "A.JPG", format::EXIF_JPEG, vec![3], "");
        let c = ImageCapture::open(cam).unwrap();
        assert_eq!(c.device().contents().children(None).len(), 3);
    }

    #[test]
    fn objects_listed_before_their_folder_are_still_placed_under_it() {
        // the file is handle 1 and its folder handle 2: the folder is placed in the second pass
        let mut cam = MemoryCamera::new("Shutter 1");
        let file = cam.add_file(2, "late.jpg", format::EXIF_JPEG, vec![1], "");
        let folder = cam.add_folder(0, "DCIM");
        assert_eq!((file, folder), (1, 2));
        let c = ImageCapture::open(cam).unwrap();
        let top = c.device().contents().children(None);
        assert_eq!(top.len(), 1);
        let inside = c.device().contents().children(Some(top[0]));
        assert_eq!(inside.len(), 1);
        assert!(matches!(c.device().contents().entry(inside[0]).unwrap().kind, EntryKind::File { size: 1, .. }));
    }

    #[test]
    fn close_returns_the_transport() {
        let c = ImageCapture::open(camera()).unwrap();
        let cam = c.close().unwrap();
        assert!(cam.deleted().is_empty());
        assert_eq!(capture_formats(&DeviceInfo { capture_formats: vec![format::EXIF_JPEG, format::UNDEFINED], ..DeviceInfo::default() }), vec![format::EXIF_JPEG]);
    }
}
