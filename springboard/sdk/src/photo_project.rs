//! Photo projects for apps (the PhotoKit `PHProjectChangeRequest` surface): a request names the sections and assets to
//! add, rename, move or remove, and performs them on a project as one unit. The model is the UI project model
//! (`bifrost_ui::photo_project`); the Photos app does not yet host project extensions, so an app keeps its own copy.

use crate::controls::photo_project::{Change, Error, Project};
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The wire object of one change: `{kind, ...}`, read by the shell's `project` method.
pub fn change_to_json(change: &Change) -> Json {
    let strs = |v: &[String]| Json::Arr(v.iter().map(|s| Json::Str(s.clone())).collect());
    let s = |k: &str, v: &str| (k.to_string(), Json::Str(v.to_string()));
    let fields: Vec<(String, Json)> = match change {
        Change::AddSection { id, title } => vec![("kind".into(), Json::Str("add_section".into())), s("id", id), s("title", title)],
        Change::RenameSection { id, title } => vec![("kind".into(), Json::Str("rename_section".into())), s("id", id), s("title", title)],
        Change::RemoveSection { id } => vec![("kind".into(), Json::Str("remove_section".into())), s("id", id)],
        Change::MoveSection { id, to } => vec![("kind".into(), Json::Str("move_section".into())), s("id", id), ("to".into(), Json::Num(*to as f64))],
        Change::AddAssets { section, assets } => vec![("kind".into(), Json::Str("add_assets".into())), s("section", section), ("assets".into(), strs(assets))],
        Change::RemoveAssets { section, assets } => vec![("kind".into(), Json::Str("remove_assets".into())), s("section", section), ("assets".into(), strs(assets))],
    };
    Json::Obj(fields)
}

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

    /// Sends the changes to the shell for the app's project `project_id` (the `project` call, `op` `apply`). The reply
    /// is the project after the changes, or an error that changed nothing.
    pub fn send(&self, ctx: &Ctx, project_id: &str) -> u32 {
        let changes = self.changes.iter().map(change_to_json).collect();
        service::call(ctx, crate::photo_library::SERVICE, "project", &project_params("apply", project_id, vec![("changes", Json::Arr(changes))]))
    }

    /// Asks the shell to create the app's project `project_id` with `title` (the `project` call, `op` `create`).
    pub fn create_project(ctx: &Ctx, project_id: &str, title: &str) -> u32 {
        service::call(ctx, crate::photo_library::SERVICE, "project", &project_params("create", project_id, vec![("title", Json::Str(title.into()))]))
    }
}

fn project_params(op: &str, project_id: &str, extra: Vec<(&str, Json)>) -> Json {
    let mut fields = vec![("op".to_string(), Json::Str(op.into())), ("id".to_string(), Json::Str(project_id.into()))];
    fields.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
    Json::Obj(fields)
}

/// The project a `project` reply describes: its title and the sections in order, each with its assets.
pub fn decode_project(reply: &Json) -> Option<(String, Vec<(String, Vec<String>)>)> {
    let sections = reply
        .get("sections")?
        .as_array()?
        .iter()
        .map(|s| {
            let assets = s.get("assets")?.as_array()?.iter().map(|a| a.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?;
            Some((s.str_field("title")?, assets))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((reply.str_field("title")?, sections))
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

    #[test]
    fn changes_encode_with_their_kind_for_the_shell() {
        let j = change_to_json(&Change::MoveSection { id: "s1".into(), to: 2 });
        assert_eq!(j.str_field("kind").as_deref(), Some("move_section"));
        assert_eq!(j.get("to").and_then(Json::as_u64), Some(2));
        let a = change_to_json(&Change::AddAssets { section: "s1".into(), assets: vec!["PH-1".into()] });
        assert_eq!(a.get("assets").and_then(Json::as_array).map(|x| x.len()), Some(1));
    }

    #[test]
    fn a_project_reply_decodes_to_title_and_sections() {
        let reply = bifrost_core::json::parse(r#"{"id":"book","title":"Trip","sections":[{"id":"s1","title":"Day one","assets":["PH-1"]}]}"#).unwrap();
        let (title, sections) = decode_project(&reply).unwrap();
        assert_eq!(title, "Trip");
        assert_eq!(sections, vec![("Day one".to_string(), vec!["PH-1".to_string()])]);
        assert!(decode_project(&Json::Null).is_none());
    }
}
