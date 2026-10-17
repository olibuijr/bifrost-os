//! The route picker for apps (AirPlay style): the outputs a media app can send its audio or video to. The rows and the
//! authorization rules come from `bifrost_ui::route_picker` and `bifrost_ui::route_auth`; this module splits the rows
//! into the sections a picker draws and turns a tap into an activation. The app never connects to a receiver itself:
//! the shell does, once the route is active. Nothing is drawn here.

pub use bifrost_ui::route_auth::{RouteAuth, RouteError};
pub use bifrost_ui::route_picker::{destinations, Destination};

/// The two sections of the picker: the outputs built into this device, and the receivers found nearby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sections {
    pub outputs: Vec<Destination>,
    pub nearby: Vec<Destination>,
}

/// Builds the rows for the picker and splits them into sections. Empty when there is nothing to show.
pub fn sections(local: &[(&str, &str)], found: &[(&str, &str)], auth: &RouteAuth) -> Sections {
    let (outputs, nearby): (Vec<Destination>, Vec<Destination>) =
        destinations(local, found, auth).into_iter().partition(|d| d.local);
    Sections { outputs, nearby }
}

/// A tap on a destination. An active route is turned off and `Ok(false)` is returned. An inactive route is activated
/// when the user has authorized it (`Ok(true)`); otherwise the tap is refused with `Unauthorized`, and the app asks
/// the user to authorize the route first.
pub fn tap(auth: &mut RouteAuth, id: &str) -> Result<bool, RouteError> {
    if auth.is_active(id) {
        auth.set_active(id, false)?;
        return Ok(false);
    }
    auth.set_active(id, true)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_with(routes: &[&str]) -> RouteAuth {
        let mut auth = RouteAuth::new();
        for r in routes {
            auth.authorize(r);
        }
        auth
    }

    #[test]
    fn outputs_and_nearby_receivers_are_split_into_their_sections() {
        let auth = RouteAuth::new();
        let s = sections(&[("speaker", "Speakers")], &[("tv", "Living room TV"), ("aa", "Amp")], &auth);
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].id, "speaker");
        let names: Vec<&str> = s.nearby.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["Amp", "Living room TV"], "nearby receivers are sorted by name");
    }

    #[test]
    fn an_authorized_route_is_activated_and_a_second_tap_turns_it_off() {
        let mut auth = auth_with(&["tv"]);
        assert_eq!(tap(&mut auth, "tv"), Ok(true));
        assert!(auth.is_active("tv"));
        assert_eq!(tap(&mut auth, "tv"), Ok(false));
        assert!(!auth.is_active("tv"));
    }

    #[test]
    fn an_unauthorized_route_is_refused_and_stays_off() {
        let mut auth = RouteAuth::new();
        assert_eq!(tap(&mut auth, "tv"), Err(RouteError::Unauthorized));
        assert!(!auth.is_active("tv"));
    }

    #[test]
    fn an_active_row_is_marked_in_the_sections() {
        let mut auth = auth_with(&["tv"]);
        tap(&mut auth, "tv").unwrap();
        let s = sections(&[], &[("tv", "TV")], &auth);
        assert!(s.nearby[0].active);
    }
}
