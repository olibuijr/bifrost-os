//! Bursts: pictures taken one after another by the same camera, a second or less apart (a held shutter). The model
//! groups them; the library passes in the capture time and the camera of each picture. Pure: no clock, no files.

use std::path::PathBuf;

/// The longest gap, in seconds, between two pictures of the same burst.
pub const BURST_GAP: i64 = 1;

/// One picture as the burst model reads it: its file, when it was taken and the camera that took it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    /// Capture time, Unix seconds.
    pub taken: i64,
    /// Camera make and model as the EXIF block names them; an empty name is a camera of its own.
    pub camera: String,
}

/// The bursts among `shots`: each group holds at least two pictures, in capture order, all from one camera and each
/// at most `BURST_GAP` seconds after the one before it. Pictures outside a burst are left out.
pub fn burst_groups(shots: &[Shot]) -> Vec<Vec<PathBuf>> {
    let mut sorted: Vec<&Shot> = shots.iter().collect();
    sorted.sort_by(|a, b| a.camera.cmp(&b.camera).then(a.taken.cmp(&b.taken)).then_with(|| a.path.cmp(&b.path)));
    let mut groups = Vec::new();
    let mut current: Vec<PathBuf> = Vec::new();
    let mut previous: Option<&Shot> = None;
    for shot in sorted {
        let continues = previous.is_some_and(|p| p.camera == shot.camera && shot.taken - p.taken <= BURST_GAP);
        if !continues {
            close(&mut current, &mut groups);
        }
        current.push(shot.path.clone());
        previous = Some(shot);
    }
    close(&mut current, &mut groups);
    groups
}

/// Keeps the run as a burst when it has at least two pictures, and clears it either way.
fn close(current: &mut Vec<PathBuf>, groups: &mut Vec<Vec<PathBuf>>) {
    if current.len() > 1 {
        groups.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(path: &str, taken: i64, camera: &str) -> Shot {
        Shot { path: PathBuf::from(path), taken, camera: camera.to_string() }
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn pictures_a_second_apart_from_one_camera_form_a_burst() {
        let shots = vec![shot("/p/1.jpg", 100, "Cam"), shot("/p/2.jpg", 101, "Cam"), shot("/p/3.jpg", 102, "Cam")];
        assert_eq!(burst_groups(&shots), vec![paths(&["/p/1.jpg", "/p/2.jpg", "/p/3.jpg"])]);
    }

    #[test]
    fn a_gap_longer_than_one_second_starts_a_new_run() {
        let shots = vec![shot("/p/a.jpg", 10, "Cam"), shot("/p/b.jpg", 11, "Cam"), shot("/p/c.jpg", 13, "Cam"), shot("/p/d.jpg", 14, "Cam")];
        assert_eq!(burst_groups(&shots), vec![paths(&["/p/a.jpg", "/p/b.jpg"]), paths(&["/p/c.jpg", "/p/d.jpg"])]);
    }

    #[test]
    fn pictures_from_different_cameras_are_never_one_burst() {
        let shots = vec![shot("/p/a.jpg", 5, "Cam A"), shot("/p/b.jpg", 5, "Cam B")];
        assert!(burst_groups(&shots).is_empty(), "two cameras at the same second are not a burst");
    }

    #[test]
    fn a_lone_picture_is_not_a_burst() {
        assert!(burst_groups(&[shot("/p/a.jpg", 5, "Cam")]).is_empty());
        assert!(burst_groups(&[]).is_empty());
    }

    #[test]
    fn unsorted_input_is_grouped_in_capture_order() {
        let shots = vec![shot("/p/late.jpg", 21, "Cam"), shot("/p/early.jpg", 20, "Cam"), shot("/p/other.jpg", 90, "Other")];
        assert_eq!(burst_groups(&shots), vec![paths(&["/p/early.jpg", "/p/late.jpg"])]);
    }

    #[test]
    fn a_burst_gap_of_exactly_one_second_still_counts() {
        assert_eq!(BURST_GAP, 1);
        let shots = vec![shot("/p/a.jpg", 0, "Cam"), shot("/p/b.jpg", 1, "Cam")];
        assert_eq!(burst_groups(&shots).len(), 1);
    }
}
