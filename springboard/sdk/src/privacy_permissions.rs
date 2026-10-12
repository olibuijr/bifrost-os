//! Typed client for the shell's `privacy` service: the camera, microphone and screen-recording permission this app has,
//! as the person answered it in Settings > Privacy > App Permissions.
//!
//! ```ignore
//! let id = privacy_permissions::status(ctx, Media::Video);
//! // in App::service_reply, once the reply for `id` arrives:
//! //   let status = privacy_permissions::decode_status(&reply);   // Some(Status::Authorized)
//! ```
//! The calls only ask; the shell never shows a prompt from them. An app that has not been answered yet reads
//! `NotDetermined`, which `can_capture` treats as not allowed. Decoding is pure, so the replies can be tested without a
//! shell. The wire names are the shell's (`springboard/src/shell/services/privacy_permissions.rs`).
use crate::controls::capture_access::{Media, Status};
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "privacy";

/// The wire name of a media type: `camera`, `microphone` or `screen`.
pub fn media_name(media: Media) -> &'static str {
    match media {
        Media::Video => "camera",
        Media::Audio => "microphone",
        Media::Screen => "screen",
    }
}

/// Every media type, in the order the shell lists them.
pub const MEDIA: [Media; 3] = [Media::Video, Media::Audio, Media::Screen];

fn media_params(media: Media) -> Json {
    Json::Obj(vec![("media".into(), Json::Str(media_name(media).into()))])
}

/// Asks for the status of one media type. The reply decodes with [`decode_status`].
pub fn status(ctx: &Ctx, media: Media) -> u32 {
    service::call(ctx, SERVICE, "status", &media_params(media))
}

/// Asks whether the app may open a device for one media type. The reply decodes with [`decode_allowed`].
pub fn can_capture(ctx: &Ctx, media: Media) -> u32 {
    service::call(ctx, SERVICE, "can_capture", &media_params(media))
}

/// Asks for the status of every media type. The reply decodes with [`decode_all`].
pub fn all(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "all", &Json::Null)
}

/// The status in a `status` reply, or `None` for a reply of another shape.
pub fn decode_status(reply: &Json) -> Option<Status> {
    parse_status(reply.get("status")?.as_str()?)
}

/// Whether a `can_capture` reply allows the media type, or `None` for a reply of another shape.
pub fn decode_allowed(reply: &Json) -> Option<bool> {
    match reply.get("allowed")? {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

/// The three statuses of an `all` reply, in [`MEDIA`] order, or `None` when one is missing or unknown.
pub fn decode_all(reply: &Json) -> Option<[(Media, Status); 3]> {
    let mut out = [(Media::Video, Status::NotDetermined); 3];
    for (slot, media) in out.iter_mut().zip(MEDIA) {
        *slot = (media, parse_status(reply.get(media_name(media))?.as_str()?)?);
    }
    Some(out)
}

fn parse_status(name: &str) -> Option<Status> {
    match name {
        "not_determined" => Some(Status::NotDetermined),
        "restricted" => Some(Status::Restricted),
        "denied" => Some(Status::Denied),
        "authorized" => Some(Status::Authorized),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_carry_the_media_name() {
        assert_eq!(media_params(Media::Audio).get("media").and_then(Json::as_str), Some("microphone"));
        assert_eq!(media_name(Media::Screen), "screen");
    }

    #[test]
    fn a_status_reply_decodes_to_its_status() {
        let reply = Json::Obj(vec![("media".into(), Json::Str("camera".into())), ("status".into(), Json::Str("authorized".into()))]);
        assert_eq!(decode_status(&reply), Some(Status::Authorized));
        let denied = Json::Obj(vec![("status".into(), Json::Str("denied".into()))]);
        assert_eq!(decode_status(&denied), Some(Status::Denied));
        assert_eq!(decode_status(&Json::Obj(vec![("status".into(), Json::Str("maybe".into()))])), None);
    }

    #[test]
    fn a_can_capture_reply_decodes_to_its_bool() {
        let on = Json::Obj(vec![("allowed".into(), Json::Bool(true))]);
        assert_eq!(decode_allowed(&on), Some(true));
        assert_eq!(decode_allowed(&Json::Null), None);
    }

    #[test]
    fn an_all_reply_decodes_in_media_order() {
        let reply = Json::Obj(vec![
            ("camera".into(), Json::Str("authorized".into())),
            ("microphone".into(), Json::Str("not_determined".into())),
            ("screen".into(), Json::Str("restricted".into())),
        ]);
        let all = decode_all(&reply).unwrap();
        assert_eq!(all[0], (Media::Video, Status::Authorized));
        assert_eq!(all[1], (Media::Audio, Status::NotDetermined));
        assert_eq!(all[2], (Media::Screen, Status::Restricted));
        assert_eq!(decode_all(&Json::Obj(vec![("camera".into(), Json::Str("authorized".into()))])), None);
    }
}
