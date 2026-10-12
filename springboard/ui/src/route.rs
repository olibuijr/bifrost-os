//! Routes as pure data: an ordered list of steps, each with written instructions, a distance, a transport type and
//! the geometry of its path. The route's distance and geometry are sums and joins of the steps, and a caller can ask
//! which step is current after travelling a given distance along the route. Nothing here plans a route or asks a
//! service for one: the caller builds the steps from data it already has. The types mirror the official route and
//! route step classes in outline; expected travel time is left to the caller because it needs a speed model.

use crate::geo::Coordinate;

/// How a route or a step is travelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportType {
    Automobile,
    Walking,
    Transit,
}

/// One portion of a route: the instruction to follow it, its length and its path.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteStep {
    /// The written instruction. May be empty for a step that needs no words, such as a straight continuation.
    pub instructions: String,
    /// An advisory note for the step, such as a toll or a closure. Empty when there is none.
    pub notice: String,
    /// Length of the step in metres, never negative.
    pub distance_m: f64,
    pub transport: TransportType,
    /// The path of the step, in order. May be empty when the caller has no geometry.
    pub polyline: Vec<Coordinate>,
}

impl RouteStep {
    /// `None` when `distance_m` is not finite or is negative.
    pub fn new(
        instructions: &str,
        distance_m: f64,
        transport: TransportType,
        polyline: Vec<Coordinate>,
    ) -> Option<Self> {
        if !distance_m.is_finite() || distance_m < 0.0 {
            return None;
        }
        Some(RouteStep {
            instructions: instructions.to_string(),
            notice: String::new(),
            distance_m,
            transport,
            polyline,
        })
    }
}

/// An ordered route with at least one step.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The name the route is known by, such as a road name. May be empty.
    pub name: String,
    pub transport: TransportType,
    pub steps: Vec<RouteStep>,
}

impl Route {
    /// `None` when `steps` is empty.
    pub fn new(name: &str, transport: TransportType, steps: Vec<RouteStep>) -> Option<Self> {
        (!steps.is_empty()).then(|| Route { name: name.to_string(), transport, steps })
    }

    /// Total length of the route in metres: the sum of the step distances.
    pub fn distance_m(&self) -> f64 {
        self.steps.iter().map(|s| s.distance_m).sum()
    }

    /// The geometry of the whole route. The first point of a step that starts where the previous step ended is not
    /// repeated, so consecutive steps join into one line.
    pub fn polyline(&self) -> Vec<Coordinate> {
        let mut out: Vec<Coordinate> = Vec::new();
        for step in &self.steps {
            for (i, p) in step.polyline.iter().enumerate() {
                if i == 0 && out.last() == Some(p) {
                    continue;
                }
                out.push(*p);
            }
        }
        out
    }

    /// The non-empty instructions, in route order.
    pub fn instructions(&self) -> Vec<&str> {
        self.steps.iter().map(|s| s.instructions.as_str()).filter(|s| !s.is_empty()).collect()
    }

    /// The index of the step that covers the point `along_m` metres from the start. A step owns the interval from its
    /// start up to, not including, its end, so a distance on a boundary belongs to the next step. Steps of length
    /// zero own nothing. `None` for a negative or non-finite distance, or one at or past the end of the route.
    pub fn step_at(&self, along_m: f64) -> Option<usize> {
        if !along_m.is_finite() || along_m < 0.0 {
            return None;
        }
        let mut start = 0.0;
        for (i, step) in self.steps.iter().enumerate() {
            let end = start + step.distance_m;
            if along_m < end {
                return Some(i);
            }
            start = end;
        }
        None
    }

    /// Metres left to travel after `along_m` metres from the start. Clamped to the route: it is the full length
    /// before the start and zero past the end.
    pub fn remaining_m(&self, along_m: f64) -> f64 {
        let total = self.distance_m();
        if along_m.is_nan() {
            return total;
        }
        (total - along_m.max(0.0)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    fn step(text: &str, distance_m: f64, polyline: Vec<Coordinate>) -> RouteStep {
        RouteStep::new(text, distance_m, TransportType::Automobile, polyline).expect("valid step")
    }

    #[test]
    fn distance_is_the_sum_of_the_steps() {
        let route = Route::new(
            "Route 1",
            TransportType::Automobile,
            vec![step("Head north", 120.0, vec![]), step("Continue", 0.0, vec![]), step("Turn right", 480.5, vec![])],
        )
        .expect("one or more steps");
        assert!((route.distance_m() - 600.5).abs() < 1e-9);
    }

    #[test]
    fn step_at_gives_the_step_that_owns_each_distance() {
        // Steps of 100, 50 and 200 metres: the route is 350 metres long.
        let route = Route::new(
            "",
            TransportType::Walking,
            vec![step("A", 100.0, vec![]), step("B", 50.0, vec![]), step("C", 200.0, vec![])],
        )
        .expect("steps");
        assert_eq!(route.step_at(0.0), Some(0));
        assert_eq!(route.step_at(99.9), Some(0));
        // A boundary belongs to the next step.
        assert_eq!(route.step_at(100.0), Some(1));
        assert_eq!(route.step_at(149.0), Some(1));
        assert_eq!(route.step_at(150.0), Some(2));
        assert_eq!(route.step_at(349.9), Some(2));
        // At or past the end, and before the start, there is no step.
        assert_eq!(route.step_at(350.0), None);
        assert_eq!(route.step_at(-1.0), None);
        assert_eq!(route.step_at(f64::NAN), None);
    }

    #[test]
    fn step_at_skips_steps_of_zero_length() {
        let route = Route::new(
            "",
            TransportType::Automobile,
            vec![step("A", 100.0, vec![]), step("Zero", 0.0, vec![]), step("C", 50.0, vec![])],
        )
        .expect("steps");
        assert_eq!(route.step_at(100.0), Some(2));
    }

    #[test]
    fn remaining_distance_is_clamped_to_the_route() {
        let route = Route::new(
            "",
            TransportType::Transit,
            vec![step("A", 100.0, vec![]), step("B", 50.0, vec![]), step("C", 200.0, vec![])],
        )
        .expect("steps");
        assert!((route.remaining_m(120.0) - 230.0).abs() < 1e-9);
        assert!((route.remaining_m(-5.0) - 350.0).abs() < 1e-9);
        assert!(route.remaining_m(400.0).abs() < 1e-9);
    }

    #[test]
    fn polyline_joins_steps_without_repeating_the_shared_point() {
        let p1 = c(0.0, 0.0);
        let p2 = c(0.0, 0.01);
        let p3 = c(0.01, 0.01);
        let p4 = c(0.02, 0.02);
        let route = Route::new(
            "",
            TransportType::Automobile,
            vec![step("A", 1111.9, vec![p1, p2]), step("B", 1111.9, vec![p2, p3]), step("C", 10.0, vec![p4])],
        )
        .expect("steps");
        // p2 is shared by the first two steps and appears once; p4 starts a separate piece and is kept.
        assert_eq!(route.polyline(), vec![p1, p2, p3, p4]);
    }

    #[test]
    fn instructions_skip_empty_text() {
        let route = Route::new(
            "",
            TransportType::Walking,
            vec![step("Turn left", 10.0, vec![]), step("", 20.0, vec![]), step("Arrive", 0.0, vec![])],
        )
        .expect("steps");
        assert_eq!(route.instructions(), vec!["Turn left", "Arrive"]);
    }

    #[test]
    fn invalid_steps_and_empty_routes_are_rejected() {
        assert!(RouteStep::new("x", -1.0, TransportType::Walking, vec![]).is_none());
        assert!(RouteStep::new("x", f64::NAN, TransportType::Walking, vec![]).is_none());
        assert!(RouteStep::new("x", 0.0, TransportType::Walking, vec![]).is_some());
        assert!(Route::new("", TransportType::Walking, Vec::new()).is_none());
    }
}
