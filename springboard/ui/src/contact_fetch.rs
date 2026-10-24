//! Partial records: a fetch names the fields it needs (its keys) and gets back a record that holds only those fields.
//! Asking for a field the fetch did not name is an error, so a caller that left out a key fails loudly instead of
//! reading an empty value. Pure: the store keeps full records and calls `project` for each fetch.

use std::collections::{BTreeMap, BTreeSet};

/// One field of a contact that a fetch can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    GivenName,
    FamilyName,
    /// The middle names (`CNContactMiddleNameKey`), between the given and family name.
    MiddleName,
    /// The suffix after the name, such as `Jr.` or `III` (`CNContactNameSuffixKey`).
    NameSuffix,
    Organization,
    Phones,
    Emails,
    Address,
    Note,
    Urls,
    Birthday,
    Dates,
    /// The title before the name, such as `Dr.` (`CNContactNamePrefixKey`).
    NamePrefix,
    /// `CNContactNicknameKey`.
    Nickname,
    /// `CNContactJobTitleKey`.
    JobTitle,
    /// `CNContactDepartmentNameKey`.
    Department,
    /// Related people (`CNContactRelationsKey`): labeled names, such as a `spouse` or a `manager`.
    Relations,
    /// Social profiles (`CNContactSocialProfilesKey`): labeled [`Profile`]s with a service, user name and URL.
    SocialProfiles,
    /// Instant message addresses (`CNContactInstantMessageAddressesKey`): labeled [`Profile`]s with a service and user
    /// name.
    InstantMessages,
}

impl Key {
    /// Every key, in declaration order.
    pub const ALL: [Key; 19] = [
        Key::GivenName,
        Key::FamilyName,
        Key::MiddleName,
        Key::NameSuffix,
        Key::Organization,
        Key::Phones,
        Key::Emails,
        Key::Address,
        Key::Note,
        Key::Urls,
        Key::Birthday,
        Key::Dates,
        Key::NamePrefix,
        Key::Nickname,
        Key::JobTitle,
        Key::Department,
        Key::Relations,
        Key::SocialProfiles,
        Key::InstantMessages,
    ];
}

impl Key {
    /// The wire name of the key, as the `contacts` service and the SDK spell it.
    pub fn name(self) -> &'static str {
        match self {
            Key::GivenName => "given_name",
            Key::FamilyName => "family_name",
            Key::MiddleName => "middle_name",
            Key::NameSuffix => "name_suffix",
            Key::Organization => "organization",
            Key::Phones => "phones",
            Key::Emails => "emails",
            Key::Address => "address",
            Key::Note => "note",
            Key::Urls => "urls",
            Key::Birthday => "birthday",
            Key::Dates => "dates",
            Key::NamePrefix => "name_prefix",
            Key::Nickname => "nickname",
            Key::JobTitle => "job_title",
            Key::Department => "department",
            Key::Relations => "relations",
            Key::SocialProfiles => "social_profiles",
            Key::InstantMessages => "instant_messages",
        }
    }

    /// The key a wire name stands for, or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.iter().copied().find(|key| key.name() == name)
    }
}

/// A field value: text, or a list (phone numbers, email addresses, postal addresses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    /// Labeled multi-values (`CNLabeledValue`): phone numbers, email addresses, URLs.
    Labeled(Vec<LabeledValue>),
    /// A date given as components (`birthday`).
    Date(DateComponents),
    /// Labeled dates (`dates`).
    Dates(Vec<(String, DateComponents)>),
    /// Labeled accounts on a service (`social_profiles`, `instant_messages`).
    Profiles(Vec<Profile>),
}

/// One account of a person on a service (`CNSocialProfile`, `CNInstantMessageAddress`): the label it is filed under,
/// the service name, the user name, and for a social profile its URL and the service's own user id. An instant message
/// address leaves `url` and `user_identifier` empty. The identifier is stable within the contact, as for
/// [`LabeledValue`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub label: String,
    pub service: String,
    pub username: String,
    pub url: String,
    pub user_identifier: String,
    pub identifier: String,
}

impl Profile {
    /// A profile on `service` for `username`, with no URL or user id.
    pub fn new(label: &str, service: &str, username: &str) -> Self {
        Profile { label: label.into(), service: service.into(), username: username.into(), ..Profile::default() }
    }

    /// A profile with no user name, URL or user id says nothing about the person.
    pub fn is_blank(&self) -> bool {
        [&self.username, &self.url, &self.user_identifier].iter().all(|v| v.trim().is_empty())
    }

    /// The profiles with identifiers `1`, `2`, ... in order.
    pub fn numbered(mut items: Vec<Profile>) -> Vec<Profile> {
        for (i, item) in items.iter_mut().enumerate() {
            item.identifier = (i + 1).to_string();
        }
        items
    }
}

/// `CNLabelHome`, `CNLabelWork`, `CNLabelOther` and the phone labels. The labels are plain names, so a label the
/// person typed is stored the same way.
pub const LABEL_HOME: &str = "home";
pub const LABEL_WORK: &str = "work";
pub const LABEL_OTHER: &str = "other";
/// `CNLabelPhoneNumberMobile`.
pub const LABEL_PHONE_MOBILE: &str = "mobile";
/// `CNLabelPhoneNumberMain`.
pub const LABEL_PHONE_MAIN: &str = "main";
/// `CNLabelPhoneNumberHomeFax`, `CNLabelPhoneNumberWorkFax`, `CNLabelPhoneNumberOtherFax`, `CNLabelPhoneNumberPager`.
pub const LABEL_PHONE_HOME_FAX: &str = "home fax";
pub const LABEL_PHONE_WORK_FAX: &str = "work fax";
pub const LABEL_PHONE_OTHER_FAX: &str = "other fax";
pub const LABEL_PHONE_PAGER: &str = "pager";
/// `CNLabelSchool`.
pub const LABEL_SCHOOL: &str = "school";
/// `CNLabelURLAddressHomePage`.
pub const LABEL_URL_HOME_PAGE: &str = "homepage";
/// `CNLabelDateAnniversary`.
pub const LABEL_DATE_ANNIVERSARY: &str = "anniversary";

/// The relation labels (`CNLabelContactRelation*`), the common ones of the reference set. Gendered and family-specific
/// variants beyond these are kept as typed text.
pub const RELATION_LABELS: [&str; 12] =
    ["father", "mother", "parent", "brother", "sister", "child", "son", "daughter", "friend", "spouse", "partner", "assistant"];
/// `CNLabelContactRelationManager`, kept apart from the family labels above.
pub const LABEL_RELATION_MANAGER: &str = "manager";

/// Every predefined label name this model knows, in the order a label menu offers them.
pub const PREDEFINED_LABELS: [&str; 13] = [
    LABEL_HOME,
    LABEL_WORK,
    LABEL_SCHOOL,
    LABEL_OTHER,
    LABEL_PHONE_MOBILE,
    LABEL_PHONE_MAIN,
    LABEL_PHONE_HOME_FAX,
    LABEL_PHONE_WORK_FAX,
    LABEL_PHONE_OTHER_FAX,
    LABEL_PHONE_PAGER,
    LABEL_URL_HOME_PAGE,
    LABEL_DATE_ANNIVERSARY,
    LABEL_RELATION_MANAGER,
];

/// Whether `label` is one of the predefined names (a relation label counts), ignoring case and surrounding spaces.
pub fn is_predefined_label(label: &str) -> bool {
    let l = label.trim().to_lowercase();
    PREDEFINED_LABELS.contains(&l.as_str()) || RELATION_LABELS.contains(&l.as_str())
}

/// The label as a person reads it (`CNLabeledValue.localizedString(forLabel:)`): a predefined name in title case
/// ("home fax" reads "Home Fax", "homepage" reads "Home Page"), and a label the person typed as they typed it. An
/// empty label reads as "Other". Only English is built: there is no string table for other languages yet.
pub fn localized_label(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return "Other".into();
    }
    if !is_predefined_label(trimmed) {
        return trimmed.to_string();
    }
    let lower = trimmed.to_lowercase();
    let spaced = if lower == LABEL_URL_HOME_PAGE { "home page".to_string() } else { lower };
    spaced
        .split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One entry of a multi-value property (`CNLabeledValue`): a label, a value and an identifier that is stable within
/// the contact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabeledValue {
    pub label: String,
    pub value: String,
    pub identifier: String,
}

impl LabeledValue {
    /// An entry whose identifier is derived from its position by [`LabeledValue::numbered`].
    pub fn new(label: &str, value: &str) -> Self {
        LabeledValue { label: label.into(), value: value.into(), identifier: String::new() }
    }

    /// The entries with identifiers `1`, `2`, ... in order, so the same list always has the same identifiers.
    pub fn numbered(mut items: Vec<LabeledValue>) -> Vec<LabeledValue> {
        for (i, item) in items.iter_mut().enumerate() {
            item.identifier = (i + 1).to_string();
        }
        items
    }
}

/// A date as components (`DateComponents`). A birthday may lack the year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateComponents {
    pub year: Option<i32>,
    pub month: u8,
    pub day: u8,
}

impl DateComponents {
    /// The components, or `None` for a month outside 1..=12 or a day outside 1..=31.
    pub fn new(year: Option<i32>, month: u8, day: u8) -> Option<Self> {
        ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(DateComponents { year, month, day })
    }

    /// `YYYY-MM-DD`, or `--MM-DD` without a year.
    pub fn to_text(self) -> String {
        match self.year {
            Some(y) => format!("{y:04}-{:02}-{:02}", self.month, self.day),
            None => format!("--{:02}-{:02}", self.month, self.day),
        }
    }

    /// Reads what [`DateComponents::to_text`] wrote.
    pub fn from_text(text: &str) -> Option<Self> {
        let (year, rest) = match text.strip_prefix("--") {
            Some(rest) => (None, rest.to_string()),
            None => {
                let (y, rest) = text.split_once('-')?;
                (Some(y.parse().ok()?), rest.to_string())
            }
        };
        let (m, d) = rest.split_once('-')?;
        DateComponents::new(year, m.parse().ok()?, d.parse().ok()?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A fetch with no keys would return nothing.
    NoKeys,
    /// The field was not part of the fetch.
    NotFetched(Key),
}

/// The keys one fetch asks for. Duplicates collapse into one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys(BTreeSet<Key>);

impl Keys {
    pub fn new(keys: &[Key]) -> Result<Keys, Error> {
        if keys.is_empty() {
            return Err(Error::NoKeys);
        }
        Ok(Keys(keys.iter().copied().collect()))
    }

    /// Every key.
    pub fn all() -> Keys {
        Keys(Key::ALL.iter().copied().collect())
    }

    pub fn contains(&self, key: Key) -> bool {
        self.0.contains(&key)
    }
}

/// A stored contact: an identifier and the fields it has. A blank value is not stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    fields: BTreeMap<Key, Value>,
}

impl Record {
    pub fn new(id: u64) -> Self {
        Record { id, fields: BTreeMap::new() }
    }

    /// The value of a field the record holds, or `None` when it is blank or was never set.
    pub fn get(&self, key: Key) -> Option<&Value> {
        self.fields.get(&key)
    }

    /// Sets a field, replacing an earlier value. A value that is all blank removes the field.
    pub fn set(&mut self, key: Key, value: Value) {
        let blank = match &value {
            Value::Text(text) => text.trim().is_empty(),
            Value::List(items) => items.iter().all(|item| item.trim().is_empty()),
            Value::Labeled(items) => items.iter().all(|item| item.value.trim().is_empty()),
            Value::Date(_) => false,
            Value::Dates(items) => items.is_empty(),
            Value::Profiles(items) => items.iter().all(Profile::is_blank),
        };
        if blank {
            self.fields.remove(&key);
        } else {
            self.fields.insert(key, value);
        }
    }
}

/// A record as a fetch returns it: its identifier and only the fields the fetch asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub id: u64,
    fetched: BTreeSet<Key>,
    values: BTreeMap<Key, Value>,
}

impl Partial {
    /// The value of a fetched field, or `None` when the record has none. Any other field is an error.
    pub fn get(&self, key: Key) -> Result<Option<&Value>, Error> {
        if !self.fetched.contains(&key) {
            return Err(Error::NotFetched(key));
        }
        Ok(self.values.get(&key))
    }

    /// The text of a fetched text field; an empty string when there is none.
    pub fn text(&self, key: Key) -> Result<&str, Error> {
        match self.get(key)? {
            Some(Value::Text(text)) => Ok(text.as_str()),
            _ => Ok(""),
        }
    }

    /// The keys this record was fetched with.
    pub fn fetched(&self) -> impl Iterator<Item = Key> + '_ {
        self.fetched.iter().copied()
    }
}

/// The partial record a fetch with `keys` returns for a stored record.
pub fn project(record: &Record, keys: &Keys) -> Partial {
    let values: BTreeMap<Key, Value> = record
        .fields
        .iter()
        .filter(|(key, _)| keys.contains(**key))
        .map(|(key, value)| (*key, value.clone()))
        .collect();
    Partial { id: record.id, fetched: keys.0.clone(), values }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        let mut r = Record::new(7);
        r.set(Key::GivenName, Value::Text("Ada".into()));
        r.set(Key::FamilyName, Value::Text("Lovelace".into()));
        r.set(Key::Emails, Value::List(vec!["ada@example.com".into()]));
        r.set(Key::Note, Value::Text("   ".into()));
        r
    }

    #[test]
    fn a_fetch_needs_at_least_one_key_and_duplicates_collapse() {
        assert_eq!(Keys::new(&[]), Err(Error::NoKeys));
        let keys = Keys::new(&[Key::Note, Key::Note]).unwrap();
        assert!(keys.contains(Key::Note));
        assert_eq!(keys.0.len(), 1);
    }

    #[test]
    fn the_partial_record_holds_only_the_fetched_fields() {
        let keys = Keys::new(&[Key::GivenName, Key::Phones]).unwrap();
        let p = project(&sample(), &keys);
        assert_eq!(p.id, 7);
        assert_eq!(p.text(Key::GivenName), Ok("Ada"));
        assert_eq!(p.get(Key::Phones), Ok(None));
        assert_eq!(p.get(Key::Emails), Err(Error::NotFetched(Key::Emails)));
        assert_eq!(p.text(Key::FamilyName), Err(Error::NotFetched(Key::FamilyName)));
    }

    #[test]
    fn a_missing_field_reads_as_absent_not_as_an_error() {
        let keys = Keys::new(&[Key::Organization]).unwrap();
        let p = project(&sample(), &keys);
        assert_eq!(p.get(Key::Organization), Ok(None));
        assert_eq!(p.text(Key::Organization), Ok(""));
    }

    #[test]
    fn blank_values_are_not_stored() {
        let p = project(&sample(), &Keys::all());
        assert_eq!(p.get(Key::Note), Ok(None));
        assert_eq!(p.fetched().count(), Key::ALL.len());
    }

    #[test]
    fn all_keys_fetch_every_stored_field() {
        let p = project(&sample(), &Keys::all());
        assert_eq!(p.text(Key::FamilyName), Ok("Lovelace"));
        assert_eq!(p.get(Key::Emails), Ok(Some(&Value::List(vec!["ada@example.com".into()]))));
        assert_eq!(p.get(Key::Address), Ok(None));
    }

    #[test]
    fn every_key_has_a_wire_name_that_reads_back() {
        for key in Key::ALL {
            assert_eq!(Key::from_name(key.name()), Some(key));
        }
        assert_eq!(Key::from_name("shoe_size"), None);
        assert_eq!(Key::GivenName.name(), "given_name");
        assert_eq!(Key::from_name("nickname"), Some(Key::Nickname));
    }

    #[test]
    fn a_record_reads_back_the_fields_it_holds() {
        let r = sample();
        assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("Lovelace".into())));
        assert_eq!(r.get(Key::Note), None);
    }

    #[test]
    fn setting_a_field_again_replaces_it() {
        let mut r = sample();
        r.set(Key::GivenName, Value::Text("Augusta".into()));
        let p = project(&r, &Keys::new(&[Key::GivenName]).unwrap());
        assert_eq!(p.text(Key::GivenName), Ok("Augusta"));
    }

    #[test]
    fn labeled_values_get_stable_identifiers_and_blank_lists_are_dropped() {
        let items = LabeledValue::numbered(vec![
            LabeledValue::new(LABEL_HOME, "a@example.com"),
            LabeledValue::new(LABEL_WORK, "b@example.com"),
        ]);
        assert_eq!(items[1].identifier, "2");
        let mut r = Record::new(1);
        r.set(Key::Emails, Value::Labeled(items));
        assert!(r.get(Key::Emails).is_some());
        r.set(Key::Emails, Value::Labeled(vec![LabeledValue::new(LABEL_HOME, " ")]));
        assert_eq!(r.get(Key::Emails), None);
    }

    #[test]
    fn date_components_validate_and_round_trip_with_and_without_a_year() {
        assert_eq!(DateComponents::new(Some(1815), 12, 10).unwrap().to_text(), "1815-12-10");
        assert_eq!(DateComponents::new(None, 3, 7).unwrap().to_text(), "--03-07");
        assert_eq!(DateComponents::from_text("--03-07"), DateComponents::new(None, 3, 7));
        assert_eq!(DateComponents::from_text("1815-12-10"), DateComponents::new(Some(1815), 12, 10));
        assert_eq!(DateComponents::new(None, 13, 1), None);
        assert_eq!(DateComponents::from_text("nonsense"), None);
    }

    #[test]
    fn middle_name_and_suffix_are_fetchable_and_default_to_absent() {
        let mut r = sample();
        r.set(Key::MiddleName, Value::Text("Augusta".into()));
        r.set(Key::NameSuffix, Value::Text("III".into()));
        let keys = Keys::new(&[Key::MiddleName, Key::NameSuffix, Key::FamilyName]).unwrap();
        let p = project(&r, &keys);
        assert_eq!(p.text(Key::MiddleName), Ok("Augusta"));
        assert_eq!(p.text(Key::NameSuffix), Ok("III"));
        assert_eq!(p.get(Key::GivenName), Err(Error::NotFetched(Key::GivenName)));
        let bare = project(&Record::new(1), &Keys::new(&[Key::MiddleName, Key::NameSuffix]).unwrap());
        assert_eq!(bare.get(Key::MiddleName), Ok(None));
        assert_eq!(Key::from_name("name_suffix"), Some(Key::NameSuffix));
        assert_eq!(Key::from_name("middle_name"), Some(Key::MiddleName));
    }

    #[test]
    fn work_and_social_fields_are_fetchable_and_blank_profiles_are_dropped() {
        let mut r = Record::new(3);
        r.set(Key::JobTitle, Value::Text("Engineer".into()));
        r.set(Key::Relations, Value::Labeled(LabeledValue::numbered(vec![LabeledValue::new("spouse", "William")])));
        r.set(Key::SocialProfiles, Value::Profiles(Profile::numbered(vec![Profile::new("", "mastodon", "@ada")])));
        r.set(Key::InstantMessages, Value::Profiles(vec![Profile::new("home", "xmpp", "  ")]));
        let p = project(&r, &Keys::new(&[Key::JobTitle, Key::SocialProfiles, Key::InstantMessages]).unwrap());
        assert_eq!(p.text(Key::JobTitle), Ok("Engineer"));
        assert_eq!(p.get(Key::InstantMessages), Ok(None), "a profile without a user name is blank");
        let Ok(Some(Value::Profiles(profiles))) = p.get(Key::SocialProfiles) else { panic!("profiles") };
        assert_eq!((profiles[0].service.as_str(), profiles[0].identifier.as_str()), ("mastodon", "1"));
        assert_eq!(p.get(Key::Relations), Err(Error::NotFetched(Key::Relations)));
    }

    #[test]
    fn predefined_labels_read_in_title_case_and_typed_labels_as_typed() {
        assert_eq!(localized_label(LABEL_PHONE_HOME_FAX), "Home Fax");
        assert_eq!(localized_label(" Mobile "), "Mobile");
        assert_eq!(localized_label(LABEL_URL_HOME_PAGE), "Home Page");
        assert_eq!(localized_label("spouse"), "Spouse");
        assert_eq!(localized_label("boat"), "boat");
        assert_eq!(localized_label(""), "Other");
        assert!(is_predefined_label("MANAGER"));
        assert!(!is_predefined_label("boat"));
    }
}
