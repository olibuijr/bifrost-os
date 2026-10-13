//! A photo library the way PhotoKit shows it: `PHPhotoLibrary.performChanges` transactions, asset change requests
//! (create from an image or a video, add resources, favourite, hide, delete), albums and collection lists, fetch
//! options with a predicate and sort descriptors, fetch result change details and change observers.
//!
//! A transaction works on a copy of the library. When the closure fails nothing is kept, so a transaction is atomic;
//! when it succeeds the copy replaces the library and every observer is told once, with a `Change` that can diff any
//! earlier fetch result (inserted, removed, changed and moved indexes). Access comes from `photo_access`: add-only
//! access may create assets but not read or edit them, and limited access sees only the chosen items (assets the app
//! creates join that selection). Pure: the shell owns the files; a video's length is read from its container bytes.

use crate::diff;
use crate::photo_access::{Access, Status};
use crate::photo_changes;
use crate::photo_media::{kind_for_name, Kind};
use std::collections::HashMap;
use std::rc::Rc;

/// `PHAccessLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessLevel {
    AddOnly,
    ReadWrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The app has no access (not asked, denied or restricted).
    NotAuthorized(Status),
    /// Add-only access cannot read or edit.
    AddOnlyAccess,
    /// No asset, album or list with this identifier is visible.
    NotFound(String),
    /// The resource file name is not an image or a video.
    UnsupportedResource(String),
    /// A creation request has no resource.
    NoResource,
}

/// One resource of an asset (`PHAssetResource`): a file name and its kind.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub file: String,
    pub kind: Kind,
}

/// `PHAsset`.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub id: String,
    pub media_type: Kind,
    pub creation_date: u64,
    pub modification_date: u64,
    pub is_favorite: bool,
    pub is_hidden: bool,
    pub is_trashed: bool,
    /// Seconds, for a video.
    pub duration: Option<f64>,
    pub resources: Vec<Resource>,
}

/// `PHAssetCollection` (a user album).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetCollection {
    pub id: String,
    pub title: String,
    pub asset_ids: Vec<String>,
}

/// `PHCollectionList`: a folder of albums.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionList {
    pub id: String,
    pub title: String,
    pub album_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    CreationDate,
    ModificationDate,
    Duration,
}

/// `SortDescriptor` over an asset key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortDescriptor {
    pub key: SortKey,
    pub ascending: bool,
}

impl SortDescriptor {
    pub fn new(key: SortKey, ascending: bool) -> Self {
        SortDescriptor { key, ascending }
    }
}

/// `PHFetchOptions`. The default sorts by creation date, newest first, and leaves hidden assets out.
#[derive(Clone, Default)]
pub struct FetchOptions {
    pub predicate: Option<Rc<dyn Fn(&Asset) -> bool>>,
    pub sort_descriptors: Vec<SortDescriptor>,
    pub include_hidden_assets: bool,
    pub fetch_limit: Option<usize>,
}

impl FetchOptions {
    pub fn with_predicate(mut self, f: impl Fn(&Asset) -> bool + 'static) -> Self {
        self.predicate = Some(Rc::new(f));
        self
    }
}

/// What a fetch result was asked for, so it can be run again on a newer library state.
#[derive(Clone)]
enum Query {
    Assets(FetchOptions),
    InAlbum(String, FetchOptions),
}

/// `PHFetchResult<PHAsset>`: the identifiers in order, plus the query that produced them.
#[derive(Clone)]
pub struct FetchResult {
    ids: Vec<String>,
    query: Query,
}

impl FetchResult {
    pub fn count(&self) -> usize {
        self.ids.len()
    }
    pub fn ids(&self) -> &[String] {
        &self.ids
    }
    pub fn object_at(&self, index: usize) -> Option<&str> {
        self.ids.get(index).map(|s| s.as_str())
    }
}

/// `PHFetchResultChangeDetails`. Indexes of removed items refer to the old result, the others to the new one.
#[derive(Clone)]
pub struct FetchResultChangeDetails {
    pub fetch_result_after_changes: FetchResult,
    pub has_incremental_changes: bool,
    pub removed_indexes: Vec<usize>,
    pub inserted_indexes: Vec<usize>,
    pub changed_indexes: Vec<usize>,
    /// `(from, to)` pairs.
    pub moves: Vec<(usize, usize)>,
}

impl FetchResultChangeDetails {
    pub fn has_moves(&self) -> bool {
        !self.moves.is_empty()
    }
}

#[derive(Clone, Default)]
struct Snapshot {
    assets: Vec<Asset>,
    albums: Vec<AssetCollection>,
    lists: Vec<CollectionList>,
}

/// `PHChange`: the library before and after one transaction.
pub struct Change {
    before: Snapshot,
    after: Snapshot,
    access: Access,
}

impl Change {
    /// `changeDetails(for:)`: None when the fetch result is unaffected.
    pub fn change_details(&self, fetch: &FetchResult) -> Option<FetchResultChangeDetails> {
        let new_ids = run_query(&self.after, &self.access, &fetch.query);
        let stamp = |snap: &Snapshot, ids: &[String]| -> Vec<photo_changes::Asset> {
            ids.iter()
                .map(|id| {
                    let modified = snap.assets.iter().find(|a| &a.id == id).map_or(0, |a| a.modification_date);
                    photo_changes::Asset { id: id.clone(), modified }
                })
                .collect()
        };
        let d = photo_changes::details(&stamp(&self.before, &fetch.ids), &stamp(&self.after, &new_ids)).ok()?;
        if d.is_empty() {
            return None;
        }
        let moves = diff::diff(&fetch.ids, &new_ids)
            .into_iter()
            .filter_map(|c| match c {
                diff::Change::Move { from, to } => Some((from, to)),
                _ => None,
            })
            .collect();
        Some(FetchResultChangeDetails {
            fetch_result_after_changes: FetchResult { ids: new_ids, query: fetch.query.clone() },
            has_incremental_changes: true,
            removed_indexes: d.removed,
            inserted_indexes: d.inserted,
            changed_indexes: d.changed,
            moves,
        })
    }
}

/// `PHPhotoLibraryChangeObserver.photoLibraryDidChange(_:)`.
pub type Observer = Box<dyn FnMut(&Change)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObserverId(u32);

/// `PHPhotoLibrary`.
pub struct PhotoLibrary {
    state: Snapshot,
    access: Access,
    level: AccessLevel,
    clock: u64,
    next_id: u32,
    next_observer: u32,
    observers: Vec<(ObserverId, Observer)>,
}

impl PhotoLibrary {
    pub fn new(access: Access, level: AccessLevel) -> Self {
        PhotoLibrary { state: Snapshot::default(), access, level, clock: 0, next_id: 1, next_observer: 1, observers: Vec::new() }
    }

    pub fn authorization_status(&self) -> Status {
        self.access.status()
    }

    pub fn register_change_observer(&mut self, o: Observer) -> ObserverId {
        let id = ObserverId(self.next_observer);
        self.next_observer += 1;
        self.observers.push((id, o));
        id
    }

    pub fn unregister_change_observer(&mut self, id: ObserverId) {
        self.observers.retain(|(i, _)| *i != id);
    }

    /// `performChanges`: runs `f` on a copy; on `Ok` the copy is kept and observers hear once.
    pub fn perform_changes<F>(&mut self, f: F) -> Result<(), Error>
    where
        F: FnOnce(&mut ChangeRequests) -> Result<(), Error>,
    {
        self.check_write()?;
        let mut work = ChangeRequests {
            snap: self.state.clone(),
            access: self.access.clone(),
            level: self.level,
            clock: self.clock + 1,
            next_id: self.next_id,
            changed: false,
        };
        f(&mut work)?;
        if !work.changed {
            return Ok(());
        }
        let change = Change { before: std::mem::replace(&mut self.state, work.snap.clone()), after: work.snap, access: work.access.clone() };
        self.access = work.access;
        self.clock = work.clock;
        self.next_id = work.next_id;
        let mut observers = std::mem::take(&mut self.observers);
        for (_, o) in observers.iter_mut() {
            o(&change);
        }
        self.observers = observers;
        Ok(())
    }

    fn check_write(&self) -> Result<(), Error> {
        match self.access.status() {
            Status::Authorized | Status::Limited => Ok(()),
            s => Err(Error::NotAuthorized(s)),
        }
    }

    fn check_read(&self) -> Result<(), Error> {
        self.check_write()?;
        if self.level == AccessLevel::AddOnly {
            return Err(Error::AddOnlyAccess);
        }
        Ok(())
    }

    /// `PHAsset.fetchAssets(with:)`.
    pub fn fetch_assets(&self, opts: FetchOptions) -> Result<FetchResult, Error> {
        self.check_read()?;
        let query = Query::Assets(opts);
        Ok(FetchResult { ids: run_query(&self.state, &self.access, &query), query })
    }

    /// `PHAsset.fetchAssets(in:options:)`.
    pub fn fetch_assets_in(&self, album: &str, opts: FetchOptions) -> Result<FetchResult, Error> {
        self.check_read()?;
        if !self.state.albums.iter().any(|a| a.id == album) {
            return Err(Error::NotFound(album.to_string()));
        }
        let query = Query::InAlbum(album.to_string(), opts);
        Ok(FetchResult { ids: run_query(&self.state, &self.access, &query), query })
    }

    pub fn fetch_asset_collections(&self) -> Result<Vec<AssetCollection>, Error> {
        self.check_read()?;
        Ok(self.state.albums.clone())
    }

    pub fn fetch_collection_lists(&self) -> Result<Vec<CollectionList>, Error> {
        self.check_read()?;
        Ok(self.state.lists.clone())
    }

    /// The asset with this identifier, if the app may see it.
    pub fn asset(&self, id: &str) -> Result<Asset, Error> {
        self.check_read()?;
        self.state.assets.iter().find(|a| a.id == id && self.access.allows(id)).cloned().ok_or_else(|| Error::NotFound(id.to_string()))
    }
}

fn run_query(snap: &Snapshot, access: &Access, q: &Query) -> Vec<String> {
    let (opts, scope): (&FetchOptions, Option<&AssetCollection>) = match q {
        Query::Assets(o) => (o, None),
        Query::InAlbum(id, o) => (o, snap.albums.iter().find(|a| &a.id == id)),
    };
    if matches!(q, Query::InAlbum(..)) && scope.is_none() {
        return Vec::new();
    }
    let mut list: Vec<&Asset> = snap
        .assets
        .iter()
        .filter(|a| access.allows(&a.id) && !a.is_trashed)
        .filter(|a| opts.include_hidden_assets || !a.is_hidden)
        .filter(|a| scope.map_or(true, |s| s.asset_ids.contains(&a.id)))
        .filter(|a| opts.predicate.as_ref().map_or(true, |p| p(a)))
        .collect();
    if opts.sort_descriptors.is_empty() {
        match scope {
            // An album keeps its own order unless the caller sorts.
            Some(s) => {
                let pos: HashMap<&str, usize> = s.asset_ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
                list.sort_by_key(|a| pos[a.id.as_str()]);
            }
            None => list.sort_by(|a, b| b.creation_date.cmp(&a.creation_date).then_with(|| a.id.cmp(&b.id))),
        }
    } else {
        list.sort_by(|a, b| {
            for d in &opts.sort_descriptors {
                let ord = match d.key {
                    SortKey::CreationDate => a.creation_date.cmp(&b.creation_date),
                    SortKey::ModificationDate => a.modification_date.cmp(&b.modification_date),
                    SortKey::Duration => a
                        .duration
                        .unwrap_or(0.0)
                        .partial_cmp(&b.duration.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal),
                };
                let ord = if d.ascending { ord } else { ord.reverse() };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            a.id.cmp(&b.id)
        });
    }
    let mut ids: Vec<String> = list.into_iter().map(|a| a.id.clone()).collect();
    if let Some(n) = opts.fetch_limit {
        ids.truncate(n);
    }
    ids
}

/// The change requests of one transaction (`PHAssetChangeRequest`, `PHAssetCreationRequest`,
/// `PHAssetCollectionChangeRequest`, `PHCollectionListChangeRequest`).
pub struct ChangeRequests {
    snap: Snapshot,
    access: Access,
    level: AccessLevel,
    clock: u64,
    next_id: u32,
    changed: bool,
}

impl ChangeRequests {
    fn fresh_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.next_id);
        self.next_id += 1;
        id
    }

    fn edit_allowed(&self) -> Result<(), Error> {
        if self.level == AccessLevel::AddOnly {
            Err(Error::AddOnlyAccess)
        } else {
            Ok(())
        }
    }

    fn asset_mut(&mut self, id: &str) -> Result<&mut Asset, Error> {
        self.edit_allowed()?;
        let visible = self.access.allows(id);
        self.snap.assets.iter_mut().find(|a| a.id == id && visible).ok_or_else(|| Error::NotFound(id.to_string()))
    }

    /// `creationRequestForAsset(from:)`: a new image asset from a file name; returns the placeholder identifier.
    pub fn creation_request_for_asset_from_image(&mut self, file: &str) -> Result<String, Error> {
        self.create_asset(file, Kind::Image, None)
    }

    /// `creationRequestForAssetFromVideo(atFileURL:)`; `duration` comes from `video_duration`.
    pub fn creation_request_for_asset_from_video(&mut self, file: &str, duration: Option<f64>) -> Result<String, Error> {
        self.create_asset(file, Kind::Video, duration)
    }

    /// `PHAssetCreationRequest.forAsset()` followed by `addResource(with:fileURL:options:)`; the kind comes from the
    /// file name, and a second resource (for example a paired movie) joins the same asset.
    pub fn creation_request_with_resources(&mut self, files: &[(&str, Option<f64>)]) -> Result<String, Error> {
        let Some(&(first, duration)) = files.first() else {
            return Err(Error::NoResource);
        };
        let id = self.create_asset(first, kind_for_name(first), duration)?;
        for &(file, _) in &files[1..] {
            self.add_resource(&id, file)?;
        }
        Ok(id)
    }

    /// `addResource(with:fileURL:options:)` on an asset created in this transaction.
    pub fn add_resource(&mut self, asset: &str, file: &str) -> Result<(), Error> {
        let kind = kind_for_name(file);
        if !matches!(kind, Kind::Image | Kind::Video) {
            return Err(Error::UnsupportedResource(file.to_string()));
        }
        let a = self.snap.assets.iter_mut().find(|a| a.id == asset).ok_or_else(|| Error::NotFound(asset.to_string()))?;
        a.resources.push(Resource { file: file.to_string(), kind });
        self.changed = true;
        Ok(())
    }

    fn create_asset(&mut self, file: &str, kind: Kind, duration: Option<f64>) -> Result<String, Error> {
        if !matches!(kind, Kind::Image | Kind::Video) {
            return Err(Error::UnsupportedResource(file.to_string()));
        }
        let id = self.fresh_id("asset-");
        self.snap.assets.push(Asset {
            id: id.clone(),
            media_type: kind,
            creation_date: self.clock,
            modification_date: self.clock,
            is_favorite: false,
            is_hidden: false,
            is_trashed: false,
            duration: if kind == Kind::Video { duration } else { None },
            resources: vec![Resource { file: file.to_string(), kind }],
        });
        // A created asset is visible to the app that made it, also in limited access.
        if self.access.status() == Status::Limited {
            let mut chosen = self.access.chosen();
            chosen.push(id.clone());
            let _ = self.access.select(&chosen);
        }
        self.changed = true;
        Ok(id)
    }

    /// `PHAssetChangeRequest.isFavorite`.
    pub fn set_favorite(&mut self, asset: &str, on: bool) -> Result<(), Error> {
        let clock = self.clock;
        let a = self.asset_mut(asset)?;
        if a.is_favorite != on {
            a.is_favorite = on;
            a.modification_date = clock;
            self.changed = true;
        }
        Ok(())
    }

    /// `PHAssetChangeRequest.isHidden`.
    pub fn set_hidden(&mut self, asset: &str, on: bool) -> Result<(), Error> {
        let clock = self.clock;
        let a = self.asset_mut(asset)?;
        if a.is_hidden != on {
            a.is_hidden = on;
            a.modification_date = clock;
            self.changed = true;
        }
        Ok(())
    }

    /// `PHAssetChangeRequest.deleteAssets`: the assets leave the library and every album.
    pub fn delete_assets(&mut self, ids: &[&str]) -> Result<(), Error> {
        for id in ids {
            self.asset_mut(id)?;
        }
        self.snap.assets.retain(|a| !ids.contains(&a.id.as_str()));
        for album in &mut self.snap.albums {
            album.asset_ids.retain(|a| !ids.contains(&a.as_str()));
        }
        self.changed |= !ids.is_empty();
        Ok(())
    }

    /// `PHAssetCollectionChangeRequest.creationRequestForAssetCollection(withTitle:)`.
    pub fn create_album(&mut self, title: &str) -> Result<String, Error> {
        self.edit_allowed()?;
        let id = self.fresh_id("album-");
        self.snap.albums.push(AssetCollection { id: id.clone(), title: title.to_string(), asset_ids: Vec::new() });
        self.changed = true;
        Ok(id)
    }

    fn album_mut(&mut self, id: &str) -> Result<&mut AssetCollection, Error> {
        self.edit_allowed()?;
        self.snap.albums.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn rename_album(&mut self, id: &str, title: &str) -> Result<(), Error> {
        self.album_mut(id)?.title = title.to_string();
        self.changed = true;
        Ok(())
    }

    pub fn delete_album(&mut self, id: &str) -> Result<(), Error> {
        self.album_mut(id)?;
        self.snap.albums.retain(|a| a.id != id);
        for l in &mut self.snap.lists {
            l.album_ids.retain(|a| a != id);
        }
        self.changed = true;
        Ok(())
    }

    /// `addAssets`: appends, skipping assets already in the album.
    pub fn add_assets(&mut self, album: &str, ids: &[&str]) -> Result<(), Error> {
        for id in ids {
            self.asset_mut(id)?;
        }
        let a = self.album_mut(album)?;
        for id in ids {
            if !a.asset_ids.iter().any(|x| x == id) {
                a.asset_ids.push(id.to_string());
            }
        }
        self.changed = true;
        Ok(())
    }

    /// `removeAssets`: the assets stay in the library.
    pub fn remove_assets(&mut self, album: &str, ids: &[&str]) -> Result<(), Error> {
        let a = self.album_mut(album)?;
        a.asset_ids.retain(|x| !ids.contains(&x.as_str()));
        self.changed = true;
        Ok(())
    }

    /// `PHCollectionListChangeRequest.creationRequestForCollectionList(withTitle:)`.
    pub fn create_collection_list(&mut self, title: &str) -> Result<String, Error> {
        self.edit_allowed()?;
        let id = self.fresh_id("list-");
        self.snap.lists.push(CollectionList { id: id.clone(), title: title.to_string(), album_ids: Vec::new() });
        self.changed = true;
        Ok(id)
    }

    pub fn add_child_album(&mut self, list: &str, album: &str) -> Result<(), Error> {
        self.album_mut(album)?;
        let l = self.snap.lists.iter_mut().find(|l| l.id == list).ok_or_else(|| Error::NotFound(list.to_string()))?;
        if !l.album_ids.iter().any(|a| a == album) {
            l.album_ids.push(album.to_string());
        }
        self.changed = true;
        Ok(())
    }
}

/// The length in seconds of an MP4 or QuickTime file, from the `mvhd` box inside `moov`. None when the bytes are not
/// such a container or the timescale is zero.
pub fn video_duration(bytes: &[u8]) -> Option<f64> {
    fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> + '_ {
        let mut pos = 0usize;
        std::iter::from_fn(move || {
            let head = data.get(pos..pos + 8)?;
            let size = u32::from_be_bytes(head[0..4].try_into().ok()?) as usize;
            let end = if size == 0 { data.len() } else { pos.checked_add(size)? };
            if (size < 8 && size != 0) || end > data.len() {
                return None;
            }
            let kind: [u8; 4] = head[4..8].try_into().ok()?;
            let body = &data[pos + 8..end];
            pos = end;
            Some((kind, body))
        })
    }
    let moov = boxes(bytes).find(|(t, _)| t == b"moov")?.1;
    let mvhd = boxes(moov).find(|(t, _)| t == b"mvhd")?.1;
    let (scale, dur) = match *mvhd.first()? {
        0 => (u32::from_be_bytes(mvhd.get(12..16)?.try_into().ok()?) as u64, u32::from_be_bytes(mvhd.get(16..20)?.try_into().ok()?) as u64),
        1 => (u32::from_be_bytes(mvhd.get(20..24)?.try_into().ok()?) as u64, u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?)),
        _ => return None,
    };
    if scale == 0 {
        return None;
    }
    Some(dur as f64 / scale as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn lib() -> PhotoLibrary {
        PhotoLibrary::new(Access::new(Status::Authorized), AccessLevel::ReadWrite)
    }

    fn add(l: &mut PhotoLibrary, files: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        l.perform_changes(|r| {
            for f in files {
                out.push(r.creation_request_for_asset_from_image(f)?);
            }
            Ok(())
        })
        .unwrap();
        out
    }

    fn all(l: &PhotoLibrary) -> FetchResult {
        l.fetch_assets(FetchOptions::default()).unwrap()
    }

    #[test]
    fn a_failed_transaction_rolls_everything_back() {
        let mut l = lib();
        let r = l.perform_changes(|r| {
            r.creation_request_for_asset_from_image("a.jpg")?;
            r.creation_request_for_asset_from_image("notes.txt")?;
            Ok(())
        });
        assert_eq!(r, Err(Error::UnsupportedResource("notes.txt".into())));
        assert_eq!(all(&l).count(), 0);
    }

    #[test]
    fn creates_from_image_and_from_video_with_duration() {
        let mut l = lib();
        l.perform_changes(|r| {
            r.creation_request_for_asset_from_image("a.jpg")?;
            r.creation_request_for_asset_from_video("b.mov", Some(12.5))?;
            Ok(())
        })
        .unwrap();
        let ids = all(&l).ids().to_vec();
        let kinds: Vec<_> = ids.iter().map(|i| l.asset(i).unwrap()).map(|a| (a.media_type, a.duration)).collect();
        assert!(kinds.contains(&(Kind::Image, None)));
        assert!(kinds.contains(&(Kind::Video, Some(12.5))));
    }

    #[test]
    fn a_creation_request_takes_more_resources() {
        let mut l = lib();
        let mut id = String::new();
        l.perform_changes(|r| {
            id = r.creation_request_with_resources(&[("live.jpg", None), ("live.mov", None)])?;
            assert_eq!(r.creation_request_with_resources(&[]), Err(Error::NoResource));
            Ok(())
        })
        .unwrap();
        assert_eq!(l.asset(&id).unwrap().resources.len(), 2);
    }

    #[test]
    fn favorite_toggles_and_bumps_the_modification_date() {
        let mut l = lib();
        let ids = add(&mut l, &["a.jpg"]);
        l.perform_changes(|r| r.set_favorite(&ids[0], true)).unwrap();
        let a = l.asset(&ids[0]).unwrap();
        assert!(a.is_favorite);
        assert!(a.modification_date > a.creation_date);
        let fav = l.fetch_assets(FetchOptions::default().with_predicate(|a| a.is_favorite)).unwrap();
        assert_eq!(fav.ids(), &ids[..]);
    }

    #[test]
    fn album_membership_create_add_remove_delete() {
        let mut l = lib();
        let ids = add(&mut l, &["a.jpg", "b.jpg", "c.jpg"]);
        let (a, c) = (ids[0].as_str(), ids[2].as_str());
        let mut album = String::new();
        l.perform_changes(|r| {
            album = r.create_album("Trip")?;
            r.add_assets(&album, &[c, a, c])
        })
        .unwrap();
        // The album keeps its own order and ignores the repeat.
        assert_eq!(l.fetch_assets_in(&album, FetchOptions::default()).unwrap().ids(), &[ids[2].clone(), ids[0].clone()]);
        l.perform_changes(|r| r.remove_assets(&album, &[c])).unwrap();
        assert_eq!(l.fetch_assets_in(&album, FetchOptions::default()).unwrap().count(), 1);
        assert_eq!(all(&l).count(), 3);
        l.perform_changes(|r| r.rename_album(&album, "Iceland")).unwrap();
        assert_eq!(l.fetch_asset_collections().unwrap()[0].title, "Iceland");
        l.perform_changes(|r| r.delete_assets(&[a])).unwrap();
        assert!(l.fetch_asset_collections().unwrap()[0].asset_ids.is_empty());
        l.perform_changes(|r| r.delete_album(&album)).unwrap();
        assert!(l.fetch_asset_collections().unwrap().is_empty());
        assert_eq!(l.fetch_assets_in(&album, FetchOptions::default()).err(), Some(Error::NotFound(album.clone())));
    }

    #[test]
    fn collection_lists_hold_albums() {
        let mut l = lib();
        l.perform_changes(|r| {
            let a = r.create_album("A")?;
            let f = r.create_collection_list("Folder")?;
            r.add_child_album(&f, &a)
        })
        .unwrap();
        assert_eq!(l.fetch_collection_lists().unwrap()[0].album_ids.len(), 1);
    }

    #[test]
    fn fetch_options_predicate_sort_hidden_and_limit() {
        let mut l = lib();
        let ids = add(&mut l, &["a.jpg", "b.jpg"]);
        l.perform_changes(|r| {
            r.creation_request_for_asset_from_video("c.mov", Some(3.0))?;
            r.creation_request_for_asset_from_video("d.mov", Some(9.0))?;
            Ok(())
        })
        .unwrap();
        let b = ids[1].as_str();
        l.perform_changes(|r| r.set_hidden(b, true)).unwrap();
        let by_len = FetchOptions {
            sort_descriptors: vec![SortDescriptor::new(SortKey::Duration, false)],
            ..Default::default()
        }
        .with_predicate(|a| a.media_type.is_video());
        let v = l.fetch_assets(by_len).unwrap();
        let lens: Vec<_> = v.ids().iter().map(|i| l.asset(i).unwrap().duration.unwrap()).collect();
        assert_eq!(lens, vec![9.0, 3.0]);
        assert_eq!(all(&l).count(), 3); // hidden left out
        let with_hidden = FetchOptions { include_hidden_assets: true, ..Default::default() };
        assert_eq!(l.fetch_assets(with_hidden).unwrap().count(), 4);
        let limited = FetchOptions { fetch_limit: Some(1), ..Default::default() };
        assert_eq!(l.fetch_assets(limited).unwrap().count(), 1);
    }

    #[test]
    fn observers_hear_once_per_transaction_and_not_on_rollback_or_noop() {
        let mut l = lib();
        let n = Rc::new(RefCell::new(0));
        let n2 = n.clone();
        let id = l.register_change_observer(Box::new(move |_| *n2.borrow_mut() += 1));
        l.perform_changes(|r| {
            r.creation_request_for_asset_from_image("a.jpg")?;
            r.creation_request_for_asset_from_image("b.jpg")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(*n.borrow(), 1);
        let _ = l.perform_changes(|r| {
            r.creation_request_for_asset_from_image("c.jpg")?;
            Err(Error::NoResource)
        });
        l.perform_changes(|_| Ok(())).unwrap();
        assert_eq!(*n.borrow(), 1);
        l.unregister_change_observer(id);
        add(&mut l, &["d.jpg"]);
        assert_eq!(*n.borrow(), 1);
    }

    #[test]
    fn change_details_report_inserted_removed_changed_and_unaffected() {
        let mut l = lib();
        let ids = add(&mut l, &["a.jpg", "b.jpg"]);
        let before = all(&l); // equal creation dates, so by identifier: a, b
        let seen: Rc<RefCell<Option<FetchResultChangeDetails>>> = Rc::default();
        let (s, b) = (seen.clone(), before.clone());
        l.register_change_observer(Box::new(move |c| *s.borrow_mut() = c.change_details(&b)));
        let (a, bb) = (ids[0].as_str(), ids[1].as_str());
        l.perform_changes(|r| {
            r.set_favorite(a, true)?; // a changes
            r.delete_assets(&[bb])?; // b removed
            r.creation_request_for_asset_from_image("c.jpg")?; // c inserted, newest, so first
            Ok(())
        })
        .unwrap();
        let d = seen.borrow().clone().unwrap();
        assert_eq!(d.removed_indexes, vec![1]);
        assert_eq!(d.inserted_indexes, vec![0]);
        assert_eq!(d.changed_indexes, vec![1]);
        assert_eq!(d.fetch_result_after_changes.count(), 2);
        // A change that touches no fetched asset gives None.
        let after = d.fetch_result_after_changes.clone();
        let s2: Rc<RefCell<Option<bool>>> = Rc::default();
        let s3 = s2.clone();
        l.register_change_observer(Box::new(move |c| *s3.borrow_mut() = Some(c.change_details(&after).is_none())));
        l.perform_changes(|r| r.create_album("Empty").map(|_| ())).unwrap();
        assert_eq!(*s2.borrow(), Some(true));
    }

    #[test]
    fn change_details_report_moves_when_the_sort_changes() {
        let mut l = lib();
        let ids = add(&mut l, &["a.jpg", "b.jpg", "c.jpg"]);
        let o = FetchOptions { sort_descriptors: vec![SortDescriptor::new(SortKey::ModificationDate, true)], ..Default::default() };
        let before = l.fetch_assets(o).unwrap(); // a, b, c
        let out: Rc<RefCell<Option<FetchResultChangeDetails>>> = Rc::default();
        let (o2, b2) = (out.clone(), before.clone());
        l.register_change_observer(Box::new(move |c| *o2.borrow_mut() = c.change_details(&b2)));
        let a = ids[0].as_str();
        l.perform_changes(|r| r.set_favorite(a, true)).unwrap(); // a is newest now: b, c, a
        let d = out.borrow().clone().unwrap();
        assert!(d.has_moves());
        assert_eq!(d.moves, vec![(0, 2)]);
        assert_eq!(d.changed_indexes, vec![2]);
    }

    #[test]
    fn unauthorized_and_add_only_access_are_enforced() {
        let mut denied = PhotoLibrary::new(Access::new(Status::Denied), AccessLevel::ReadWrite);
        assert_eq!(denied.perform_changes(|_| Ok(())), Err(Error::NotAuthorized(Status::Denied)));
        let mut add_only = PhotoLibrary::new(Access::new(Status::Authorized), AccessLevel::AddOnly);
        let mut id = String::new();
        add_only
            .perform_changes(|r| {
                id = r.creation_request_for_asset_from_image("a.jpg")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(add_only.fetch_assets(FetchOptions::default()).err(), Some(Error::AddOnlyAccess));
        assert_eq!(add_only.perform_changes(|r| r.set_favorite(&id, true)), Err(Error::AddOnlyAccess));
        assert_eq!(add_only.perform_changes(|r| r.create_album("x").map(|_| ())), Err(Error::AddOnlyAccess));
    }

    #[test]
    fn limited_access_sees_chosen_and_created_assets_only() {
        let mut full = lib();
        let ids = add(&mut full, &["a.jpg", "b.jpg"]);
        let mut access = Access::new(Status::NotDetermined);
        access.request(crate::photo_access::Answer::AllowLimited(vec![ids[0].clone()])).unwrap();
        let mut l = PhotoLibrary::new(access, AccessLevel::ReadWrite);
        l.state = full.state.clone();
        l.next_id = full.next_id;
        assert_eq!(all(&l).ids(), &[ids[0].clone()]);
        let hidden = ids[1].as_str();
        assert_eq!(l.perform_changes(|r| r.set_favorite(hidden, true)), Err(Error::NotFound(ids[1].clone())));
        let new = add(&mut l, &["c.jpg"]);
        assert_eq!(all(&l).count(), 2);
        assert!(l.asset(&new[0]).is_ok());
    }

    fn mp4(timescale: u32, duration: u32) -> Vec<u8> {
        let mut mvhd = vec![0u8; 4]; // version 0, flags
        mvhd.extend([0u8; 8]); // creation, modification
        mvhd.extend(timescale.to_be_bytes());
        mvhd.extend(duration.to_be_bytes());
        let mut mvhd_box = ((mvhd.len() + 8) as u32).to_be_bytes().to_vec();
        mvhd_box.extend(b"mvhd");
        mvhd_box.extend(mvhd);
        let mut moov = ((mvhd_box.len() + 8) as u32).to_be_bytes().to_vec();
        moov.extend(b"moov");
        moov.extend(mvhd_box);
        let mut out = vec![0, 0, 0, 8];
        out.extend(b"free");
        out.extend(moov);
        out
    }

    #[test]
    fn video_duration_reads_the_movie_header() {
        assert_eq!(video_duration(&mp4(600, 3000)), Some(5.0));
        assert_eq!(video_duration(&mp4(0, 3000)), None);
        assert_eq!(video_duration(b"not a movie"), None);
        assert_eq!(video_duration(&[]), None);
    }
}
