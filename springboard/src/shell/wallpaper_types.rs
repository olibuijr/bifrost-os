//! Typed wallpaper kinds, and the wallpapers an app provides through its manifest.
//!
//! A *still* is one picture. A *day* is a set of pictures that changes through the day: the day is cut into equal
//! slices, one per picture, by `wallpaper_schedule`. An app declares its wallpapers in `manifest.toml`:
//!
//! ```toml
//! wallpaper_kind = "day"
//! wallpapers = ["wallpapers/dawn.png", "wallpapers/noon.png", "wallpapers/dusk.png"]
//! ```
//!
//! The store checks them against the package when it installs the app (`store::install::validate`), so a
//! provided wallpaper always names a picture inside the package. `listed` is the list Settings reads for the
//! installed apps' wallpapers, and `find` maps a listed id back to its wallpaper; the Settings page and the
//! shell that draws the chosen wallpaper are still separate work.

use super::wallpaper_schedule;

/// The most pictures one day wallpaper may hold.
pub const MAX_PICTURES: usize = 24;
/// Picture types a package may ship.
pub const PICTURE_TYPES: [&str; 3] = [".png", ".jpg", ".jpeg"];

/// What a wallpaper is: one picture, or a day of pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Still,
    Day,
}

impl Kind {
    /// The manifest word for the kind: `still` or `day`.
    pub fn parse(word: &str) -> Option<Kind> {
        match word {
            "still" => Some(Kind::Still),
            "day" => Some(Kind::Day),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Kind::Still => "still",
            Kind::Day => "day",
        }
    }
}

/// A wallpaper an app provides: the app's id, its kind and its pictures, as package-relative paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provided {
    pub app: String,
    pub kind: Kind,
    pub images: Vec<String>,
}

/// True for a package-relative picture path: no absolute start, no empty, `.` or `..` parts, and a picture type.
pub fn picture_path_ok(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains('\0')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
        && PICTURE_TYPES.iter().any(|t| lower.ends_with(t))
}

/// The wallpaper a manifest declares, checked. Neither field set means the app provides none (`Ok(None)`).
pub fn from_manifest(app: &str, kind: &str, images: &[String]) -> Result<Option<Provided>, String> {
    if kind.is_empty() && images.is_empty() {
        return Ok(None);
    }
    let kind = Kind::parse(kind).ok_or_else(|| format!("wallpaper_kind {kind:?} is not \"still\" or \"day\""))?;
    let shape_ok = match kind {
        Kind::Still => images.len() == 1,
        Kind::Day => images.len() >= 2 && images.len() <= MAX_PICTURES,
    };
    if !shape_ok {
        return Err(match kind {
            Kind::Still => "a still wallpaper is one picture in wallpapers = [...]".to_string(),
            Kind::Day => format!("a day wallpaper needs two to {MAX_PICTURES} pictures in wallpapers = [...]"),
        });
    }
    if let Some(bad) = images.iter().find(|p| !picture_path_ok(p)) {
        return Err(format!("wallpaper {bad:?} is not a picture (png, jpg or jpeg) inside the package"));
    }
    Ok(Some(Provided { app: app.to_string(), kind, images: images.to_vec() }))
}

/// One installed app's wallpaper as Settings lists it: a stable id (`app:<app id>`), the app, the kind and the
/// package-relative picture that stands for it (the first one of a day).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub app: String,
    pub kind: Kind,
    pub picture: String,
}

/// The wallpapers the installed apps provide, in the order given. An app declares one wallpaper, so an app id
/// gives one entry.
pub fn listed(provided: &[Provided]) -> Vec<Listed> {
    provided
        .iter()
        .filter_map(|p| {
            let picture = p.images.first()?.clone();
            Some(Listed { id: format!("app:{}", p.app), app: p.app.clone(), kind: p.kind, picture })
        })
        .collect()
}

/// The provided wallpaper a listed id names, if an installed app still provides it.
pub fn find<'a>(provided: &'a [Provided], id: &str) -> Option<&'a Provided> {
    provided.iter().find(|p| format!("app:{}", p.app) == id)
}

impl Provided {
    /// The picture shown at `minute` of the day (0..1440): the only picture of a still, or the day's slice.
    pub fn picture_at(&self, minute: u32) -> Option<&str> {
        match self.kind {
            Kind::Still => self.images.first().map(String::as_str),
            Kind::Day => wallpaper_schedule::still_at(&self.images, minute),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pics(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn kinds_parse_from_their_manifest_words() {
        assert_eq!(Kind::parse("still"), Some(Kind::Still));
        assert_eq!(Kind::parse("day"), Some(Kind::Day));
        assert_eq!(Kind::parse("video"), None);
        assert_eq!(Kind::Day.word(), "day");
    }

    #[test]
    fn an_app_without_a_wallpaper_provides_none() {
        assert_eq!(from_manifest("is.x.app", "", &[]), Ok(None));
    }

    #[test]
    fn a_still_is_one_picture_and_a_day_is_two_or_more() {
        assert!(from_manifest("is.x.app", "still", &pics(&["wallpapers/a.png"])).unwrap().is_some());
        assert!(from_manifest("is.x.app", "still", &pics(&["a.png", "b.png"])).is_err());
        assert!(from_manifest("is.x.app", "day", &pics(&["a.png"])).is_err());
        assert!(from_manifest("is.x.app", "day", &pics(&["a.png", "b.png"])).is_ok());
        let too_many: Vec<String> = (0..=MAX_PICTURES).map(|i| format!("p{i}.png")).collect();
        assert!(from_manifest("is.x.app", "day", &too_many).is_err());
    }

    #[test]
    fn a_kind_must_be_named_and_known() {
        assert!(from_manifest("is.x.app", "", &pics(&["a.png"])).is_err());
        assert!(from_manifest("is.x.app", "movie", &pics(&["a.mov"])).is_err());
    }

    #[test]
    fn pictures_stay_inside_the_package_and_are_pictures() {
        assert!(picture_path_ok("wallpapers/dawn.PNG"));
        for bad in ["", "/etc/a.png", "../a.png", "w/../a.png", "w//a.png", "a.mov", "a\\b.png", "a.png\0"] {
            assert!(!picture_path_ok(bad), "{bad:?}");
        }
        assert!(from_manifest("is.x.app", "still", &pics(&["../a.png"])).is_err());
    }

    #[test]
    fn installed_apps_list_one_wallpaper_each_in_install_order() {
        let dawn = from_manifest("is.x.dawn", "day", &pics(&["dawn.png", "dusk.png"])).unwrap().unwrap();
        let still = from_manifest("is.y.still", "still", &pics(&["wallpapers/s.jpg"])).unwrap().unwrap();
        let provided = [dawn, still];
        let list = listed(&provided);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "app:is.x.dawn");
        assert_eq!(list[0].picture, "dawn.png", "a day is listed by its first picture");
        assert_eq!(list[1].kind, Kind::Still);
        assert_eq!(list[1].picture, "wallpapers/s.jpg");
    }

    #[test]
    fn a_listed_wallpaper_resolves_to_its_app_and_a_gone_app_does_not() {
        let still = from_manifest("is.y.still", "still", &pics(&["s.png"])).unwrap().unwrap();
        let provided = [still];
        assert_eq!(find(&provided, "app:is.y.still").map(|p| p.app.as_str()), Some("is.y.still"));
        assert!(find(&provided, "app:is.z.removed").is_none());
        assert!(find(&provided, "is.y.still").is_none(), "only the listed id form resolves");
        assert!(listed(&[]).is_empty());
    }

    #[test]
    fn a_day_changes_through_the_day_and_a_still_does_not() {
        let day = from_manifest("is.x.app", "day", &pics(&["dawn.png", "noon.png", "dusk.png", "night.png"])).unwrap().unwrap();
        assert_eq!(day.picture_at(0), Some("dawn.png"));
        assert_eq!(day.picture_at(720), Some("dusk.png"), "the third slice starts at 12:00 of four");
        assert_eq!(day.picture_at(1439), Some("night.png"));
        let still = from_manifest("is.x.app", "still", &pics(&["only.png"])).unwrap().unwrap();
        assert_eq!(still.picture_at(0), Some("only.png"));
        assert_eq!(still.picture_at(900), Some("only.png"));
    }
}
