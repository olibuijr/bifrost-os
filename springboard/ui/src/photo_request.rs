//! A batch of changes to a photo library, planned before anything is written: create assets from files, set or clear
//! favourites, and move assets to the trash. The batch names each asset at most once and checks it against the
//! listing the caller has. A new file gets a name that clashes with neither the library nor an earlier file in the
//! batch (`IMG.jpg` becomes `IMG-2.jpg`). `plan` returns what to write; the shell does the writing.

use crate::photo_media::{self, Kind};
use std::collections::HashSet;

/// One requested change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Add a file, named as the caller gives it.
    Create(String),
    /// Set or clear the favourite flag of the asset with this identifier.
    SetFavourite(String, bool),
    /// Move the asset with this identifier to the trash.
    Trash(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A file name is empty or has a path separator in it.
    BadName(String),
    /// The file is not an image, a movie or audio, so the library cannot hold it.
    UnsupportedType(String),
    /// The asset identifier is not in the library listing.
    UnknownAsset(String),
    /// An asset is named by two operations in one batch.
    Repeated(String),
}

/// A file to create, with the name it gets in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub requested: String,
    pub name: String,
    pub kind: Kind,
}

/// What the batch does, once checked.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Plan {
    pub created: Vec<Created>,
    pub favourites: Vec<(String, bool)>,
    pub trashed: Vec<String>,
}

/// A batch under construction. Operations keep the order they were added in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Request {
    ops: Vec<Op>,
}

impl Request {
    pub fn new() -> Self {
        Request { ops: Vec::new() }
    }

    pub fn create(&mut self, name: &str) -> &mut Self {
        self.ops.push(Op::Create(name.to_string()));
        self
    }

    pub fn set_favourite(&mut self, id: &str, on: bool) -> &mut Self {
        self.ops.push(Op::SetFavourite(id.to_string(), on));
        self
    }

    pub fn trash(&mut self, id: &str) -> &mut Self {
        self.ops.push(Op::Trash(id.to_string()));
        self
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Checks the batch against the names already in the library and plans it. Nothing is changed when it fails.
    pub fn plan(&self, library: &[&str]) -> Result<Plan, Error> {
        let known: HashSet<&str> = library.iter().copied().collect();
        let mut taken: HashSet<String> = library.iter().map(|s| s.to_string()).collect();
        let mut named: HashSet<&str> = HashSet::new();
        let mut plan = Plan::default();
        for op in &self.ops {
            match op {
                Op::Create(requested) => {
                    if requested.is_empty() || requested.contains('/') {
                        return Err(Error::BadName(requested.clone()));
                    }
                    let kind = photo_media::kind_for_name(requested);
                    if kind == Kind::Unknown {
                        return Err(Error::UnsupportedType(requested.clone()));
                    }
                    let name = unique_name(requested, &taken);
                    taken.insert(name.clone());
                    plan.created.push(Created { requested: requested.clone(), name, kind });
                }
                Op::SetFavourite(id, on) => {
                    check_asset(id, &known, &mut named)?;
                    plan.favourites.push((id.clone(), *on));
                }
                Op::Trash(id) => {
                    check_asset(id, &known, &mut named)?;
                    plan.trashed.push(id.clone());
                }
            }
        }
        Ok(plan)
    }
}

fn check_asset<'a>(id: &'a str, known: &HashSet<&str>, named: &mut HashSet<&'a str>) -> Result<(), Error> {
    if !known.contains(id) {
        return Err(Error::UnknownAsset(id.to_string()));
    }
    if !named.insert(id) {
        return Err(Error::Repeated(id.to_string()));
    }
    Ok(())
}

/// The first of `name`, `stem-2.ext`, `stem-3.ext`, ... that is not in `taken`. A dot at the start of a name does
/// not start an extension, so `.hidden` gets `.hidden-2`.
pub fn unique_name(name: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}{ext}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_free_name_is_kept_and_a_taken_one_gets_a_number() {
        assert_eq!(unique_name("IMG.jpg", &taken(&["other.jpg"])), "IMG.jpg");
        assert_eq!(unique_name("IMG.jpg", &taken(&["IMG.jpg"])), "IMG-2.jpg");
        assert_eq!(unique_name("IMG.jpg", &taken(&["IMG.jpg", "IMG-2.jpg"])), "IMG-3.jpg");
    }

    #[test]
    fn a_name_without_an_extension_or_with_a_leading_dot_is_numbered_at_the_end() {
        assert_eq!(unique_name("notes", &taken(&["notes"])), "notes-2");
        assert_eq!(unique_name(".hidden", &taken(&[".hidden"])), ".hidden-2");
    }

    #[test]
    fn creations_clash_with_the_library_and_with_each_other() {
        let mut req = Request::new();
        req.create("a.jpg").create("a.jpg").create("b.jpg");
        let plan = req.plan(&["a.jpg"]).unwrap();
        let names: Vec<&str> = plan.created.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a-2.jpg", "a-3.jpg", "b.jpg"]);
        assert_eq!(plan.created[0].requested, "a.jpg");
        assert_eq!(plan.created[0].kind, Kind::Image);
    }

    #[test]
    fn a_file_type_the_library_cannot_hold_is_refused() {
        let mut req = Request::new();
        req.create("readme.txt");
        assert_eq!(req.plan(&[]), Err(Error::UnsupportedType("readme.txt".to_string())));
    }

    #[test]
    fn a_bad_file_name_is_refused() {
        let mut req = Request::new();
        req.create("");
        assert_eq!(req.plan(&[]), Err(Error::BadName(String::new())));
        let mut slash = Request::new();
        slash.create("dir/a.jpg");
        assert_eq!(slash.plan(&[]), Err(Error::BadName("dir/a.jpg".to_string())));
    }

    #[test]
    fn an_unknown_asset_fails_the_whole_batch() {
        let mut req = Request::new();
        req.create("c.jpg").set_favourite("missing.jpg", true);
        assert_eq!(req.plan(&["a.jpg"]), Err(Error::UnknownAsset("missing.jpg".to_string())));
    }

    #[test]
    fn an_asset_named_twice_in_one_batch_is_refused() {
        let mut req = Request::new();
        req.trash("a.jpg").set_favourite("a.jpg", true);
        assert_eq!(req.plan(&["a.jpg"]), Err(Error::Repeated("a.jpg".to_string())));
    }

    #[test]
    fn favourites_and_trash_keep_the_order_they_were_added_in() {
        let mut req = Request::new();
        req.set_favourite("b.jpg", true).trash("a.jpg").set_favourite("c.jpg", false);
        let plan = req.plan(&["a.jpg", "b.jpg", "c.jpg"]).unwrap();
        assert_eq!(plan.favourites, vec![("b.jpg".to_string(), true), ("c.jpg".to_string(), false)]);
        assert_eq!(plan.trashed, vec!["a.jpg".to_string()]);
        assert!(plan.created.is_empty());
        assert_eq!(req.ops().len(), 3);
    }
}
