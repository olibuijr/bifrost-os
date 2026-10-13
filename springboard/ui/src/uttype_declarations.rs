//! App-declared types wired into the `uttype` lookups: `UTExportedTypeDeclarations` and
//! `UTImportedTypeDeclarations` from the app manifest (`[[exported_types]]`, `[[imported_types]]`) or from
//! Info.plist-shaped entries, and the `UTType` value with `UTType(exportedAs:conformingTo:)`,
//! `UTType(importedAs:conformingTo:)`, `UTType(filenameExtension:conformingTo:)` and
//! `UTType(tag:tagClass:conformingTo:)`. Checking and precedence come from `utdecl::Declared`.
//! The system owns the built-in table: an app cannot take over a built-in extension or MIME type.
//! Only the first MIME type of a declaration is indexed for lookups; a type has one preferred MIME type.
use crate::utdecl::{DeclError, Declaration, Declared, Direction};
use crate::uttype::{self, Entry};
use std::sync::Mutex;

/// `UTTagClass.filenameExtension`.
pub const TAG_FILENAME_EXTENSION: &str = "public.filename-extension";
/// `UTTagClass.mimeType`.
pub const TAG_MIME_TYPE: &str = "public.mime-type";

/// A tag class identifier (`UTTagClass`): the two built-in ones, or any other string such as a pasteboard class.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UtTagClass(pub String);

impl UtTagClass {
    pub fn filename_extension() -> Self {
        Self(TAG_FILENAME_EXTENSION.into())
    }
    pub fn mime_type() -> Self {
        Self(TAG_MIME_TYPE.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A line that is not a table header, a comment or `key = value`.
    Syntax(usize),
    UnknownKey { line: usize, key: String },
    /// A `[[...]]` block without `id`.
    MissingId(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Manifest(ManifestError),
    Decl(DeclError),
}

/// Tags other than extensions and MIME types, per declared type: `(id, class, tags)`.
static EXTRA_TAGS: Mutex<Vec<(String, String, Vec<String>)>> = Mutex::new(Vec::new());

fn extra_tags() -> std::sync::MutexGuard<'static, Vec<(String, String, Vec<String>)>> {
    EXTRA_TAGS.lock().unwrap_or_else(|e| e.into_inner())
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_all(v: &[String]) -> &'static [&'static str] {
    Box::leak(v.iter().map(|s| leak(s)).collect::<Vec<_>>().into_boxed_slice())
}

/// The quoted strings in a value: `"a"` gives one, `["a", "b"]` gives two. `None` when a quote is left open.
fn quoted(value: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_str = false;
    let mut escaped = false;
    for c in value.chars() {
        if in_str {
            if escaped {
                buf.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                out.push(std::mem::take(&mut buf));
                in_str = false;
            } else {
                buf.push(c);
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '#' {
            break;
        }
    }
    if in_str { None } else { Some(out) }
}

/// Parses the `[[exported_types]]` and `[[imported_types]]` blocks of an app manifest. Other tables and keys
/// outside those blocks are skipped. Keys: `id`, `conforms_to`, `extensions`, `mime_types`, `description`.
pub fn parse_manifest(text: &str) -> Result<Vec<Declaration>, ManifestError> {
    let mut out: Vec<(usize, Declaration)> = Vec::new();
    let mut in_block = false;
    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') {
            let name = t.trim_start_matches('[').split(']').next().unwrap_or("").trim();
            let direction = match (t.starts_with("[["), name) {
                (true, "exported_types") => Some(Direction::Exported),
                (true, "imported_types") => Some(Direction::Imported),
                _ => None,
            };
            in_block = direction.is_some();
            if let Some(direction) = direction {
                let d = Declaration { id: String::new(), direction, conforms_to: vec![], extensions: vec![], mime_types: vec![], description: String::new() };
                out.push((line, d));
            }
            continue;
        }
        if !in_block {
            continue;
        }
        let (key, value) = t.split_once('=').ok_or(ManifestError::Syntax(line))?;
        let values = quoted(value).ok_or(ManifestError::Syntax(line))?;
        let d = &mut out.last_mut().ok_or(ManifestError::Syntax(line))?.1;
        match key.trim() {
            "id" => d.id = values.into_iter().next().ok_or(ManifestError::Syntax(line))?,
            "description" => d.description = values.into_iter().next().ok_or(ManifestError::Syntax(line))?,
            "conforms_to" => d.conforms_to = values,
            "extensions" => d.extensions = values.into_iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect(),
            "mime_types" => d.mime_types = values,
            other => return Err(ManifestError::UnknownKey { line, key: other.into() }),
        }
    }
    if let Some((line, _)) = out.iter().find(|(_, d)| d.id.is_empty()) {
        return Err(ManifestError::MissingId(*line));
    }
    Ok(out.into_iter().map(|(_, d)| d).collect())
}

/// One Info.plist type declaration: `UTTypeIdentifier`, `UTTypeConformsTo`, `UTTypeDescription` and
/// `UTTypeTagSpecification` (class to tags, for example `public.filename-extension` to `["bnb"]`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlistDeclaration {
    pub identifier: String,
    pub conforms_to: Vec<String>,
    pub description: String,
    pub tag_specification: Vec<(String, Vec<String>)>,
}

impl PlistDeclaration {
    fn tags(&self, class: &str) -> Vec<String> {
        self.tag_specification.iter().filter(|(c, _)| c == class).flat_map(|(_, t)| t.iter().cloned()).collect()
    }

    fn declaration(&self, direction: Direction) -> Declaration {
        Declaration {
            id: self.identifier.clone(),
            direction,
            conforms_to: self.conforms_to.clone(),
            extensions: self.tags(TAG_FILENAME_EXTENSION).into_iter().map(|e| e.to_ascii_lowercase()).collect(),
            mime_types: self.tags(TAG_MIME_TYPE),
            description: self.description.clone(),
        }
    }
}

/// Keeps one declaration per identifier (an exported one beats an imported one) and puts exported types first.
/// Two exports of one identifier are kept so that `Declared::new` reports the duplicate.
fn merge(decls: Vec<Declaration>) -> Vec<Declaration> {
    let mut out: Vec<Declaration> = Vec::new();
    for d in decls {
        match out.iter().position(|o| o.id == d.id) {
            Some(i) if out[i].direction == Direction::Imported && d.direction == Direction::Exported => out[i] = d,
            Some(i) if out[i].direction == d.direction => out.push(d),
            Some(_) => {}
            None => out.push(d),
        }
    }
    out.sort_by_key(|d| d.direction == Direction::Imported);
    out
}

/// Checks the declarations and makes them visible to every `uttype` lookup, replacing earlier ones.
/// A declaration with no parent conforms to `public.data`. On an error the earlier declarations stay.
pub fn register(decls: Vec<Declaration>) -> Result<(), DeclError> {
    register_with_tags(decls, vec![])
}

fn register_with_tags(decls: Vec<Declaration>, extra: Vec<(String, String, Vec<String>)>) -> Result<(), DeclError> {
    let decls: Vec<Declaration> = merge(decls)
        .into_iter()
        .map(|mut d| {
            if d.conforms_to.is_empty() {
                d.conforms_to.push("public.data".into());
            }
            d
        })
        .collect();
    Declared::new(decls.clone())?;
    let entries = decls
        .iter()
        .map(|d| -> &'static Entry {
            Box::leak(Box::new(Entry {
                id: leak(&d.id),
                parent: leak_all(&d.conforms_to),
                ext: leak_all(&d.extensions),
                mime: d.mime_types.first().map_or("", |m| leak(&m.to_ascii_lowercase())),
                desc: leak(&d.description),
            }))
        })
        .collect();
    uttype::set_declared(entries);
    *extra_tags() = extra;
    Ok(())
}

/// Registers the exported and imported declarations of an app manifest.
pub fn register_manifest(text: &str) -> Result<(), LoadError> {
    register(parse_manifest(text).map_err(LoadError::Manifest)?).map_err(LoadError::Decl)
}

/// Registers `UTExportedTypeDeclarations` and `UTImportedTypeDeclarations` entries, including tag classes
/// other than extension and MIME type.
pub fn register_plist(exported: &[PlistDeclaration], imported: &[PlistDeclaration]) -> Result<(), DeclError> {
    let mut decls = Vec::new();
    let mut extra = Vec::new();
    for (list, dir) in [(exported, Direction::Exported), (imported, Direction::Imported)] {
        for p in list {
            decls.push(p.declaration(dir));
            for (class, tags) in &p.tag_specification {
                if class != TAG_FILENAME_EXTENSION && class != TAG_MIME_TYPE {
                    extra.push((p.identifier.clone(), class.clone(), tags.clone()));
                }
            }
        }
    }
    register_with_tags(decls, extra)
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The identifier of the dynamic type for a tag nobody declared: `dyn.ext.<ext>`, `dyn.mime.<hex>`,
/// `dyn.tag.<hex>.<hex>`, plus `.c-<hex>` when it must conform to a given type.
fn dynamic_id(class: &UtTagClass, tag: &str, parent: Option<&str>) -> String {
    let base = match class.0.as_str() {
        TAG_FILENAME_EXTENSION => format!("dyn.ext.{}", tag.to_ascii_lowercase()),
        TAG_MIME_TYPE => format!("dyn.mime.{}", hex(&tag.to_ascii_lowercase())),
        c => format!("dyn.tag.{}.{}", hex(c), hex(tag)),
    };
    match parent {
        Some(p) => format!("{base}.c-{}", hex(p)),
        None => base,
    }
}

/// `UTType`: a type identifier, declared or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtType {
    id: String,
}

impl UtType {
    /// `UTType(_ identifier:)`: `None` unless the system or the app declares it, or it is dynamic.
    pub fn new(identifier: &str) -> Option<UtType> {
        (uttype::is_known(identifier) || identifier.starts_with("dyn.")).then(|| UtType { id: identifier.into() })
    }

    /// `UTType(exportedAs:conformingTo:)`. An undeclared identifier still makes a value (`is_declared` is false).
    pub fn exported_as(identifier: &str, conforming_to: Option<&UtType>) -> UtType {
        Self::declared_as(identifier, conforming_to)
    }

    /// `UTType(importedAs:conformingTo:)`.
    pub fn imported_as(identifier: &str, conforming_to: Option<&UtType>) -> UtType {
        Self::declared_as(identifier, conforming_to)
    }

    fn declared_as(identifier: &str, conforming_to: Option<&UtType>) -> UtType {
        if !uttype::is_known(identifier) {
            Self::make_dynamic_entry(identifier, conforming_to);
        }
        UtType { id: identifier.into() }
    }

    /// `UTType(filenameExtension:conformingTo:)`: the declared type for the extension, else a dynamic one.
    /// `None` for an empty extension or one with a dot.
    pub fn filename_extension(ext: &str, conforming_to: Option<&UtType>) -> Option<UtType> {
        Self::tag(ext, &UtTagClass::filename_extension(), conforming_to)
    }

    /// `UTType(mimeType:conformingTo:)`.
    pub fn mime_type(mime: &str, conforming_to: Option<&UtType>) -> Option<UtType> {
        Self::tag(mime, &UtTagClass::mime_type(), conforming_to)
    }

    /// `UTType(tag:tagClass:conformingTo:)`: a declared type that carries the tag and conforms to `conforming_to`,
    /// otherwise a dynamic type that conforms to it (or to `public.data`).
    pub fn tag(tag: &str, class: &UtTagClass, conforming_to: Option<&UtType>) -> Option<UtType> {
        if tag.is_empty() || (class.0 == TAG_FILENAME_EXTENSION && tag.contains('.')) {
            return None;
        }
        let found = match class.0.as_str() {
            TAG_FILENAME_EXTENSION => uttype::type_for_extension(tag).map(str::to_string),
            TAG_MIME_TYPE => uttype::type_for_mime(tag).map(str::to_string),
            c => extra_tags().iter().find(|(_, k, tags)| k == c && tags.iter().any(|t| t == tag)).map(|(id, _, _)| id.clone()),
        };
        if let Some(id) = found {
            if conforming_to.map_or(true, |c| uttype::conforms(&id, &c.id)) {
                return Some(UtType { id });
            }
        }
        let id = dynamic_id(class, tag, conforming_to.map(|c| c.id.as_str()));
        Self::make_dynamic_entry(&id, conforming_to);
        Some(UtType { id })
    }

    fn make_dynamic_entry(id: &str, conforming_to: Option<&UtType>) {
        let parent = conforming_to.map_or("public.data", |c| c.id.as_str());
        let entry: &'static Entry = Box::leak(Box::new(Entry { id: leak(id), parent: leak_all(&[parent.to_string()]), ext: &[], mime: "", desc: "Document" }));
        uttype::add_dynamic(entry);
    }

    pub fn identifier(&self) -> &str {
        &self.id
    }

    /// `UTType.conforms(to:)`.
    pub fn conforms_to(&self, other: &UtType) -> bool {
        uttype::conforms(&self.id, &other.id)
    }

    /// `UTType.isDeclared`: the system or the app declares the identifier. Dynamic types are not declared.
    pub fn is_declared(&self) -> bool {
        uttype::is_known(&self.id) && !uttype::is_dynamic_entry(&self.id)
    }

    /// `UTType.isDynamic`: made up for a tag nobody declared.
    pub fn is_dynamic(&self) -> bool {
        self.id.starts_with("dyn.")
    }

    /// `UTType.isPublic`: declared by the system. App-declared types are not.
    pub fn is_public(&self) -> bool {
        uttype::is_builtin(&self.id)
    }

    /// `UTType.tags`: the tags this type declares for a class (empty for a dynamic type).
    pub fn tags(&self, class: &UtTagClass) -> Vec<String> {
        match class.0.as_str() {
            TAG_FILENAME_EXTENSION => uttype::tags(&self.id, uttype::TagClass::FilenameExtension).iter().map(|s| s.to_string()).collect(),
            TAG_MIME_TYPE => uttype::tags(&self.id, uttype::TagClass::MimeType).iter().map(|s| s.to_string()).collect(),
            c => extra_tags().iter().filter(|(id, k, _)| *id == self.id && k == c).flat_map(|(_, _, t)| t.clone()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-wide, so tests that register take turns.
    static LOCK: Mutex<()> = Mutex::new(());
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    const MANIFEST: &str = r#"
name = "Sketch"

[[exported_types]]
id = "com.example.sketch.board"
conforms_to = ["public.data"]
extensions = ["skb", ".SKX"]
mime_types = ["application/x-sketch-board"]
description = "Sketch Board"

[[exported_types]]
id = "com.example.sketch.note"
conforms_to = ["public.plain-text", "com.example.sketch.board"]
extensions = ["skn"]

[[imported_types]]
id = "com.other.shared"
conforms_to = ["public.json"]
extensions = ["shr"]
"#;

    #[test]
    fn a_manifest_block_parses_into_declarations() {
        let d = parse_manifest(MANIFEST).unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].id, "com.example.sketch.board");
        assert_eq!(d[0].direction, Direction::Exported);
        assert_eq!(d[0].extensions, vec!["skb", "skx"]);
        assert_eq!(d[0].mime_types, vec!["application/x-sketch-board"]);
        assert_eq!(d[0].description, "Sketch Board");
        assert_eq!(d[1].conforms_to, vec!["public.plain-text", "com.example.sketch.board"]);
        assert_eq!(d[2].direction, Direction::Imported);
    }

    #[test]
    fn a_bad_manifest_block_is_an_error() {
        assert_eq!(parse_manifest("[[exported_types]]\nextensions = [\"a\"]\n"), Err(ManifestError::MissingId(1)));
        assert_eq!(parse_manifest("[[exported_types]]\nid = \"a.b\"\nbogus = \"x\"\n"), Err(ManifestError::UnknownKey { line: 3, key: "bogus".into() }));
        assert_eq!(parse_manifest("[[exported_types]]\nid \"a.b\"\n"), Err(ManifestError::Syntax(2)));
        assert_eq!(parse_manifest("[[exported_types]]\nid = \"a.b\n"), Err(ManifestError::Syntax(2)));
    }

    #[test]
    fn registered_types_answer_the_uttype_lookups() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        assert_eq!(uttype::type_for_extension("Plan.SKB"), Some("com.example.sketch.board"));
        assert_eq!(uttype::type_for_extension("x.skx"), Some("com.example.sketch.board"));
        assert_eq!(uttype::type_for_mime("Application/X-Sketch-Board; v=1"), Some("com.example.sketch.board"));
        assert_eq!(uttype::description("com.example.sketch.board"), "Sketch Board");
        assert_eq!(uttype::preferred_extension("com.example.sketch.board"), Some("skb"));
        assert_eq!(uttype::mime("com.example.sketch.board"), "application/x-sketch-board");
    }

    #[test]
    fn conformance_chains_to_public_data_and_across_declarations() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        assert!(uttype::conforms("com.example.sketch.board", "public.data"));
        assert!(uttype::conforms("com.example.sketch.note", "public.text"));
        assert!(uttype::conforms("com.example.sketch.note", "com.example.sketch.board"));
        assert!(uttype::conforms("com.other.shared", "public.json"));
        assert_eq!(uttype::distance("com.example.sketch.note", "com.example.sketch.board"), Some(1));
        assert!(!uttype::conforms("com.example.sketch.board", "public.text"));
    }

    #[test]
    fn a_declaration_without_a_parent_conforms_to_public_data() {
        let _g = lock();
        register_manifest("[[exported_types]]\nid = \"com.example.bare\"\nextensions = [\"bre\"]\n").unwrap();
        assert!(uttype::conforms("com.example.bare", "public.data"));
    }

    #[test]
    fn an_exported_type_wins_over_an_imported_one_with_the_same_id() {
        let _g = lock();
        let text = "[[imported_types]]\nid = \"com.example.twin\"\nextensions = [\"twi\"]\n\n[[exported_types]]\nid = \"com.example.twin\"\nextensions = [\"twe\"]\n";
        register_manifest(text).unwrap();
        assert_eq!(uttype::type_for_extension("a.twe"), Some("com.example.twin"));
        assert_eq!(uttype::type_for_extension("a.twi"), None);
    }

    #[test]
    fn app_types_do_not_take_over_built_in_extensions() {
        let _g = lock();
        register_manifest("[[exported_types]]\nid = \"com.example.greedy\"\nextensions = [\"txt\"]\n").unwrap();
        assert_eq!(uttype::type_for_extension("a.txt"), Some("public.plain-text"));
    }

    #[test]
    fn a_conformance_cycle_is_rejected_and_leaves_the_registry_alone() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        let cyc = "[[exported_types]]\nid = \"com.example.a\"\nconforms_to = [\"com.example.b\"]\n[[exported_types]]\nid = \"com.example.b\"\nconforms_to = [\"com.example.a\"]\n";
        assert!(matches!(register_manifest(cyc), Err(LoadError::Decl(DeclError::Cycle(_)))));
        assert_eq!(uttype::type_for_extension("a.skb"), Some("com.example.sketch.board"));
    }

    #[test]
    fn reserved_and_unknown_parents_are_rejected() {
        let _g = lock();
        assert!(matches!(register_manifest("[[exported_types]]\nid = \"public.mine\"\n"), Err(LoadError::Decl(DeclError::ReservedIdentifier(_)))));
        assert!(matches!(register_manifest("[[exported_types]]\nid = \"com.example.x\"\nconforms_to = [\"com.nowhere.y\"]\n"), Err(LoadError::Decl(DeclError::UnknownParent { .. }))));
    }

    #[test]
    fn an_unknown_extension_makes_a_dynamic_type() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        let t = UtType::filename_extension("zzq", None).unwrap();
        assert!(t.is_dynamic());
        assert!(!t.is_declared());
        assert!(!t.is_public());
        assert!(t.identifier().starts_with("dyn."));
        assert!(t.conforms_to(&UtType::new("public.data").unwrap()));
        assert_eq!(uttype::type_for_extension("a.zzq"), None);
    }

    #[test]
    fn a_dynamic_type_conforms_to_the_requested_parent() {
        let _g = lock();
        let text = UtType::new("public.text").unwrap();
        let t = UtType::filename_extension("zzt", Some(&text)).unwrap();
        assert!(t.is_dynamic() && t.conforms_to(&text));
        assert_eq!(UtType::filename_extension("zzt", Some(&text)).unwrap(), t);
        assert_ne!(UtType::filename_extension("zzt", None).unwrap(), t);
    }

    #[test]
    fn a_declared_extension_gives_the_declared_type_when_it_conforms() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        let board = UtType::new("com.example.sketch.board").unwrap();
        let t = UtType::filename_extension("SKB", Some(&UtType::new("public.data").unwrap())).unwrap();
        assert_eq!(t, board);
        assert!(t.is_declared() && !t.is_dynamic() && !t.is_public());
        // The type does not conform to text, so the answer is a dynamic text type instead.
        let other = UtType::filename_extension("skb", Some(&UtType::new("public.text").unwrap())).unwrap();
        assert!(other.is_dynamic());
    }

    #[test]
    fn system_types_are_public_and_declared() {
        let png = UtType::filename_extension("png", None).unwrap();
        assert_eq!(png.identifier(), "public.png");
        assert!(png.is_public() && png.is_declared() && !png.is_dynamic());
        assert_eq!(UtType::new("com.nobody.nothing"), None);
        assert_eq!(UtType::filename_extension("a.b", None), None);
        assert_eq!(UtType::filename_extension("", None), None);
    }

    #[test]
    fn exported_as_and_imported_as_name_declared_or_undeclared_types() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        let data = UtType::new("public.data").unwrap();
        let e = UtType::exported_as("com.example.sketch.board", Some(&data));
        assert!(e.is_declared());
        let i = UtType::imported_as("com.example.undeclared-thing", Some(&data));
        assert!(!i.is_declared() && !i.is_dynamic() && !i.is_public());
        assert!(i.conforms_to(&data));
    }

    #[test]
    fn mime_tags_find_declared_types() {
        let _g = lock();
        register_manifest(MANIFEST).unwrap();
        let t = UtType::mime_type("application/x-sketch-board", None).unwrap();
        assert_eq!(t.identifier(), "com.example.sketch.board");
        assert_eq!(t.tags(&UtTagClass::mime_type()), vec!["application/x-sketch-board"]);
        assert_eq!(t.tags(&UtTagClass::filename_extension()), vec!["skb", "skx"]);
    }

    #[test]
    fn plist_declarations_register_with_generic_tag_classes() {
        let _g = lock();
        let board = PlistDeclaration {
            identifier: "com.example.plist.board".into(),
            conforms_to: vec!["public.data".into()],
            description: "Plist Board".into(),
            tag_specification: vec![
                (TAG_FILENAME_EXTENSION.into(), vec!["plb".into()]),
                (TAG_MIME_TYPE.into(), vec!["application/x-plb".into()]),
                ("com.example.pboard".into(), vec!["example.plb-board".into()]),
            ],
        };
        let imported = PlistDeclaration { identifier: "com.example.plist.board".into(), tag_specification: vec![(TAG_FILENAME_EXTENSION.into(), vec!["zzz9".into()])], ..Default::default() };
        register_plist(&[board], &[imported]).unwrap();
        assert_eq!(uttype::type_for_extension("a.plb"), Some("com.example.plist.board"));
        assert_eq!(uttype::type_for_extension("a.zzz9"), None);
        let class = UtTagClass("com.example.pboard".into());
        let t = UtType::tag("example.plb-board", &class, None).unwrap();
        assert_eq!(t.identifier(), "com.example.plist.board");
        assert_eq!(t.tags(&class), vec!["example.plb-board"]);
        assert!(UtType::tag("other", &class, None).unwrap().is_dynamic());
    }
}
