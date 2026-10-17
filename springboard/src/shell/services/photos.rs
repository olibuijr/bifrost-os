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
//! - `observe {}` returns `{observing: true}` and starts a change observer for the app: the shell keeps the listing
//!   (default fetch order, oldest first) and, every couple of seconds, pushes `library_did_change` with
//!   `{removed, inserted, moved, changed}` indexes into that listing when it differs. `unobserve {}` stops it.
//! - `editing {op, id, format_identifier?, format_version?, data?}`: `get` returns `{adjustment}` (null or
//!   `{format_identifier, format_version, data}`), `commit` stores the adjustment record as a sidecar file next to the
//!   picture (the original is never rewritten), `revert` removes the sidecar. `data` is lower-case hex. Needs access.
//! - `picker {filters?, limit?}` opens the shell's photo picker sheet (`photo_pick_sheet`) and returns
//!   `{status: "pending"}` at once. `filters` are `images`, `videos` and `live_photos`; `limit` is the most pictures the
//!   person may choose (0: no limit, absent: 1). The person chooses in the sheet, from the whole library: the app needs
//!   no photos permission and sees nothing of the library. When the person ends the sheet the shell pushes
//!   `picker_did_finish` with `{results: [{id}], cancelled}` (ids in the order picked; none and `cancelled: true` for
//!   Cancel) and lets the app read exactly those pictures with `image {id}`. A second `picker` while the sheet is up is
//!   an error.
//! - `project {op, id, title?, changes?}` with `op` `create`, `get` or `apply` (a list of change objects, all or
//!   nothing). Projects are kept per app for the session only; they are not written to disk yet. Needs the permission.
use super::{app_has_permission, Event, Shell, SystemService};
use crate::cg::imageio;
use crate::shell::photos::library::{self, Photo};
use bifrost_core::json::Json;
use bifrost_ui::photo_access::{Access, Answer, Status};
use bifrost_ui::photo_adjustment::AdjustmentData;
use bifrost_ui::photo_changes::{self, Asset};
use bifrost_ui::photo_fetch::{self, FetchOptions, Item, Sort};
use bifrost_ui::photo_media::{self, kind_for_name, Kind};
use bifrost_ui::photo_project::{Change, Project};
use crate::shell::photo_pick::{self, Entry as PickEntry, Filter as PickFilter, Pick};
use bifrost_ui::photo_request::Request;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The manifest permission that lets an app use the library.
pub const PERMISSION: &str = "photos";
/// How much of a file is read to find its pixel size.
const HEADER_BYTES: u64 = 256 * 1024;
/// The seconds between two looks at the library for change observers.
pub const OBSERVE_INTERVAL: f64 = 2.0;
/// The event a change observer receives.
pub const LIBRARY_DID_CHANGE: &str = "library_did_change";
/// The event an app receives when the person ends its photo picker.
pub const PICKER_DID_FINISH: &str = "picker_did_finish";
/// The most assets one app may hold from its pickers.
const MAX_PICKED: usize = 4096;
/// The suffix of the sidecar file that holds an adjustment record.
const SIDECAR_SUFFIX: &str = ".adjust.json";

thread_local! {
    static ACCESS: RefCell<BTreeMap<String, Access>> = RefCell::new(BTreeMap::new());
    /// The listing each observing app last saw.
    static OBSERVERS: RefCell<BTreeMap<String, Vec<Asset>>> = RefCell::new(BTreeMap::new());
    /// The projects of each app, keyed by (app id, project id).
    static PROJECTS: RefCell<BTreeMap<(String, String), Project>> = RefCell::new(BTreeMap::new());
    /// The assets each app was given by its photo pickers: it may read these and no others without library access.
    static PICKED: RefCell<BTreeMap<String, BTreeSet<String>>> = RefCell::new(BTreeMap::new());
}

/// Lets `app_id` read the assets the person chose in its picker (`image {id}`), whatever its library access.
pub fn grant_picked(app_id: &str, ids: &[String]) {
    PICKED.with(|p| {
        let mut all = p.borrow_mut();
        let set = all.entry(app_id.to_string()).or_default();
        for id in ids {
            if set.len() < MAX_PICKED {
                set.insert(id.clone());
            }
        }
    });
}

/// The assets the person chose for `app_id` so far.
pub fn picked_of(app_id: &str) -> BTreeSet<String> {
    PICKED.with(|p| p.borrow().get(app_id).cloned().unwrap_or_default())
}

/// The access that applies to `image {id}`: the app's own access, plus the assets it was given by a picker. A picker
/// works whatever the library answer (the person chose the pictures, not the app).
fn image_access(app_id: &str, own: &Access) -> Access {
    let picked = picked_of(app_id);
    let mut ids = match own.status() {
        Status::Authorized => return own.clone(),
        Status::Limited => own.chosen(),
        _ if picked.is_empty() => return own.clone(),
        _ => Vec::new(),
    };
    ids.extend(picked);
    let mut access = Access::new(Status::NotDetermined);
    let _ = access.request(Answer::AllowLimited(ids));
    access
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

/// The pixel size read from the head of the file: the picture's header, or for a movie the first video track's `moov`
/// (a movie with its `moov` box after the first `HEADER_BYTES` reports 0 x 0). 0 x 0 when it cannot be read.
fn pixel_size(p: &Photo) -> (usize, usize) {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(&p.path) else { return (0, 0) };
    let mut head = Vec::new();
    if f.take(HEADER_BYTES).read_to_end(&mut head).is_err() {
        return (0, 0);
    }
    let name = p.path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if kind_for_name(name).is_video() {
        return photo_media::movie_pixel_size(&head).unwrap_or((0, 0));
    }
    imageio::pixel_size(&head).unwrap_or((0, 0))
}

fn num(v: usize) -> Json {
    Json::Num(v as f64)
}

fn asset_json(p: &Photo, kind: Kind, live: bool) -> Json {
    let (w, h) = pixel_size(p);
    Json::Obj(vec![
        ("id".into(), Json::Str(local_identifier(p))),
        ("media_type".into(), Json::Str(kind_name(kind).into())),
        ("width".into(), num(w)),
        ("height".into(), num(h)),
        ("created".into(), Json::Num(p.mtime.max(0) as f64)),
        ("modified".into(), Json::Num(p.mtime.max(0) as f64)),
        ("favorite".into(), Json::Bool(false)),
        ("live".into(), Json::Bool(live)),
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

fn granted(access: &Access) -> bool {
    matches!(access.status(), Status::Authorized | Status::Limited)
}

thread_local! {
    /// The Live Photo identifier of each file, with the time and size it was read at (a changed file is read again).
    static CONTENT_IDS: RefCell<BTreeMap<PathBuf, (i64, u64, Option<String>)>> = RefCell::new(BTreeMap::new());
}

/// The Live Photo identifier of a file, read once per change of its time or size.
fn content_id_of(p: &Photo) -> Option<String> {
    if let Some((mtime, size, id)) = CONTENT_IDS.with(|c| c.borrow().get(&p.path).cloned()) {
        if mtime == p.mtime && size == p.size {
            return id;
        }
    }
    let id = library::content_id(&p.path);
    CONTENT_IDS.with(|c| c.borrow_mut().insert(p.path.clone(), (p.mtime, p.size, id.clone())));
    id
}

/// The photos an app with `access` sees, in library order, each with whether it is a Live Photo still. The movie of a
/// Live Photo is not listed on its own: it belongs to its still.
pub fn listed<'a>(photos: &'a [Photo], access: &Access) -> Vec<(&'a Photo, bool)> {
    let seen: Vec<&Photo> = photos.iter().filter(|p| access.allows(&local_identifier(p))).collect();
    let items: Vec<photo_media::Item> = seen.iter().map(|p| photo_media::Item { name: p.name(), content_id: content_id_of(p) }).collect();
    let pairs = photo_media::pair_moving(&items);
    let movies: BTreeSet<usize> = pairs.iter().map(|p| p.movie).collect();
    let stills: BTreeSet<usize> = pairs.iter().map(|p| p.still).collect();
    seen.iter().enumerate().filter(|(i, _)| !movies.contains(i)).map(|(i, p)| (*p, stills.contains(&i))).collect()
}

/// The visible assets in the default fetch order (oldest first): the listing a change observer compares.
pub fn listing(photos: &[Photo], access: &Access) -> Vec<Asset> {
    let seen = listed(photos, access);
    let items: Vec<Item> = seen.iter().enumerate().map(|(i, (p, _))| library_item(i, p)).collect();
    let opts = FetchOptions { sort: Sort::OldestFirst, ..FetchOptions::default() };
    photo_fetch::fetch(&items, &opts)
        .into_iter()
        .filter_map(|it| it.id.parse::<usize>().ok())
        .map(|i| Asset { id: local_identifier(seen[i].0), modified: seen[i].0.mtime.max(0) as u64 })
        .collect()
}

fn library_item(index: usize, p: &Photo) -> Item {
    Item { id: format!("{index:08}"), kind: kind_for_name(&p.name()), favourite: false, trashed: false, created: p.mtime.max(0) as u64 }
}

/// The change details of an observer's listing against the new one, or None when nothing changed (or a listing
/// names an asset twice, which the library never does).
pub fn change_body(old: &[Asset], new: &[Asset]) -> Option<Json> {
    let d = photo_changes::details(old, new).ok()?;
    if d.is_empty() {
        return None;
    }
    let list = |v: &[usize]| Json::Arr(v.iter().map(|i| num(*i)).collect());
    Some(Json::Obj(vec![
        ("removed".into(), list(&d.removed)),
        ("inserted".into(), list(&d.inserted)),
        ("moved".into(), list(&d.moved)),
        ("changed".into(), list(&d.changed)),
    ]))
}

/// The sidecar file that holds the adjustment record of a picture: its path with `.adjust.json` added.
pub fn sidecar_path(picture: &Path) -> PathBuf {
    let mut name = picture.as_os_str().to_os_string();
    name.push(SIDECAR_SUFFIX);
    PathBuf::from(name)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

fn adjustment_json(a: &AdjustmentData) -> Json {
    Json::Obj(vec![
        ("format_identifier".into(), Json::Str(a.format_identifier().into())),
        ("format_version".into(), Json::Str(a.format_version().into())),
        ("data".into(), Json::Str(to_hex(a.data()))),
    ])
}

/// The record in a sidecar file: None when there is none, an error when the file is not a record.
fn read_adjustment(sidecar: &Path) -> Result<Option<AdjustmentData>, String> {
    let text = match std::fs::read_to_string(sidecar) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read the edit: {e}")),
    };
    let j = bifrost_core::json::parse(&text)?;
    let bytes = from_hex(&j.str_field("data").ok_or("the edit record has no data")?).ok_or("the edit record data is not hex")?;
    AdjustmentData::new(&j.str_field("format_identifier").ok_or("no format")?, &j.str_field("format_version").ok_or("no version")?, bytes)
        .map(Some)
        .ok_or_else(|| "the edit record has a blank format".to_string())
}

/// Writes the record beside the picture, through a temporary file so a crash leaves the old record whole.
fn write_adjustment(sidecar: &Path, record: &AdjustmentData) -> Result<(), String> {
    let mut tmp = sidecar.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, adjustment_json(record).encode()).map_err(|e| format!("cannot save the edit: {e}"))?;
    std::fs::rename(&tmp, sidecar).map_err(|e| format!("cannot save the edit: {e}"))
}

fn strings(v: &Json) -> Option<Vec<String>> {
    v.as_array()?.iter().map(|x| x.as_str().map(str::to_string)).collect()
}

/// One change from its wire object (see `PHProjectChangeRequest::to_params` in the SDK).
pub fn change_of(j: &Json) -> Option<Change> {
    Some(match j.str_field("kind")?.as_str() {
        "add_section" => Change::AddSection { id: j.str_field("id")?, title: j.str_field("title")? },
        "rename_section" => Change::RenameSection { id: j.str_field("id")?, title: j.str_field("title")? },
        "remove_section" => Change::RemoveSection { id: j.str_field("id")? },
        "move_section" => Change::MoveSection { id: j.str_field("id")?, to: j.get("to").and_then(Json::as_u64)? as usize },
        "add_assets" => Change::AddAssets { section: j.str_field("section")?, assets: strings(j.get("assets")?)? },
        "remove_assets" => Change::RemoveAssets { section: j.str_field("section")?, assets: strings(j.get("assets")?)? },
        _ => return None,
    })
}

fn project_json(p: &Project) -> Json {
    let sections = p
        .sections()
        .iter()
        .map(|s| {
            Json::Obj(vec![
                ("id".into(), Json::Str(s.id().into())),
                ("title".into(), Json::Str(s.title().into())),
                ("assets".into(), Json::Arr(s.assets().iter().map(|a| Json::Str(a.clone())).collect())),
            ])
        })
        .collect();
    Json::Obj(vec![("id".into(), Json::Str(p.id().into())), ("title".into(), Json::Str(p.title().into())), ("sections".into(), Json::Arr(sections))])
}

fn project_call(app_id: &str, params: &Json) -> Result<Json, String> {
    let id = params.str_field("id").ok_or("id is required")?;
    let key = (app_id.to_string(), id.clone());
    PROJECTS.with(|m| {
        let mut m = m.borrow_mut();
        match params.str_field("op").as_deref() {
            Some("create") => {
                if m.contains_key(&key) {
                    return Err(format!("a project named {id} already exists"));
                }
                let project = Project::new(&id, &params.str_field("title").unwrap_or_default()).map_err(|e| e.to_string())?;
                let out = project_json(&project);
                m.insert(key, project);
                Ok(out)
            }
            Some("get") => m.get(&key).map(project_json).ok_or_else(|| format!("no project {id}")),
            Some("apply") => {
                let changes = params
                    .get("changes")
                    .and_then(Json::as_array)
                    .ok_or("changes are required")?
                    .iter()
                    .map(change_of)
                    .collect::<Option<Vec<Change>>>()
                    .ok_or("a change is malformed")?;
                let project = m.get_mut(&key).ok_or_else(|| format!("no project {id}"))?;
                project.apply(&changes).map_err(|e| e.to_string())?;
                Ok(project_json(project))
            }
            _ => Err("project needs op create, get or apply".into()),
        }
    })
}

/// The assets a picker sheet lists: everything the library holds, since the person is choosing, not the app.
pub fn pick_entries(photos: &[Photo]) -> Vec<PickEntry> {
    let everything = Access::new(Status::Authorized);
    listed(photos, &everything)
        .into_iter()
        .map(|(p, live)| PickEntry { id: local_identifier(p), path: p.path.clone(), key: p.key(), kind: kind_for_name(&p.name()), live, mtime: p.mtime })
        .collect()
}

/// The `picker` call: checks the params and opens the picker sheet for the app over the whole library. The person
/// chooses; the answer comes later as `picker_did_finish` (see `Photos::tick`).
fn picker_call(app_id: &str, params: &Json, photos: &[Photo]) -> Result<Json, String> {
    let mut filters = Vec::new();
    if let Some(list) = params.get("filters") {
        for f in list.as_array().ok_or("filters must be a list")? {
            let name = f.as_str().ok_or("a filter must be a name")?;
            filters.push(PickFilter::from_name(name).ok_or_else(|| format!("no picker filter {name}"))?);
        }
    }
    let limit = match params.get("limit") {
        None | Some(Json::Null) => 1,
        Some(v) => v.as_u64().ok_or("limit must be a whole number")? as usize,
    };
    let entries = pick_entries(photos);
    let name = crate::shell::apphost::installed().iter().find(|a| a.manifest.id == app_id).map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
    photo_pick::start(Pick::new(app_id, &name, filters, limit, entries)).map_err(|_| "a photo picker is already open for this app".to_string())?;
    Ok(Json::Obj(vec![("status".into(), Json::Str("pending".into()))]))
}

/// Checks a batch of library changes (`ops`: create, favourite or trash) against the assets the app can see and reports
/// what it would write. Nothing is written yet: the favourite and trash flags live in the Photos app, so the service
/// only plans the batch. The plan is checked by `photo_request::Request::plan`, which takes the names of the files
/// (for the clash check) and the asset identifiers (for the favourite and trash checks) together.
fn batch_call(params: &Json, seen: &[&Photo]) -> Result<Json, String> {
    let ops = params.get("ops").and_then(Json::as_array).ok_or("perform needs ops")?;
    let mut request = Request::new();
    for op in ops {
        match op.str_field("op").as_deref() {
            Some("create") => {
                request.create(&op.str_field("name").ok_or("create needs name")?);
            }
            Some("favourite") => {
                let on = matches!(op.get("on"), Some(Json::Bool(true)));
                request.set_favourite(&op.str_field("id").ok_or("favourite needs id")?, on);
            }
            Some("trash") => {
                request.trash(&op.str_field("id").ok_or("trash needs id")?);
            }
            _ => return Err("perform ops are create, favourite or trash".into()),
        }
    }
    let mut known: Vec<String> = Vec::new();
    for p in seen {
        known.push(local_identifier(p));
        if let Some(name) = p.path.file_name().and_then(|n| n.to_str()) {
            known.push(name.to_string());
        }
    }
    let refs: Vec<&str> = known.iter().map(String::as_str).collect();
    let plan = request.plan(&refs).map_err(|e| format!("{e:?}"))?;
    let created = plan.created.iter().map(|c| Json::Obj(vec![("requested".into(), Json::Str(c.requested.clone())), ("name".into(), Json::Str(c.name.clone())), ("media_type".into(), Json::Str(kind_name(c.kind).into()))]));
    let favourites = plan.favourites.iter().map(|(id, on)| Json::Obj(vec![("id".into(), Json::Str(id.clone())), ("on".into(), Json::Bool(*on))]));
    let trashed = plan.trashed.iter().map(|id| Json::Str(id.clone()));
    Ok(Json::Obj(vec![
        ("created".into(), Json::Arr(created.collect())),
        ("favourites".into(), Json::Arr(favourites.collect())),
        ("trashed".into(), Json::Arr(trashed.collect())),
        ("written".into(), Json::Bool(false)),
    ]))
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
            // `image` also serves the assets the app's picker gave it, which need no library access
            let access = if method == "image" { image_access(app_id, &access) } else { access };
            if !matches!(access.status(), Status::Authorized | Status::Limited) {
                return Err("photo library access is not granted".into());
            }
            if method == "image" {
                let id = params.str_field("id").ok_or("id is required")?;
                let p = listed(photos, &access).into_iter().map(|(p, _)| p).find(|p| local_identifier(p) == id).ok_or_else(|| format!("no asset {id}"))?;
                let (w, h) = pixel_size(p);
                return Ok(Json::Obj(vec![("path".into(), Json::Str(p.path.to_string_lossy().into_owned())), ("width".into(), num(w)), ("height".into(), num(h))]));
            }
            let opts = options_of(params)?;
            let seen = listed(photos, &access);
            let items: Vec<Item> = seen.iter().enumerate().map(|(i, (p, _))| library_item(i, p)).collect();
            let rows: Vec<Json> = photo_fetch::fetch(&items, &opts)
                .into_iter()
                .filter_map(|it| it.id.parse::<usize>().ok().map(|i| asset_json(seen[i].0, it.kind, seen[i].1)))
                .collect();
            Ok(Json::Obj(vec![("assets".into(), Json::Arr(rows))]))
        }
        "observe" => {
            if !granted(&access) {
                return Err("photo library access is not granted".into());
            }
            OBSERVERS.with(|o| o.borrow_mut().insert(app_id.to_string(), listing(photos, &access)));
            Ok(Json::Obj(vec![("observing".into(), Json::Bool(true))]))
        }
        "unobserve" => {
            OBSERVERS.with(|o| o.borrow_mut().remove(app_id));
            Ok(Json::Obj(vec![("observing".into(), Json::Bool(false))]))
        }
        "editing" => {
            if !granted(&access) {
                return Err("photo library access is not granted".into());
            }
            let id = params.str_field("id").ok_or("id is required")?;
            let p = listed(photos, &access).into_iter().map(|(p, _)| p).find(|p| local_identifier(p) == id).ok_or_else(|| format!("no asset {id}"))?;
            let sidecar = sidecar_path(&p.path);
            match params.str_field("op").as_deref() {
                Some("get") => {
                    let adjustment = read_adjustment(&sidecar)?.map_or(Json::Null, |a| adjustment_json(&a));
                    Ok(Json::Obj(vec![("adjustment".into(), adjustment)]))
                }
                Some("commit") => {
                    let format = params.str_field("format_identifier").ok_or("format_identifier is required")?;
                    let version = params.str_field("format_version").ok_or("format_version is required")?;
                    let bytes = from_hex(&params.str_field("data").unwrap_or_default()).ok_or("data is not hex")?;
                    let record = AdjustmentData::new(&format, &version, bytes).ok_or("format identifier and version are required")?;
                    write_adjustment(&sidecar, &record)?;
                    Ok(Json::Obj(vec![("committed".into(), Json::Bool(true))]))
                }
                Some("revert") => {
                    match std::fs::remove_file(&sidecar) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(format!("cannot revert the edit: {e}")),
                    }
                    Ok(Json::Obj(vec![("reverted".into(), Json::Bool(true))]))
                }
                _ => Err("editing needs op get, commit or revert".into()),
            }
        }
        "picker" => picker_call(app_id, params, photos),
        "perform" => {
            if !granted(&access) {
                return Err("photo library access is not granted".into());
            }
            let seen: Vec<&Photo> = listed(photos, &access).into_iter().map(|(p, _)| p).collect();
            batch_call(params, &seen)
        }
        "project" => {
            if !has_permission {
                return Err("the photos permission is not granted".into());
            }
            project_call(app_id, params)
        }
        _ => Err(format!("photos has no method {method}")),
    }
}

/// The `picker_did_finish` event for a picker the person ended, after granting the app the chosen assets.
fn finish_event(app: &str, ids: &[String]) -> Event {
    grant_picked(app, ids);
    let results = ids.iter().map(|id| Json::Obj(vec![("id".into(), Json::Str(id.clone()))])).collect();
    let body = Json::Obj(vec![("results".into(), Json::Arr(results)), ("cancelled".into(), Json::Bool(ids.is_empty()))]);
    Event { app: Some(app.to_string()), event: PICKER_DID_FINISH.into(), body }
}

/// The photos service. Keeps the change observers' listings and the per-app projects.
#[derive(Default)]
pub struct Photos {
    /// The time of the last look at the library for observers.
    last_look: f64,
}

impl SystemService for Photos {
    fn name(&self) -> &'static str {
        "photos"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let has_permission = app_has_permission(app_id, PERMISSION);
        let is_granted = granted(&access_of(app_id, has_permission));
        let reads_library = matches!(method, "fetch" | "image" | "observe" | "editing" | "perform");
        // a picker lists the library for the person, and a picked asset is read without library access
        let may_read = is_granted || method == "picker" || (method == "image" && !picked_of(app_id).is_empty());
        let library = if (reads_library || method == "picker") && may_read { library::scan(&library::dirs()) } else { Vec::new() };
        answer(app_id, has_permission, method, params, &library)
    }

    /// Looks at the library for the change observers every `OBSERVE_INTERVAL` seconds and pushes what changed to each.
    /// An observer whose access was taken away is dropped without an event.
    fn tick(&mut self, _shell: &mut Shell, t: f64) -> Vec<Event> {
        // the pickers the person ended: the app may read what was chosen, then it hears of it
        let mut out: Vec<Event> = photo_pick::take_finished().into_iter().map(|(app, ids)| finish_event(&app, &ids)).collect();
        if t - self.last_look < OBSERVE_INTERVAL {
            return out;
        }
        self.last_look = t;
        let observers: Vec<String> = OBSERVERS.with(|o| o.borrow().keys().cloned().collect());
        if observers.is_empty() {
            return out;
        }
        let library = library::scan(&library::dirs());
        for app in observers {
            let access = access_of(&app, app_has_permission(&app, PERMISSION));
            if !granted(&access) {
                OBSERVERS.with(|o| o.borrow_mut().remove(&app));
                continue;
            }
            let now = listing(&library, &access);
            let before = OBSERVERS.with(|o| o.borrow().get(&app).cloned().unwrap_or_default());
            if let Some(body) = change_body(&before, &now) {
                out.push(Event { app: Some(app.clone()), event: LIBRARY_DID_CHANGE.into(), body });
            }
            OBSERVERS.with(|o| o.borrow_mut().insert(app, now));
        }
        out
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
    fn plain_pictures_are_listed_and_not_live() {
        let app = "is.example.photos-plain";
        granted(app);
        let r = answer(app, true, "fetch", &params(vec![("limit", Json::Num(0.0))]), &library()).unwrap();
        let rows = r.get("assets").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), 4, "the video is a movie, not a live photo, and is listed");
        assert!(rows.iter().all(|a| a.get("live") == Some(&Json::Bool(false))));
        assert!(rows.iter().all(|a| a.get("modified") == a.get("created")));
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("is.example.photos-x", true, "frobnicate", &Json::Null, &[]).is_err());
    }

    /// A real picture file in a fresh directory, so the sidecar can be written next to it.
    fn real_library(tag: &str) -> (PathBuf, Vec<Photo>) {
        let dir = std::env::temp_dir().join(format!("bifrost-photos-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pic.png");
        std::fs::write(&path, b"not really a picture").unwrap();
        (dir, vec![Photo { path, mtime: 100, size: 20 }])
    }

    #[test]
    fn observing_needs_access_and_reports_nothing_until_the_library_changes() {
        let app = "is.example.photos-observe";
        assert!(answer(app, true, "observe", &Json::Null, &library()).is_err());
        granted(app);
        assert_eq!(answer(app, true, "observe", &Json::Null, &library()).unwrap().get("observing"), Some(&Json::Bool(true)));
        let access = access_of(app, true);
        let before = listing(&library(), &access);
        assert_eq!(before.len(), 4);
        assert!(change_body(&before, &listing(&library(), &access)).is_none());
        assert_eq!(answer(app, true, "unobserve", &Json::Null, &[]).unwrap().get("observing"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_library_change_becomes_an_event_body_of_indexes() {
        let access = Access::new(Status::Authorized);
        let old = listing(&library(), &access);
        let mut newer = library();
        newer.remove(0);
        newer.push(photo("e.png", 400));
        let body = change_body(&old, &listing(&newer, &access)).unwrap();
        assert_eq!(body.get("removed").and_then(Json::as_array).map(|a| a.len()), Some(1));
        assert_eq!(body.get("inserted").and_then(Json::as_array).map(|a| a.len()), Some(1));
    }

    #[test]
    fn the_sidecar_path_sits_beside_the_picture() {
        assert_eq!(sidecar_path(Path::new("/p/a.jpg")), PathBuf::from("/p/a.jpg.adjust.json"));
    }

    #[test]
    fn hex_round_trips_and_rejects_odd_or_non_hex_text() {
        assert_eq!(from_hex(&to_hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn an_edit_is_committed_read_and_reverted() {
        let app = "is.example.photos-edit";
        granted(app);
        let (dir, photos) = real_library("edit");
        let id = local_identifier(&photos[0]);
        let commit = params(vec![
            ("op", Json::Str("commit".into())),
            ("id", Json::Str(id.clone())),
            ("format_identifier", Json::Str("com.example.crop".into())),
            ("format_version", Json::Str("1".into())),
            ("data", Json::Str("0a0b".into())),
        ]);
        assert_eq!(answer(app, true, "editing", &commit, &photos).unwrap().get("committed"), Some(&Json::Bool(true)));
        let get = params(vec![("op", Json::Str("get".into())), ("id", Json::Str(id.clone()))]);
        let got = answer(app, true, "editing", &get, &photos).unwrap();
        let adj = got.get("adjustment").unwrap();
        assert_eq!(adj.str_field("format_identifier").as_deref(), Some("com.example.crop"));
        assert_eq!(adj.str_field("data").as_deref(), Some("0a0b"));
        assert!(sidecar_path(&photos[0].path).exists());
        let revert = params(vec![("op", Json::Str("revert".into())), ("id", Json::Str(id.clone()))]);
        answer(app, true, "editing", &revert, &photos).unwrap();
        assert_eq!(answer(app, true, "editing", &get, &photos).unwrap().get("adjustment"), Some(&Json::Null));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_edit_without_access_or_with_a_blank_format_is_refused() {
        let app = "is.example.photos-edit-denied";
        let (dir, photos) = real_library("denied");
        let id = local_identifier(&photos[0]);
        let get = params(vec![("op", Json::Str("get".into())), ("id", Json::Str(id.clone()))]);
        assert!(answer(app, true, "editing", &get, &photos).is_err());
        granted(app);
        let blank = params(vec![
            ("op", Json::Str("commit".into())),
            ("id", Json::Str(id)),
            ("format_identifier", Json::Str(" ".into())),
            ("format_version", Json::Str("1".into())),
            ("data", Json::Str("00".into())),
        ]);
        assert!(answer(app, true, "editing", &blank, &photos).is_err());
        assert!(!sidecar_path(&photos[0].path).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_picker_checks_its_filters_and_opens_a_pending_pick() {
        let app = "is.example.photos-picker";
        let good = params(vec![("filters", Json::Arr(vec![Json::Str("images".into())])), ("limit", Json::Num(3.0))]);
        let reply = answer(app, true, "picker", &good, &library()).unwrap();
        assert_eq!(reply.str_field("status").as_deref(), Some("pending"), "the person has not chosen yet");
        let pick = photo_pick::take_waiting().expect("the sheet has a pick to show");
        assert_eq!((pick.app_id.as_str(), pick.limit, pick.allowed.clone()), (app, 3, vec![PickFilter::Images]));
        let shown: Vec<&str> = pick.visible().into_iter().map(|i| pick.entries[i].id.as_str()).collect();
        assert_eq!(shown, [id("b.png", 300), id("d.png", 200), id("a.png", 100)], "pictures only, newest first");
        let bad = params(vec![("filters", Json::Arr(vec![Json::Str("holograms".into())]))]);
        let err = answer(app, true, "picker", &bad, &library()).unwrap_err();
        assert!(err.contains("holograms"), "{err}");
        assert!(photo_pick::take_waiting().is_none(), "a refused call opens nothing");
    }

    #[test]
    fn the_picker_defaults_to_one_choice_and_refuses_a_second_sheet() {
        let app = "is.example.photos-picker-one";
        answer(app, false, "picker", &Json::Null, &library()).unwrap();
        let pick = photo_pick::take_waiting().unwrap();
        assert_eq!((pick.limit, pick.allowed.len(), pick.visible().len()), (1, 0, 4), "no filter lists everything");
        let err = answer(app, false, "picker", &Json::Null, &library()).unwrap_err();
        assert!(err.contains("already open"), "{err}");
    }

    #[test]
    fn what_the_person_chose_is_readable_and_nothing_else() {
        let app = "is.example.photos-picked";
        let (chosen, other) = (id("b.png", 300), id("d.png", 200));
        let read = |asset: &str| answer(app, false, "image", &params(vec![("id", Json::Str(asset.into()))]), &library());
        assert!(read(&chosen).is_err(), "no library access, no pick yet");
        answer(app, false, "picker", &params(vec![("limit", Json::Num(2.0))]), &library()).unwrap();
        let mut pick = photo_pick::take_waiting().unwrap();
        pick.toggle(&chosen);
        photo_pick::finish(app, pick.selected().to_vec());
        let mut shell = crate::shell::tests::shell();
        let events = Photos::default().tick(&mut shell, 10.0);
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].app.as_deref(), events[0].event.as_str()), (Some(app), PICKER_DID_FINISH));
        let results = events[0].body.get("results").and_then(Json::as_array).unwrap();
        assert_eq!(results.iter().filter_map(|r| r.str_field("id")).collect::<Vec<_>>(), [chosen.clone()]);
        assert_eq!(events[0].body.get("cancelled"), Some(&Json::Bool(false)));
        assert!(read(&chosen).is_ok(), "the chosen picture is readable without library access");
        assert!(read(&other).is_err(), "a picture the person did not choose is not");
        assert!(answer(app, false, "fetch", &Json::Null, &library()).is_err(), "the app still cannot list the library");
        assert_eq!(answer(app, false, "authorization", &Json::Null, &[]).unwrap().str_field("status").as_deref(), Some("denied"));
    }

    #[test]
    fn a_cancelled_picker_tells_the_app_and_grants_nothing() {
        let app = "is.example.photos-cancelled";
        answer(app, true, "picker", &Json::Null, &library()).unwrap();
        photo_pick::take_waiting().unwrap();
        photo_pick::cancel(app);
        let mut shell = crate::shell::tests::shell();
        let events = Photos::default().tick(&mut shell, 10.0);
        assert_eq!(events[0].body.get("cancelled"), Some(&Json::Bool(true)));
        assert_eq!(events[0].body.get("results").and_then(Json::as_array).map(|r| r.len()), Some(0));
        assert!(picked_of(app).is_empty());
    }

    #[test]
    fn a_project_is_created_changed_and_read_back_per_app() {
        let app = "is.example.photos-project";
        let create = params(vec![("op", Json::Str("create".into())), ("id", Json::Str("book".into())), ("title", Json::Str("Trip".into()))]);
        answer(app, true, "project", &create, &[]).unwrap();
        assert!(answer(app, true, "project", &create, &[]).is_err());
        let change = |kind: &str, fields: Vec<(&str, Json)>| {
            let mut all = vec![("kind", Json::Str(kind.into()))];
            all.extend(fields);
            params(all)
        };
        let apply = params(vec![
            ("op", Json::Str("apply".into())),
            ("id", Json::Str("book".into())),
            (
                "changes",
                Json::Arr(vec![
                    change("add_section", vec![("id", Json::Str("s1".into())), ("title", Json::Str("Day one".into()))]),
                    change("add_assets", vec![("section", Json::Str("s1".into())), ("assets", Json::Arr(vec![Json::Str("PH-1".into())]))]),
                ]),
            ),
        ]);
        answer(app, true, "project", &apply, &[]).unwrap();
        let got = answer(app, true, "project", &params(vec![("op", Json::Str("get".into())), ("id", Json::Str("book".into()))]), &[]).unwrap();
        let sections = got.get("sections").and_then(Json::as_array).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].get("assets").and_then(Json::as_array).map(|a| a.len()), Some(1));
        // another app does not see the project
        assert!(answer("is.example.photos-other", true, "project", &params(vec![("op", Json::Str("get".into())), ("id", Json::Str("book".into()))]), &[]).is_err());
    }

    #[test]
    fn a_project_needs_the_permission_and_a_failing_apply_changes_nothing() {
        assert!(answer("is.example.photos-noperm", false, "project", &params(vec![("op", Json::Str("create".into())), ("id", Json::Str("b".into()))]), &[]).is_err());
        let app = "is.example.photos-atomic";
        answer(app, true, "project", &params(vec![("op", Json::Str("create".into())), ("id", Json::Str("b".into())), ("title", Json::Str("Book".into()))]), &[]).unwrap();
        let bad = params(vec![
            ("op", Json::Str("apply".into())),
            ("id", Json::Str("b".into())),
            ("changes", Json::Arr(vec![params(vec![("kind", Json::Str("add_section".into())), ("id", Json::Str("s".into())), ("title", Json::Str("t".into()))]), params(vec![("kind", Json::Str("remove_section".into())), ("id", Json::Str("missing".into()))])])),
        ]);
        assert!(answer(app, true, "project", &bad, &[]).is_err());
        let got = answer(app, true, "project", &params(vec![("op", Json::Str("get".into())), ("id", Json::Str("b".into()))]), &[]).unwrap();
        assert_eq!(got.get("sections").and_then(Json::as_array).map(|a| a.len()), Some(0));
    }

    fn op(fields: Vec<(&str, Json)>) -> Json {
        params(fields)
    }

    fn batch(ops: Vec<Json>) -> Json {
        params(vec![("ops", Json::Arr(ops))])
    }

    #[test]
    fn a_batch_is_planned_and_nothing_is_written() {
        let app = "is.example.photos-perform-plan";
        granted(app);
        let fav = id("a.png", 100);
        let trash = id("b.png", 300);
        let req = batch(vec![
            op(vec![("op", Json::Str("favourite".into())), ("id", Json::Str(fav.clone())), ("on", Json::Bool(true))]),
            op(vec![("op", Json::Str("trash".into())), ("id", Json::Str(trash.clone()))]),
            op(vec![("op", Json::Str("create".into())), ("name", Json::Str("a.png".into()))]),
        ]);
        let r = answer(app, true, "perform", &req, &library()).unwrap();
        assert_eq!(r.get("written").cloned(), Some(Json::Bool(false)));
        let favs = r.get("favourites").and_then(Json::as_array).unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].str_field("id").as_deref(), Some(fav.as_str()));
        assert_eq!(r.get("trashed").and_then(Json::as_array).unwrap()[0].as_str(), Some(trash.as_str()));
        let created = r.get("created").and_then(Json::as_array).unwrap();
        assert_eq!(created[0].str_field("name").as_deref(), Some("a-2.png"));
        assert_eq!(created[0].str_field("media_type").as_deref(), Some("image"));
    }

    #[test]
    fn a_batch_refuses_an_unknown_or_repeated_asset_and_a_bad_name() {
        let app = "is.example.photos-perform-refuse";
        granted(app);
        let unknown = batch(vec![op(vec![("op", Json::Str("favourite".into())), ("id", Json::Str("PH-nope".into())), ("on", Json::Bool(true))])]);
        assert!(answer(app, true, "perform", &unknown, &library()).is_err());
        let fav = id("a.png", 100);
        let twice = batch(vec![
            op(vec![("op", Json::Str("favourite".into())), ("id", Json::Str(fav.clone())), ("on", Json::Bool(true))]),
            op(vec![("op", Json::Str("trash".into())), ("id", Json::Str(fav))]),
        ]);
        assert!(answer(app, true, "perform", &twice, &library()).is_err());
        let bad = batch(vec![op(vec![("op", Json::Str("create".into())), ("name", Json::Str("../x.jpg".into()))])]);
        assert!(answer(app, true, "perform", &bad, &library()).is_err());
    }

    #[test]
    fn a_batch_needs_ops_known_op_names_and_the_permission() {
        let app = "is.example.photos-perform-shape";
        granted(app);
        assert!(answer(app, true, "perform", &Json::Null, &library()).is_err());
        let unknown_op = batch(vec![op(vec![("op", Json::Str("rename".into()))])]);
        assert!(answer(app, true, "perform", &unknown_op, &library()).is_err());
        let denied = "is.example.photos-perform-denied";
        assert!(answer(denied, false, "perform", &batch(vec![]), &library()).is_err());
    }
}
