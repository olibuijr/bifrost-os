//! The `directory` service: the local account directory the shell reads for apps, the shell's side of
//! `usermanagerhelper`. The models are `bifrost_ui::directory_node` (a session of named nodes, each holding records by
//! type and name) and `bifrost_ui::directory_local` (the users of `/etc/passwd` and the groups of `/etc/group`).
//!
//! The session is built once, when the shell starts: a node `/Local` with a child `/Local/Default`, filled from the
//! two system tables. It is read-only for apps. Apps cannot add nodes or records, so the directory shows only what the
//! system tables already show to any program on the machine.
//!
//! Methods (JSON parameters):
//! - `children {path}` answers `{names}`: the immediate subnodes of `path`, in name order. `path` may be empty for
//!   the top level.
//! - `records {path, type?}` answers `{records: [{type, name}]}` for the node, optionally one record type only.
//! - `record {path, type, name}` answers `{type, name, attributes}`: `attributes` maps each attribute name to its list
//!   of text values. A missing record is a service error.
//!
//! Not built: directory servers (LDAP, Active Directory) and the write side. The node a remote service would fill is
//! named but nothing fills it.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::directory_local::{fill_node, read_system_tables};
use bifrost_ui::directory_node::{Error, Node, Session};

/// The service name.
pub const SERVICE: &str = "directory";

/// The node that holds this machine's accounts.
pub const LOCAL_NODE: &str = "/Local/Default";

/// The `directory` service.
pub struct Directory {
    session: Session,
}

impl Directory {
    /// The directory filled from the given system tables (`/etc/passwd` and `/etc/group` text).
    pub fn with_tables(passwd: &str, group: &str) -> Directory {
        let mut session = Session::new();
        let _ = session.add_node("/Local");
        let _ = session.add_node(LOCAL_NODE);
        if let Ok(node) = session.node_mut(LOCAL_NODE) {
            fill_node(node, passwd, group);
        }
        Directory { session }
    }

    /// The directory filled from this machine's system tables.
    pub fn load() -> Directory {
        let (passwd, group) = read_system_tables();
        Directory::with_tables(&passwd, &group)
    }
}

impl Default for Directory {
    fn default() -> Self {
        Directory::load()
    }
}

impl SystemService for Directory {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(&self.session, method, params)
    }
}

/// Runs one read-only method against the session.
pub fn answer(session: &Session, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "children" => {
            let path = params.str_field("path").unwrap_or_default();
            let names = session.subnode_names(&path).map_err(describe)?.into_iter().map(Json::Str).collect();
            Ok(Json::Obj(vec![("names".into(), Json::Arr(names))]))
        }
        "records" => {
            let node = node_at(session, params)?;
            let wanted = params.str_field("type");
            let records = node
                .records()
                .into_iter()
                .filter(|r| wanted.as_deref().is_none_or(|t| r.record_type() == t))
                .map(|r| Json::Obj(vec![("type".into(), Json::Str(r.record_type().to_string())), ("name".into(), Json::Str(r.name().to_string()))]))
                .collect();
            Ok(Json::Obj(vec![("records".into(), Json::Arr(records))]))
        }
        "record" => {
            let node = node_at(session, params)?;
            let kind = params.str_field("type").ok_or("type must be a string")?;
            let name = params.str_field("name").ok_or("name must be a string")?;
            let record = node.record(&kind, &name).ok_or_else(|| format!("no {kind} record named {name}"))?;
            let attributes = record
                .attribute_names()
                .into_iter()
                .map(|attr| {
                    let values = record.values(attr).into_iter().map(Json::Str).collect();
                    (attr.to_string(), Json::Arr(values))
                })
                .collect();
            Ok(Json::Obj(vec![("type".into(), Json::Str(kind.clone())), ("name".into(), Json::Str(name.clone())), ("attributes".into(), Json::Obj(attributes))]))
        }
        _ => Err(format!("directory has no method {method}")),
    }
}

fn node_at<'a>(session: &'a Session, params: &Json) -> Result<&'a Node, String> {
    let path = params.str_field("path").ok_or("path must be a string")?;
    session.node(&path).map_err(describe)
}

fn describe(e: Error) -> String {
    format!("directory: {e:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/sh\nalice:x:1000:1000:Alice:/home/alice:/bin/bash\n";
    const GROUP: &str = "wheel:x:10:alice\n";

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn local_node_lists_the_system_accounts() {
        let dir = Directory::with_tables(PASSWD, GROUP);
        let top = answer(&dir.session, "children", &obj(vec![("path", Json::Str("/Local".into()))])).unwrap();
        assert_eq!(top.get("names").and_then(Json::as_array).map(|a| a.len()), Some(1));
        let users = answer(&dir.session, "records", &obj(vec![("path", Json::Str(LOCAL_NODE.into())), ("type", Json::Str("users".into()))])).unwrap();
        let rows = users.get("records").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.get("type").and_then(Json::as_str) == Some("users")));
    }

    #[test]
    fn one_record_shows_its_attributes() {
        let dir = Directory::with_tables(PASSWD, GROUP);
        let p = obj(vec![("path", Json::Str(LOCAL_NODE.into())), ("type", Json::Str("users".into())), ("name", Json::Str("alice".into()))]);
        let r = answer(&dir.session, "record", &p).unwrap();
        assert!(r.get("attributes").is_some());
        let missing = obj(vec![("path", Json::Str(LOCAL_NODE.into())), ("type", Json::Str("users".into())), ("name", Json::Str("nobody".into()))]);
        assert!(answer(&dir.session, "record", &missing).is_err());
    }

    #[test]
    fn unknown_nodes_and_methods_are_errors() {
        let dir = Directory::with_tables(PASSWD, GROUP);
        assert!(answer(&dir.session, "children", &obj(vec![("path", Json::Str("/Nope".into()))])).is_err());
        assert!(answer(&dir.session, "add_node", &Json::Null).is_err());
    }
}
