//! Contact containers: the places a contact lives, such as the local book or a synced account. Every contact is in
//! exactly one container. The book always has the local container (`LOCAL`, id 1), which cannot be removed; a contact
//! the sidecar does not name is in it. Removing another container moves its contacts back to the local one, so a
//! container never takes contacts with it.
//!
//! The sidecar sits beside the contacts file so the contacts file keeps its format and an older shell ignores it. It
//! is a header line, a `last` line with the last identifier handed out, one `container` line per container added (id,
//! kind, escaped name) and one `member` line per contact outside the local container (contact id, container id).
use super::prefs::{escape_field as escape, unescape_field as unescape};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

const HEADER: &str = "springboard-contact-containers-v1";
pub const MAX_NAME: usize = 160;
pub const MAX_CONTAINERS: usize = 64;
/// The identifier of the local container every book has.
pub const LOCAL: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The book on this machine.
    Local,
    /// A synced account (a CardDAV-style server or the BifrOSt account).
    Account,
    /// Anything else, such as an imported file.
    Other,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Local => "local",
            Kind::Account => "account",
            Kind::Other => "other",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        match name {
            "local" => Some(Kind::Local),
            "account" => Some(Kind::Account),
            "other" => Some(Kind::Other),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unknown,
    EmptyName,
    NameTooLong,
    TooMany,
    /// The local container cannot be removed, and only one container is the local one.
    LocalIsFixed,
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Error::Unknown => "no such container",
            Error::EmptyName => "a container needs a name",
            Error::NameTooLong => "the container name is too long",
            Error::TooMany => "too many containers",
            Error::LocalIsFixed => "the local container cannot be removed or made another kind",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Containers {
    rows: BTreeMap<u64, Container>,
    /// The container of each contact outside the local one.
    placed: BTreeMap<u64, u64>,
    last_id: u64,
}

impl Default for Containers {
    fn default() -> Self {
        let local = Container { id: LOCAL, name: "On My Computer".into(), kind: Kind::Local };
        Containers { rows: BTreeMap::from([(LOCAL, local)]), placed: BTreeMap::new(), last_id: LOCAL }
    }
}

fn checked_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty() {
        Err(Error::EmptyName)
    } else if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        Err(Error::NameTooLong)
    } else {
        Ok(name.to_string())
    }
}

impl Containers {
    pub fn all(&self) -> impl Iterator<Item = &Container> {
        self.rows.values()
    }

    pub fn get(&self, id: u64) -> Option<&Container> {
        self.rows.get(&id)
    }

    /// Adds a container. A second local container is refused: the book has one.
    pub fn add(&mut self, name: &str, kind: Kind) -> Result<u64, Error> {
        let name = checked_name(name)?;
        if kind == Kind::Local {
            return Err(Error::LocalIsFixed);
        }
        if self.rows.len() >= MAX_CONTAINERS {
            return Err(Error::TooMany);
        }
        self.last_id += 1;
        let id = self.last_id;
        self.rows.insert(id, Container { id, name, kind });
        Ok(id)
    }

    pub fn rename(&mut self, id: u64, name: &str) -> Result<(), Error> {
        let name = checked_name(name)?;
        self.rows.get_mut(&id).ok_or(Error::Unknown)?.name = name;
        Ok(())
    }

    /// Removes a container and moves its contacts to the local one. Returns the contacts moved.
    pub fn remove(&mut self, id: u64) -> Result<Vec<u64>, Error> {
        if id == LOCAL {
            return Err(Error::LocalIsFixed);
        }
        self.rows.remove(&id).ok_or(Error::Unknown)?;
        let moved: Vec<u64> = self.placed.iter().filter(|(_, c)| **c == id).map(|(contact, _)| *contact).collect();
        for contact in &moved {
            self.placed.remove(contact);
        }
        Ok(moved)
    }

    /// The container a contact lives in.
    pub fn container_of(&self, contact: u64) -> u64 {
        self.placed.get(&contact).copied().unwrap_or(LOCAL)
    }

    /// Moves a contact to a container. Returns whether it was somewhere else before.
    pub fn assign(&mut self, contact: u64, container: u64) -> Result<bool, Error> {
        if !self.rows.contains_key(&container) {
            return Err(Error::Unknown);
        }
        let before = self.container_of(contact);
        if container == LOCAL {
            self.placed.remove(&contact);
        } else {
            self.placed.insert(contact, container);
        }
        Ok(before != container)
    }

    /// The contacts of `book` (identifiers) that live in `container`, in book order.
    pub fn members(&self, container: u64, book: &[u64]) -> Result<Vec<u64>, Error> {
        if !self.rows.contains_key(&container) {
            return Err(Error::Unknown);
        }
        Ok(book.iter().copied().filter(|c| self.container_of(*c) == container).collect())
    }

    /// Forgets a deleted contact.
    pub fn forget_contact(&mut self, contact: u64) -> bool {
        self.placed.remove(&contact).is_some()
    }
}

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contact-containers.tsv")
}

/// Writes the containers as the sidecar text.
pub fn render(containers: &Containers) -> String {
    let mut out = format!("{HEADER}\nlast\t{}\n", containers.last_id);
    for c in containers.rows.values().filter(|c| c.id != LOCAL) {
        out.push_str(&format!("container\t{}\t{}\t{}\n", c.id, c.kind.name(), escape(&c.name)));
    }
    if let Some(local) = containers.rows.get(&LOCAL) {
        out.push_str(&format!("local\t{}\n", escape(&local.name)));
    }
    for (contact, container) in &containers.placed {
        out.push_str(&format!("member\t{contact}\t{container}\n"));
    }
    out
}

/// Reads the sidecar text. Text that is not a containers sidecar, or whose members name a container that is not there,
/// is an error.
pub fn parse(text: &str) -> Result<Containers, String> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err("Unknown contact containers format".into());
    }
    let mut out = Containers::default();
    let mut last = None;
    let mut members = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["last", n] => last = Some(n.parse::<u64>().map_err(|_| "bad container counter".to_string())?),
            ["local", name] => {
                let name = unescape(name, MAX_NAME).ok_or_else(|| "bad container name".to_string())?;
                out.rows.get_mut(&LOCAL).ok_or_else(|| "no local container".to_string())?.name = checked_name(&name).map_err(|e| e.message().to_string())?;
            }
            ["container", id, kind, name] => {
                let id = id.parse::<u64>().map_err(|_| "bad container id".to_string())?;
                let kind = Kind::from_name(kind).filter(|k| *k != Kind::Local).ok_or_else(|| "bad container kind".to_string())?;
                let name = unescape(name, MAX_NAME).ok_or_else(|| "bad container name".to_string())?;
                if id == LOCAL || out.rows.contains_key(&id) || out.rows.len() >= MAX_CONTAINERS {
                    return Err("bad container id".into());
                }
                out.rows.insert(id, Container { id, name: checked_name(&name).map_err(|e| e.message().to_string())?, kind });
            }
            ["member", contact, container] => {
                let contact = contact.parse::<u64>().map_err(|_| "bad member".to_string())?;
                let container = container.parse::<u64>().map_err(|_| "bad member".to_string())?;
                members.push((contact, container));
            }
            _ => return Err("bad contact containers line".into()),
        }
    }
    let highest = out.rows.keys().copied().max().unwrap_or(LOCAL);
    out.last_id = last.ok_or_else(|| "missing container counter".to_string())?.max(highest);
    for (contact, container) in members {
        if container == LOCAL || !out.rows.contains_key(&container) {
            return Err("a member names a container that is not there".into());
        }
        out.placed.insert(contact, container);
    }
    Ok(out)
}

/// The sidecar at `path`; just the local container when it is missing or unreadable.
pub fn load(path: &Path) -> Containers {
    std::fs::read_to_string(path).ok().and_then(|t| parse(&t).ok()).unwrap_or_default()
}

/// Writes the sidecar atomically.
pub fn save(path: &Path, containers: &Containers) -> io::Result<()> {
    super::fsutil::write_atomic(path, render(containers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_book_has_only_the_local_container_and_every_contact_is_in_it() {
        let c = Containers::default();
        assert_eq!(c.all().count(), 1);
        assert_eq!(c.get(LOCAL).unwrap().kind, Kind::Local);
        assert_eq!(c.container_of(42), LOCAL);
        assert_eq!(c.members(LOCAL, &[1, 2, 3]).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn contacts_move_between_containers_and_each_is_in_exactly_one() {
        let mut c = Containers::default();
        let acct = c.add("Work account", Kind::Account).unwrap();
        assert!(c.assign(2, acct).unwrap());
        assert!(!c.assign(2, acct).unwrap(), "moving where it already is changes nothing");
        assert_eq!(c.members(acct, &[1, 2, 3]).unwrap(), vec![2]);
        assert_eq!(c.members(LOCAL, &[1, 2, 3]).unwrap(), vec![1, 3]);
        assert!(c.assign(2, LOCAL).unwrap());
        assert_eq!(c.members(acct, &[1, 2, 3]).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn removing_a_container_sends_its_contacts_home_and_never_deletes_them() {
        let mut c = Containers::default();
        let acct = c.add("Work", Kind::Account).unwrap();
        c.assign(5, acct).unwrap();
        c.assign(6, acct).unwrap();
        assert_eq!(c.remove(acct).unwrap(), vec![5, 6]);
        assert_eq!(c.container_of(5), LOCAL);
        assert_eq!(c.remove(acct), Err(Error::Unknown));
        assert_eq!(c.assign(5, acct), Err(Error::Unknown));
    }

    #[test]
    fn the_local_container_stays_and_is_the_only_one() {
        let mut c = Containers::default();
        assert_eq!(c.remove(LOCAL), Err(Error::LocalIsFixed));
        assert_eq!(c.add("Second local", Kind::Local), Err(Error::LocalIsFixed));
        c.rename(LOCAL, "This Mac-like machine").unwrap();
        assert_eq!(c.get(LOCAL).unwrap().name, "This Mac-like machine");
    }

    #[test]
    fn names_are_checked_and_identifiers_are_never_reused() {
        let mut c = Containers::default();
        assert_eq!(c.add("   ", Kind::Other), Err(Error::EmptyName));
        assert_eq!(c.add(&"x".repeat(MAX_NAME + 1), Kind::Other), Err(Error::NameTooLong));
        let a = c.add("A", Kind::Other).unwrap();
        c.remove(a).unwrap();
        assert!(c.add("B", Kind::Other).unwrap() > a);
    }

    #[test]
    fn the_sidecar_round_trips_names_kinds_and_members() {
        let mut c = Containers::default();
        let a = c.add("Work \\ account", Kind::Account).unwrap();
        let b = c.add("Imported", Kind::Other).unwrap();
        c.rename(LOCAL, "Here").unwrap();
        c.assign(3, a).unwrap();
        c.assign(9, b).unwrap();
        let text = render(&c);
        let back = parse(&text).unwrap();
        assert_eq!(back, c);
        assert_eq!(render(&back), text);
    }

    #[test]
    fn a_restored_set_never_reuses_an_identifier() {
        let mut c = Containers::default();
        let a = c.add("A", Kind::Other).unwrap();
        c.remove(a).unwrap();
        let mut back = parse(&render(&c)).unwrap();
        assert_eq!(back.add("B", Kind::Other).unwrap(), a + 1);
    }

    #[test]
    fn a_foreign_or_inconsistent_sidecar_is_refused_and_a_missing_one_is_the_default() {
        assert!(parse("junk").is_err());
        assert!(parse(&format!("{HEADER}\nlast\t3\nmember\t7\t9\n")).is_err());
        assert!(parse(&format!("{HEADER}\nlast\t3\ncontainer\t1\taccount\tX\n")).is_err());
        assert!(parse(&format!("{HEADER}\ncontainer\t2\taccount\tX\n")).is_err());
        assert_eq!(load(Path::new("/nonexistent/contact-containers.tsv")), Containers::default());
    }

    #[test]
    fn a_deleted_contact_leaves_its_container() {
        let mut c = Containers::default();
        let a = c.add("A", Kind::Account).unwrap();
        c.assign(4, a).unwrap();
        assert!(c.forget_contact(4));
        assert!(!c.forget_contact(4));
        assert_eq!(c.container_of(4), LOCAL);
    }

    #[test]
    fn the_sidecar_sits_beside_the_contacts_file() {
        assert_eq!(path_for(Path::new("/data/Contacts/contacts.tsv")), PathBuf::from("/data/Contacts/contact-containers.tsv"));
    }
}
