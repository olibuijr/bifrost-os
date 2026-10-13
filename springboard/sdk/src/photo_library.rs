//! The photo library for apps (the PhotoKit surface): `PHPhotoLibrary` authorization, `PHAsset` fetches with
//! `PHFetchOptions`, the `PHFetchResult` list and `PHImageManager` thumbnails.
//!
//! ```ignore
//! let id = PHPhotoLibrary::request_authorization(ctx);
//! // App::service_reply: PHPhotoLibrary::decode_status(&reply)
//! let mut opts = PHFetchOptions::default();
//! opts.sort_descriptors = vec![SortDescriptor::creation_date(false)];
//! opts.predicate = Some(PHPredicate::media_type(PHAssetMediaType::Image));
//! opts.fetch_limit = 50;
//! let id = PHAsset::fetch_assets(ctx, &opts);
//! // App::service_reply: let result = PHFetchResult::decode(&reply)?; result.object_at(0)
//! let id = PHImageManager::request_image_data(ctx, &asset);   // reply: PHImageManager::decode_path
//! let img = PHImageManager::image_from_bytes(&bytes, (200, 200), PHImageContentMode::AspectFit)?;
//! ```
//! The shell side is `springboard/src/shell/services/photos.rs`. Without the `photos` permission in the manifest the
//! status is `denied` and a fetch is an error. Gaps: no change observation, no collections, no writes, `isFavorite`
//! is always false until the Photos app shares its marks.
pub use crate::controls::photo_access::Status as PHAuthorizationStatus;
use crate::image_source::{ImageSource, SourceError};
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use bifrost_render::gfx::Image;

/// The shell service name.
pub const SERVICE: &str = "photos";

/// `PHAssetMediaType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PHAssetMediaType {
    Unknown,
    Image,
    Video,
    Audio,
}

impl PHAssetMediaType {
    pub fn name(self) -> &'static str {
        match self {
            PHAssetMediaType::Unknown => "unknown",
            PHAssetMediaType::Image => "image",
            PHAssetMediaType::Video => "video",
            PHAssetMediaType::Audio => "audio",
        }
    }
    pub fn from_name(name: &str) -> Option<PHAssetMediaType> {
        Some(match name {
            "unknown" => PHAssetMediaType::Unknown,
            "image" => PHAssetMediaType::Image,
            "video" => PHAssetMediaType::Video,
            "audio" => PHAssetMediaType::Audio,
            _ => return None,
        })
    }
}

/// The sort descriptor of a fetch, on `creationDate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortDescriptor {
    pub ascending: bool,
}

impl SortDescriptor {
    pub fn creation_date(ascending: bool) -> SortDescriptor {
        SortDescriptor { ascending }
    }
}

/// The `Predicate` subset a fetch understands: `mediaType == x` and `creationDate >= from AND creationDate < to`
/// (seconds since the epoch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PHPredicate {
    pub media_type: Option<PHAssetMediaType>,
    pub creation_date_from: Option<u64>,
    pub creation_date_to: Option<u64>,
}

impl PHPredicate {
    pub fn media_type(kind: PHAssetMediaType) -> PHPredicate {
        PHPredicate { media_type: Some(kind), ..Default::default() }
    }
}

/// `PHFetchOptions`. `fetch_limit` 0 means no limit, as in PhotoKit. With no sort descriptor the order is oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PHFetchOptions {
    pub sort_descriptors: Vec<SortDescriptor>,
    pub predicate: Option<PHPredicate>,
    pub fetch_limit: usize,
}

impl PHFetchOptions {
    /// The wire params of a `fetch` call.
    pub fn to_params(&self) -> Json {
        let ascending = self.sort_descriptors.first().map_or(true, |d| d.ascending);
        let mut f = vec![("ascending".to_string(), Json::Bool(ascending)), ("limit".to_string(), Json::Num(self.fetch_limit as f64))];
        if let Some(p) = &self.predicate {
            if let Some(m) = p.media_type {
                f.push(("media_type".into(), Json::Str(m.name().into())));
            }
            if let Some(v) = p.creation_date_from {
                f.push(("from".into(), Json::Num(v as f64)));
            }
            if let Some(v) = p.creation_date_to {
                f.push(("to".into(), Json::Num(v as f64)));
            }
        }
        Json::Obj(f)
    }
}

/// `PHAsset`.
#[derive(Clone, Debug, PartialEq)]
pub struct PHAsset {
    pub local_identifier: String,
    pub media_type: PHAssetMediaType,
    pub pixel_width: usize,
    pub pixel_height: usize,
    /// Seconds since the epoch.
    pub creation_date: u64,
    pub is_favorite: bool,
}

impl PHAsset {
    /// Asks for the assets `opts` selects. The reply decodes with [`PHFetchResult::decode`].
    pub fn fetch_assets(ctx: &Ctx, opts: &PHFetchOptions) -> u32 {
        service::call(ctx, SERVICE, "fetch", &opts.to_params())
    }

    /// The asset in one wire record, or `None` for a malformed record.
    pub fn decode(record: &Json) -> Option<PHAsset> {
        Some(PHAsset {
            local_identifier: record.str_field("id")?,
            media_type: PHAssetMediaType::from_name(&record.str_field("media_type")?)?,
            pixel_width: record.get("width").and_then(Json::as_u64)? as usize,
            pixel_height: record.get("height").and_then(Json::as_u64)? as usize,
            creation_date: record.get("created").and_then(Json::as_u64)?,
            is_favorite: matches!(record.get("favorite"), Some(Json::Bool(true))),
        })
    }
}

/// `PHFetchResult<PHAsset>`: the matching assets in fetch order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PHFetchResult {
    assets: Vec<PHAsset>,
}

impl PHFetchResult {
    pub fn count(&self) -> usize {
        self.assets.len()
    }
    pub fn object_at(&self, index: usize) -> Option<&PHAsset> {
        self.assets.get(index)
    }
    pub fn iter(&self) -> std::slice::Iter<'_, PHAsset> {
        self.assets.iter()
    }
    /// The result in a `fetch` reply; `None` for a reply of another shape or with a malformed record.
    pub fn decode(reply: &Json) -> Option<PHFetchResult> {
        let assets = reply.get("assets")?.as_array()?.iter().map(PHAsset::decode).collect::<Option<Vec<_>>>()?;
        Some(PHFetchResult { assets })
    }
}

/// `PHPhotoLibrary`.
pub struct PHPhotoLibrary;

impl PHPhotoLibrary {
    /// Asks for `authorizationStatus`. The reply decodes with [`PHPhotoLibrary::decode_status`].
    pub fn authorization_status(ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "authorization", &Json::Null)
    }
    /// Asks the shell to settle the first request (the manifest permission is the person's consent).
    pub fn request_authorization(ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "request_authorization", &Json::Null)
    }
    pub fn decode_status(reply: &Json) -> Option<PHAuthorizationStatus> {
        status_from_name(reply.get("status")?.as_str()?)
    }
}

/// The wire name of a status.
pub fn status_name(s: PHAuthorizationStatus) -> &'static str {
    match s {
        PHAuthorizationStatus::NotDetermined => "not_determined",
        PHAuthorizationStatus::Restricted => "restricted",
        PHAuthorizationStatus::Denied => "denied",
        PHAuthorizationStatus::Authorized => "authorized",
        PHAuthorizationStatus::Limited => "limited",
    }
}

pub fn status_from_name(name: &str) -> Option<PHAuthorizationStatus> {
    [
        PHAuthorizationStatus::NotDetermined,
        PHAuthorizationStatus::Restricted,
        PHAuthorizationStatus::Denied,
        PHAuthorizationStatus::Authorized,
        PHAuthorizationStatus::Limited,
    ]
    .into_iter()
    .find(|s| status_name(*s) == name)
}

/// `PHImageContentMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PHImageContentMode {
    /// The whole picture fits inside the target size.
    AspectFit,
    /// The picture covers the target size; the result may be larger than it on one side.
    AspectFill,
}

/// `PHImageManager`.
pub struct PHImageManager;

impl PHImageManager {
    /// Asks where the file of `asset` is. The reply decodes with [`PHImageManager::decode_path`].
    pub fn request_image_data(ctx: &Ctx, asset: &PHAsset) -> u32 {
        service::call(ctx, SERVICE, "image", &Json::Obj(vec![("id".into(), Json::Str(asset.local_identifier.clone()))]))
    }

    pub fn decode_path(reply: &Json) -> Option<String> {
        reply.str_field("path")
    }

    /// `requestImage(for:targetSize:contentMode:)` on the picture bytes: a thumbnail that fits `target` (pixels), or
    /// for `AspectFill` one that covers it. A picture is never enlarged.
    pub fn image_from_bytes(bytes: &[u8], target: (usize, usize), mode: PHImageContentMode) -> Result<Image, SourceError> {
        let src = ImageSource::new(bytes)?;
        match mode {
            PHImageContentMode::AspectFit => src.thumbnail(target.0, target.1),
            PHImageContentMode::AspectFill => {
                let (tw, th) = fill_box(src.size(), target);
                src.thumbnail(tw, th)
            }
        }
    }
}

/// The box whose fit gives a picture of `size` that covers `target` on both sides.
fn fill_box(size: (usize, usize), target: (usize, usize)) -> (usize, usize) {
    if size.0 == 0 || size.1 == 0 || target.0 == 0 || target.1 == 0 {
        return target;
    }
    let s = (target.0 as f64 / size.0 as f64).max(target.1 as f64 / size.1 as f64);
    (((size.0 as f64 * s).ceil() as usize).max(1), ((size.1 as f64 * s).ceil() as usize).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::cg::imageio::encode::{encode_png, PngOptions};

    fn png(w: usize, h: usize) -> Vec<u8> {
        encode_png(w, h, &vec![200u8; w * h * 4], &PngOptions::default()).unwrap()
    }

    fn record(id: &str, kind: &str) -> Json {
        Json::Obj(vec![
            ("id".into(), Json::Str(id.into())),
            ("media_type".into(), Json::Str(kind.into())),
            ("width".into(), Json::Num(400.0)),
            ("height".into(), Json::Num(300.0)),
            ("created".into(), Json::Num(1000.0)),
            ("favorite".into(), Json::Bool(false)),
        ])
    }

    #[test]
    fn options_become_wire_params() {
        let opts = PHFetchOptions {
            sort_descriptors: vec![SortDescriptor::creation_date(false)],
            predicate: Some(PHPredicate { media_type: Some(PHAssetMediaType::Image), creation_date_from: Some(10), creation_date_to: Some(20) }),
            fetch_limit: 5,
        };
        let p = opts.to_params();
        assert_eq!(p.get("ascending"), Some(&Json::Bool(false)));
        assert_eq!(p.str_field("media_type").as_deref(), Some("image"));
        assert_eq!(p.get("from").and_then(Json::as_u64), Some(10));
        assert_eq!(p.get("to").and_then(Json::as_u64), Some(20));
        assert_eq!(p.get("limit").and_then(Json::as_u64), Some(5));
        assert_eq!(PHFetchOptions::default().to_params().get("ascending"), Some(&Json::Bool(true)));
    }

    #[test]
    fn a_reply_decodes_into_a_fetch_result() {
        let reply = Json::Obj(vec![("assets".into(), Json::Arr(vec![record("a", "image"), record("b", "video")]))]);
        let r = PHFetchResult::decode(&reply).unwrap();
        assert_eq!(r.count(), 2);
        let a = r.object_at(0).unwrap();
        assert_eq!((a.local_identifier.as_str(), a.pixel_width, a.pixel_height, a.creation_date), ("a", 400, 300, 1000));
        assert_eq!(r.object_at(1).unwrap().media_type, PHAssetMediaType::Video);
        assert!(r.object_at(2).is_none());
    }

    #[test]
    fn a_malformed_record_or_reply_is_refused() {
        let bad = Json::Obj(vec![("assets".into(), Json::Arr(vec![record("a", "hologram")]))]);
        assert!(PHFetchResult::decode(&bad).is_none());
        assert!(PHFetchResult::decode(&Json::Null).is_none());
    }

    #[test]
    fn status_names_round_trip() {
        for s in [PHAuthorizationStatus::NotDetermined, PHAuthorizationStatus::Denied, PHAuthorizationStatus::Limited, PHAuthorizationStatus::Authorized, PHAuthorizationStatus::Restricted] {
            assert_eq!(status_from_name(status_name(s)), Some(s));
        }
        assert_eq!(status_from_name("maybe"), None);
    }

    #[test]
    fn request_image_fits_the_target_size() {
        let img = PHImageManager::image_from_bytes(&png(400, 200), (100, 100), PHImageContentMode::AspectFit).unwrap();
        assert_eq!((img.w, img.h), (100, 50));
        let small = PHImageManager::image_from_bytes(&png(40, 20), (100, 100), PHImageContentMode::AspectFit).unwrap();
        assert_eq!((small.w, small.h), (40, 20));
    }

    #[test]
    fn aspect_fill_covers_the_target() {
        let img = PHImageManager::image_from_bytes(&png(400, 200), (100, 100), PHImageContentMode::AspectFill).unwrap();
        assert!(img.w >= 100 && img.h >= 100, "{}x{}", img.w, img.h);
    }

    #[test]
    fn request_image_refuses_data_that_is_not_a_picture() {
        assert_eq!(PHImageManager::image_from_bytes(b"not a picture at all", (10, 10), PHImageContentMode::AspectFit).unwrap_err(), SourceError::NotImage);
    }
}
