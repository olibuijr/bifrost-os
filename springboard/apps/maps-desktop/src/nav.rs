//! Turn-by-turn guidance: follows a route against a stream of positions. It finds where along the route the person
//! is, which manoeuvre comes next and how far away it is, says when the next manoeuvre should be announced, and
//! tells when the person has left the route or arrived. It has no clock, no window and no location service of its
//! own, so every rule is tested with plain coordinates; `state::Maps` feeds it the positions the shell reports.
//!
//! A route step is a stretch of road that begins with a manoeuvre ("Turn left onto Laugavegur") and has a length.
//! While the person is on step `i`, the manoeuvre ahead is the one that begins step `i + 1`, at the end of step `i`.

use crate::route::{haversine_m, Profile, Route};

/// Metres from the route line beyond which the person counts as off the route (and a new route is asked for). A
/// position that is less accurate than this widens it, so a coarse fix never sends the person round in circles.
pub const OFF_ROUTE_M: f64 = 50.0;
/// Metres from the end of the route at which the person has arrived.
pub const ARRIVED_M: f64 = 25.0;
/// The widest fix inaccuracy (metres) that still counts for arriving; a coarser fix cannot decide it.
const ARRIVED_SLACK_MAX_M: f64 = 100.0;
/// Metres per degree of latitude on the mean-radius sphere `haversine_m` uses.
const METRES_PER_DEGREE: f64 = 111_194.9;

/// Whether guidance can follow this route: a computed one with a line and steps. A straight-line estimate and a
/// transit journey cannot be followed.
pub fn can_follow(route: &Route, profile: Profile) -> bool {
    !route.approximate && profile != Profile::Transit && !route.steps.is_empty() && route.geometry.len() >= 2
}

/// The distances (metres) at which the next manoeuvre is announced, farthest first: far, near and now.
pub fn announce_at(profile: Profile) -> [f64; 3] {
    match profile {
        Profile::Driving => [500.0, 150.0, 30.0],
        _ => [100.0, 40.0, 12.0],
    }
}

/// One announcement that is due: which step's manoeuvre, how far away it is, and whether it is the last call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Announce {
    pub step: usize,
    pub distance_m: f64,
    /// the nearest of the distances: the manoeuvre is here, so no distance is spoken
    pub now: bool,
}

/// What the person needs to know after a position.
#[derive(Clone, Debug, PartialEq)]
pub struct Guidance {
    /// the step whose manoeuvre comes next; `None` on the last stretch of a route with nothing left to do but arrive
    pub ahead: Option<usize>,
    /// metres to that manoeuvre, or to the end of the route when there is no manoeuvre ahead
    pub distance_m: f64,
    pub remaining_m: f64,
    pub remaining_s: f64,
    pub off_route: bool,
    pub arrived: bool,
    /// set once for each call: a position that already announced it never announces it again
    pub announce: Option<Announce>,
}

#[derive(Clone, Debug)]
pub struct Navigator {
    route: Route,
    profile: Profile,
    /// metres along the line at each vertex of the route geometry
    cumulative: Vec<f64>,
    /// metres along the line at which step `i` ends
    ends: Vec<f64>,
    along_m: f64,
    announced: Option<(usize, usize)>,
    arrival_announced: bool,
}

impl Navigator {
    /// Guidance needs a computed route with a line and steps. A straight-line estimate, a transit journey and a route
    /// without steps cannot be followed, so they give `None`.
    pub fn new(route: Route, profile: Profile) -> Option<Navigator> {
        if !can_follow(&route, profile) {
            return None;
        }
        let mut cumulative = vec![0.0];
        for pair in route.geometry.windows(2) {
            let last = cumulative.last().copied().unwrap_or(0.0);
            cumulative.push(last + haversine_m(pair[0], pair[1]));
        }
        let length = cumulative.last().copied().unwrap_or(0.0);
        if length <= 0.0 {
            return None;
        }
        // the server's step lengths and the line rarely agree to the metre, so the steps are scaled onto the line
        let total: f64 = route.steps.iter().map(|s| s.distance_m).sum();
        let scale = if total > 0.0 { length / total } else { 0.0 };
        let mut ends = Vec::with_capacity(route.steps.len());
        let mut sum = 0.0;
        for s in &route.steps {
            sum += s.distance_m * scale;
            ends.push(sum.min(length));
        }
        // the last step ends where the line does
        if let Some(last) = ends.last_mut() {
            *last = length;
        }
        Some(Navigator { route, profile, cumulative, ends, along_m: 0.0, announced: None, arrival_announced: false })
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// Length of the route line in metres.
    pub fn length_m(&self) -> f64 {
        self.cumulative.last().copied().unwrap_or(0.0)
    }

    /// Takes a position (lat, lon) and the fix's inaccuracy in metres.
    pub fn update(&mut self, pos: (f64, f64), accuracy_m: f64) -> Guidance {
        let accuracy = if accuracy_m.is_finite() { accuracy_m.max(0.0) } else { 0.0 };
        let threshold = OFF_ROUTE_M.max(accuracy);
        let length = self.length_m();
        // look ahead of where the person was first, so a route that crosses itself is not mistaken for a shortcut
        let (mut along, mut off) = self.nearest(pos, Some(self.along_m - threshold));
        if off > threshold {
            let (a, o) = self.nearest(pos, None);
            if o < off {
                (along, off) = (a, o);
            }
        }
        let off_route = off > threshold;
        if !off_route {
            self.along_m = along;
        }
        let along = self.along_m;
        let step = self.ends.iter().position(|&e| e > along).unwrap_or(self.ends.len() - 1);
        let remaining_m = (length - along).max(0.0);
        let arrived = !off_route && remaining_m <= ARRIVED_M.max(accuracy.min(ARRIVED_SLACK_MAX_M));
        let ahead = (step + 1 < self.route.steps.len()).then_some(step + 1);
        let distance_m = if ahead.is_some() { (self.ends[step] - along).max(0.0) } else { remaining_m };
        let remaining_s = self.route.duration_s * remaining_m / length;
        let announce = if off_route { None } else { self.announcement(arrived, ahead, distance_m) };
        Guidance { ahead, distance_m, remaining_m, remaining_s, off_route, arrived, announce }
    }

    /// The call that is due, if any. Each manoeuvre is announced once at each of the three distances.
    fn announcement(&mut self, arrived: bool, ahead: Option<usize>, distance_m: f64) -> Option<Announce> {
        if arrived {
            if self.arrival_announced {
                return None;
            }
            self.arrival_announced = true;
            return Some(Announce { step: self.route.steps.len() - 1, distance_m: 0.0, now: true });
        }
        let step = ahead?;
        let at = announce_at(self.profile);
        let level = at.iter().rposition(|&d| distance_m <= d)?;
        let seen = self.announced.filter(|&(s, l)| s == step && l >= level);
        if seen.is_some() {
            return None;
        }
        self.announced = Some((step, level));
        Some(Announce { step, distance_m, now: level == at.len() - 1 })
    }

    /// The nearest point of the line to `pos`: (metres along the line, metres away). With `min_along`, only the
    /// stretches that end past it are looked at.
    fn nearest(&self, pos: (f64, f64), min_along: Option<f64>) -> (f64, f64) {
        let g = &self.route.geometry;
        let mut best = (0.0, f64::MAX);
        for i in 0..g.len() - 1 {
            if min_along.is_some_and(|min| self.cumulative[i + 1] < min) {
                continue;
            }
            let (away, t) = distance_to_segment(pos, g[i], g[i + 1]);
            if away < best.1 {
                best = (self.cumulative[i] + t * (self.cumulative[i + 1] - self.cumulative[i]), away);
            }
        }
        best
    }
}

/// Metres from `p` to the segment `a`-`b`, and how far along the segment (0 to 1) the nearest point is. The ground
/// is treated as flat around `a`, which is exact enough over a road segment.
fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let lon_scale = a.0.to_radians().cos();
    let xy = |q: (f64, f64)| ((q.1 - a.1) * lon_scale * METRES_PER_DEGREE, (q.0 - a.0) * METRES_PER_DEGREE);
    let (px, py) = xy(p);
    let (bx, by) = xy(b);
    let len2 = bx * bx + by * by;
    let t = if len2 <= 0.0 { 0.0 } else { ((px * bx + py * by) / len2).clamp(0.0, 1.0) };
    let (dx, dy) = (px - t * bx, py - t * by);
    ((dx * dx + dy * dy).sqrt(), t)
}

/// The words of an announcement: "In 150 m, turn left onto Main Street", or just the instruction when it is now.
/// `distance_text` is the distance in the person's units.
pub fn announcement_text(instruction: &str, distance_text: &str, now: bool) -> String {
    if now {
        return instruction.to_string();
    }
    let mut chars = instruction.chars();
    let lowered = chars.next().map_or(String::new(), |c| c.to_lowercase().collect::<String>() + chars.as_str());
    format!("In {distance_text}, {lowered}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::Step;

    const A: (f64, f64) = (64.000, -21.0);
    /// 1000 m north of A
    fn b() -> (f64, f64) {
        (A.0 + 1000.0 / METRES_PER_DEGREE, A.1)
    }
    /// 1000 m east of B
    fn c() -> (f64, f64) {
        (b().0, A.1 + 1000.0 / (METRES_PER_DEGREE * b().0.to_radians().cos()))
    }

    fn north(m: f64) -> (f64, f64) {
        (A.0 + m / METRES_PER_DEGREE, A.1)
    }

    fn east_of_b(m: f64) -> (f64, f64) {
        (b().0, A.1 + m / (METRES_PER_DEGREE * b().0.to_radians().cos()))
    }

    fn step(name: &str, maneuver: &str, distance_m: f64) -> Step {
        Step { name: name.into(), maneuver: maneuver.into(), distance_m }
    }

    /// North for 1000 m, then a right turn and east for 1000 m.
    fn l_route() -> Route {
        Route {
            distance_m: 2000.0,
            duration_s: 200.0,
            steps: vec![step("Main Street", "depart", 1000.0), step("Side Road", "turn right", 1000.0), step("", "arrive", 0.0)],
            approximate: false,
            geometry: vec![A, b(), c()],
        }
    }

    fn nav() -> Navigator {
        Navigator::new(l_route(), Profile::Driving).unwrap()
    }

    #[test]
    fn only_a_computed_route_with_steps_can_be_followed() {
        assert!(Navigator::new(l_route(), Profile::Driving).is_some());
        assert!(Navigator::new(Route { approximate: true, ..l_route() }, Profile::Driving).is_none());
        assert!(Navigator::new(Route { steps: vec![], ..l_route() }, Profile::Driving).is_none());
        assert!(Navigator::new(Route { geometry: vec![A], ..l_route() }, Profile::Driving).is_none());
        assert!(Navigator::new(l_route(), Profile::Transit).is_none());
    }

    #[test]
    fn at_the_start_the_first_turn_is_a_kilometre_ahead() {
        let g = nav().update(A, 10.0);
        assert_eq!(g.ahead, Some(1));
        assert!((g.distance_m - 1000.0).abs() < 2.0, "{}", g.distance_m);
        assert!((g.remaining_m - 2000.0).abs() < 4.0);
        assert!(!g.off_route && !g.arrived);
        assert_eq!(g.announce, None, "a kilometre is too far to announce");
    }

    #[test]
    fn the_next_turn_is_announced_once_at_each_distance() {
        let mut n = nav();
        assert_eq!(n.update(north(400.0), 10.0).announce, None, "600 m away");
        let a = n.update(north(520.0), 10.0).announce.expect("480 m is inside the far distance");
        assert_eq!(a.step, 1);
        assert!(!a.now);
        assert!((a.distance_m - 480.0).abs() < 2.0);
        assert_eq!(n.update(north(600.0), 10.0).announce, None);
        let near = n.update(north(860.0), 10.0).announce.expect("140 m is inside the near distance");
        assert!(!near.now && (near.distance_m - 140.0).abs() < 2.0);
        let now = n.update(north(980.0), 10.0).announce.expect("20 m is inside the last distance");
        assert!(now.now);
        assert_eq!(n.update(north(990.0), 10.0).announce, None, "each call is made once");
    }

    #[test]
    fn a_driver_who_starts_close_to_the_turn_hears_only_the_nearest_call() {
        let mut n = nav();
        let a = n.update(north(900.0), 10.0).announce.expect("100 m away");
        assert!(!a.now);
        assert_eq!(n.update(north(920.0), 10.0).announce, None, "the far call is not made late");
    }

    #[test]
    fn walkers_are_called_at_shorter_distances() {
        let mut n = Navigator::new(l_route(), Profile::Walking).unwrap();
        assert_eq!(n.update(north(700.0), 10.0).announce, None, "300 m is far for a walker");
        assert!(n.update(north(930.0), 10.0).announce.is_some());
    }

    #[test]
    fn after_the_turn_the_arrival_is_the_manoeuvre_ahead() {
        let mut n = nav();
        n.update(north(990.0), 10.0);
        let g = n.update(east_of_b(300.0), 10.0);
        assert_eq!(g.ahead, Some(2), "the arrival step");
        assert!((g.distance_m - 700.0).abs() < 3.0, "{}", g.distance_m);
        assert!((g.remaining_s - 70.0).abs() < 1.0, "{}", g.remaining_s);
    }

    #[test]
    fn arriving_is_announced_once() {
        let mut n = nav();
        n.update(east_of_b(500.0), 10.0);
        let g = n.update(east_of_b(990.0), 10.0);
        assert!(g.arrived);
        let a = g.announce.expect("arrival call");
        assert_eq!((a.step, a.now), (2, true));
        assert_eq!(n.update(east_of_b(995.0), 10.0).announce, None);
    }

    #[test]
    fn a_fix_far_from_the_line_is_off_route() {
        let mut n = nav();
        let beside = (north(500.0).0, A.1 - 200.0 / (METRES_PER_DEGREE * A.0.to_radians().cos()));
        let g = n.update(beside, 10.0);
        assert!(g.off_route);
        assert_eq!(g.announce, None, "nothing is announced off the route");
        assert!(!g.arrived);
    }

    #[test]
    fn a_coarse_fix_widens_what_counts_as_the_route() {
        let mut n = nav();
        let beside = (north(500.0).0, A.1 - 200.0 / (METRES_PER_DEGREE * A.0.to_radians().cos()));
        assert!(!n.update(beside, 300.0).off_route, "200 m away is inside a 300 m fix");
    }

    #[test]
    fn coming_back_to_the_route_clears_off_route() {
        let mut n = nav();
        let beside = (north(500.0).0, A.1 - 200.0 / (METRES_PER_DEGREE * A.0.to_radians().cos()));
        assert!(n.update(beside, 10.0).off_route);
        assert!(!n.update(north(520.0), 10.0).off_route);
    }

    #[test]
    fn progress_follows_a_route_that_doubles_back_beside_itself() {
        // out 1000 m north, then back south on a parallel line 20 m east, so both legs are near the same points
        let off = 20.0 / (METRES_PER_DEGREE * A.0.to_radians().cos());
        let up = (north(1000.0).0, A.1);
        let top = (up.0, A.1 + off);
        let back = (A.0, A.1 + off);
        let route = Route {
            distance_m: 2000.0,
            duration_s: 200.0,
            steps: vec![step("Up", "depart", 1000.0), step("Down", "turn right", 1000.0), step("", "arrive", 0.0)],
            approximate: false,
            geometry: vec![A, up, top, back],
        };
        let mut n = Navigator::new(route, Profile::Driving).unwrap();
        n.update(north(900.0), 5.0);
        let g = n.update((north(1000.0).0, A.1 + off / 4.0), 5.0);
        assert_eq!(g.ahead, Some(1), "still on the way up at the top");
        let down = n.update((north(800.0).0, A.1 + off), 5.0);
        assert_eq!(down.ahead, Some(2), "now on the way back");
    }

    #[test]
    fn steps_are_scaled_onto_the_line() {
        // the server says 1100 m and 1100 m, the line is 2000 m: the turn is still at the corner
        let mut r = l_route();
        r.steps[0].distance_m = 1100.0;
        r.steps[1].distance_m = 1100.0;
        let mut n = Navigator::new(r, Profile::Driving).unwrap();
        let g = n.update(north(500.0), 10.0);
        assert!((g.distance_m - 500.0).abs() < 2.0, "{}", g.distance_m);
    }

    #[test]
    fn announcement_words() {
        assert_eq!(announcement_text("Turn left onto Main Street", "150 m", false), "In 150 m, turn left onto Main Street");
        assert_eq!(announcement_text("Turn left onto Main Street", "30 m", true), "Turn left onto Main Street");
    }
}
