//! Screen pixel sampling for apps that show what is under the pointer (the Color Meter). A sample is
//! a request to the shell's `screen` service for the codes of one rectangle of the display. The shell
//! answers only when the app holds the Screen permission (see `privacy_permissions::can_capture`), so
//! callers check that first and treat an error reply as "nothing readable".
//!
//! Replies are asynchronous, like every service call: each request returns a request id, and the reply
//! arrives in `App::service_reply`, where [`decode`], [`decode_size`] or [`decode_point`] reads it.
//! Shells that predate the service answer with an error, which callers treat as "nothing readable".

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

const SERVICE: &str = "screen";

/// The sample reply: `{"w": width, "h": height, "rgb": [r, g, b, ...]}`, row by row, 3 bytes per pixel.
/// Returns `None` for any other shape, or when the byte count does not match the size.
pub fn decode(reply: &Json) -> Option<(u32, u32, Vec<u8>)> {
    let w = reply.get("w")?.as_u64()? as u32;
    let h = reply.get("h")?.as_u64()? as u32;
    let bytes: Vec<u8> = reply
        .get("rgb")?
        .as_array()?
        .iter()
        .map(|v| match v {
            Json::Num(n) if (0.0..=255.0).contains(n) && n.fract() == 0.0 => Some(*n as u8),
            _ => None,
        })
        .collect::<Option<Vec<u8>>>()?;
    (bytes.len() == (w as usize) * (h as usize) * 3).then_some((w, h, bytes))
}

/// The `display` reply: `{"w": width, "h": height}`, the frame size in display pixels.
pub fn decode_size(reply: &Json) -> Option<(u32, u32)> {
    let w = reply.get("w")?.as_u64()? as u32;
    let h = reply.get("h")?.as_u64()? as u32;
    Some((w, h))
}

/// The `pointer` reply: `{"x": x, "y": y}` in display pixels, or null when the pointer is not on the display.
pub fn decode_point(reply: &Json) -> Option<(u32, u32)> {
    let x = reply.get("x")?.as_u64()? as u32;
    let y = reply.get("y")?.as_u64()? as u32;
    Some((x, y))
}

/// Asks the shell for the codes of the display rectangle `(x, y, w, h)` in display pixels.
pub fn sample(ctx: &Ctx, x: u32, y: u32, w: u32, h: u32) -> u32 {
    let params = Json::Obj(vec![
        ("x".into(), Json::Num(x as f64)),
        ("y".into(), Json::Num(y as f64)),
        ("w".into(), Json::Num(w as f64)),
        ("h".into(), Json::Num(h as f64)),
    ]);
    service::call(ctx, SERVICE, "sample", &params)
}

/// Asks the shell for the frame size in display pixels (see [`decode_size`]).
pub fn display(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "display", &Json::Null)
}

/// Asks the shell where the pointer is in display pixels (see [`decode_point`]).
pub fn pointer(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "pointer", &Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_matching_reply() {
        let reply = Json::Obj(vec![
            ("w".into(), Json::Num(2.0)),
            ("h".into(), Json::Num(1.0)),
            ("rgb".into(), Json::Arr(vec![Json::Num(1.0), Json::Num(2.0), Json::Num(3.0), Json::Num(4.0), Json::Num(5.0), Json::Num(6.0)])),
        ]);
        assert_eq!(decode(&reply), Some((2, 1, vec![1, 2, 3, 4, 5, 6])));
    }

    #[test]
    fn decodes_the_display_and_pointer_replies() {
        let size = Json::Obj(vec![("w".into(), Json::Num(1280.0)), ("h".into(), Json::Num(800.0))]);
        assert_eq!(decode_size(&size), Some((1280, 800)));
        assert_eq!(decode_size(&Json::Null), None);
        let point = Json::Obj(vec![("x".into(), Json::Num(5.0)), ("y".into(), Json::Num(9.0))]);
        assert_eq!(decode_point(&point), Some((5, 9)));
        // off the display the shell answers null, which is no position
        assert_eq!(decode_point(&Json::Null), None);
    }

    #[test]
    fn refuses_a_short_or_malformed_reply() {
        let short = Json::Obj(vec![
            ("w".into(), Json::Num(2.0)),
            ("h".into(), Json::Num(1.0)),
            ("rgb".into(), Json::Arr(vec![Json::Num(1.0)])),
        ]);
        assert_eq!(decode(&short), None);
        let bad = Json::Obj(vec![
            ("w".into(), Json::Num(1.0)),
            ("h".into(), Json::Num(1.0)),
            ("rgb".into(), Json::Arr(vec![Json::Num(300.0), Json::Num(0.0), Json::Num(0.0)])),
        ]);
        assert_eq!(decode(&bad), None);
        assert_eq!(decode(&Json::Null), None);
    }
}
