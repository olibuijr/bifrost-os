//! Links for the locator: `locate:` links that open a device or a person, and the directions link that hands a route
//! from a position to a device over to the maps app.

use bifrost_maps::geocode::{percent_decode, percent_encode};
use bifrost_maps::links::route_link;
use bifrost_maps::route::Profile;

pub const SCHEME: &str = "locate:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Device(String),
    Person(String),
}

/// Reads `locate:device/<id>` or `locate:person/<address>`. `None` for any other link and for an empty id.
pub fn parse(link: &str) -> Option<Target> {
    let rest = link.trim().strip_prefix(SCHEME)?.trim_start_matches("//");
    let (kind, id) = rest.split_once('/')?;
    let id = percent_decode(id.trim());
    if id.is_empty() {
        return None;
    }
    match kind {
        "device" => Some(Target::Device(id)),
        "person" => Some(Target::Person(id)),
        _ => None,
    }
}

/// The link that opens a device.
pub fn device_link(id: &str) -> String {
    format!("{SCHEME}device/{}", percent_encode(id.trim()))
}

/// Driving directions from a position to a device, as a maps link.
pub fn directions(from: (f64, f64), to: (f64, f64)) -> String {
    route_link(from, to, Profile::Driving)
}
