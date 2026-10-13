//! The `photos` service: the photo library (`~/Pictures`, as the Photos app scans it) for the apps that may read it.
//! The SDK side is `bifrost_app::photo_library` (`PHPhotoLibrary`, `PHAsset`, `PHFetchResult`, `PHImageManager`); the
//! fetch model is `bifrost_ui::photo_fetch` and the access model is `bifrost_ui::photo_access`.
//!
//! Methods (params are a JSON object):
//! - `authorization {}` returns `{status}` (`not_determined`, `restricted`, `denied`, `authorized`, `limited`). An app
//!   whose manifest lacks the `photos` permission is always `denied`.
//! - `request_authorization {}` returns `{status}`. The first request of an app with the permission is allowed (the
//!   manifest permission is the person's consent); later requests return the state they already have.
//! - `fetch {media_type?, from?, to?, ascending?, limit?}` returns `{assets: [{id, media_type, width, height, created,
//!   favorite}]}`, filtered by media type and creation date (`from` inclusive, `to` exclusive, Unix seconds), sorted by
//!   creation date (oldest first unless `ascending` is false), at most `limit` assets (0 or absent: no limit). An
//!   app without a read grant gets an error.
//! - `image {id}` returns `{path, width, height}` of the picture file, for an asset the app may see.
//!
//! The asset id is `PH-` and a hash of the file path alone, so it is the same after every rescan and after an edit.
//! In limited access only the granted ids are visible. Gap: `favorite` is always false, the Photos app keeps its marks.
use super::{Shell, SystemService};
use crate::cg::imageio;
use crate::shell::photos::library::{self, Photo};
use bifrost_core::json::Json;
use bifrost_ui::photo_access::{Access, Answer, Status};
use bifrost_ui::photo_fetch::{self, FetchOptions, Item, Sort};
use bifrost_ui::photo_media::{kind_for_name, Kind};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The manifest permission that lets an app use the library.
pub const PERMISSION: &str = "photos";
/// How much of a file is read to find its pixel size.
const HEADER_BYTES: u64 = 256 * 1024;

thread_local! {
    static ACCESS: RefCell<BTreeMap<String, Access>> = RefCell::new(BTreeMap::new());
}

/// The stable asset identifier of a photo: it depends on the path only.
pub fn local_identifier(p: &Photo) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in p.path.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("PH-{h:016x}")
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::NotDetermined => "not_determined",
        Status::Restricted => "restricted",
        Status::Denied => "denied",
        Status::Authorized => "authorized",
        Status::Limited => "limited",
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Audio => "audio",
        Kind::Unknown => "unknown",
    }
}

fn kind_of_name(name: &str) -> Option<Kind> {
    [Kind::Image, Kind::Video, Kind::Audio, Kind::Unknown].into_iter().find(|k| kind_name(*k) == name)
}

/// The access an app holds. Without the manifest permission it is denied, whatever was answered before.
pub fn access_of(app_id: &str, has_permission: bool) -> Access {
    if !has_permission {
        return Access::new(Status::Denied);
    }
    ACCESS.with(|a| a.borrow().get(app_id).cloned().unwrap_or_else(|| Access::new(Status::NotDetermined)))
}

fn request(app_id: &str, has_permission: bool) -> Status {
    if !has_permission {
        return Status::Denied;
    }
    ACCESS.with(|a| {
        let mut map = a.borrow_mut();
        let access = map.entry(app_id.to_string()).or_insert_with(|| Access::new(Status::NotDetermined));
        access.request(Answer::Allow).unwrap_or_else(|_| access.status())
    })
}

/// The pixel size read from the head of the picture file; 0 x 0 when it cannot be read.
fn pixel_size(p: &Photo) -> (usize, usize) {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(&p.path) else { return (0, 0) };
    let mut head = Vec::new();
    if f.take(HEADER_BYTES).read_to_end(&mut head).is_err() {
        return (0, 0);
    }
    imageio::pixel_size(&head).unwrap_or((0, 0))
}

fn num(v: usize) -> Json {
    Json::Num(v as f64)
}

fn asset_json(p: &Photo, kind: Kind) -> Json {
    let (w, h) = pixel_size(p);
    Json::Obj(vec![
        ("id".into(), Json::Str(local_identifier(p))),
        ("media_type".into(), Json::Str(kind_name(kind).into())),
        ("width".into(), num(w)),
        ("height".into(), num(h)),
        ("created".into(), Json::Num(p.mtime.max(0) as f64)),
        ("favorite".into(), Json::Bool(false)),
    ])
}

fn options_of(params: &Json) -> Result<FetchOptions, String> {
    let media = match params.str_field("media_type") {
        Some(name) => Some(kind_of_name(&name).ok_or_else(|| format!("no media type {name}"))?),
        None => None,
    };
    let ascending = !matches!(params.get("ascending"), Some(Json::Bool(false)));
    let limit = params.get("limit").and_then(Json::as_u64).filter(|n| *n > 0).map(|n| n as usize);
    Ok(FetchOptions {
        media,
        from: params.get("from").and_then(Json::as_u64),
        to: params.get("to").and_then(Json::as_u64),
        sort: if ascending { Sort::OldestFirst } else { Sort::NewestFirst },
        limit,
        ..FetchOptions::default()
    })
}

/// The answer to one method call. `photos` is the library scan; the caller loads it only for the methods that read it.
/// Pure apart from reading picture headers.
pub fn answer(app_id: &str, has_permission: bool, method: &str, params: &Json, photos: &[Photo]) -> Result<Json, String> {
    let access = access_of(app_id, has_permission);
    let status = Json::Obj(vec![("status".into(), Json::Str(status_name(access.status()).into()))]);
    match method {
        "authorization" => Ok(status),
        "request_authorization" => Ok(Json::Obj(vec![("status".into(), Json::Str(status_name(request(app_id, has_permission)).into()))])),
        "fetch" | "image" => {
            if !matches!(access.status(), Status::Authorized | Status::Limited) {
                return Err("photo library access is not granted".into());
            }
            let visible = |p: &&Photo| access.allows(&local_identifier(p));
            if method == "image" {
                let id = params.str_field("id").ok_or("id is required")?;
                let p = photos.iter().filter(visible).find(|p| local_identifier(p) == id).ok_or_else(|| format!("no asset {id}"))?;
                let (w, h) = pixel_size(p);
                return Ok(Json::Obj(vec![("path".into(), Json::Str(p.path.to_string_lossy().into_owned())), ("width".into(), num(w)), ("height".into(), num(h))]));
            }
            let opts = options_of(params)?;
            let seen: Vec<&Photo> = photos.iter().filter(visible).collect();
            let items: Vec<Item> = seen
                .iter()
                .enumerate()
                .map(|(i, p)| Item { id: format!("{i:08}"), kind: kind_for_name(&p.name()), favourite: false, trashed: false, created: p.mtime.max(0) as u64 })
                .collect();
            let rows: Vec<Json> = photo_fetch::fetch(&items, &opts)
                .into_iter()
                .filter_map(|it| it.id.parse::<usize>().ok().map(|i| asset_json(seen[i], it.kind)))
                .collect();
            Ok(Json::Obj(vec![("assets".into(), Json::Arr(rows))]))
        }
        _ => Err(format!("photos has no method {method}")),
    }
}

pub struct Photos;

impl SystemService for Photos {
    fn name(&self) -> &'static str {
        "photos"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let has_permission = super::app_has_permission(app_id, PERMISSION);
        let granted = matches!(access_of(app_id, has_permission).status(), Status::Authorized | Status::Limited);
        let library = if matches!(method, "fetch" | "image") && granted { library::scan(&library::dirs()) } else { Vec::new() };
        answer(app_id, has_permission, method, params, &library)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn photo(name: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/nonexistent/Pictures/{name}")), mtime, size: 1 }
    }

    fn library() -> Vec<Photo> {
        vec![photo("a.png", 100), photo("b.png", 300), photo("c.mp4", 200), photo("d.png", 200)]
    }

    fn granted(app: &str) {
        let r = answer(app, true, "request_authorization", &Json::Null, &[]).unwrap();
        assert_eq!(r.str_field("status").as_deref(), Some("authorized"));
    }

    fn params(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn ids(reply: &Json) -> Vec<String> {
        reply.get("assets").and_then(Json::as_array).unwrap().iter().filter_map(|a| a.str_field("id")).collect()
    }

    fn id(name: &str, mtime: i64) -> String {
        local_identifier(&photo(name, mtime))
    }

    #[test]
    fn an_app_without_the_manifest_permission_is_denied() {
        let app = "is.example.photos-noperm";
        let s = answer(app, false, "authorization", &Json::Null, &[]).unwrap();
        assert_eq!(s.str_field("status").as_deref(), Some("denied"));
        let r = answer(app, false, "request_authorization", &Json::Null, &[]).unwrap();
        assert_eq!(r.str_field("status").as_deref(), Some("denied"));
        assert!(answer(app, false, "fetch", &Json::Null, &library()).is_err());
    }

    #[test]
    fn a_fetch_before_the_request_is_an_error() {
        let app = "is.example.photos-undetermined";
        let s = answer(app, true, "authorization", &Json::Null, &[]).unwrap();
        assert_eq!(s.str_field("status").as_deref(), Some("not_determined"));
        assert!(answer(app, true, "fetch", &Json::Null, &library()).is_err());
    }

    #[test]
    fn fetch_sorts_by_creation_date_descending() {
        let app = "is.example.photos-sort";
        granted(app);
        let r = answer(app, true, "fetch", &params(vec![("ascending", Json::Bool(false))]), &library()).unwrap();
        // b (300), then c and d at 200 ordered by position, then a (100)
        assert_eq!(ids(&r), vec![id("b.png", 300), id("c.mp4", 200), id("d.png", 200), id("a.png", 100)]);
    }

    #[test]
    fn fetch_filters_by_media_type_and_date() {
        let app = "is.example.photos-filter";
        granted(app);
        let video = answer(app, true, "fetch", &params(vec![("media_type", Json::Str("video".into()))]), &library()).unwrap();
        assert_eq!(ids(&video), vec![id("c.mp4", 200)]);
        let range = answer(app, true, "fetch", &params(vec![("from", Json::Num(200.0)), ("to", Json::Num(300.0))]), &library()).unwrap();
        assert_eq!(ids(&range), vec![id("c.mp4", 200), id("d.png", 200)]);
        assert!(answer(app, true, "fetch", &params(vec![("media_type", Json::Str("hologram".into()))]), &library()).is_err());
    }

    #[test]
    fn fetch_limit_keeps_the_first_assets_after_sorting() {
        let app = "is.example.photos-limit";
        granted(app);
        let r = answer(app, true, "fetch", &params(vec![("limit", Json::Num(2.0)), ("ascending", Json::Bool(false))]), &library()).unwrap();
        assert_eq!(ids(&r).len(), 2);
        assert_eq!(ids(&r)[0], id("b.png", 300));
        let all = answer(app, true, "fetch", &params(vec![("limit", Json::Num(0.0))]), &library()).unwrap();
        assert_eq!(ids(&all).len(), 4);
    }

    #[test]
    fn the_identifier_is_stable_across_rescans_and_edits() {
        assert_eq!(id("a.png", 100), id("a.png", 999));
        assert_ne!(id("a.png", 100), id("b.png", 100));
        assert!(id("a.png", 100).starts_with("PH-"));
    }

    #[test]
    fn the_image_method_names_the_file_of_a_visible_asset() {
        let app = "is.example.photos-image";
        granted(app);
        let r = answer(app, true, "image", &params(vec![("id", Json::Str(id("a.png", 100)))]), &library()).unwrap();
        assert_eq!(r.str_field("path").as_deref(), Some("/nonexistent/Pictures/a.png"));
        assert!(answer(app, true, "image", &params(vec![("id", Json::Str("PH-0".into()))]), &library()).is_err());
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("is.example.photos-x", true, "frobnicate", &Json::Null, &[]).is_err());
    }
}
