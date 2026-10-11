//! Camera device model: what a connected camera holds (folders and files, as a tree), what its
//! capabilities allow, deletion with one result per file, the locked state, the tethered-capture gate,
//! the clock offset and the battery level. This is pure. The transport that speaks to the camera fills
//! in the listing and performs the operations; this module owns the rules: which requests are allowed,
//! what each file's outcome is, and how the listing changes.

/// What a camera can do while it is connected. Each capability is one bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Capabilities(u32);

impl Capabilities {
    pub const NONE: Capabilities = Capabilities(0);
    /// The app can ask the camera to take a picture while connected.
    pub const TAKE_PICTURE: Capabilities = Capabilities(1 << 0);
    /// The camera can delete one file per request.
    pub const DELETE_ONE: Capabilities = Capabilities(1 << 1);
    /// The camera can delete all of its files in one operation.
    pub const DELETE_ALL: Capabilities = Capabilities(1 << 2);
    /// The app can send files to the camera.
    pub const RECEIVE_FILE: Capabilities = Capabilities(1 << 3);
    /// The camera's clock can be set from the host.
    pub const SYNC_CLOCK: Capabilities = Capabilities(1 << 4);
    /// The camera accepts raw protocol commands.
    pub const PROTOCOL_COMMANDS: Capabilities = Capabilities(1 << 5);

    /// True when every capability in `other` is present.
    pub fn contains(self, other: Capabilities) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Capabilities) -> Capabilities {
        Capabilities(self.0 | other.0)
    }
}

/// The kind of media a file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Image,
    Raw,
    Movie,
    Audio,
}

/// Whether an entry is a folder or a file, and for a file its size and media kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    File { size: u64, media: Media },
}

/// One item on the camera: a folder or a file, under `parent` (None for the top level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub parent: Option<usize>,
    pub kind: EntryKind,
    removed: bool,
}

/// The camera's storage as a tree. Entries keep their index for as long as the camera is connected; a
/// deleted entry leaves a tombstone so the indices of the others do not move.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraContents {
    entries: Vec<Entry>,
}

impl CameraContents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a folder under `parent`. Returns None when the parent is not a live folder, the name is
    /// blank, or a live sibling already has the name.
    pub fn add_folder(&mut self, parent: Option<usize>, name: &str) -> Option<usize> {
        self.add(parent, name, EntryKind::Folder)
    }

    /// Adds a file under `parent`, with the same refusals as `add_folder`.
    pub fn add_file(&mut self, parent: Option<usize>, name: &str, size: u64, media: Media) -> Option<usize> {
        self.add(parent, name, EntryKind::File { size, media })
    }

    fn add(&mut self, parent: Option<usize>, name: &str, kind: EntryKind) -> Option<usize> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(p) = parent {
            match self.entry(p) {
                Some(e) if e.kind == EntryKind::Folder => {}
                _ => return None,
            }
        }
        if self.children(parent).iter().any(|&i| self.entries[i].name == name) {
            return None;
        }
        self.entries.push(Entry { name: name.to_string(), parent, kind, removed: false });
        Some(self.entries.len() - 1)
    }

    /// The live entry at `index`, or None for an index that never existed or was deleted.
    pub fn entry(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index).filter(|e| !e.removed)
    }

    /// The live children of `parent` (None lists the top level), in the order they were added.
    pub fn children(&self, parent: Option<usize>) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && e.parent == parent)
            .map(|(i, _)| i)
            .collect()
    }

    /// The path of an entry from the top level, with names joined by '/'. None for a missing entry.
    pub fn path(&self, index: usize) -> Option<String> {
        let mut parts = Vec::new();
        let mut current = Some(index);
        while let Some(i) = current {
            let e = self.entry(i)?;
            parts.push(e.name.clone());
            current = e.parent;
        }
        parts.reverse();
        Some(parts.join("/"))
    }

    /// Every live file of `media`, in the order they were added, whatever folder holds them.
    pub fn files_of(&self, media: Media) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && matches!(e.kind, EntryKind::File { media: m, .. } if m == media))
            .map(|(i, _)| i)
            .collect()
    }

    fn mark_removed(&mut self, index: usize) {
        self.entries[index].removed = true;
    }
}

/// Why a camera request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraError {
    /// The camera does not have the capability.
    NotSupported,
    /// The request needs tethered capture to be on first.
    NotTethered,
}

/// Why one file could not be deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteError {
    /// The camera or its storage card is locked.
    Locked,
    /// The camera cannot delete files.
    NotSupported,
    /// No live entry has that index.
    NotFound,
    /// The index is a folder. Only files are deleted.
    NotAFile,
}

/// A connected camera: its contents, capabilities and the state the app can read or change.
#[derive(Debug)]
pub struct CameraDevice {
    capabilities: Capabilities,
    contents: CameraContents,
    locked: bool,
    tethered: bool,
    battery: Option<u8>,
    clock_offset_seconds: Option<i64>,
}

impl CameraDevice {
    pub fn new(capabilities: Capabilities) -> Self {
        CameraDevice {
            capabilities,
            contents: CameraContents::new(),
            locked: false,
            tethered: false,
            battery: None,
            clock_offset_seconds: None,
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    pub fn contents(&self) -> &CameraContents {
        &self.contents
    }

    /// The transport fills in the listing through this as it reads the camera.
    pub fn contents_mut(&mut self) -> &mut CameraContents {
        &mut self.contents
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    pub fn is_tethered(&self) -> bool {
        self.tethered
    }

    /// Turns tethered capture on. Needs the take-picture capability.
    pub fn request_enable_tethering(&mut self) -> Result<(), CameraError> {
        if !self.capabilities.contains(Capabilities::TAKE_PICTURE) {
            return Err(CameraError::NotSupported);
        }
        self.tethered = true;
        Ok(())
    }

    pub fn request_disable_tethering(&mut self) {
        self.tethered = false;
    }

    /// Checks that the camera may take a picture now: it must have the capability and be tethered. The
    /// transport performs the capture itself.
    pub fn request_take_picture(&mut self) -> Result<(), CameraError> {
        if !self.capabilities.contains(Capabilities::TAKE_PICTURE) {
            return Err(CameraError::NotSupported);
        }
        if !self.tethered {
            return Err(CameraError::NotTethered);
        }
        Ok(())
    }

    /// Deletes files. Each index gets its own result, in the order given, so one failure does not hide
    /// the others. A locked camera refuses everything, and so does a camera with neither delete
    /// capability. A deleted entry leaves the listing and its index is not reused.
    pub fn request_delete(&mut self, indices: &[usize]) -> Vec<(usize, Result<(), DeleteError>)> {
        let can_delete = self.capabilities.contains(Capabilities::DELETE_ONE)
            || self.capabilities.contains(Capabilities::DELETE_ALL);
        let mut results = Vec::with_capacity(indices.len());
        for &index in indices {
            let outcome = if self.locked {
                Err(DeleteError::Locked)
            } else if !can_delete {
                Err(DeleteError::NotSupported)
            } else {
                match self.contents.entry(index).map(|e| e.kind) {
                    None => Err(DeleteError::NotFound),
                    Some(EntryKind::Folder) => Err(DeleteError::NotAFile),
                    Some(EntryKind::File { .. }) => {
                        self.contents.mark_removed(index);
                        Ok(())
                    }
                }
            };
            results.push((index, outcome));
        }
        results
    }

    /// The offset between the camera's clock and the host's, in seconds (camera minus host), as it was
    /// before the last sync. None until a sync has happened.
    pub fn time_offset_seconds(&self) -> Option<i64> {
        self.clock_offset_seconds
    }

    /// Sets the camera's clock to the host's. Returns the offset that existed before the sync, measured
    /// from `camera_time` and `host_time` (both in seconds since the same epoch). After a successful sync
    /// the stored offset is zero.
    pub fn request_sync_clock(&mut self, camera_time: i64, host_time: i64) -> Result<i64, CameraError> {
        if !self.capabilities.contains(Capabilities::SYNC_CLOCK) {
            return Err(CameraError::NotSupported);
        }
        let before = camera_time.saturating_sub(host_time);
        self.clock_offset_seconds = Some(0);
        Ok(before)
    }

    pub fn battery_level(&self) -> Option<u8> {
        self.battery
    }

    /// Records the battery level as a percentage, clamped to 100. None means the camera does not report it.
    pub fn set_battery_level(&mut self, percent: Option<u8>) {
        self.battery = percent.map(|p| p.min(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(caps: Capabilities) -> CameraDevice {
        CameraDevice::new(caps)
    }

    /// A card with DCIM/100 holding one JPEG and one movie, and a folder with a single raw file.
    fn card(c: &mut CameraDevice) -> (usize, usize, usize, usize) {
        let contents = c.contents_mut();
        let dcim = contents.add_folder(None, "DCIM").unwrap();
        let shots = contents.add_folder(Some(dcim), "100MEDIA").unwrap();
        let jpeg = contents.add_file(Some(shots), "IMG_0001.JPG", 2_400_000, Media::Image).unwrap();
        let movie = contents.add_file(Some(shots), "MOV_0002.MOV", 90_000_000, Media::Movie).unwrap();
        let raw = contents.add_file(None, "RAW_0003.DNG", 25_000_000, Media::Raw).unwrap();
        (jpeg, movie, raw, dcim)
    }

    #[test]
    fn the_contents_tree_has_paths_and_refuses_bad_names_and_parents() {
        let mut c = camera(Capabilities::NONE);
        let (jpeg, _, raw, dcim) = card(&mut c);
        assert_eq!(c.contents().path(jpeg).as_deref(), Some("DCIM/100MEDIA/IMG_0001.JPG"));
        assert_eq!(c.contents().path(raw).as_deref(), Some("RAW_0003.DNG"));
        assert_eq!(c.contents().children(None), vec![dcim, raw], "the top level lists the folder then the raw file");
        let contents = c.contents_mut();
        assert!(contents.add_folder(None, "DCIM").is_none(), "a live sibling already has the name");
        assert!(contents.add_folder(None, "   ").is_none(), "a blank name is refused");
        assert!(contents.add_folder(Some(raw), "Inside a file").is_none(), "a file cannot hold children");
        assert!(contents.add_folder(Some(999), "Nowhere").is_none(), "a missing parent is refused");
    }

    #[test]
    fn files_are_found_by_media_across_folders() {
        let mut c = camera(Capabilities::NONE);
        let (jpeg, movie, raw, _) = card(&mut c);
        assert_eq!(c.contents().files_of(Media::Image), vec![jpeg]);
        assert_eq!(c.contents().files_of(Media::Movie), vec![movie]);
        assert_eq!(c.contents().files_of(Media::Raw), vec![raw]);
        assert!(c.contents().files_of(Media::Audio).is_empty());
    }

    #[test]
    fn tethered_capture_needs_the_capability_and_tethering_first() {
        let mut bare = camera(Capabilities::DELETE_ONE);
        assert_eq!(bare.request_enable_tethering(), Err(CameraError::NotSupported));

        let mut c = camera(Capabilities::TAKE_PICTURE);
        assert_eq!(c.request_take_picture(), Err(CameraError::NotTethered));
        assert!(c.request_enable_tethering().is_ok());
        assert!(c.is_tethered());
        assert!(c.request_take_picture().is_ok());
        c.request_disable_tethering();
        assert_eq!(c.request_take_picture(), Err(CameraError::NotTethered));
    }

    #[test]
    fn each_file_gets_its_own_delete_result_and_leaves_the_listing() {
        let mut c = camera(Capabilities::DELETE_ONE);
        let (jpeg, movie, raw, dcim) = card(&mut c);
        let results = c.request_delete(&[jpeg, dcim, 999, raw]);
        assert_eq!(
            results,
            vec![
                (jpeg, Ok(())),
                (dcim, Err(DeleteError::NotAFile)),
                (999, Err(DeleteError::NotFound)),
                (raw, Ok(())),
            ]
        );
        assert!(c.contents().entry(jpeg).is_none());
        assert_eq!(c.contents().files_of(Media::Image), Vec::<usize>::new());
        assert_eq!(c.contents().path(movie).as_deref(), Some("DCIM/100MEDIA/MOV_0002.MOV"), "other indices do not move");
        assert_eq!(c.request_delete(&[jpeg]), vec![(jpeg, Err(DeleteError::NotFound))], "a deleted file is gone");
    }

    #[test]
    fn a_locked_camera_refuses_every_delete_and_keeps_its_files() {
        let mut c = camera(Capabilities::DELETE_ONE.union(Capabilities::DELETE_ALL));
        let (jpeg, _, _, _) = card(&mut c);
        c.set_locked(true);
        assert!(c.is_locked());
        assert_eq!(c.request_delete(&[jpeg]), vec![(jpeg, Err(DeleteError::Locked))]);
        assert!(c.contents().entry(jpeg).is_some());
        c.set_locked(false);
        assert_eq!(c.request_delete(&[jpeg]), vec![(jpeg, Ok(()))]);
    }

    #[test]
    fn a_camera_without_delete_capability_refuses_deletes() {
        let mut c = camera(Capabilities::TAKE_PICTURE);
        let (jpeg, _, _, _) = card(&mut c);
        assert_eq!(c.request_delete(&[jpeg]), vec![(jpeg, Err(DeleteError::NotSupported))]);
        assert!(c.contents().entry(jpeg).is_some());
    }

    #[test]
    fn clock_sync_reports_the_offset_before_the_sync() {
        let mut c = camera(Capabilities::TAKE_PICTURE);
        assert_eq!(c.request_sync_clock(1_000, 990), Err(CameraError::NotSupported));
        assert_eq!(c.time_offset_seconds(), None);

        let mut clock = camera(Capabilities::SYNC_CLOCK);
        assert_eq!(clock.time_offset_seconds(), None, "no offset until a sync");
        assert_eq!(clock.request_sync_clock(1_030, 1_000), Ok(30), "the camera was 30 s ahead");
        assert_eq!(clock.time_offset_seconds(), Some(0), "after the sync the clocks agree");
    }

    #[test]
    fn battery_level_is_optional_and_clamped() {
        let mut c = camera(Capabilities::NONE);
        assert_eq!(c.battery_level(), None, "a camera that does not report a level has none");
        c.set_battery_level(Some(140));
        assert_eq!(c.battery_level(), Some(100));
        c.set_battery_level(None);
        assert_eq!(c.battery_level(), None);
    }
}
