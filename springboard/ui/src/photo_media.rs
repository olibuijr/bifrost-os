//! Media kinds of library assets and the pairing of a still with its movie. The kind of a file comes from its type
//! (`uttype`), so the same extension rules as the file manager decide it. A still and a short movie form one moving
//! photo when both carry the same content identifier, the key the capture writes into each file. The pairing only
//! reads identifiers the caller already has; no file is opened here.

use crate::uttype;
use std::collections::BTreeMap;

/// The general type of an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl Kind {
    pub fn is_video(self) -> bool {
        self == Kind::Video
    }
}

/// The kind of a file, from its name. A name whose type is not an image, a movie or audio is `Unknown`.
pub fn kind_for_name(name: &str) -> Kind {
    let Some(id) = uttype::type_for_extension(name) else {
        return Kind::Unknown;
    };
    if uttype::conforms(id, "public.image") {
        Kind::Image
    } else if uttype::conforms(id, "public.movie") {
        Kind::Video
    } else if uttype::conforms(id, "public.audio") {
        Kind::Audio
    } else {
        Kind::Unknown
    }
}

/// One file in a listing, with the content identifier read from it (if it has one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub content_id: Option<String>,
}

/// A still image and its movie, as indexes into the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub still: usize,
    pub movie: usize,
}

/// Pairs stills with movies that share a content identifier. Within one identifier the first still pairs with the
/// first movie, the second with the second, and so on; extra files stay single. Items without an identifier never
/// pair. The result is ordered by the index of the still.
pub fn pair_moving(items: &[Item]) -> Vec<Pair> {
    let mut groups: BTreeMap<&str, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        let Some(id) = item.content_id.as_deref() else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let entry = groups.entry(id).or_default();
        match kind_for_name(&item.name) {
            Kind::Image => entry.0.push(i),
            Kind::Video => entry.1.push(i),
            _ => {}
        }
    }
    let mut pairs: Vec<Pair> = groups
        .values()
        .flat_map(|(stills, movies)| stills.iter().zip(movies.iter()).map(|(&still, &movie)| Pair { still, movie }))
        .collect();
    pairs.sort_by_key(|p| p.still);
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, id: Option<&str>) -> Item {
        Item { name: name.to_string(), content_id: id.map(|s| s.to_string()) }
    }

    #[test]
    fn the_kind_follows_the_file_type() {
        assert_eq!(kind_for_name("holiday.jpg"), Kind::Image);
        assert_eq!(kind_for_name("CLIP.MOV"), Kind::Video);
        assert_eq!(kind_for_name("clip.mp4"), Kind::Video);
        assert_eq!(kind_for_name("song.m4a"), Kind::Audio);
        assert_eq!(kind_for_name("notes.txt"), Kind::Unknown);
        assert_eq!(kind_for_name("no-extension"), Kind::Unknown);
        assert!(kind_for_name("a.mov").is_video());
    }

    #[test]
    fn a_still_and_a_movie_with_one_identifier_pair_up() {
        let items = [item("IMG_1.jpg", Some("X")), item("IMG_1.mov", Some("X"))];
        assert_eq!(pair_moving(&items), vec![Pair { still: 0, movie: 1 }]);
    }

    #[test]
    fn different_identifiers_do_not_pair() {
        let items = [item("a.jpg", Some("1")), item("b.mov", Some("2"))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn files_without_an_identifier_never_pair() {
        let items = [item("a.jpg", None), item("b.mov", None), item("c.mov", Some(""))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn two_stills_with_no_movie_stay_single() {
        let items = [item("a.jpg", Some("X")), item("b.jpg", Some("X"))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn pairs_are_ordered_by_the_still_and_extra_stills_stay_single() {
        // Stills 0 and 1 share X with movies 2 and 3: the first still takes the first movie.
        let items = [
            item("s1.jpg", Some("X")),
            item("s2.jpg", Some("X")),
            item("m1.mov", Some("X")),
            item("m2.mov", Some("X")),
        ];
        assert_eq!(pair_moving(&items), vec![Pair { still: 0, movie: 2 }, Pair { still: 1, movie: 3 }]);
        // With one movie, the second still stays single.
        let one_movie = [item("s1.jpg", Some("X")), item("s2.jpg", Some("X")), item("m1.mov", Some("X"))];
        assert_eq!(pair_moving(&one_movie), vec![Pair { still: 0, movie: 2 }]);
    }

    #[test]
    fn a_movie_listed_before_its_still_still_pairs() {
        let items = [item("m.mov", Some("Y")), item("s.jpg", Some("Y"))];
        assert_eq!(pair_moving(&items), vec![Pair { still: 1, movie: 0 }]);
    }
}
