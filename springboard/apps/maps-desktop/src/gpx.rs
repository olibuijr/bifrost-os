//! GPX files: waypoints become pins and each track a line on the map. The reader is a small tag scanner, enough for the
//! `wpt`, `trk`, `trkseg`, `trkpt` and `name` elements of GPX 1.0 and 1.1.

/// What a GPX file holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpx {
    /// waypoints: (name, lat, lon)
    pub waypoints: Vec<(String, f64, f64)>,
    /// tracks: (name, points as (lat, lon))
    pub tracks: Vec<(String, Vec<(f64, f64)>)>,
}

impl Gpx {
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty() && self.tracks.iter().all(|(_, pts)| pts.is_empty())
    }

    /// Every track point, for fitting the view and measuring.
    pub fn all_points(&self) -> Vec<(f64, f64)> {
        self.waypoints.iter().map(|(_, a, b)| (*a, *b)).chain(self.tracks.iter().flat_map(|(_, p)| p.iter().copied())).collect()
    }
}

/// The attribute value in a start tag's text: `lat="64.1"` gives `64.1`. Single quotes work too.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = tag[from..].find(name) {
        let start = from + at;
        let before_ok = start == 0 || tag[..start].ends_with(|c: char| c.is_whitespace());
        let after = &tag[start + name.len()..];
        let after = after.trim_start();
        if before_ok && after.starts_with('=') {
            let value = after[1..].trim_start();
            let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let rest = &value[1..];
            let end = rest.find(quote)?;
            return Some(rest[..end].to_string());
        }
        from = start + name.len();
    }
    None
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text of the first `<name>` element in `body`, with entities and CDATA resolved.
fn name_of(body: &str) -> String {
    let Some(start) = body.find("<name>").map(|i| i + 6) else { return String::new() };
    let Some(len) = body[start..].find("</name>") else { return String::new() };
    let raw = body[start..start + len].trim();
    let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).map_or_else(|| unescape(raw), str::to_string);
    raw.trim().to_string()
}

fn position(tag: &str) -> Option<(f64, f64)> {
    let lat: f64 = attribute(tag, "lat")?.trim().parse().ok()?;
    let lon: f64 = attribute(tag, "lon")?.trim().parse().ok()?;
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

/// The elements `<tag ...>...</tag>` (and self-closing `<tag .../>`) of `text`, as (start tag text, body).
fn elements<'a>(text: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find(&open) {
        let start = at + i;
        let after = start + open.len();
        // `<wpt` must not match `<wptx`
        if !text[after..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            at = after;
            continue;
        }
        let Some(gt) = text[after..].find('>') else { break };
        let tag_end = after + gt;
        let head = &text[start..tag_end];
        if head.ends_with('/') {
            out.push((head, ""));
            at = tag_end + 1;
            continue;
        }
        let Some(end) = text[tag_end + 1..].find(&close) else { break };
        out.push((head, &text[tag_end + 1..tag_end + 1 + end]));
        at = tag_end + 1 + end + close.len();
    }
    out
}

/// Reads a GPX document. Text that holds no waypoint and no track point is an error.
pub fn parse(text: &str) -> Result<Gpx, String> {
    if !text.contains("<gpx") {
        return Err("this is not a GPX file".into());
    }
    let mut gpx = Gpx::default();
    // waypoints are the `wpt` elements outside tracks; track points are `trkpt`, so the names cannot mix
    for (head, body) in elements(text, "wpt") {
        if let Some((lat, lon)) = position(head) {
            gpx.waypoints.push((name_of(body), lat, lon));
        }
    }
    for (_, track) in elements(text, "trk") {
        let mut points = Vec::new();
        for (_, seg) in elements(track, "trkseg") {
            for (head, _) in elements(seg, "trkpt") {
                if let Some(p) = position(head) {
                    points.push(p);
                }
            }
        }
        // the track's own name is the first `<name>` before its first segment
        let before_segment = track.split("<trkseg").next().unwrap_or("");
        gpx.tracks.push((name_of(before_segment), points));
    }
    if gpx.is_empty() {
        return Err("the GPX file has no waypoints or track points".into());
    }
    Ok(gpx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="x" xmlns="http://www.topografix.com/GPX/1/1">
  <wpt lat="64.1466" lon="-21.9426"><name>Café &amp; Bar</name></wpt>
  <wpt lat='65.68' lon='-18.12'><name><![CDATA[Akureyri <north>]]></name></wpt>
  <wpt lat="200" lon="0"><name>Broken</name></wpt>
  <trk><name>Morning run</name><trkseg>
    <trkpt lat="64.10" lon="-21.90"><ele>5</ele></trkpt>
    <trkpt lat="64.11" lon="-21.91"/>
    <trkpt lat="x" lon="1"></trkpt>
  </trkseg></trk>
</gpx>"#;

    #[test]
    fn waypoints_and_tracks_are_read() {
        let gpx = parse(SAMPLE).unwrap();
        assert_eq!(gpx.waypoints.len(), 2, "the waypoint outside the world is skipped");
        assert_eq!(gpx.waypoints[0], ("Café & Bar".to_string(), 64.1466, -21.9426));
        assert_eq!(gpx.waypoints[1], ("Akureyri <north>".to_string(), 65.68, -18.12));
        assert_eq!(gpx.tracks.len(), 1);
        assert_eq!(gpx.tracks[0].0, "Morning run");
        assert_eq!(gpx.tracks[0].1, vec![(64.10, -21.90), (64.11, -21.91)]);
        assert_eq!(gpx.all_points().len(), 4);
    }

    #[test]
    fn text_that_is_not_gpx_or_holds_nothing_is_an_error() {
        assert!(parse("hello").is_err());
        assert!(parse("<gpx></gpx>").is_err());
        assert!(parse(r#"<gpx><wpt lat="1" lon="2"/></gpx>"#).unwrap().waypoints.len() == 1);
    }

    #[test]
    fn similar_element_names_do_not_match() {
        let gpx = parse(r#"<gpx><wptx lat="1" lon="2"></wptx><wpt lat="3" lon="4"></wpt></gpx>"#).unwrap();
        assert_eq!(gpx.waypoints, vec![(String::new(), 3.0, 4.0)]);
    }
}
