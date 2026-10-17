//! Web links as App Intents values: a [`LinkMetadata`] becomes an `AppEntity` of type [`LINK_TYPE`], a JSON value an
//! intent can return, and a result snippet; and a link value an intent receives is read back and checked.
//!
//! Rules of this model:
//! - The entity id is the checked address ([`link_metadata::checked_url`]: `http` or `https` with a host).
//! - The title is the page title when it has one, else the host name. The subtitle is the host name.
//! - A link value is an object `{"url", "title"?, "icon"?, "image"?}`. Reading one back refuses an address that is not
//!   a web address, so a `javascript:` or `data:` link never reaches an app through an intent.

use crate::intents::{Entity, Snippet};
use crate::link_metadata::{checked_url, LinkError, LinkMetadata};
use bifrost_core::json::Json;

/// The entity type name links carry.
pub const LINK_TYPE: &str = "link";

/// The host part of a checked web address (no scheme, no port, no path).
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    authority.split(':').next().unwrap_or(authority)
}

/// The link as an entity. `None` when the address is not a web address.
pub fn link_entity(link: &LinkMetadata) -> Option<Entity> {
    let url = checked_url(&link.url).ok()?;
    let host = host_of(&url).to_string();
    let title = link.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&host).to_string();
    Some(Entity::new(LINK_TYPE, &url, &title).subtitle(&host))
}

/// The link as the JSON value an intent returns.
pub fn link_value(link: &LinkMetadata) -> Json {
    let mut f = vec![("url".to_string(), Json::Str(link.url.clone()))];
    for (name, v) in [("title", &link.title), ("icon", &link.icon), ("image", &link.image)] {
        if let Some(s) = v {
            f.push((name.to_string(), Json::Str(s.clone())));
        }
    }
    Json::Obj(f)
}

/// A link value read back from an intent parameter. The address is checked; icon and image must be web addresses too
/// or they are dropped.
pub fn link_from_value(v: &Json) -> Result<LinkMetadata, LinkError> {
    let url = checked_url(v.get("url").and_then(Json::as_str).ok_or(LinkError::InvalidUrl)?)?;
    let web = |name: &str| v.get(name).and_then(Json::as_str).and_then(|s| checked_url(s).ok());
    let title = v.get("title").and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    Ok(LinkMetadata { url, title, icon: web("icon"), image: web("image") })
}

/// The result snippet for a link: its title, with the address and host as rows.
pub fn link_snippet(link: &LinkMetadata) -> Option<Snippet> {
    let entity = link_entity(link)?;
    Some(Snippet::new(&entity.title).row("Address", &entity.id).row("Site", entity.subtitle.as_deref().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> LinkMetadata {
        LinkMetadata {
            url: "https://news.example.org:8443/a/b?x=1#top".into(),
            title: Some("  Headline  ".into()),
            icon: Some("https://news.example.org/favicon.png".into()),
            image: None,
        }
    }

    #[test]
    fn a_link_is_an_entity_titled_by_the_page_and_subtitled_by_the_host() {
        let e = link_entity(&page()).expect("a web link");
        assert_eq!((e.ty.as_str(), e.title.as_str()), (LINK_TYPE, "Headline"));
        assert_eq!(e.subtitle.as_deref(), Some("news.example.org"));
        assert_eq!(e.id, "https://news.example.org:8443/a/b?x=1#top");
    }

    #[test]
    fn a_link_without_a_title_is_titled_by_its_host() {
        let l = LinkMetadata { url: "http://user@example.com/path".into(), ..Default::default() };
        assert_eq!(link_entity(&l).unwrap().title, "example.com");
    }

    #[test]
    fn an_address_that_is_not_a_web_address_has_no_entity() {
        for bad in ["javascript:alert(1)", "data:text/html,x", "ftp://example.com/", "https://", ""] {
            let l = LinkMetadata { url: bad.into(), ..Default::default() };
            assert!(link_entity(&l).is_none(), "{bad}");
            assert!(link_snippet(&l).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_link_value_round_trips_and_drops_unsafe_pictures() {
        let l = page();
        let back = link_from_value(&link_value(&l)).expect("reads back");
        assert_eq!(back.url, l.url);
        assert_eq!(back.title.as_deref(), Some("Headline"));
        assert_eq!(back.icon, l.icon);
        let hostile = Json::Obj(vec![
            ("url".into(), Json::Str("https://example.com/".into())),
            ("image".into(), Json::Str("javascript:x".into())),
        ]);
        assert_eq!(link_from_value(&hostile).unwrap().image, None);
    }

    #[test]
    fn a_link_value_with_a_bad_or_missing_address_is_refused() {
        assert_eq!(link_from_value(&Json::Obj(vec![])), Err(LinkError::InvalidUrl));
        let js = Json::Obj(vec![("url".into(), Json::Str("javascript:x".into()))]);
        assert!(link_from_value(&js).is_err());
    }

    #[test]
    fn the_snippet_shows_the_address_and_the_site() {
        let s = link_snippet(&page()).expect("snippet");
        assert_eq!(s.title, "Headline");
        assert_eq!(s.rows.len(), 2);
    }
}
