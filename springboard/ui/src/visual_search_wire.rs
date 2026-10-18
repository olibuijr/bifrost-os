//! The wire format of a visual search between the shell and the apps (the transport is each app's control socket: one
//! JSON-RPC request line, one reply line). Pure encoding and decoding, shared by the shell, which sends scenes, and the
//! SDK, which answers them (`bifrost_app::visual_search_wire`).
//!
//! `request_line` shrinks the scene so the line stays under the control endpoint's request limit. Params of
//! `visual_search.search`: `{kind: "screenshot"|"photo"|"stream", width, height, labels: [..], pixels: base64 of
//! width * height RGBA8 bytes}`. Result: `[{intent, title, subtitle?, score}]`; the app id is not sent, the shell stamps
//! it from the socket it asked.

use crate::visual_search::{Match, Scene, SceneError, SceneKind, MAX_MATCHES};
use bifrost_core::json::{self, Json};
use bifrost_core::plist::{base64_decode, base64_encode};

/// The control method an app serves to answer a visual search.
pub const METHOD: &str = "visual_search.search";
/// The longest side of a delivered scene, in pixels. A 192 x 192 scene is 196 KB of base64, inside the 256 KiB the
/// control endpoint accepts for one request.
pub const WIRE_MAX_SIDE: u32 = 192;
/// The manifest `[[intent]]` id an app declares to say that it answers visual searches.
pub const PROVIDER_INTENT: &str = "visual-search";

/// Why a scene or a reply could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The text is not JSON.
    Json(String),
    /// A field is missing or has the wrong type.
    Missing(&'static str),
    /// The pixels do not make a scene.
    Scene(SceneError),
    /// The pixels are not base64.
    Pixels,
    /// The app answered with an error.
    Remote(String),
}

fn kind_word(kind: SceneKind) -> &'static str {
    match kind {
        SceneKind::Screenshot => "screenshot",
        SceneKind::Photo => "photo",
        SceneKind::Stream => "stream",
    }
}

fn kind_from(word: &str) -> Option<SceneKind> {
    match word {
        "screenshot" => Some(SceneKind::Screenshot),
        "photo" => Some(SceneKind::Photo),
        "stream" => Some(SceneKind::Stream),
        _ => None,
    }
}

/// A scene from the shell's packed pixels (`0xAABBGGRR` per `u32`, red in the low byte, as the screenshot encoder
/// reads them), `width` x `height` of them.
pub fn scene_from_pixels(kind: SceneKind, width: u32, height: u32, pixels: &[u32]) -> Result<Scene, SceneError> {
    let mut bytes = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        bytes.extend_from_slice(&p.to_le_bytes());
    }
    Scene::new(kind, width, height, bytes)
}

/// The scene shrunk so that no side is longer than [`WIRE_MAX_SIDE`], by nearest-neighbour sampling. A scene that is
/// small enough is returned as it is. Labels and kind are kept.
pub fn fit(scene: &Scene) -> Scene {
    let longest = scene.width.max(scene.height);
    if longest <= WIRE_MAX_SIDE {
        return scene.clone();
    }
    let scale = WIRE_MAX_SIDE as f64 / longest as f64;
    let w = ((scene.width as f64 * scale).round() as u32).max(1);
    let h = ((scene.height as f64 * scale).round() as u32).max(1);
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let sy = ((y as u64 * scene.height as u64) / h as u64) as u32;
        for x in 0..w {
            let sx = ((x as u64 * scene.width as u64) / w as u64) as u32;
            pixels.extend_from_slice(&scene.pixel(sx, sy).unwrap_or([0, 0, 0, 255]));
        }
    }
    Scene { kind: scene.kind, width: w, height: h, pixels, labels: scene.labels.clone() }
}

/// The JSON-RPC request line (newline-terminated) that asks an app to search for `scene`. The scene is shrunk first.
pub fn request_line(scene: &Scene, id: u64) -> String {
    let s = fit(scene);
    let params = Json::Obj(vec![
        ("kind".to_string(), Json::Str(kind_word(s.kind).to_string())),
        ("width".to_string(), Json::Num(s.width as f64)),
        ("height".to_string(), Json::Num(s.height as f64)),
        ("labels".to_string(), Json::Arr(s.labels.iter().map(|l| Json::Str(l.clone())).collect())),
        ("pixels".to_string(), Json::Str(base64_encode(&s.pixels))),
    ]);
    let request = Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), Json::Num(id as f64)),
        ("method".to_string(), Json::Str(METHOD.to_string())),
        ("params".to_string(), params),
    ]);
    format!("{}\n", request.encode())
}

/// The scene a request's params describe (the app side of [`request_line`]).
pub fn scene_from_params(params: &Json) -> Result<Scene, WireError> {
    let kind = params.get("kind").and_then(Json::as_str).and_then(kind_from).ok_or(WireError::Missing("kind"))?;
    let dimension = |key: &'static str| params.get(key).and_then(Json::as_u64).filter(|v| *v <= u32::MAX as u64).map(|v| v as u32).ok_or(WireError::Missing(key));
    let (width, height) = (dimension("width")?, dimension("height")?);
    let pixels = base64_decode(params.get("pixels").and_then(Json::as_str).ok_or(WireError::Missing("pixels"))?).ok_or(WireError::Pixels)?;
    let scene = Scene::new(kind, width, height, pixels).map_err(WireError::Scene)?;
    let labels: Vec<&str> = params.get("labels").and_then(Json::as_array).unwrap_or_default().iter().filter_map(Json::as_str).collect();
    Ok(scene.with_labels(&labels))
}

/// The matches as the result of a search call. The app id is not sent: the shell stamps it.
pub fn matches_json(matches: &[Match]) -> Json {
    Json::Arr(
        matches
            .iter()
            .take(MAX_MATCHES)
            .map(|m| {
                Json::Obj(vec![
                    ("intent".to_string(), Json::Str(m.intent.clone())),
                    ("title".to_string(), Json::Str(m.title.clone())),
                    ("subtitle".to_string(), Json::Str(m.subtitle.clone())),
                    ("score".to_string(), Json::Num(m.score as f64)),
                ])
            })
            .collect(),
    )
}

/// The matches in an app's reply line (the shell side), with an empty `app`. At most [`MAX_MATCHES`] are read; a score
/// above 100 is capped. An error reply is `Err(Remote)`.
pub fn matches_from_reply(line: &str) -> Result<Vec<Match>, WireError> {
    let reply = json::parse(line.trim()).map_err(WireError::Json)?;
    if let Some(err) = reply.get("error") {
        return Err(WireError::Remote(err.get("message").and_then(Json::as_str).unwrap_or("error").to_string()));
    }
    let list = reply.get("result").and_then(Json::as_array).ok_or(WireError::Missing("result"))?;
    Ok(list
        .iter()
        .take(MAX_MATCHES)
        .filter_map(|m| {
            Some(Match {
                app: String::new(),
                intent: m.get("intent").and_then(Json::as_str)?.to_string(),
                title: m.get("title").and_then(Json::as_str)?.to_string(),
                subtitle: m.get("subtitle").and_then(Json::as_str).unwrap_or("").to_string(),
                score: m.get("score").and_then(Json::as_u64).unwrap_or(0).min(100) as u32,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Scene {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 7, 255]);
            }
        }
        Scene::new(SceneKind::Screenshot, w, h, px).unwrap().with_labels(&["Plant", "leaf"])
    }

    #[test]
    fn a_scene_survives_the_trip_through_a_request() {
        let scene = gradient(40, 30);
        let params = json::parse(&request_line(&scene, 3)).unwrap();
        assert_eq!(params.get("method").and_then(Json::as_str), Some(METHOD));
        let seen = scene_from_params(params.get("params").unwrap()).unwrap();
        assert_eq!(seen, scene);
    }

    #[test]
    fn a_large_scene_is_shrunk_to_fit_one_control_request() {
        let big = gradient(1000, 500);
        let line = request_line(&big, 1);
        assert!(line.len() < 256 * 1024, "{} bytes", line.len());
        let params = json::parse(&line).unwrap();
        let seen = scene_from_params(params.get("params").unwrap()).unwrap();
        assert_eq!((seen.width, seen.height), (WIRE_MAX_SIDE, 96));
        assert_eq!(seen.labels, vec!["plant", "leaf"]);
        assert_eq!(seen.pixel(0, 0), big.pixel(0, 0), "the corner pixel comes from the matching corner of the source");
        let widest = gradient(WIRE_MAX_SIDE, WIRE_MAX_SIDE);
        assert_eq!(fit(&widest), widest, "a scene already small enough is not touched");
        assert!(request_line(&widest, 1).len() < 256 * 1024);
    }

    #[test]
    fn a_broken_scene_is_refused_with_the_field_that_is_wrong() {
        assert_eq!(scene_from_params(&Json::Obj(vec![])), Err(WireError::Missing("kind")));
        let bad = Json::Obj(vec![
            ("kind".into(), Json::Str("photo".into())),
            ("width".into(), Json::Num(2.0)),
            ("height".into(), Json::Num(2.0)),
            ("pixels".into(), Json::Str(base64_encode(&[0u8; 3]))),
        ]);
        assert_eq!(scene_from_params(&bad), Err(WireError::Scene(SceneError::PixelCount { expected: 16, got: 3 })));
        let not_base64 = Json::Obj(vec![
            ("kind".into(), Json::Str("photo".into())),
            ("width".into(), Json::Num(1.0)),
            ("height".into(), Json::Num(1.0)),
            ("pixels".into(), Json::Str("!!".into())),
        ]);
        assert_eq!(scene_from_params(&not_base64), Err(WireError::Pixels));
    }

    #[test]
    fn matches_round_trip_without_the_app_id() {
        let sent = vec![Match { app: "is.x.plants".into(), intent: "open".into(), title: "Fern".into(), subtitle: "Care".into(), score: 80 }];
        let reply = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}", matches_json(&sent).encode());
        let got = matches_from_reply(&reply).unwrap();
        assert_eq!(got, vec![Match { app: String::new(), ..sent[0].clone() }]);
    }

    #[test]
    fn replies_are_bounded_and_scores_capped() {
        let rows: Vec<String> = (0..40).map(|i| format!("{{\"intent\":\"i{i}\",\"title\":\"T{i}\",\"score\":{}}}", 500 + i)).collect();
        let reply = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{},{{\"title\":\"no intent\"}}]}}", rows.join(","));
        let got = matches_from_reply(&reply).unwrap();
        assert_eq!(got.len(), MAX_MATCHES);
        assert!(got.iter().all(|m| m.score == 100 && m.subtitle.is_empty()));
        let skipped = matches_from_reply("{\"result\":[{\"title\":\"no intent\"},{\"intent\":\"go\",\"title\":\"Ok\",\"score\":3}]}").unwrap();
        assert_eq!(skipped.len(), 1, "an entry without an intent is dropped");
    }

    #[test]
    fn an_error_reply_or_garbage_is_an_error() {
        assert_eq!(matches_from_reply("{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"no method\"}}"), Err(WireError::Remote("no method".into())));
        assert!(matches!(matches_from_reply("not json"), Err(WireError::Json(_))));
        assert_eq!(matches_from_reply("{}"), Err(WireError::Missing("result")));
    }

    #[test]
    fn shell_pixels_become_a_scene_with_red_first() {
        let scene = scene_from_pixels(SceneKind::Screenshot, 2, 1, &[0xFF04_0302, 0xFF09_0807]).unwrap();
        assert_eq!(scene.pixel(0, 0), Some([0x02, 0x03, 0x04, 0xFF]));
        assert_eq!(scene.pixel(1, 0), Some([0x07, 0x08, 0x09, 0xFF]));
        assert_eq!(scene_from_pixels(SceneKind::Photo, 2, 2, &[0]).err(), Some(SceneError::PixelCount { expected: 16, got: 4 }));
    }
}
