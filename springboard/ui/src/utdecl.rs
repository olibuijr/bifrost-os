//! App-declared file types: the types an app exports (its own formats) and the types it imports from others.
//! The declarations are checked against the built-in table in `uttype`, so Files, pickers and drag and drop
//! read one conformance tree. Pure data and checks; the shell reads the declarations from the app manifest.
//! Identifiers in the `public.` and `dyn.` namespaces belong to the system and are refused.

use crate::uttype;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The app owns this format and opens it.
    Exported,
    /// The app reads a type that another app owns.
    Imported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub id: String,
    pub direction: Direction,
    /// Parent types: declared identifiers or built-in identifiers such as `public.plain-text`.
    pub conforms_to: Vec<String>,
    /// Lowercase extensions without the dot, such as `bnb`.
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    /// Shown in Files' Kind column. Empty falls back to the built-in description, or to nothing.
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclError {
    /// Not reverse-DNS form: lowercase segments joined by dots, at least two of them.
    BadIdentifier(String),
    /// In a namespace the system owns.
    ReservedIdentifier(String),
    Duplicate(String),
    /// Extensions are lowercase ASCII letters and digits, without a dot.
    BadExtension(String),
    UnknownParent { id: String, parent: String },
    Cycle(String),
}

/// The checked set of declarations; built once at app launch.
#[derive(Clone, Debug, Default)]
pub struct Declared {
    decls: Vec<Declaration>,
}

fn check_id(id: &str) -> Result<(), DeclError> {
    if id.starts_with("public.") || id.starts_with("dyn.") {
        return Err(DeclError::ReservedIdentifier(id.into()));
    }
    let ok = id.contains('.')
        && id.split('.').all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    if ok { Ok(()) } else { Err(DeclError::BadIdentifier(id.into())) }
}

fn valid_ext(e: &str) -> bool {
    !e.is_empty() && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The extension of a file name or a bare extension, lowercased. `Report.BNB` gives `bnb`.
fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(_, e)| e).to_ascii_lowercase()
}

impl Declared {
    /// Checks identifiers, extensions, parents and cycles. Parents must be declared here or built in.
    pub fn new(decls: Vec<Declaration>) -> Result<Declared, DeclError> {
        for (i, d) in decls.iter().enumerate() {
            check_id(&d.id)?;
            if decls[..i].iter().any(|o| o.id == d.id) {
                return Err(DeclError::Duplicate(d.id.clone()));
            }
            if let Some(bad) = d.extensions.iter().find(|e| !valid_ext(e)) {
                return Err(DeclError::BadExtension(bad.clone()));
            }
        }
        let set = Declared { decls };
        for d in &set.decls {
            for p in &d.conforms_to {
                if set.find(p).is_none() && uttype::description(p).is_empty() {
                    return Err(DeclError::UnknownParent { id: d.id.clone(), parent: p.clone() });
                }
            }
        }
        for d in &set.decls {
            if d.conforms_to.iter().any(|p| p == &d.id || set.reaches(p, &d.id, 1)) {
                return Err(DeclError::Cycle(d.id.clone()));
            }
        }
        Ok(set)
    }

    fn find(&self, id: &str) -> Option<&Declaration> {
        self.decls.iter().find(|d| d.id == id)
    }

    /// True when `target` is `from` or one of its declared ancestors. The depth bound stops a
    /// malformed chain from looping; `new` rejects cycles, so the bound is only a guard.
    fn reaches(&self, from: &str, target: &str, depth: usize) -> bool {
        if from == target {
            return true;
        }
        if depth > self.decls.len() {
            return true;
        }
        match self.find(from) {
            Some(d) => d.conforms_to.iter().any(|p| self.reaches(p, target, depth + 1)),
            None => false,
        }
    }

    /// Conformance across declared and built-in types: true when `id` is `ancestor` or descends from it.
    pub fn conforms(&self, id: &str, ancestor: &str) -> bool {
        self.conforms_at(id, ancestor, 0)
    }

    fn conforms_at(&self, id: &str, ancestor: &str, depth: usize) -> bool {
        if id == ancestor {
            return true;
        }
        if depth > self.decls.len() {
            return false;
        }
        match self.find(id) {
            Some(d) => d.conforms_to.iter().any(|p| self.conforms_at(p, ancestor, depth + 1)),
            None => uttype::conforms(id, ancestor),
        }
    }

    /// The type that owns an extension. Exported declarations win over imported ones, which win over built-in types.
    pub fn type_for_extension(&self, name: &str) -> Option<String> {
        let ext = ext_of(name);
        for dir in [Direction::Exported, Direction::Imported] {
            if let Some(d) = self.decls.iter().find(|d| d.direction == dir && d.extensions.iter().any(|x| *x == ext)) {
                return Some(d.id.clone());
            }
        }
        uttype::type_for_extension(name).map(|s| s.to_string())
    }

    /// The type for a MIME type (parameters ignored, case-insensitive), declared types first.
    pub fn type_for_mime(&self, mime: &str) -> Option<String> {
        let essence = mime.split(';').next()?.trim().to_ascii_lowercase();
        if essence.is_empty() {
            return None;
        }
        for dir in [Direction::Exported, Direction::Imported] {
            if let Some(d) = self.decls.iter().find(|d| d.direction == dir && d.mime_types.iter().any(|m| m.eq_ignore_ascii_case(&essence))) {
                return Some(d.id.clone());
            }
        }
        uttype::type_for_mime(&essence).map(|s| s.to_string())
    }

    /// The Kind-column text for a type: the app's own description, else the built-in one.
    pub fn description(&self, id: &str) -> String {
        match self.find(id) {
            Some(d) if !d.description.is_empty() => d.description.clone(),
            _ => uttype::description(id).to_string(),
        }
    }

    /// Every extension a type claims: its declared extensions, else the built-in list.
    pub fn extensions(&self, id: &str) -> Vec<String> {
        match self.find(id) {
            Some(d) => d.extensions.clone(),
            None => uttype::extensions(id).iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(id: &str, dir: Direction, parents: &[&str], exts: &[&str]) -> Declaration {
        Declaration {
            id: id.into(),
            direction: dir,
            conforms_to: parents.iter().map(|s| s.to_string()).collect(),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
            mime_types: vec![],
            description: String::new(),
        }
    }

    #[test]
    fn declared_type_conforms_through_builtin_parents() {
        let set = Declared::new(vec![decl("com.example.notebook", Direction::Exported, &["public.plain-text"], &["bnb"])]).unwrap();
        assert!(set.conforms("com.example.notebook", "public.plain-text"));
        // plain-text -> text -> data, all built in.
        assert!(set.conforms("com.example.notebook", "public.text"));
        assert!(set.conforms("com.example.notebook", "public.data"));
        assert!(!set.conforms("com.example.notebook", "public.url"));
        assert!(!set.conforms("public.plain-text", "com.example.notebook"));
    }

    #[test]
    fn extension_lookup_is_case_insensitive_and_falls_back_to_builtin() {
        let set = Declared::new(vec![decl("com.example.notebook", Direction::Exported, &["public.data"], &["bnb"])]).unwrap();
        assert_eq!(set.type_for_extension("Report.BNB").as_deref(), Some("com.example.notebook"));
        assert_eq!(set.type_for_extension("notes.txt").as_deref(), Some("public.plain-text"));
        assert_eq!(set.type_for_extension("archive.unknownext"), None);
    }

    #[test]
    fn exported_declaration_wins_over_imported_for_one_extension() {
        let set = Declared::new(vec![
            decl("com.other.format", Direction::Imported, &["public.data"], &["bnb"]),
            decl("com.example.notebook", Direction::Exported, &["public.data"], &["bnb"]),
        ])
        .unwrap();
        assert_eq!(set.type_for_extension("a.bnb").as_deref(), Some("com.example.notebook"));
    }

    #[test]
    fn identifiers_must_be_reverse_dns_and_outside_system_namespaces() {
        let bad = |id: &str| Declared::new(vec![decl(id, Direction::Exported, &["public.data"], &[])]);
        assert_eq!(bad("public.thing").unwrap_err(), DeclError::ReservedIdentifier("public.thing".into()));
        assert_eq!(bad("dyn.abc").unwrap_err(), DeclError::ReservedIdentifier("dyn.abc".into()));
        assert_eq!(bad("NoDots").unwrap_err(), DeclError::BadIdentifier("NoDots".into()));
        assert_eq!(bad("com..example").unwrap_err(), DeclError::BadIdentifier("com..example".into()));
        assert!(bad("com.example.ok-1").is_ok());
    }

    #[test]
    fn duplicates_unknown_parents_and_cycles_are_refused() {
        let dup = Declared::new(vec![
            decl("com.example.a", Direction::Exported, &["public.data"], &[]),
            decl("com.example.a", Direction::Imported, &["public.data"], &[]),
        ]);
        assert_eq!(dup.unwrap_err(), DeclError::Duplicate("com.example.a".into()));

        let unknown = Declared::new(vec![decl("com.example.a", Direction::Exported, &["com.example.missing"], &[])]);
        assert_eq!(
            unknown.unwrap_err(),
            DeclError::UnknownParent { id: "com.example.a".into(), parent: "com.example.missing".into() }
        );

        let cycle = Declared::new(vec![
            decl("com.example.a", Direction::Exported, &["com.example.b"], &[]),
            decl("com.example.b", Direction::Exported, &["com.example.a"], &[]),
        ]);
        assert!(matches!(cycle.unwrap_err(), DeclError::Cycle(_)));
    }

    #[test]
    fn bad_extensions_are_refused_and_description_falls_back() {
        let dotted = Declared::new(vec![decl("com.example.a", Direction::Exported, &["public.data"], &[".bnb"])]);
        assert_eq!(dotted.unwrap_err(), DeclError::BadExtension(".bnb".into()));

        let mut d = decl("com.example.a", Direction::Exported, &["public.plain-text"], &[]);
        let set = Declared::new(vec![d.clone()]).unwrap();
        assert_eq!(set.description("com.example.a"), "");
        assert_eq!(set.description("public.plain-text"), "Plain Text");
        d.description = "Notebook".into();
        let set = Declared::new(vec![d]).unwrap();
        assert_eq!(set.description("com.example.a"), "Notebook");
    }
}
