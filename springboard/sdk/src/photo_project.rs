//! Photo projects for apps (the PhotoKit `PHProjectChangeRequest` surface): a request names the sections and assets to
//! add, rename, move or remove, and performs them on a project as one unit. The model is the UI project model
//! (`bifrost_ui::photo_project`); the Photos app does not yet host project extensions, so an app keeps its own copy.

use crate::controls::photo_project::{Change, Error, Project};

/// `PHProjectChangeRequest`: a builder of changes, applied all together by [`PHProjectChangeRequest::perform`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PHProjectChangeRequest {
    changes: Vec<Change>,
}

impl PHProjectChangeRequest {
    pub fn new() -> PHProjectChangeRequest {
        PHProjectChangeRequest::default()
    }

    pub fn add_section(mut self, id: &str, title: &str) -> Self {
        self.changes.push(Change::AddSection { id: id.into(), title: title.into() });
        self
    }

    pub fn rename_section(mut self, id: &str, title: &str) -> Self {
        self.changes.push(Change::RenameSection { id: id.into(), title: title.into() });
        self
    }

    pub fn remove_section(mut self, id: &str) -> Self {
        self.changes.push(Change::RemoveSection { id: id.into() });
        self
    }

    /// `to` is the position the section takes once it is taken out of its old place.
    pub fn move_section(mut self, id: &str, to: usize) -> Self {
        self.changes.push(Change::MoveSection { id: id.into(), to });
        self
    }

    pub fn add_assets(mut self, section: &str, assets: &[&str]) -> Self {
        self.changes.push(Change::AddAssets { section: section.into(), assets: assets.iter().map(|a| a.to_string()).collect() });
        self
    }

    pub fn remove_assets(mut self, section: &str, assets: &[&str]) -> Self {
        self.changes.push(Change::RemoveAssets { section: section.into(), assets: assets.iter().map(|a| a.to_string()).collect() });
        self
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Applies every change to `project`. Either all apply or the project is left as it was.
    pub fn perform(&self, project: &mut Project) -> Result<(), Error> {
        project.apply(&self.changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_builds_the_changes_in_order() {
        let r = PHProjectChangeRequest::new().add_section("s1", "Trip").add_assets("s1", &["a", "b"]).move_section("s1", 0);
        assert_eq!(r.changes().len(), 3);
        assert_eq!(r.changes()[1], Change::AddAssets { section: "s1".into(), assets: vec!["a".into(), "b".into()] });
    }

    #[test]
    fn a_request_applies_to_a_project() {
        let mut project = Project::new("book", "Holiday").unwrap();
        PHProjectChangeRequest::new().add_section("s1", "Day one").add_assets("s1", &["a"]).perform(&mut project).unwrap();
        assert_eq!(project.section("s1").unwrap().assets(), &["a".to_string()]);
    }

    #[test]
    fn a_failing_request_leaves_the_project_unchanged() {
        let mut project = Project::new("book", "Holiday").unwrap();
        let before = project.clone();
        let r = PHProjectChangeRequest::new().add_section("s1", "Day one").remove_section("missing");
        assert!(r.perform(&mut project).is_err());
        assert_eq!(project, before);
    }
}
