//! Asset resolution: turns the name an app uses for a resource into the location that holds it.
//! Three resolvers cover the usual ways a resource is named: a path under a base directory (`PathResolver`), a path
//! relative to the document that refers to it (`RelativeResolver`), and a resource inside an app bundle
//! (`BundleResolver`). The work is string handling only; nothing is read from disk, so the caller supplies the list
//! of bundle resources.

use std::collections::BTreeSet;

/// Maps a resource name to the location that holds it. `None` means this resolver does not know the name.
pub trait AssetResolver {
    fn resolve(&self, name: &str) -> Option<String>;
}

/// Resolves names under one base directory. An absolute name is taken as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathResolver {
    base: String,
}

impl PathResolver {
    pub fn new(base: &str) -> Self {
        PathResolver { base: base.to_string() }
    }
}

impl AssetResolver for PathResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        if name.starts_with('/') {
            return Some(clean(name));
        }
        Some(clean(&join(&self.base, name)))
    }
}

/// Resolves names next to the document that refers to them. A name is read from the directory that holds the
/// document. An absolute name is taken as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativeResolver {
    document: String,
}

impl RelativeResolver {
    pub fn new(document: &str) -> Self {
        RelativeResolver { document: document.to_string() }
    }

    /// The directory that holds the document: `/` for a file at the root, empty for a bare file name.
    fn directory(&self) -> &str {
        match self.document.rfind('/') {
            Some(i) => &self.document[..i.max(1)],
            None => "",
        }
    }
}

impl AssetResolver for RelativeResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        if name.starts_with('/') {
            return Some(clean(name));
        }
        Some(clean(&join(self.directory(), name)))
    }
}

/// Resolves names among the resources of one app bundle. Only names in the bundle's resource list resolve; the
/// location is the bundle root, then the resource directory, then the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleResolver {
    root: String,
    resource_dir: String,
    resources: BTreeSet<String>,
}

impl BundleResolver {
    pub fn new<I, S>(root: &str, resource_dir: &str, resources: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        BundleResolver {
            root: root.to_string(),
            resource_dir: resource_dir.to_string(),
            resources: resources.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether the bundle lists this resource.
    pub fn contains(&self, name: &str) -> bool {
        self.resources.contains(name)
    }
}

impl AssetResolver for BundleResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        if !self.contains(name) {
            return None;
        }
        Some(clean(&join(&join(&self.root, &self.resource_dir), name)))
    }
}

/// Asks each resolver in turn and returns the first location found.
pub fn resolve_first(resolvers: &[&dyn AssetResolver], name: &str) -> Option<String> {
    resolvers.iter().find_map(|r| r.resolve(name))
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Removes empty and `.` segments and folds `..` into the segment before it. An absolute path cannot climb above
/// its root. An empty relative path becomes `.`.
fn clean(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                let climbs = matches!(parts.last(), Some(last) if *last != "..");
                if climbs {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let body = parts.join("/");
    if absolute {
        format!("/{body}")
    } else if body.is_empty() {
        ".".to_string()
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_resolver_joins_the_name_onto_its_base() {
        let r = PathResolver::new("/home/app/data");
        assert_eq!(r.resolve("models/cube.obj").as_deref(), Some("/home/app/data/models/cube.obj"));
    }

    #[test]
    fn path_resolver_keeps_absolute_names_and_cleans_dots() {
        let r = PathResolver::new("/home/app");
        assert_eq!(r.resolve("/srv/./x//y.txt").as_deref(), Some("/srv/x/y.txt"));
        assert_eq!(r.resolve("a/../b/c.txt").as_deref(), Some("/home/app/b/c.txt"));
    }

    #[test]
    fn dot_dot_cannot_climb_above_an_absolute_root() {
        assert_eq!(clean("/../../x"), "/x");
        assert_eq!(clean("../../x"), "../../x");
    }

    #[test]
    fn empty_and_nul_names_do_not_resolve() {
        let r = PathResolver::new("/base");
        assert_eq!(r.resolve(""), None);
        assert_eq!(r.resolve("a\0b"), None);
    }

    #[test]
    fn relative_resolver_reads_from_the_document_directory() {
        let r = RelativeResolver::new("/scenes/main/scene.usda");
        assert_eq!(r.resolve("textures/wood.png").as_deref(), Some("/scenes/main/textures/wood.png"));
        assert_eq!(r.resolve("../shared/cube.obj").as_deref(), Some("/scenes/shared/cube.obj"));
    }

    #[test]
    fn relative_resolver_handles_root_and_bare_documents() {
        assert_eq!(RelativeResolver::new("/scene.obj").resolve("tex.png").as_deref(), Some("/tex.png"));
        assert_eq!(RelativeResolver::new("scene.obj").resolve("tex.png").as_deref(), Some("tex.png"));
    }

    #[test]
    fn relative_resolver_keeps_absolute_names() {
        let r = RelativeResolver::new("/a/b.obj");
        assert_eq!(r.resolve("/etc/x.png").as_deref(), Some("/etc/x.png"));
    }

    #[test]
    fn bundle_resolver_finds_only_listed_resources() {
        let r = BundleResolver::new("/Apps/Viewer.app", "Resources", ["cube.obj", "icon.png"]);
        assert!(r.contains("cube.obj"));
        assert_eq!(r.resolve("cube.obj").as_deref(), Some("/Apps/Viewer.app/Resources/cube.obj"));
        assert_eq!(r.resolve("missing.obj"), None);
    }

    #[test]
    fn resolve_first_takes_the_first_resolver_that_knows_the_name() {
        let bundle = BundleResolver::new("/Apps/Viewer.app", "Resources", ["cube.obj"]);
        let path = PathResolver::new("/tmp/work");
        let chain: [&dyn AssetResolver; 2] = [&bundle, &path];
        assert_eq!(resolve_first(&chain, "cube.obj").as_deref(), Some("/Apps/Viewer.app/Resources/cube.obj"));
        assert_eq!(resolve_first(&chain, "other.obj").as_deref(), Some("/tmp/work/other.obj"));
        let only: [&dyn AssetResolver; 1] = [&bundle];
        assert_eq!(resolve_first(&only, "other.obj"), None);
    }
}
