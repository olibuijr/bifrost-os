//! Parity app-maps: headless tests of the transit departure time, the stops in a journey leg, and the route and pin
//! overlays of the printed map picture. Nothing here opens a window or touches the network.

use bifrost_maps::printmap::{draw_dot, draw_line, draw_tiles};
use bifrost_maps::transit::{parse_itineraries, Departure, Planner};

const REYKJAVIK: (f64, f64) = (64.1466, -21.9426);
const KOPAVOGUR: (f64, f64) = (64.1090, -21.9100);
const WHITE: [u8; 3] = [255, 255, 255];
const INK: [u8; 3] = [10, 20, 30];

fn planner() -> Planner {
    Planner::new(Some("https://otp.example.org/otp/routers/default")).unwrap()
}

fn pixel(picture: &bifrost_app::print::PageImage, x: usize, y: usize) -> [u8; 3] {
    let i = (y * picture.width + x) * 3;
    [picture.rgb[i], picture.rgb[i + 1], picture.rgb[i + 2]]
}

#[test]
fn a_journey_now_asks_for_no_time() {
    let p = planner();
    assert_eq!(p.plan_url_at(REYKJAVIK, KOPAVOGUR, &Departure::Now).unwrap(), p.plan_url(REYKJAVIK, KOPAVOGUR).unwrap());
    assert!(!p.plan_url(REYKJAVIK, KOPAVOGUR).unwrap().contains("date="));
}

#[test]
fn a_leave_at_request_carries_the_local_date_and_time() {
    let when = Departure::leave_at("2026-10-14", "08:05").unwrap();
    let url = planner().plan_url_at(REYKJAVIK, KOPAVOGUR, &when).unwrap();
    assert!(url.ends_with("&numItineraries=3&date=2026-10-14&time=08:05"), "{url}");
    assert!(!url.contains("arriveBy"));
}

#[test]
fn an_arrive_by_request_sets_the_flag_after_the_time() {
    let when = Departure::arrive_by(" 2026-12-31 ", "23:59").unwrap();
    assert_eq!(when, Departure::ArriveBy { date: "2026-12-31".into(), time: "23:59".into() });
    let url = planner().plan_url_at(REYKJAVIK, KOPAVOGUR, &when).unwrap();
    assert!(url.ends_with("&date=2026-12-31&time=23:59&arriveBy=true"), "{url}");
}

#[test]
fn a_date_or_time_that_is_not_plain_digits_in_range_is_refused() {
    for date in ["2026/10/14", "14-10-2026", "2026-13-01", "2026-00-10", "2026-10-32", "2026-1-14", "２026-10-14"] {
        assert!(Departure::leave_at(date, "08:00").is_err(), "{date}");
    }
    for time in ["24:00", "08:60", "8:00", "08.00", "08:0a", "08:00:00", ""] {
        assert!(Departure::arrive_by("2026-10-14", time).is_err(), "{time}");
    }
}

#[test]
fn a_planner_without_a_url_refuses_a_timed_request_too() {
    let none = Planner::new(None).unwrap();
    let when = Departure::leave_at("2026-10-14", "08:00").unwrap();
    assert!(none.plan_url_at(REYKJAVIK, KOPAVOGUR, &when).is_err());
}

fn reply_with_stops(stops: &str) -> String {
    format!(
        r#"{{"plan":{{"itineraries":[{{"duration":960,"legs":[{{"mode":"BUS","routeShortName":"14","distance":5200,
        "startTime":1790000400000,"endTime":1790001360000,"agencyTimeZoneOffset":0,
        "from":{{"name":"Hlemmur","lat":64.1427,"lon":-21.9148}},"to":{{"name":"Hamraborg","lat":64.1090,"lon":-21.9100}},
        "intermediateStops":[{stops}]}}]}}]}}}}"#
    )
}

fn stop(name: &str) -> String {
    format!(r#"{{"name":"{name}","lat":64.13,"lon":-21.92}}"#)
}

#[test]
fn a_leg_names_the_stops_in_between() {
    let stops = [stop("Snorrabraut"), stop("Hofsvallagata"), stop("Hringbraut")].join(",");
    let routes = parse_itineraries(&reply_with_stops(&stops)).unwrap();
    assert_eq!(routes[0].steps[0].instruction(), "Bus 14 from Hlemmur 14:20 to Hamraborg 14:36, 3 stops in between");
}

#[test]
fn one_stop_in_between_is_said_in_the_singular() {
    let routes = parse_itineraries(&reply_with_stops(&stop("Snorrabraut"))).unwrap();
    assert_eq!(routes[0].steps[0].instruction(), "Bus 14 from Hlemmur 14:20 to Hamraborg 14:36, 1 stop in between");
}

#[test]
fn a_leg_with_no_stops_in_between_names_none() {
    let routes = parse_itineraries(&reply_with_stops("")).unwrap();
    assert_eq!(routes[0].steps[0].instruction(), "Bus 14 from Hlemmur 14:20 to Hamraborg 14:36");
}

#[test]
fn a_line_is_drawn_over_the_picture_between_its_points() {
    let mut picture = draw_tiles(&[], 10.0, 40.0, 30.0);
    draw_line(&mut picture, &[(0.0, 5.0), (20.0, 5.0)], 2.0, INK);
    // the line is 2 points thick: 4 pixels, centred on the pixel row 10
    assert_eq!(pixel(&picture, 20, 10), INK);
    assert_eq!(pixel(&picture, 40, 10), INK, "the stamp at the last point covers the pixel column 40");
    assert_eq!(pixel(&picture, 20, 2), WHITE, "well above the line");
    assert_eq!(pixel(&picture, 20, 19), WHITE, "well below the line");
}

#[test]
fn a_diagonal_line_has_no_gaps() {
    let mut picture = draw_tiles(&[], 10.0, 40.0, 30.0);
    draw_line(&mut picture, &[(0.0, 0.0), (10.0, 10.0)], 1.0, INK);
    for k in 0..=20 {
        assert_eq!(pixel(&picture, k, k), INK, "pixel {k} on the diagonal");
    }
}

#[test]
fn a_line_off_the_picture_is_clipped_and_one_point_draws_nothing() {
    let mut picture = draw_tiles(&[], 10.0, 40.0, 30.0);
    draw_line(&mut picture, &[(-50.0, -50.0), (-40.0, -40.0)], 2.0, INK);
    assert!(picture.rgb.iter().all(|&c| c == 255));
    draw_line(&mut picture, &[(5.0, 5.0)], 2.0, INK);
    assert!(picture.rgb.iter().all(|&c| c == 255), "a single point is not a line");
}

#[test]
fn a_pin_is_a_dot_at_its_point() {
    let mut picture = draw_tiles(&[], 10.0, 40.0, 30.0);
    draw_dot(&mut picture, 5.0, 5.0, 1.0, INK);
    assert_eq!(pixel(&picture, 10, 10), INK);
    assert_eq!(pixel(&picture, 0, 0), WHITE);
    assert_eq!(pixel(&picture, 30, 20), WHITE);
}
