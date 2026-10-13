//! The contact data model and an in-memory store, mirroring the reference contacts framework type by type: `Contact`
//! (CNMutableContact), `LabeledValue` (CNLabeledValue), the `label` constants (CNLabel*), `Group`, `Container`
//! and `ContainerType`, `Predicate` (CNContact.predicateFor*), `SaveRequest` (CNSaveRequest), `ContactStore`
//! (CNContactStore) and `FetchedContact`, whose getters fail with `Error::PropertyNotFetched` for a key the fetch
//! did not name (the property-not-fetched exception). `format_name` is CNContactFormatter's full-name style.
//! Lookups by identifier, email and phone go through indexes, so a query does not walk every contact.
//! Pure. Persistence is the caller's: `from_legacy` migrates a row of the old single-field store.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Predefined labels (CNLabel*). A label is `None` (no label), one of these, or free text.
pub mod label {
    pub const HOME: &str = "_$!<Home>!$_";
    pub const WORK: &str = "_$!<Work>!$_";
    pub const OTHER: &str = "_$!<Other>!$_";
    pub const MOBILE: &str = "_$!<Mobile>!$_";
    pub const DEVICE: &str = "_$!<Device>!$_";
    pub const MAIN: &str = "_$!<Main>!$_";
    pub const HOME_FAX: &str = "_$!<HomeFAX>!$_";
    pub const WORK_FAX: &str = "_$!<WorkFAX>!$_";
    pub const OTHER_FAX: &str = "_$!<OtherFAX>!$_";
    pub const PAGER: &str = "_$!<Pager>!$_";
    pub const HOME_PAGE: &str = "_$!<HomePage>!$_";
    pub const ANNIVERSARY: &str = "_$!<Anniversary>!$_";
    pub const FATHER: &str = "_$!<Father>!$_";
    pub const MOTHER: &str = "_$!<Mother>!$_";
    pub const PARENT: &str = "_$!<Parent>!$_";
    pub const BROTHER: &str = "_$!<Brother>!$_";
    pub const SISTER: &str = "_$!<Sister>!$_";
    pub const CHILD: &str = "_$!<Child>!$_";
    pub const FRIEND: &str = "_$!<Friend>!$_";
    pub const SPOUSE: &str = "_$!<Spouse>!$_";
    pub const PARTNER: &str = "_$!<Partner>!$_";
    pub const ASSISTANT: &str = "_$!<Assistant>!$_";
    pub const MANAGER: &str = "_$!<Manager>!$_";

    const NAMES: [&str; 22] = [
        "Home", "Work", "Other", "Mobile", "Main", "HomeFAX", "WorkFAX", "OtherFAX", "Pager", "HomePage", "Anniversary",
        "Father", "Mother", "Parent", "Brother", "Sister", "Child", "Friend", "Spouse", "Partner", "Assistant", "Manager",
    ];

    /// The readable name of a predefined label ("Home", "HomeFAX"), or `None` for free text.
    pub fn standard_name(label: &str) -> Option<&str> {
        label.strip_prefix("_$!<")?.strip_suffix(">!$_")
    }

    /// The predefined label with this name (case-insensitive), if there is one.
    pub fn from_standard_name(name: &str) -> Option<String> {
        NAMES.iter().find(|n| n.eq_ignore_ascii_case(name)).map(|n| format!("_$!<{n}>!$_"))
    }
}

/// A value with an optional label and a stable identifier (CNLabeledValue).
#[derive(Clone, Debug, PartialEq)]
pub struct LabeledValue<T> {
    pub identifier: u64,
    pub label: Option<String>,
    pub value: T,
}

impl<T> LabeledValue<T> {
    pub fn new(label: Option<&str>, value: T) -> Self {
        LabeledValue { identifier: 0, label: label.map(str::to_string), value }
    }

    /// The same value under another label (`settingLabel`).
    pub fn with_label(mut self, label: Option<&str>) -> Self {
        self.label = label.map(str::to_string);
        self
    }

    /// The same label with another value (`settingValue`).
    pub fn with_value<U>(&self, value: U) -> LabeledValue<U> {
        LabeledValue { identifier: self.identifier, label: self.label.clone(), value }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostalAddress {
    pub street: String,
    pub sub_locality: String,
    pub city: String,
    pub sub_administrative_area: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub iso_country_code: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialProfile {
    pub url_string: String,
    pub username: String,
    pub user_identifier: String,
    pub service: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstantMessageAddress {
    pub username: String,
    pub service: String,
}

/// A date with optional year (DateComponents for birthdays and dates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateComponents {
    pub year: Option<i32>,
    pub month: u8,
    pub day: u8,
}

impl DateComponents {
    /// `None` unless month is 1-12 and day is 1-31.
    pub fn new(year: Option<i32>, month: u8, day: u8) -> Option<Self> {
        ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(DateComponents { year, month, day })
    }
}

/// A property a fetch can name (the CNContact*Key constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactKey {
    NamePrefix,
    GivenName,
    MiddleName,
    FamilyName,
    NameSuffix,
    Nickname,
    OrganizationName,
    DepartmentName,
    JobTitle,
    Note,
    PhoneNumbers,
    EmailAddresses,
    PostalAddresses,
    UrlAddresses,
    ContactRelations,
    SocialProfiles,
    InstantMessageAddresses,
    Birthday,
    Dates,
    ImageData,
    ThumbnailImageData,
}

/// A contact (CNMutableContact). The identifier is assigned by the store when empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contact {
    pub identifier: String,
    pub name_prefix: String,
    pub given_name: String,
    pub middle_name: String,
    pub family_name: String,
    pub name_suffix: String,
    pub nickname: String,
    pub organization_name: String,
    pub department_name: String,
    pub job_title: String,
    pub note: String,
    pub phone_numbers: Vec<LabeledValue<String>>,
    pub email_addresses: Vec<LabeledValue<String>>,
    pub postal_addresses: Vec<LabeledValue<PostalAddress>>,
    pub url_addresses: Vec<LabeledValue<String>>,
    pub contact_relations: Vec<LabeledValue<String>>,
    pub social_profiles: Vec<LabeledValue<SocialProfile>>,
    pub instant_message_addresses: Vec<LabeledValue<InstantMessageAddress>>,
    pub birthday: Option<DateComponents>,
    pub dates: Vec<LabeledValue<DateComponents>>,
    pub image_data: Option<Vec<u8>>,
    pub thumbnail_image_data: Option<Vec<u8>>,
}

fn number<T>(list: &mut [LabeledValue<T>], next: &mut u64) {
    for item in list.iter_mut().filter(|i| i.identifier == 0) {
        *next += 1;
        item.identifier = *next;
    }
}

impl Contact {
    /// A copy that holds only the named properties; the rest are blank. The identifier always stays.
    pub fn projected(&self, keys: &BTreeSet<ContactKey>) -> Contact {
        use ContactKey as K;
        let mut c = Contact { identifier: self.identifier.clone(), ..Contact::default() };
        macro_rules! keep {
            ($key:expr, $field:ident) => {
                if keys.contains(&$key) {
                    c.$field = self.$field.clone();
                }
            };
        }
        keep!(K::NamePrefix, name_prefix);
        keep!(K::GivenName, given_name);
        keep!(K::MiddleName, middle_name);
        keep!(K::FamilyName, family_name);
        keep!(K::NameSuffix, name_suffix);
        keep!(K::Nickname, nickname);
        keep!(K::OrganizationName, organization_name);
        keep!(K::DepartmentName, department_name);
        keep!(K::JobTitle, job_title);
        keep!(K::Note, note);
        keep!(K::PhoneNumbers, phone_numbers);
        keep!(K::EmailAddresses, email_addresses);
        keep!(K::PostalAddresses, postal_addresses);
        keep!(K::UrlAddresses, url_addresses);
        keep!(K::ContactRelations, contact_relations);
        keep!(K::SocialProfiles, social_profiles);
        keep!(K::InstantMessageAddresses, instant_message_addresses);
        keep!(K::Birthday, birthday);
        keep!(K::Dates, dates);
        keep!(K::ImageData, image_data);
        keep!(K::ThumbnailImageData, thumbnail_image_data);
        c
    }

    /// Gives every multi-value entry that has no identifier one of its own.
    fn number_values(&mut self, next: &mut u64) {
        number(&mut self.phone_numbers, next);
        number(&mut self.email_addresses, next);
        number(&mut self.postal_addresses, next);
        number(&mut self.url_addresses, next);
        number(&mut self.contact_relations, next);
        number(&mut self.social_profiles, next);
        number(&mut self.instant_message_addresses, next);
        number(&mut self.dates, next);
    }

    fn name_parts(&self) -> [&str; 5] {
        [
            self.given_name.as_str(),
            self.family_name.as_str(),
            self.middle_name.as_str(),
            self.nickname.as_str(),
            self.organization_name.as_str(),
        ]
    }
}

/// Migrates one row of the old single-field store (name, phone, email, notes, company, address, labels) to a
/// contact. `labels` index the old label sets: phone `mobile home work other`, email and address `home work other`.
pub fn from_legacy(name: &str, phone: &str, email: &str, notes: &str, company: &str, address: &str, labels: [u8; 3]) -> Contact {
    const PHONE: [&str; 4] = [label::MOBILE, label::HOME, label::WORK, label::OTHER];
    const OTHERS: [&str; 3] = [label::HOME, label::WORK, label::OTHER];
    let name = name.trim();
    let (given, family) = match name.rsplit_once(' ') {
        Some((g, f)) => (g.trim().to_string(), f.trim().to_string()),
        None => (name.to_string(), String::new()),
    };
    let mut c = Contact {
        given_name: given,
        family_name: family,
        note: notes.to_string(),
        organization_name: company.trim().to_string(),
        ..Contact::default()
    };
    if !phone.trim().is_empty() {
        c.phone_numbers.push(LabeledValue::new(PHONE.get(labels[0] as usize).copied(), phone.trim().to_string()));
    }
    if !email.trim().is_empty() {
        c.email_addresses.push(LabeledValue::new(OTHERS.get(labels[1] as usize).copied(), email.trim().to_string()));
    }
    if !address.trim().is_empty() {
        let street = address.trim().to_string();
        let postal = PostalAddress { street, ..PostalAddress::default() };
        c.postal_addresses.push(LabeledValue::new(OTHERS.get(labels[2] as usize).copied(), postal));
    }
    c
}

/// A group (CNGroup / CNMutableGroup).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub identifier: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerType {
    Unassigned,
    Local,
    Exchange,
    CardDav,
}

/// An account that holds contacts and groups (CNContainer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub identifier: String,
    pub name: String,
    pub kind: ContainerType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A getter asked for a property the fetch did not name (CNContactPropertyNotFetchedException).
    PropertyNotFetched(ContactKey),
    NoKeys,
    UnknownContact,
    UnknownGroup,
    UnknownContainer,
    /// An add with an identifier already in use.
    Duplicate,
    EmptyName,
}

/// The ways to pick contacts (CNContact.predicateFor...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    MatchingName(String),
    MatchingEmailAddress(String),
    MatchingPhoneNumber(String),
    WithIdentifiers(Vec<String>),
    InGroup(String),
    InContainer(String),
}

/// Group predicates (CNGroup.predicateForGroups...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupPredicate {
    WithIdentifiers(Vec<String>),
    InContainer(String),
}

/// Container predicates (CNContainer.predicateForContainers...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerPredicate {
    WithIdentifiers(Vec<String>),
    OfContact(String),
    OfGroup(String),
}

/// A contact as a fetch returns it: only the fetched properties hold data, and any other getter fails.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedContact {
    contact: Contact,
    keys: BTreeSet<ContactKey>,
}

macro_rules! getters {
    ($($name:ident: $key:expr => $ty:ty;)*) => {
        $(pub fn $name(&self) -> Result<&$ty, Error> {
            self.need($key)?;
            Ok(&self.contact.$name)
        })*
    };
}

impl FetchedContact {
    pub fn identifier(&self) -> &str {
        &self.contact.identifier
    }

    pub fn is_key_available(&self, key: ContactKey) -> bool {
        self.keys.contains(&key)
    }

    fn need(&self, key: ContactKey) -> Result<(), Error> {
        if self.keys.contains(&key) {
            Ok(())
        } else {
            Err(Error::PropertyNotFetched(key))
        }
    }

    getters! {
        name_prefix: ContactKey::NamePrefix => String;
        given_name: ContactKey::GivenName => String;
        middle_name: ContactKey::MiddleName => String;
        family_name: ContactKey::FamilyName => String;
        name_suffix: ContactKey::NameSuffix => String;
        nickname: ContactKey::Nickname => String;
        organization_name: ContactKey::OrganizationName => String;
        department_name: ContactKey::DepartmentName => String;
        job_title: ContactKey::JobTitle => String;
        note: ContactKey::Note => String;
        phone_numbers: ContactKey::PhoneNumbers => Vec<LabeledValue<String>>;
        email_addresses: ContactKey::EmailAddresses => Vec<LabeledValue<String>>;
        postal_addresses: ContactKey::PostalAddresses => Vec<LabeledValue<PostalAddress>>;
        url_addresses: ContactKey::UrlAddresses => Vec<LabeledValue<String>>;
        contact_relations: ContactKey::ContactRelations => Vec<LabeledValue<String>>;
        social_profiles: ContactKey::SocialProfiles => Vec<LabeledValue<SocialProfile>>;
        instant_message_addresses: ContactKey::InstantMessageAddresses => Vec<LabeledValue<InstantMessageAddress>>;
        birthday: ContactKey::Birthday => Option<DateComponents>;
        dates: ContactKey::Dates => Vec<LabeledValue<DateComponents>>;
        image_data: ContactKey::ImageData => Option<Vec<u8>>;
        thumbnail_image_data: ContactKey::ThumbnailImageData => Option<Vec<u8>>;
    }

    /// The fetched data as a contact; properties that were not fetched are blank.
    pub fn mutable_copy(&self) -> Contact {
        self.contact.clone()
    }
}

/// Name order for the formatter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameOrder {
    GivenNameFirst,
    FamilyNameFirst,
}

/// The keys `format_name` reads (`descriptorForRequiredKeys`).
pub fn formatter_required_keys() -> Vec<ContactKey> {
    use ContactKey::*;
    vec![NamePrefix, GivenName, MiddleName, FamilyName, NameSuffix, Nickname, OrganizationName]
}

/// The full name of a fetched contact (CNContactFormatter full-name style): prefix, given, middle, family and
/// suffix in order; else the nickname; else the organization. Fails if a required key was not fetched.
pub fn format_name(contact: &FetchedContact, order: NameOrder) -> Result<String, Error> {
    let (prefix, given, middle) = (contact.name_prefix()?, contact.given_name()?, contact.middle_name()?);
    let (family, suffix) = (contact.family_name()?, contact.name_suffix()?);
    let (nick, org) = (contact.nickname()?, contact.organization_name()?);
    let join = |parts: &[&str]| parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    let full = match order {
        NameOrder::GivenNameFirst => join(&[prefix.as_str(), given.as_str(), middle.as_str(), family.as_str(), suffix.as_str()]),
        NameOrder::FamilyNameFirst => {
            let rest = join(&[prefix.as_str(), given.as_str(), middle.as_str(), suffix.as_str()]);
            let family = family.trim();
            match (family.is_empty(), rest.is_empty()) {
                (false, false) => format!("{family}, {rest}"),
                (false, true) => family.to_string(),
                _ => rest,
            }
        }
    };
    if !full.is_empty() {
        return Ok(full);
    }
    Ok([nick.trim(), org.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("").to_string())
}

/// A batch of changes applied together (CNSaveRequest): all of them or none.
#[derive(Clone, Debug, Default)]
pub struct SaveRequest {
    ops: Vec<Op>,
}

#[derive(Clone, Debug)]
enum Op {
    AddContact(Contact, Option<String>),
    UpdateContact(Contact),
    DeleteContact(String),
    AddGroup(Group, Option<String>),
    UpdateGroup(Group),
    DeleteGroup(String),
    AddMember(String, String),
    RemoveMember(String, String),
}

impl SaveRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_contact(&mut self, contact: Contact, container: Option<&str>) -> &mut Self {
        self.ops.push(Op::AddContact(contact, container.map(str::to_string)));
        self
    }
    pub fn update_contact(&mut self, contact: Contact) -> &mut Self {
        self.ops.push(Op::UpdateContact(contact));
        self
    }
    pub fn delete_contact(&mut self, identifier: &str) -> &mut Self {
        self.ops.push(Op::DeleteContact(identifier.to_string()));
        self
    }
    pub fn add_group(&mut self, group: Group, container: Option<&str>) -> &mut Self {
        self.ops.push(Op::AddGroup(group, container.map(str::to_string)));
        self
    }
    pub fn update_group(&mut self, group: Group) -> &mut Self {
        self.ops.push(Op::UpdateGroup(group));
        self
    }
    pub fn delete_group(&mut self, identifier: &str) -> &mut Self {
        self.ops.push(Op::DeleteGroup(identifier.to_string()));
        self
    }
    pub fn add_member(&mut self, contact: &str, group: &str) -> &mut Self {
        self.ops.push(Op::AddMember(contact.to_string(), group.to_string()));
        self
    }
    pub fn remove_member(&mut self, contact: &str, group: &str) -> &mut Self {
        self.ops.push(Op::RemoveMember(contact.to_string(), group.to_string()));
        self
    }
}

#[derive(Clone)]
struct GroupRec {
    group: Group,
    container: String,
    members: BTreeSet<String>,
}

/// The store (CNContactStore).
pub struct ContactStore {
    containers: Vec<Container>,
    default_container: String,
    contacts: BTreeMap<String, (Contact, String)>,
    groups: BTreeMap<String, GroupRec>,
    by_email: HashMap<String, BTreeSet<String>>,
    by_phone: HashMap<String, BTreeSet<String>>,
    next_id: u64,
    next_value_id: u64,
    scanned: Cell<usize>,
}

impl Clone for ContactStore {
    fn clone(&self) -> Self {
        ContactStore {
            containers: self.containers.clone(),
            default_container: self.default_container.clone(),
            contacts: self.contacts.clone(),
            groups: self.groups.clone(),
            by_email: self.by_email.clone(),
            by_phone: self.by_phone.clone(),
            next_id: self.next_id,
            next_value_id: self.next_value_id,
            scanned: Cell::new(0),
        }
    }
}

impl Default for ContactStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Digits of a phone number; the key under which it is indexed is its last seven.
fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

fn phone_key(digits: &str) -> &str {
    &digits[digits.len().saturating_sub(7)..]
}

fn folded(s: &str) -> String {
    s.trim().to_lowercase()
}

impl ContactStore {
    /// An empty store with one local container, which is the default.
    pub fn new() -> Self {
        let local = Container { identifier: "local".into(), name: "Local".into(), kind: ContainerType::Local };
        ContactStore {
            default_container: local.identifier.clone(),
            containers: vec![local],
            contacts: BTreeMap::new(),
            groups: BTreeMap::new(),
            by_email: HashMap::new(),
            by_phone: HashMap::new(),
            next_id: 0,
            next_value_id: 0,
            scanned: Cell::new(0),
        }
    }

    pub fn add_container(&mut self, container: Container) -> Result<(), Error> {
        if self.containers.iter().any(|c| c.identifier == container.identifier) {
            return Err(Error::Duplicate);
        }
        self.containers.push(container);
        Ok(())
    }

    pub fn default_container_identifier(&self) -> &str {
        &self.default_container
    }

    /// How many contacts the last predicate query looked at. A lookup by identifier, email or phone looks at only
    /// the candidates its index holds.
    pub fn last_scanned(&self) -> usize {
        self.scanned.get()
    }

    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// Applies a request atomically: on any error nothing changes.
    pub fn execute(&mut self, request: &SaveRequest) -> Result<(), Error> {
        let mut next = self.clone();
        for op in &request.ops {
            next.apply(op)?;
        }
        *self = next;
        Ok(())
    }

    fn require_container(&self, id: Option<&String>) -> Result<String, Error> {
        let id = id.cloned().unwrap_or_else(|| self.default_container.clone());
        if self.containers.iter().any(|c| c.identifier == id) {
            Ok(id)
        } else {
            Err(Error::UnknownContainer)
        }
    }

    fn apply(&mut self, op: &Op) -> Result<(), Error> {
        match op {
            Op::AddContact(contact, container) => {
                let container = self.require_container(container.as_ref())?;
                let mut c = contact.clone();
                if c.identifier.is_empty() {
                    self.next_id += 1;
                    c.identifier = format!("contact-{}", self.next_id);
                }
                if self.contacts.contains_key(&c.identifier) {
                    return Err(Error::Duplicate);
                }
                c.number_values(&mut self.next_value_id);
                self.index(&c);
                self.contacts.insert(c.identifier.clone(), (c, container));
            }
            Op::UpdateContact(contact) => {
                let container = self.contacts.get(&contact.identifier).ok_or(Error::UnknownContact)?.1.clone();
                let mut c = contact.clone();
                c.number_values(&mut self.next_value_id);
                self.unindex_id(&c.identifier);
                self.index(&c);
                self.contacts.insert(c.identifier.clone(), (c, container));
            }
            Op::DeleteContact(id) => {
                if !self.contacts.contains_key(id) {
                    return Err(Error::UnknownContact);
                }
                self.unindex_id(id);
                self.contacts.remove(id);
                for g in self.groups.values_mut() {
                    g.members.remove(id);
                }
            }
            Op::AddGroup(group, container) => {
                let container = self.require_container(container.as_ref())?;
                if group.name.trim().is_empty() {
                    return Err(Error::EmptyName);
                }
                let mut g = group.clone();
                if g.identifier.is_empty() {
                    self.next_id += 1;
                    g.identifier = format!("group-{}", self.next_id);
                }
                if self.groups.contains_key(&g.identifier) {
                    return Err(Error::Duplicate);
                }
                self.groups.insert(g.identifier.clone(), GroupRec { group: g, container, members: BTreeSet::new() });
            }
            Op::UpdateGroup(group) => {
                if group.name.trim().is_empty() {
                    return Err(Error::EmptyName);
                }
                self.groups.get_mut(&group.identifier).ok_or(Error::UnknownGroup)?.group.name = group.name.clone();
            }
            Op::DeleteGroup(id) => {
                self.groups.remove(id).ok_or(Error::UnknownGroup)?;
            }
            Op::AddMember(contact, group) => {
                if !self.contacts.contains_key(contact) {
                    return Err(Error::UnknownContact);
                }
                self.groups.get_mut(group).ok_or(Error::UnknownGroup)?.members.insert(contact.clone());
            }
            Op::RemoveMember(contact, group) => {
                self.groups.get_mut(group).ok_or(Error::UnknownGroup)?.members.remove(contact);
            }
        }
        Ok(())
    }

    fn index(&mut self, c: &Contact) {
        for e in &c.email_addresses {
            self.by_email.entry(folded(&e.value)).or_default().insert(c.identifier.clone());
        }
        for p in &c.phone_numbers {
            let d = digits(&p.value);
            if !d.is_empty() {
                self.by_phone.entry(phone_key(&d).to_string()).or_default().insert(c.identifier.clone());
            }
        }
    }

    fn unindex_id(&mut self, id: &str) {
        for set in self.by_email.values_mut().chain(self.by_phone.values_mut()) {
            set.remove(id);
        }
        self.by_email.retain(|_, s| !s.is_empty());
        self.by_phone.retain(|_, s| !s.is_empty());
    }

    /// The contacts a predicate picks (all with `None`), with the named keys, sorted by name.
    /// `unifiedContacts(matching:keysToFetch:)`. An empty key list is an error.
    pub fn contacts_matching(&self, predicate: Option<&Predicate>, keys: &[ContactKey]) -> Result<Vec<FetchedContact>, Error> {
        if keys.is_empty() {
            return Err(Error::NoKeys);
        }
        let keys: BTreeSet<ContactKey> = keys.iter().copied().collect();
        let ids = self.candidates(predicate)?;
        let mut found: Vec<&Contact> = ids.iter().filter_map(|id| self.contacts.get(id)).map(|(c, _)| c).collect();
        found.sort_by_key(|c| (sort_name(c), c.identifier.clone()));
        Ok(found.into_iter().map(|c| FetchedContact { contact: c.projected(&keys), keys: keys.clone() }).collect())
    }

    fn candidates(&self, predicate: Option<&Predicate>) -> Result<Vec<String>, Error> {
        let ids = match predicate {
            None => {
                self.scanned.set(self.contacts.len());
                self.contacts.keys().cloned().collect()
            }
            Some(Predicate::WithIdentifiers(ids)) => {
                self.scanned.set(ids.len());
                ids.iter().filter(|id| self.contacts.contains_key(*id)).cloned().collect()
            }
            Some(Predicate::MatchingEmailAddress(email)) => {
                let hits: Vec<String> =
                    self.by_email.get(&folded(email)).map(|s| s.iter().cloned().collect()).unwrap_or_default();
                self.scanned.set(hits.len());
                hits
            }
            Some(Predicate::MatchingPhoneNumber(phone)) => {
                let want = digits(phone);
                let mut hits = Vec::new();
                if !want.is_empty() {
                    let pool = self.by_phone.get(phone_key(&want)).cloned().unwrap_or_default();
                    self.scanned.set(pool.len());
                    for id in pool {
                        let Some((c, _)) = self.contacts.get(&id) else { continue };
                        let same = c
                            .phone_numbers
                            .iter()
                            .map(|p| digits(&p.value))
                            .any(|d| !d.is_empty() && (d.ends_with(&want) || want.ends_with(&d)));
                        if same {
                            hits.push(id);
                        }
                    }
                }
                hits
            }
            Some(Predicate::InGroup(group)) => {
                let g = self.groups.get(group).ok_or(Error::UnknownGroup)?;
                self.scanned.set(g.members.len());
                g.members.iter().cloned().collect()
            }
            Some(Predicate::InContainer(container)) => {
                self.require_container(Some(container))?;
                self.scanned.set(self.contacts.len());
                self.contacts.iter().filter(|(_, (_, c))| c == container).map(|(id, _)| id.clone()).collect()
            }
            Some(Predicate::MatchingName(name)) => {
                self.scanned.set(self.contacts.len());
                let tokens: Vec<String> = name.split_whitespace().map(folded).collect();
                if tokens.is_empty() {
                    return Ok(vec![]);
                }
                self.contacts
                    .iter()
                    .filter(|(_, (c, _))| {
                        let parts: Vec<String> =
                            c.name_parts().iter().flat_map(|p| p.split_whitespace()).map(folded).collect();
                        tokens.iter().all(|t| parts.iter().any(|p| p.starts_with(t.as_str())))
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            }
        };
        Ok(ids)
    }

    /// `groups(matching:)`: all groups with `None`, sorted by name.
    pub fn groups_matching(&self, predicate: Option<&GroupPredicate>) -> Vec<Group> {
        let mut out: Vec<Group> = self
            .groups
            .values()
            .filter(|g| match predicate {
                None => true,
                Some(GroupPredicate::WithIdentifiers(ids)) => ids.contains(&g.group.identifier),
                Some(GroupPredicate::InContainer(c)) => &g.container == c,
            })
            .map(|g| g.group.clone())
            .collect();
        out.sort_by_key(|g| (folded(&g.name), g.identifier.clone()));
        out
    }

    /// `containers(matching:)`: all containers with `None`.
    pub fn containers_matching(&self, predicate: Option<&ContainerPredicate>) -> Vec<Container> {
        self.containers
            .iter()
            .filter(|c| match predicate {
                None => true,
                Some(ContainerPredicate::WithIdentifiers(ids)) => ids.contains(&c.identifier),
                Some(ContainerPredicate::OfContact(id)) => {
                    self.contacts.get(id).map(|(_, home)| home == &c.identifier).unwrap_or(false)
                }
                Some(ContainerPredicate::OfGroup(id)) => {
                    self.groups.get(id).map(|g| g.container == c.identifier).unwrap_or(false)
                }
            })
            .cloned()
            .collect()
    }
}

fn sort_name(c: &Contact) -> String {
    let person = format!("{} {}", c.family_name, c.given_name);
    let name = if person.trim().is_empty() { c.organization_name.clone() } else { person };
    folded(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContactKey::*;

    fn ada() -> Contact {
        Contact {
            given_name: "Ada".into(),
            family_name: "Lovelace".into(),
            phone_numbers: vec![
                LabeledValue::new(Some(label::MOBILE), "+354 555 1234".into()),
                LabeledValue::new(Some(label::WORK), "(020) 7946 0000".into()),
            ],
            email_addresses: vec![LabeledValue::new(Some(label::HOME), "Ada@Example.com".into())],
            ..Contact::default()
        }
    }

    fn store_with(contacts: Vec<Contact>) -> ContactStore {
        let mut store = ContactStore::new();
        let mut req = SaveRequest::new();
        for c in contacts {
            req.add_contact(c, None);
        }
        store.execute(&req).unwrap();
        store
    }

    #[test]
    fn labeled_multi_values_round_trip_with_their_own_identifiers() {
        let store = store_with(vec![ada()]);
        let got = store.contacts_matching(None, &[PhoneNumbers, EmailAddresses]).unwrap();
        let phones = got[0].phone_numbers().unwrap();
        assert_eq!(phones.len(), 2);
        assert_eq!(phones[0].label.as_deref(), Some(label::MOBILE));
        assert_ne!(phones[0].identifier, phones[1].identifier);
        assert!(phones[0].identifier > 0);
        assert_eq!(got[0].email_addresses().unwrap()[0].value, "Ada@Example.com");
    }

    #[test]
    fn a_getter_for_an_unfetched_key_fails_and_data_does_not_leak() {
        let store = store_with(vec![ada()]);
        let got = store.contacts_matching(None, &[GivenName]).unwrap();
        assert_eq!(got[0].given_name().unwrap(), "Ada");
        assert_eq!(got[0].family_name(), Err(Error::PropertyNotFetched(FamilyName)));
        assert_eq!(got[0].phone_numbers(), Err(Error::PropertyNotFetched(PhoneNumbers)));
        assert!(!got[0].is_key_available(PhoneNumbers));
        assert!(got[0].mutable_copy().phone_numbers.is_empty());
        assert_eq!(store.contacts_matching(None, &[]), Err(Error::NoKeys));
    }

    #[test]
    fn the_legacy_single_field_row_migrates_to_labeled_values() {
        let c = from_legacy("Ada King Lovelace", "555", "a@x.org", "n", "Engine Co", "1 Main St", [2, 1, 0]);
        assert_eq!((c.given_name.as_str(), c.family_name.as_str()), ("Ada King", "Lovelace"));
        assert_eq!(c.phone_numbers[0].label.as_deref(), Some(label::WORK));
        assert_eq!(c.email_addresses[0].label.as_deref(), Some(label::WORK));
        assert_eq!(c.postal_addresses[0].label.as_deref(), Some(label::HOME));
        assert_eq!(c.postal_addresses[0].value.street, "1 Main St");
        assert_eq!(c.organization_name, "Engine Co");
        assert!(from_legacy("", "", "", "", "", "", [0; 3]).phone_numbers.is_empty());
    }

    #[test]
    fn group_crud_membership_and_deleting_a_contact_drop_the_links() {
        let mut store = store_with(vec![Contact { identifier: "c1".into(), ..ada() }]);
        let mut req = SaveRequest::new();
        req.add_group(Group { identifier: "g1".into(), name: "Family".into() }, None).add_member("c1", "g1");
        store.execute(&req).unwrap();
        let members = store.contacts_matching(Some(&Predicate::InGroup("g1".into())), &[GivenName]).unwrap();
        assert_eq!(members.len(), 1);
        let mut rename = SaveRequest::new();
        rename.update_group(Group { identifier: "g1".into(), name: "Kin".into() });
        store.execute(&rename).unwrap();
        assert_eq!(store.groups_matching(None)[0].name, "Kin");
        let mut gone = SaveRequest::new();
        gone.delete_contact("c1");
        store.execute(&gone).unwrap();
        assert!(store.contacts_matching(Some(&Predicate::InGroup("g1".into())), &[GivenName]).unwrap().is_empty());
        let mut drop_group = SaveRequest::new();
        drop_group.delete_group("g1");
        store.execute(&drop_group).unwrap();
        assert!(store.groups_matching(None).is_empty());
        assert_eq!(
            store.contacts_matching(Some(&Predicate::InGroup("g1".into())), &[GivenName]),
            Err(Error::UnknownGroup)
        );
    }

    #[test]
    fn a_failing_request_changes_nothing() {
        let mut store = store_with(vec![ada()]);
        let mut req = SaveRequest::new();
        req.add_contact(Contact { given_name: "Grace".into(), ..Contact::default() }, None).delete_contact("missing");
        assert_eq!(store.execute(&req), Err(Error::UnknownContact));
        assert_eq!(store.len(), 1);
        let mut blank = SaveRequest::new();
        blank.add_group(Group { identifier: "g".into(), name: " ".into() }, None);
        assert_eq!(store.execute(&blank), Err(Error::EmptyName));
    }

    #[test]
    fn containers_list_and_filter_and_the_default_exists() {
        let mut store = ContactStore::new();
        let work = Container { identifier: "work".into(), name: "Work".into(), kind: ContainerType::Exchange };
        store.add_container(work.clone()).unwrap();
        assert_eq!(store.add_container(work), Err(Error::Duplicate));
        assert_eq!(store.containers_matching(None).len(), 2);
        assert_eq!(store.default_container_identifier(), "local");
        let mut req = SaveRequest::new();
        req.add_contact(Contact { identifier: "c1".into(), given_name: "Ada".into(), ..Contact::default() }, Some("work"));
        req.add_group(Group { identifier: "g1".into(), name: "Team".into() }, Some("work"));
        store.execute(&req).unwrap();
        let of = store.containers_matching(Some(&ContainerPredicate::OfContact("c1".into())));
        assert_eq!((of[0].identifier.as_str(), of[0].kind), ("work", ContainerType::Exchange));
        assert_eq!(store.containers_matching(Some(&ContainerPredicate::OfGroup("g1".into()))).len(), 1);
        let local = ContainerPredicate::WithIdentifiers(vec!["local".into()]);
        assert_eq!(store.containers_matching(Some(&local)).len(), 1);
        assert_eq!(store.groups_matching(Some(&GroupPredicate::InContainer("local".into()))).len(), 0);
        assert_eq!(store.groups_matching(Some(&GroupPredicate::InContainer("work".into()))).len(), 1);
        let in_work = store.contacts_matching(Some(&Predicate::InContainer("work".into())), &[GivenName]).unwrap();
        assert_eq!(in_work.len(), 1);
        let mut bad = SaveRequest::new();
        bad.add_contact(Contact::default(), Some("nope"));
        assert_eq!(store.execute(&bad), Err(Error::UnknownContainer));
    }

    #[test]
    fn every_predicate_kind_picks_its_contacts() {
        let grace = Contact {
            identifier: "g".into(),
            given_name: "Grace".into(),
            family_name: "Hopper".into(),
            nickname: "Amazing".into(),
            ..Contact::default()
        };
        let store = store_with(vec![Contact { identifier: "a".into(), ..ada() }, grace]);
        let ids = |p: Predicate| -> Vec<String> {
            store.contacts_matching(Some(&p), &[GivenName]).unwrap().iter().map(|c| c.identifier().to_string()).collect()
        };
        assert_eq!(ids(Predicate::MatchingName("lovel ada".into())), vec!["a"]);
        assert_eq!(ids(Predicate::MatchingName("amaz".into())), vec!["g"]);
        assert!(ids(Predicate::MatchingName("   ".into())).is_empty());
        assert_eq!(ids(Predicate::MatchingEmailAddress("ada@example.COM".into())), vec!["a"]);
        assert_eq!(ids(Predicate::MatchingPhoneNumber("5551234".into())), vec!["a"]);
        assert_eq!(ids(Predicate::MatchingPhoneNumber("020 7946 0000".into())), vec!["a"]);
        assert!(ids(Predicate::MatchingPhoneNumber("999 9999".into())).is_empty());
        assert!(ids(Predicate::MatchingPhoneNumber("abc".into())).is_empty());
        assert_eq!(ids(Predicate::WithIdentifiers(vec!["g".into(), "zz".into()])), vec!["g"]);
        let all = store.contacts_matching(None, &[GivenName]).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].identifier(), "g", "sorted by family name: Hopper before Lovelace");
    }

    #[test]
    fn indexed_predicates_look_only_at_their_candidates_and_follow_updates() {
        let mut many = Vec::new();
        for i in 0..200 {
            many.push(Contact {
                identifier: format!("c{i}"),
                given_name: format!("P{i}"),
                email_addresses: vec![LabeledValue::new(None, format!("p{i}@x.org"))],
                phone_numbers: vec![LabeledValue::new(None, format!("100{i:04}"))],
                ..Contact::default()
            });
        }
        let mut store = store_with(many);
        let hit = store.contacts_matching(Some(&Predicate::MatchingEmailAddress("p42@x.org".into())), &[GivenName]).unwrap();
        assert_eq!((hit.len(), store.last_scanned()), (1, 1));
        store.contacts_matching(Some(&Predicate::MatchingPhoneNumber("1000042".into())), &[GivenName]).unwrap();
        assert_eq!(store.last_scanned(), 1);
        store.contacts_matching(Some(&Predicate::WithIdentifiers(vec!["c7".into()])), &[GivenName]).unwrap();
        assert_eq!(store.last_scanned(), 1);
        let mut moved = Contact { identifier: "c42".into(), given_name: "P42".into(), ..Contact::default() };
        moved.email_addresses.push(LabeledValue::new(None, "new@x.org".into()));
        let mut req = SaveRequest::new();
        req.update_contact(moved);
        store.execute(&req).unwrap();
        let old = Predicate::MatchingEmailAddress("p42@x.org".into());
        assert!(store.contacts_matching(Some(&old), &[GivenName]).unwrap().is_empty());
        let new = Predicate::MatchingEmailAddress("new@x.org".into());
        assert_eq!(store.contacts_matching(Some(&new), &[GivenName]).unwrap().len(), 1);
    }

    #[test]
    fn the_formatter_orders_names_and_needs_its_keys() {
        let c = Contact {
            name_prefix: "Dr".into(),
            given_name: "Ada".into(),
            middle_name: "K".into(),
            family_name: "Lovelace".into(),
            name_suffix: "Jr".into(),
            ..Contact::default()
        };
        let org = Contact { identifier: "org".into(), organization_name: "Acme".into(), ..Contact::default() };
        let store = store_with(vec![c, org]);
        let got = store.contacts_matching(None, &formatter_required_keys()).unwrap();
        let names: Vec<String> = got.iter().map(|c| format_name(c, NameOrder::GivenNameFirst).unwrap()).collect();
        assert!(names.contains(&"Dr Ada K Lovelace Jr".to_string()));
        assert!(names.contains(&"Acme".to_string()));
        let person = got.iter().find(|c| c.family_name().unwrap() == "Lovelace").unwrap();
        assert_eq!(format_name(person, NameOrder::FamilyNameFirst).unwrap(), "Lovelace, Dr Ada K Jr");
        let partial = store.contacts_matching(None, &[GivenName]).unwrap();
        assert_eq!(format_name(&partial[0], NameOrder::GivenNameFirst), Err(Error::PropertyNotFetched(NamePrefix)));
    }

    #[test]
    fn labels_and_dates_helpers() {
        assert_eq!(label::standard_name(label::HOME_FAX), Some("HomeFAX"));
        assert_eq!(label::standard_name("Custom"), None);
        assert_eq!(label::from_standard_name("homefax").as_deref(), Some(label::HOME_FAX));
        assert_eq!(label::from_standard_name("manager").as_deref(), Some(label::MANAGER));
        assert_eq!(label::from_standard_name("nope"), None);
        let v = LabeledValue::new(Some(label::HOME), 1).with_label(None);
        assert_eq!(v.with_value("x").label, None);
        assert!(DateComponents::new(None, 13, 1).is_none());
        assert!(DateComponents::new(Some(1990), 5, 4).is_some());
    }
}
