//! Record search with compound elements: a tree of comparisons on one property (or on any property) joined by a
//! conjunction (all children match) or a disjunction (any child matches). Comparisons ignore case. The model does not
//! own the records; the caller passes a function that returns a record's values for a property.

/// How a text value is compared with the query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Contains,
    Equals,
    BeginsWith,
    EndsWith,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Element {
    /// One comparison. An empty `property` means any property of the record.
    Match { property: String, comparison: Comparison, text: String },
    /// Every child must match. An empty conjunction matches everything.
    All(Vec<Element>),
    /// At least one child must match. An empty disjunction matches nothing.
    Any(Vec<Element>),
}

impl Element {
    /// A match on any property, the single text query a plain search field makes.
    pub fn any_property(comparison: Comparison, text: &str) -> Element {
        Element::Match { property: String::new(), comparison, text: text.to_string() }
    }

    pub fn on(property: &str, comparison: Comparison, text: &str) -> Element {
        Element::Match { property: property.to_string(), comparison, text: text.to_string() }
    }
}

fn compare(value: &str, comparison: Comparison, query: &str) -> bool {
    let v = value.to_lowercase();
    let q = query.to_lowercase();
    match comparison {
        Comparison::Contains => v.contains(&q),
        Comparison::Equals => v == q,
        Comparison::BeginsWith => v.starts_with(&q),
        Comparison::EndsWith => v.ends_with(&q),
    }
}

/// Whether one record matches `element`. `values(property)` returns the record's values for a property. A match
/// with an empty property name (`any_property`) tries each name in `properties` in turn.
pub fn matches<F>(element: &Element, properties: &[&str], values: &F) -> bool
where
    F: Fn(&str) -> Vec<String>,
{
    match element {
        Element::Match { property, comparison, text } => {
            if property.is_empty() {
                properties.iter().any(|p| values(p).iter().any(|v| compare(v, *comparison, text)))
            } else {
                values(property).iter().any(|v| compare(v, *comparison, text))
            }
        }
        Element::All(children) => children.iter().all(|c| matches(c, properties, values)),
        Element::Any(children) => children.iter().any(|c| matches(c, properties, values)),
    }
}

/// Indices of the records that match `element`, in their original order.
pub fn filter<T, F>(element: &Element, records: &[T], properties: &[&str], values: F) -> Vec<usize>
where
    F: Fn(&T, &str) -> Vec<String>,
{
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| matches(element, properties, &|p: &str| values(r, p)))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Person {
        name: &'static str,
        email: &'static str,
        city: &'static str,
    }

    const PEOPLE: [Person; 3] = [
        Person { name: "Anna Lind", email: "anna@example.is", city: "Reykjavík" },
        Person { name: "Bjarni Ólafsson", email: "bjarni@example.is", city: "Akureyri" },
        Person { name: "Carla Lind", email: "carla@example.com", city: "Reykjavík" },
    ];

    fn value(p: &Person, prop: &str) -> Vec<String> {
        match prop {
            "name" => vec![p.name.to_string()],
            "email" => vec![p.email.to_string()],
            "city" => vec![p.city.to_string()],
            _ => vec![],
        }
    }

    const PROPS: [&str; 3] = ["name", "email", "city"];

    #[test]
    fn a_single_property_comparison_selects_matching_records_in_order() {
        let e = Element::on("name", Comparison::Equals, "anna lind");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![0], "equals ignores case");
        let e = Element::on("city", Comparison::Equals, "Reykjavík");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![0, 2]);
    }

    #[test]
    fn contains_begins_with_and_ends_with() {
        let e = Element::on("email", Comparison::BeginsWith, "b");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![1]);
        let e = Element::on("email", Comparison::EndsWith, ".is");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![0, 1]);
        let e = Element::on("name", Comparison::Contains, "LIND");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![0, 2]);
    }

    #[test]
    fn a_conjunction_needs_every_child_and_a_disjunction_needs_one() {
        let lind_in_rvk = Element::All(vec![
            Element::on("name", Comparison::Contains, "lind"),
            Element::on("city", Comparison::Equals, "Reykjavík"),
        ]);
        assert_eq!(filter(&lind_in_rvk, &PEOPLE, &PROPS, value), vec![0, 2]);
        let an_akureyri_or_carla = Element::Any(vec![
            Element::on("city", Comparison::Equals, "Akureyri"),
            Element::on("name", Comparison::Equals, "Carla Lind"),
        ]);
        assert_eq!(filter(&an_akureyri_or_carla, &PEOPLE, &PROPS, value), vec![1, 2]);
    }

    #[test]
    fn an_any_property_match_looks_at_every_listed_property() {
        let e = Element::any_property(Comparison::Contains, "example.is");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![0, 1]);
        let e = Element::any_property(Comparison::Contains, "akur");
        assert_eq!(filter(&e, &PEOPLE, &["email"], value), Vec::<usize>::new(), "only the listed properties count");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), vec![1]);
    }

    #[test]
    fn empty_conjunctions_and_disjunctions_have_fixed_results() {
        assert_eq!(filter(&Element::All(vec![]), &PEOPLE, &PROPS, value), vec![0, 1, 2]);
        assert_eq!(filter(&Element::Any(vec![]), &PEOPLE, &PROPS, value), Vec::<usize>::new());
    }

    #[test]
    fn a_property_with_no_values_never_matches_a_comparison() {
        let e = Element::on("phone", Comparison::Contains, "");
        assert_eq!(filter(&e, &PEOPLE, &PROPS, value), Vec::<usize>::new());
    }
}
