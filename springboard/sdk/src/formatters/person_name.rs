//! Person names: the equivalent of the reference SDK's name components and name formatter. Western
//! order only (given name first); family-first scripts, phonetic output and attributed output are not
//! matched.

/// The parts of a person's name. Every part is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PersonNameComponents {
    pub name_prefix: Option<String>,
    pub given_name: Option<String>,
    pub middle_name: Option<String>,
    pub family_name: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
}

/// How much of the name is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PersonNameStyle {
    Short,
    #[default]
    Medium,
    Long,
    Abbreviated,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PersonNameFormatter {
    pub style: PersonNameStyle,
}

const PREFIXES: [&str; 5] = ["Dr.", "Mr.", "Mrs.", "Ms.", "Prof."];
const SUFFIXES: [&str; 6] = ["Jr.", "Sr.", "II", "III", "PhD", "Ph.D."];

fn part(p: &Option<String>) -> Option<&str> {
    p.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn initial(p: &Option<String>) -> Option<String> {
    part(p).and_then(|s| s.chars().next()).map(|c| c.to_uppercase().collect())
}

impl PersonNameFormatter {
    pub fn new(style: PersonNameStyle) -> Self {
        Self { style }
    }

    /// Formats the components in the configured style, skipping empty parts.
    pub fn string(&self, c: &PersonNameComponents) -> String {
        let parts: Vec<String> = match self.style {
            PersonNameStyle::Long => [&c.name_prefix, &c.given_name, &c.middle_name, &c.family_name, &c.name_suffix]
                .into_iter()
                .filter_map(|p| part(p).map(String::from))
                .collect(),
            PersonNameStyle::Medium => [&c.given_name, &c.family_name].into_iter().filter_map(|p| part(p).map(String::from)).collect(),
            PersonNameStyle::Short => part(&c.nickname).or(part(&c.given_name)).map(String::from).into_iter().collect(),
            PersonNameStyle::Abbreviated => {
                return [&c.given_name, &c.family_name].into_iter().filter_map(initial).collect::<String>();
            }
        };
        parts.join(" ")
    }

    /// Splits a name on whitespace: a known prefix and suffix are taken off the ends, the first
    /// remaining word is the given name, the last the family name and the rest the middle name.
    pub fn components(&self, s: &str) -> PersonNameComponents {
        let mut words: Vec<&str> = s.split_whitespace().collect();
        let mut c = PersonNameComponents::default();
        if words.first().is_some_and(|w| PREFIXES.contains(w)) {
            c.name_prefix = Some(words.remove(0).to_string());
        }
        if words.len() > 1 && words.last().is_some_and(|w| SUFFIXES.contains(w)) {
            c.name_suffix = words.pop().map(String::from);
        }
        match words.len() {
            0 => {}
            1 => c.given_name = Some(words[0].to_string()),
            n => {
                c.given_name = Some(words[0].to_string());
                c.family_name = Some(words[n - 1].to_string());
                if n > 2 {
                    c.middle_name = Some(words[1..n - 1].join(" "));
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> PersonNameComponents {
        PersonNameComponents {
            name_prefix: Some("Dr.".into()),
            given_name: Some("Jane".into()),
            middle_name: Some("Q".into()),
            family_name: Some("Austen".into()),
            name_suffix: Some("Jr.".into()),
            nickname: Some("Janie".into()),
        }
    }

    fn fmt(style: PersonNameStyle, c: &PersonNameComponents) -> String {
        PersonNameFormatter::new(style).string(c)
    }

    #[test]
    fn each_style() {
        let c = full();
        assert_eq!(fmt(PersonNameStyle::Long, &c), "Dr. Jane Q Austen Jr.");
        assert_eq!(fmt(PersonNameStyle::Medium, &c), "Jane Austen");
        assert_eq!(fmt(PersonNameStyle::Short, &c), "Janie");
        assert_eq!(fmt(PersonNameStyle::Abbreviated, &c), "JA");
        assert_eq!(PersonNameStyle::default(), PersonNameStyle::Medium);
    }

    #[test]
    fn missing_parts_leave_no_gaps() {
        let c = PersonNameComponents { given_name: Some("Jane".into()), name_suffix: Some("III".into()), middle_name: Some(" ".into()), ..Default::default() };
        assert_eq!(fmt(PersonNameStyle::Long, &c), "Jane III");
        assert_eq!(fmt(PersonNameStyle::Medium, &c), "Jane");
        assert_eq!(fmt(PersonNameStyle::Abbreviated, &c), "J");
    }

    #[test]
    fn short_falls_back_to_given_name() {
        let mut c = full();
        c.nickname = None;
        assert_eq!(fmt(PersonNameStyle::Short, &c), "Jane");
        assert_eq!(fmt(PersonNameStyle::Short, &PersonNameComponents::default()), "");
    }

    #[test]
    fn long_name_round_trips() {
        let mut c = full();
        c.nickname = None;
        let f = PersonNameFormatter::new(PersonNameStyle::Long);
        let s = f.string(&c);
        assert_eq!(f.components(&s), c);
    }

    #[test]
    fn parses_middle_names_and_single_words() {
        let f = PersonNameFormatter::default();
        let c = f.components("  Mary   Ann  Lee ");
        assert_eq!(c.given_name.as_deref(), Some("Mary"));
        assert_eq!(c.middle_name.as_deref(), Some("Ann"));
        assert_eq!(c.family_name.as_deref(), Some("Lee"));
        assert_eq!(f.components("Cher").given_name.as_deref(), Some("Cher"));
    }

    #[test]
    fn empty_input_is_default() {
        let f = PersonNameFormatter::default();
        assert_eq!(f.components(""), PersonNameComponents::default());
        assert_eq!(f.components("   "), PersonNameComponents::default());
    }
}
