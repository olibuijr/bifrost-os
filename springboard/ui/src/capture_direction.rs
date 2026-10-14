//! Camera direction as pure state: which cameras face the same way as a view and which face away from it. The capture
//! service reports the cameras it sees as `DeviceDescriptor`s. A `Coordinator` groups them into a `Map` by their
//! position relative to the view, and reports a change only when the grouping differs from the last one. Nothing here
//! opens a camera or reads a sensor: the caller supplies the device list and acts on the result.

/// The physical side of the device a camera sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Unspecified,
    /// On the back, facing away from the screen.
    Back,
    /// On the front, facing the same way as the screen.
    Front,
}

/// The kind of media a camera delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaType {
    Video,
    Audio,
    Muxed,
}

/// The facts an app may read about one camera. It is a copy, so it can be passed between tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDescriptor {
    /// Identifies the camera for as long as it stays connected.
    pub unique_id: String,
    /// A name suitable for display.
    pub localized_name: String,
    /// The kind of camera, such as a wide-angle or telephoto camera.
    pub device_type: String,
    pub media_types: Vec<MediaType>,
    pub position: Position,
}

/// The direction a camera faces, relative to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The camera faces the same way as the view.
    Forward,
    /// The camera faces away from the view.
    Backward,
}

/// The cameras grouped by direction. Each list is sorted by `unique_id` so two maps of the same cameras are equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Map {
    pub forward: Vec<DeviceDescriptor>,
    pub backward: Vec<DeviceDescriptor>,
}

impl Map {
    /// The direction of the camera with `unique_id`, or `None` when it is not in either list.
    pub fn direction_of(&self, unique_id: &str) -> Option<Direction> {
        if self.forward.iter().any(|d| d.unique_id == unique_id) {
            Some(Direction::Forward)
        } else if self.backward.iter().any(|d| d.unique_id == unique_id) {
            Some(Direction::Backward)
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty() && self.backward.is_empty()
    }
}

/// Tracks the cameras a view cares about. Cameras with an unspecified position are in neither list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coordinator {
    /// The side of the device the view is on. Cameras on this side face the view's way.
    view_side: Position,
    /// The device types to track. An empty list tracks every type.
    device_types: Vec<String>,
    map: Map,
}

impl Coordinator {
    /// `view_side` is `Front` for a view on the front of the device, `Back` for a view on the back.
    pub fn new(view_side: Position, device_types: Vec<String>) -> Self {
        Self {
            view_side,
            device_types,
            map: Map::default(),
        }
    }

    /// The current grouping. This is what the view reads as its device directions.
    pub fn device_directions(&self) -> &Map {
        &self.map
    }

    /// Takes the latest device list and returns the new grouping when it differs from the last one. The caller gets
    /// `None` when nothing changed, which is the moment a change handler would not run.
    pub fn update(&mut self, devices: &[DeviceDescriptor]) -> Option<Map> {
        let next = self.group(devices);
        if next == self.map {
            return None;
        }
        self.map = next.clone();
        Some(next)
    }

    fn tracks(&self, device: &DeviceDescriptor) -> bool {
        self.device_types.is_empty() || self.device_types.iter().any(|t| *t == device.device_type)
    }

    fn group(&self, devices: &[DeviceDescriptor]) -> Map {
        let mut map = Map::default();
        for device in devices.iter().filter(|d| self.tracks(d)) {
            match device.position {
                Position::Unspecified => {}
                p if p == self.view_side => map.forward.push(device.clone()),
                _ => map.backward.push(device.clone()),
            }
        }
        map.forward.sort_by(|a, b| a.unique_id.cmp(&b.unique_id));
        map.backward.sort_by(|a, b| a.unique_id.cmp(&b.unique_id));
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(id: &str, device_type: &str, position: Position) -> DeviceDescriptor {
        DeviceDescriptor {
            unique_id: id.to_string(),
            localized_name: format!("Camera {id}"),
            device_type: device_type.to_string(),
            media_types: vec![MediaType::Video],
            position,
        }
    }

    #[test]
    fn a_front_view_groups_front_cameras_as_forward() {
        let mut c = Coordinator::new(Position::Front, Vec::new());
        c.update(&[cam("f1", "wide", Position::Front), cam("b1", "wide", Position::Back)]);
        let m = c.device_directions();
        assert_eq!(m.direction_of("f1"), Some(Direction::Forward));
        assert_eq!(m.direction_of("b1"), Some(Direction::Backward));
    }

    #[test]
    fn a_back_view_flips_the_grouping() {
        let mut c = Coordinator::new(Position::Back, Vec::new());
        c.update(&[cam("f1", "wide", Position::Front), cam("b1", "wide", Position::Back)]);
        assert_eq!(c.device_directions().direction_of("b1"), Some(Direction::Forward));
        assert_eq!(c.device_directions().direction_of("f1"), Some(Direction::Backward));
    }

    #[test]
    fn unspecified_cameras_are_in_neither_list() {
        let mut c = Coordinator::new(Position::Front, Vec::new());
        c.update(&[cam("u1", "external", Position::Unspecified)]);
        assert!(c.device_directions().is_empty());
        assert_eq!(c.device_directions().direction_of("u1"), None);
    }

    #[test]
    fn only_the_listed_device_types_are_tracked() {
        let mut c = Coordinator::new(Position::Front, vec!["wide".to_string()]);
        c.update(&[cam("f1", "wide", Position::Front), cam("f2", "telephoto", Position::Front)]);
        assert_eq!(c.device_directions().direction_of("f1"), Some(Direction::Forward));
        assert_eq!(c.device_directions().direction_of("f2"), None);
    }

    #[test]
    fn an_unchanged_list_reports_no_change() {
        let mut c = Coordinator::new(Position::Front, Vec::new());
        let devices = [cam("f1", "wide", Position::Front)];
        assert!(c.update(&devices).is_some());
        assert_eq!(c.update(&devices), None);
    }

    #[test]
    fn a_removed_camera_reports_a_change_with_the_remaining_cameras() {
        let mut c = Coordinator::new(Position::Front, Vec::new());
        c.update(&[cam("f1", "wide", Position::Front), cam("b1", "wide", Position::Back)]);
        let changed = c.update(&[cam("b1", "wide", Position::Back)]).expect("a change");
        assert!(changed.forward.is_empty());
        assert_eq!(changed.backward.len(), 1);
        assert_eq!(c.device_directions(), &changed);
    }

    #[test]
    fn lists_are_sorted_by_identifier_whatever_the_input_order() {
        let mut c = Coordinator::new(Position::Back, Vec::new());
        let m = c
            .update(&[cam("b2", "wide", Position::Back), cam("b1", "wide", Position::Back)])
            .expect("a change");
        let ids: Vec<&str> = m.forward.iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(ids, vec!["b1", "b2"]);
    }
}
