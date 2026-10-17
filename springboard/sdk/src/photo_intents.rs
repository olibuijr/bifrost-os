//! Photos App Intents (the overlay between the intents framework and the photo library): the entity types a photo
//! app offers to the system and the intents that act on them.
//!
//! Two entity types: `PhotoAsset` (one picture or movie, id = the library's local identifier, title = media kind,
//! subtitle = size) and `PhotoAlbum` (a smart album, id = a fixed slug). Three intents: open an asset, open an album,
//! and set an asset's favourite mark. [`intents`] declares them for `App::intents()`; the entity helpers answer
//! `entity.query` for albums (which need no library scan) and describe assets the app has already fetched.
//!
//! Identifiers are the same strings `PHAsset::local_identifier` carries, so an id an intent receives goes straight
//! back to `PHAsset::fetch_assets` or the `image` call. Nothing here reads the library; the app does, under its own
//! photo permission.
use crate::intents::{Entity, EntityQuery, Intent, Param, ParamType};
use crate::photo_library::{PHAsset, PHAssetMediaType};
use bifrost_ui::photo_smart_albums::SmartAlbum;

/// The entity type name of an asset.
pub const ASSET_TYPE: &str = "PhotoAsset";
/// The entity type name of an album.
pub const ALBUM_TYPE: &str = "PhotoAlbum";

/// The stable id of a smart album.
pub fn album_id(album: SmartAlbum) -> &'static str {
    match album {
        SmartAlbum::Screenshots => "screenshots",
        SmartAlbum::RecentlyAdded => "recents",
        SmartAlbum::Favourites => "favourites",
        SmartAlbum::RecentlyDeleted => "recently-deleted",
    }
}

/// The smart album an id names.
pub fn album_from_id(id: &str) -> Option<SmartAlbum> {
    SmartAlbum::ALL.into_iter().find(|a| album_id(*a) == id)
}

/// A smart album as an entity.
pub fn album_entity(album: SmartAlbum) -> Entity {
    Entity::new(ALBUM_TYPE, album_id(album), album.title())
}

/// An asset the app fetched, as an entity: the title names the kind, the subtitle the pixel size.
pub fn asset_entity(asset: &PHAsset) -> Entity {
    let kind = match asset.media_type {
        PHAssetMediaType::Image if asset.is_live_photo => "Live Photo",
        PHAssetMediaType::Image => "Photo",
        PHAssetMediaType::Video => "Video",
        PHAssetMediaType::Audio => "Audio",
        PHAssetMediaType::Unknown => "Item",
    };
    let entity = Entity::new(ASSET_TYPE, &asset.local_identifier, kind);
    if asset.pixel_width > 0 && asset.pixel_height > 0 {
        entity.subtitle(&format!("{} \u{d7} {}", asset.pixel_width, asset.pixel_height))
    } else {
        entity
    }
}

/// The albums an `entity.query` of type `PhotoAlbum` selects: all of them when suggested, those named by id, or the
/// ones whose title contains the text (case-insensitive).
pub fn query_albums(query: &EntityQuery) -> Vec<Entity> {
    SmartAlbum::ALL
        .into_iter()
        .filter(|a| match query {
            EntityQuery::Suggested => true,
            EntityQuery::Ids(ids) => ids.iter().any(|i| i == album_id(*a)),
            EntityQuery::String(text) => a.title().to_lowercase().contains(&text.trim().to_lowercase()),
        })
        .map(album_entity)
        .collect()
}

/// The intents a photo app declares: open an asset, open an album, set the favourite mark of an asset.
pub fn intents() -> Vec<Intent> {
    vec![
        Intent::new("open-photo", "Open Photo", "Shows one picture or movie of the library.")
            .foreground()
            .param(Param::new("asset", "Photo", ParamType::Entity(ASSET_TYPE.into()))),
        Intent::new("open-album", "Open Album", "Shows an album of the library.")
            .foreground()
            .param(Param::new("album", "Album", ParamType::Entity(ALBUM_TYPE.into()))),
        Intent::new("set-photo-favourite", "Favourite Photo", "Adds a picture to the favourites or removes it.")
            .param(Param::new("asset", "Photo", ParamType::Entity(ASSET_TYPE.into())))
            .param(Param::new("favourite", "Favourite", ParamType::Bool).optional()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::Json;

    fn asset(id: &str, kind: PHAssetMediaType, w: usize, h: usize, live: bool) -> PHAsset {
        PHAsset {
            local_identifier: id.into(),
            media_type: kind,
            pixel_width: w,
            pixel_height: h,
            creation_date: 1,
            modification_date: 1,
            is_favorite: false,
            is_live_photo: live,
        }
    }

    #[test]
    fn every_smart_album_has_a_unique_id_that_maps_back() {
        let ids: Vec<&str> = SmartAlbum::ALL.iter().map(|a| album_id(*a)).collect();
        for (i, a) in SmartAlbum::ALL.iter().enumerate() {
            assert_eq!(album_from_id(ids[i]), Some(*a));
            assert_eq!(ids.iter().filter(|x| **x == ids[i]).count(), 1);
        }
        assert_eq!(album_from_id("nope"), None);
    }

    #[test]
    fn an_asset_entity_names_its_kind_and_size() {
        let e = asset_entity(&asset("a1", PHAssetMediaType::Image, 4000, 3000, false));
        assert_eq!((e.ty.as_str(), e.id.as_str(), e.title.as_str()), (ASSET_TYPE, "a1", "Photo"));
        assert_eq!(e.subtitle.as_deref(), Some("4000 \u{d7} 3000"));
        assert_eq!(asset_entity(&asset("a2", PHAssetMediaType::Image, 10, 10, true)).title, "Live Photo");
        let v = asset_entity(&asset("a3", PHAssetMediaType::Video, 0, 0, false));
        assert_eq!(v.title, "Video");
        assert!(v.subtitle.is_none(), "no size is shown for a movie of unknown size");
    }

    #[test]
    fn album_queries_select_suggested_by_id_and_by_title_text() {
        assert_eq!(query_albums(&EntityQuery::Suggested).len(), SmartAlbum::ALL.len());
        let by_id = query_albums(&EntityQuery::Ids(vec!["favourites".into()]));
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].title, "Favourites");
        let by_text = query_albums(&EntityQuery::String(" SCREEN ".into()));
        assert_eq!(by_text.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["screenshots"]);
        assert!(query_albums(&EntityQuery::String("zzz".into())).is_empty());
    }

    #[test]
    fn the_intents_validate_their_parameters() {
        let all = intents();
        let open = all.iter().find(|i| i.id == "open-photo").unwrap();
        assert!(open.supports(crate::intents::IntentMode::Foreground));
        assert!(open.validate(&Json::Obj(vec![("asset".into(), Json::Str("a1".into()))])).is_ok());
        assert!(open.validate(&Json::Obj(vec![])).is_err(), "the asset is required");
        let fav = all.iter().find(|i| i.id == "set-photo-favourite").unwrap();
        let ok = Json::Obj(vec![("asset".into(), Json::Str("a1".into())), ("favourite".into(), Json::Bool(true))]);
        assert!(fav.validate(&ok).is_ok());
        let bad = Json::Obj(vec![("asset".into(), Json::Str("a1".into())), ("favourite".into(), Json::Str("yes".into()))]);
        assert!(fav.validate(&bad).is_err());
        assert!(all.iter().all(|i| !i.shortcut), "intents with a required parameter are not shortcuts");
    }
}
