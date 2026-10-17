//! Smart groups: a rule over the attributes of media objects that computes the items of a group, such as "every
//! favourite song in the Rock genre" or "every song rated 4 or more". A [`Rule`] is a tree of attribute tests joined
//! with [`Rule::All`] and [`Rule::Any`]; [`items`] runs it over the library's objects and returns their identifiers in
//! the order the objects are given. An attribute that is not set fails its test, so a missing rating never matches.
//! Text tests ignore case. A containment test with an empty needle matches nothing, so an empty search field shows no
//! items instead of the whole library. Gaps: no date ranges, no nested grouping of the rule text, and no persistence of
//! a rule; the Music app still builds its albums and artists from tags.

use crate::media_attributes::{AttributeKey, AttributeValue, MediaObject};

/// One test over a media object's attributes, or a join of tests.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// The attribute holds exactly this value.
    Equals(AttributeKey, AttributeValue),
    /// The text attribute contains this text, ignoring case. An empty needle matches nothing.
    Contains(AttributeKey, String),
    /// The integer attribute is at least this value.
    AtLeast(AttributeKey, i64),
    /// The flag attribute is set to true.
    IsSet(AttributeKey),
    /// Every rule holds. An empty list holds.
    All(Vec<Rule>),
    /// At least one rule holds. An empty list does not hold.
    Any(Vec<Rule>),
}

/// Whether `object` satisfies `rule`.
pub fn matches(rule: &Rule, object: &MediaObject) -> bool {
    let attributes = object.attributes();
    match rule {
        Rule::Equals(key, value) => attributes.get(*key) == Some(value),
        Rule::Contains(key, needle) => {
            let needle = needle.trim().to_lowercase();
            !needle.is_empty() && attributes.text(*key).is_some_and(|text| text.to_lowercase().contains(&needle))
        }
        Rule::AtLeast(key, least) => matches!(attributes.get(*key), Some(AttributeValue::Integer(n)) if n >= least),
        Rule::IsSet(key) => matches!(attributes.get(*key), Some(AttributeValue::Flag(true))),
        Rule::All(rules) => rules.iter().all(|r| matches(r, object)),
        Rule::Any(rules) => rules.iter().any(|r| matches(r, object)),
    }
}

/// The identifiers of the objects that satisfy `rule`, in the order of `objects`.
pub fn items(rule: &Rule, objects: &[MediaObject]) -> Vec<String> {
    objects.iter().filter(|o| matches(rule, o)).map(|o| o.id().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, title: &str, genre: &str, rating: i64, favourite: bool) -> MediaObject {
        let mut object = MediaObject::new(id).unwrap();
        object.set(AttributeKey::Title, AttributeValue::Text(title.to_string())).unwrap();
        object.set(AttributeKey::Genre, AttributeValue::Text(genre.to_string())).unwrap();
        object.set(AttributeKey::Rating, AttributeValue::Integer(rating)).unwrap();
        object.set(AttributeKey::Favourite, AttributeValue::Flag(favourite)).unwrap();
        object
    }

    fn library() -> Vec<MediaObject> {
        vec![
            song("a", "Night Drive", "Rock", 5, true),
            song("b", "Quiet Rain", "Ambient", 3, false),
            song("c", "Rock Anthem", "rock", 4, true),
            MediaObject::new("d").unwrap(),
        ]
    }

    #[test]
    fn an_equality_test_compares_the_value_kind_and_content() {
        let rule = Rule::Equals(AttributeKey::Genre, AttributeValue::Text("Ambient".to_string()));
        assert_eq!(items(&rule, &library()), vec!["b".to_string()]);
    }

    #[test]
    fn containment_ignores_case_and_an_empty_needle_matches_nothing() {
        let rock = Rule::Contains(AttributeKey::Genre, "ROCK".to_string());
        assert_eq!(items(&rock, &library()), vec!["a".to_string(), "c".to_string()]);
        let empty = Rule::Contains(AttributeKey::Title, "  ".to_string());
        assert!(items(&empty, &library()).is_empty());
    }

    #[test]
    fn at_least_reads_integers_and_an_unset_attribute_never_matches() {
        let rule = Rule::AtLeast(AttributeKey::Rating, 4);
        assert_eq!(items(&rule, &library()), vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn is_set_needs_a_true_flag() {
        let rule = Rule::IsSet(AttributeKey::Favourite);
        assert_eq!(items(&rule, &library()), vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn all_and_any_join_the_tests() {
        let fav_rock = Rule::All(vec![
            Rule::IsSet(AttributeKey::Favourite),
            Rule::Contains(AttributeKey::Genre, "rock".to_string()),
        ]);
        assert_eq!(items(&fav_rock, &library()), vec!["a".to_string(), "c".to_string()]);

        let either = Rule::Any(vec![
            Rule::AtLeast(AttributeKey::Rating, 5),
            Rule::Contains(AttributeKey::Title, "quiet".to_string()),
        ]);
        assert_eq!(items(&either, &library()), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn empty_joins_follow_iterator_rules() {
        assert!(matches(&Rule::All(vec![]), &MediaObject::new("x").unwrap()));
        assert!(!matches(&Rule::Any(vec![]), &MediaObject::new("x").unwrap()));
    }
}
