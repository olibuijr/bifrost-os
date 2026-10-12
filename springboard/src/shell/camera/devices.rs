//! Devices for the Camera app: the V4L2 capture nodes that can take pictures, and the storage of a camera
//! or SD card (a mounted volume with a DCIM folder) that the Import view reads from.
//!
//! The device rules live in `bifrost_ui` (`device_browser` for add and remove events, `camera_device` for
//! the folder and file tree). This file only adapts what the kernel and the mounts report to those models.
//! Nothing here opens a camera: listing nodes and volumes is a directory read.

use crate::media::v4l2::CameraInfo;
use bifrost_ui::camera_device::{CameraContents, Media};
use bifrost_ui::device_browser::{Device, DeviceKind, Transport};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Id prefix of a camera (a V4L2 capture node).
const CAMERA_PREFIX: &str = "camera:";
/// Id prefix of a volume (the storage of a camera or an SD card).
const VOLUME_PREFIX: &str = "volume:";
/// The folder cameras write their pictures into.
const DCIM: &str = "DCIM";
/// How deep the import walk goes, counting DCIM itself as level 1.
const MAX_DEPTH: usize = 3;

/// The `video*` names in /dev, sorted. A change in this list means a camera was plugged in or removed,
/// so the Camera app probes again.
pub fn node_listing() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("/dev")
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("video")).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// The stable id of each camera, in the order of `cameras`. The card name identifies a camera. A second
/// camera with the same card name gets "#2", a third "#3", in the order they were found.
pub fn camera_ids(cameras: &[CameraInfo]) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    cameras
        .iter()
        .map(|c| {
            let n = seen.entry(c.card.as_str()).and_modify(|n| *n += 1).or_insert(1);
            if *n == 1 {
                format!("{CAMERA_PREFIX}{}", c.card)
            } else {
                format!("{CAMERA_PREFIX}{}#{}", c.card, n)
            }
        })
        .collect()
}

/// The cameras as device-model entries. A V4L2 node is treated as a built-in or local device.
pub fn camera_devices(cameras: &[CameraInfo]) -> Vec<Device> {
    camera_ids(cameras)
        .into_iter()
        .zip(cameras)
        .map(|(id, c)| Device { id, name: c.card.clone(), kind: DeviceKind::Camera, transport: Transport::Other, serial: None })
        .collect()
}

/// Every mounted volume that holds a DCIM folder, by id. The mounts are read at /run/media (most desktops
/// mount `/run/media/<user>/<label>`) and at /media.
pub fn volume_devices() -> Vec<Device> {
    let mut out = Vec::new();
    for base in ["/run/media", "/media"] {
        let Ok(top) = std::fs::read_dir(base) else { continue };
        for entry in top.flatten() {
            let first = entry.path();
            let mut candidates = vec![first.clone()];
            if let Ok(inner) = std::fs::read_dir(&first) {
                candidates.extend(inner.flatten().map(|e| e.path()));
            }
            for path in candidates {
                if path.join(DCIM).is_dir() {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    out.push(Device { id: format!("{VOLUME_PREFIX}{}", path.display()), name, kind: DeviceKind::Camera, transport: Transport::Other, serial: None });
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The mount point of a volume id, or None for any other id.
pub fn volume_path(id: &str) -> Option<PathBuf> {
    id.strip_prefix(VOLUME_PREFIX).map(PathBuf::from)
}

/// The name shown for a volume: the mount's folder name.
pub fn volume_label(id: &str) -> String {
    volume_path(id).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| id.to_string())
}

/// The media kind of a file by its extension, or None for a file the Import view does not list.
pub fn media_of(name: &str) -> Option<Media> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" | "png" => Media::Image,
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" | "heic" | "heif" => Media::Raw,
        "mov" | "mp4" | "m4v" | "avi" | "mts" | "m2ts" => Media::Movie,
        "wav" | "mp3" | "m4a" | "aac" => Media::Audio,
        _ => return None,
    })
}

/// The DCIM folder of a volume as a tree. Paths in the tree start at "DCIM", so joining `contents.path(i)`
/// onto the volume's mount point gives the file's real path. Hidden entries are skipped and each folder
/// is sorted by name.
pub fn contents_of(root: &Path) -> CameraContents {
    let mut contents = CameraContents::new();
    if let Some(top) = contents.add_folder(None, DCIM) {
        walk(&mut contents, Some(top), &root.join(DCIM), 1);
    }
    contents
}

fn walk(contents: &mut CameraContents, parent: Option<usize>, dir: &Path, depth: usize) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<(String, std::fs::Metadata)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Some((name, e.metadata().ok()?))
        })
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, meta) in items {
        if meta.is_dir() {
            if depth < MAX_DEPTH {
                if let Some(folder) = contents.add_folder(parent, &name) {
                    walk(contents, Some(folder), &dir.join(&name), depth + 1);
                }
            }
        } else if let Some(media) = media_of(&name) {
            contents.add_file(parent, &name, meta.len(), media);
        }
    }
}

/// `name` inside `dir` when it is free, otherwise the first "stem 2.ext", "stem 3.ext"... that is.
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (2u32..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Copies each picture in `sources` into `dir` (the Photos folder), keeping its name and modification
/// time. Files that are not pictures or cannot be read are skipped. Returns how many were copied.
pub fn import_files(sources: &[PathBuf], dir: &Path) -> usize {
    if std::fs::create_dir_all(dir).is_err() {
        return 0;
    }
    let mut copied = 0;
    for src in sources {
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if !crate::imgdec::is_image_name(&name) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(src) else { continue };
        let dst = free_name(dir, &name);
        if std::fs::copy(src, &dst).is_err() {
            continue;
        }
        if let (Ok(modified), Ok(file)) = (meta.modified(), std::fs::File::options().write(true).open(&dst)) {
            let _ = file.set_modified(modified);
        }
        copied += 1;
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(card: &str) -> CameraInfo {
        CameraInfo { path: "/dev/video0".into(), card: card.into() }
    }

    /// A fresh scratch folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-camera-devices-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn repeated_card_names_get_numbered_ids() {
        let ids = camera_ids(&[cam("USB Cam"), cam("USB Cam"), cam("Built-in")]);
        assert_eq!(ids, ["camera:USB Cam", "camera:USB Cam#2", "camera:Built-in"]);
        let devices = camera_devices(&[cam("Built-in")]);
        assert_eq!(devices[0].name, "Built-in");
        assert_eq!(devices[0].kind, DeviceKind::Camera);
    }

    #[test]
    fn media_is_read_from_the_extension() {
        assert_eq!(media_of("IMG_0001.JPG"), Some(Media::Image));
        assert_eq!(media_of("clip.MP4"), Some(Media::Movie));
        assert_eq!(media_of("RAW.cr3"), Some(Media::Raw));
        assert_eq!(media_of("notes.txt"), None);
        assert_eq!(media_of("noextension"), None);
    }

    #[test]
    fn volume_ids_round_trip_to_paths() {
        assert_eq!(volume_path("volume:/run/media/u/SD"), Some(PathBuf::from("/run/media/u/SD")));
        assert_eq!(volume_path("camera:USB Cam"), None);
        assert_eq!(volume_label("volume:/run/media/u/SD"), "SD");
    }

    #[test]
    fn the_dcim_tree_lists_pictures_and_skips_hidden_entries() {
        let root = scratch("tree");
        let shots = root.join("DCIM/100CANON");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join("IMG_0001.JPG"), b"x").unwrap();
        std::fs::write(shots.join("IMG_0002.png"), b"xy").unwrap();
        std::fs::write(shots.join("clip.mov"), b"xyz").unwrap();
        std::fs::write(root.join("DCIM/.hidden.jpg"), b"x").unwrap();
        let contents = contents_of(&root);
        // 0 DCIM, 1 100CANON, then the files in name order
        assert_eq!(contents.path(2).as_deref(), Some("DCIM/100CANON/IMG_0001.JPG"));
        assert_eq!(contents.files_of(Media::Image), [2, 3]);
        assert_eq!(contents.files_of(Media::Movie), [4]);
        assert_eq!(contents.children(None), [0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn free_names_count_up_before_the_extension() {
        let dir = scratch("names");
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        assert_eq!(free_name(&dir, "b.jpg"), dir.join("b.jpg"));
        assert_eq!(free_name(&dir, "a.jpg"), dir.join("a 2.jpg"));
        std::fs::write(dir.join("a 2.jpg"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.jpg"), dir.join("a 3.jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_copies_pictures_only_and_never_overwrites() {
        let root = scratch("import");
        let shots = root.join("DCIM/100CANON");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join("IMG_0001.JPG"), b"one").unwrap();
        std::fs::write(shots.join("IMG_0002.png"), b"two").unwrap();
        std::fs::write(shots.join("clip.mov"), b"three").unwrap();
        let photos = root.join("Pictures");
        let sources = vec![shots.join("IMG_0001.JPG"), shots.join("IMG_0002.png"), shots.join("clip.mov")];
        assert_eq!(import_files(&sources, &photos), 2);
        assert_eq!(std::fs::read(photos.join("IMG_0001.JPG")).unwrap(), b"one");
        assert!(!photos.join("clip.mov").exists());
        // importing the same picture again keeps both copies
        assert_eq!(import_files(&sources[..1], &photos), 1);
        assert_eq!(std::fs::read(photos.join("IMG_0001 2.JPG")).unwrap(), b"one");
        let _ = std::fs::remove_dir_all(&root);
    }
}
