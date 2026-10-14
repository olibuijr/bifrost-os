//! Photo projects: a titled list of sections, and each section an ordered list of asset identifiers. This is the model
//! behind a photo book or calendar. Changes are values ([`Change`]) and [`Project::apply`] runs a whole list as one unit:
//! either every change applies, or the project is left as it was. The model is pure. The photo library owns the assets,
//! and no extension host is wired to it yet.

use std::fmt;

/// The most sections one project may hold.
pub const MAX_SECTIONS: usize = 200;
/// The longest identifier or title, in bytes, after trimming.
pub const MAX_TEXT: usize = 255;

/// Why a project or a change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An identifier or title is empty after trimming, or longer than [`MAX_TEXT`] bytes.
    InvalidText(String),
    /// A section with this identifier already exists.
    DuplicateSection(String),
    /// No section has this identifier.
    UnknownSection(String),
    /// The section already holds this asset.
    DuplicateAsset { section: String, asset: String },
    /// The section does not hold this asset.
    UnknownAsset { section: String, asset: String },
    /// Adding a section would go past [`MAX_SECTIONS`].
    TooManySections,
    /// A move names a position past the last section.
    PositionOutOfRange { position: usize, count: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidText(text) => write!(f, "invalid name {text:?}"),
            Error::DuplicateSection(id) => write!(f, "a section named {id} already exists"),
            Error::UnknownSection(id) => write!(f, "no section named {id}"),
            Error::DuplicateAsset { section, asset } => write!(f, "section {section} already holds {asset}"),
            Error::UnknownAsset { section, asset } => write!(f, "section {section} does not hold {asset}"),
            Error::TooManySections => write!(f, "a project holds at most {MAX_SECTIONS} sections"),
            Error::PositionOutOfRange { position, count } => {
                write!(f, "position {position} is past the last of {count} sections")
            }
        }
    }
}

impl std::error::Error for Error {}

/// One change to a project. A section is named by its identifier; `to` is the position the section takes once it
/// has been taken out of its old place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    AddSection { id: String, title: String },
    RenameSection { id: String, title: String },
    RemoveSection { id: String },
    MoveSection { id: String, to: usize },
    AddAssets { section: String, assets: Vec<String> },
    RemoveAssets { section: String, assets: Vec<String> },
}

/// A section of a project: its identifier, its title and the assets it holds in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    id: String,
    title: String,
    assets: Vec<String>,
}

impl Section {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn assets(&self) -> &[String] {
        &self.assets
    }

    pub fn contains(&self, asset: &str) -> bool {
        self.assets.iter().any(|a| a == asset)
    }
}

/// A photo project: its identifier, its title and its sections in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    id: String,
    title: String,
    sections: Vec<Section>,
}

impl Project {
    /// A project with no sections. The identifier and title are trimmed and checked.
    pub fn new(id: &str, title: &str) -> Result<Project, Error> {
        Ok(Project { id: text(id)?, title: text(title)?, sections: Vec::new() })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The section with this identifier, if the project has one.
    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.id == id)
    }

    /// Applies `changes` in order. If one of them fails, the project keeps its state from before the call.
    pub fn apply(&mut self, changes: &[Change]) -> Result<(), Error> {
        let mut next = self.clone();
        for change in changes {
            next.apply_one(change)?;
        }
        *self = next;
        Ok(())
    }

    fn apply_one(&mut self, change: &Change) -> Result<(), Error> {
        match change {
            Change::AddSection { id, title } => {
                let id = text(id)?;
                let title = text(title)?;
                if self.find(&id).is_some() {
                    return Err(Error::DuplicateSection(id));
                }
                if self.sections.len() >= MAX_SECTIONS {
                    return Err(Error::TooManySections);
                }
                self.sections.push(Section { id, title, assets: Vec::new() });
            }
            Change::RenameSection { id, title } => {
                let i = self.position(id)?;
                self.sections[i].title = text(title)?;
            }
            Change::RemoveSection { id } => {
                let i = self.position(id)?;
                self.sections.remove(i);
            }
            Change::MoveSection { id, to } => {
                let i = self.position(id)?;
                if *to >= self.sections.len() {
                    return Err(Error::PositionOutOfRange { position: *to, count: self.sections.len() });
                }
                let section = self.sections.remove(i);
                self.sections.insert(*to, section);
            }
            Change::AddAssets { section, assets } => {
                let i = self.position(section)?;
                for asset in assets {
                    let asset = text(asset)?;
                    if self.sections[i].contains(&asset) {
                        return Err(Error::DuplicateAsset { section: self.sections[i].id.clone(), asset });
                    }
                    self.sections[i].assets.push(asset);
                }
            }
            Change::RemoveAssets { section, assets } => {
                let i = self.position(section)?;
                for asset in assets {
                    let asset = text(asset)?;
                    match self.sections[i].assets.iter().position(|a| *a == asset) {
                        Some(j) => {
                            self.sections[i].assets.remove(j);
                        }
                        None => return Err(Error::UnknownAsset { section: self.sections[i].id.clone(), asset }),
                    }
                }
            }
        }
        Ok(())
    }

    fn find(&self, id: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.id == id)
    }

    fn position(&self, id: &str) -> Result<usize, Error> {
        let id = text(id)?;
        self.find(&id).ok_or(Error::UnknownSection(id))
    }
}

/// Trims a name and checks its length: a name is never empty and never longer than [`MAX_TEXT`] bytes.
fn text(raw: &str) -> Result<String, Error> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TEXT {
        return Err(Error::InvalidText(raw.to_string()));
    }
    Ok(trimmed.to_string())
}
