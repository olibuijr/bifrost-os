//! Input method plugins: a plugin is a descriptor that names an input source and the built-in engine that runs it.
//! Third-party code is not loaded. The descriptor list is one plugin per line, in the form
//! `id|name|language|engine`, with blank lines and lines starting with `#` ignored. The engine must be one of the
//! built-in engines below, so a plugin can add a source for an engine that exists (a second Vietnamese layout, for
//! example) but cannot add a new composer.
//!
//! Pure model: the text is passed in by the caller, which reads the plugin list from its own location.

use super::{new_controller, InputController};
use super::{AINU_KATAKANA, DEVANAGARI_PHONETIC, JYUTPING, KANA_TYPING, KOREAN_2SET, PINYIN, TAMIL_PHONETIC, VIETNAMESE_TELEX, VIETNAMESE_VNI, ZHUYIN};

/// The built-in engines a plugin can name, and the input source each one is.
const ENGINES: &[(&str, &str)] = &[
    ("kana-typing", KANA_TYPING),
    ("pinyin", PINYIN),
    ("zhuyin", ZHUYIN),
    ("tamil", TAMIL_PHONETIC),
    ("devanagari", DEVANAGARI_PHONETIC),
    ("ainu-katakana", AINU_KATAKANA),
    ("jyutping", JYUTPING),
    ("hangul-2set", KOREAN_2SET),
    ("vietnamese-telex", VIETNAMESE_TELEX),
    ("vietnamese-vni", VIETNAMESE_VNI),
];

/// One plugin as the list describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub language: String,
    pub engine: String,
}

/// Why a plugin list was refused: the line number (from 1) and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub reason: String,
}

/// The input source id of a built-in engine, or None for an engine this module does not have.
pub fn engine_source(engine: &str) -> Option<&'static str> {
    ENGINES.iter().find(|(name, _)| *name == engine).map(|(_, source)| *source)
}

/// A fresh controller for a plugin: the controller of its engine's source.
pub fn controller(plugin: &Descriptor) -> Option<Box<dyn InputController>> {
    engine_source(&plugin.engine).and_then(new_controller)
}

/// One input source as the shell lists it: a built-in source or an installed plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSource {
    pub id: String,
    pub name: String,
    pub language: String,
}

/// The sources the shell offers: the built-in sources first, then each plugin whose id no built-in source uses.
pub fn listed_sources(plugins: &[Descriptor]) -> Vec<ListedSource> {
    let mut out: Vec<ListedSource> = super::sources()
        .iter()
        .map(|s| ListedSource { id: s.id.to_string(), name: s.localized_name.to_string(), language: s.language.to_string() })
        .collect();
    for p in plugins {
        if out.iter().any(|s| s.id == p.id) {
            continue;
        }
        out.push(ListedSource { id: p.id.clone(), name: p.name.clone(), language: p.language.clone() });
    }
    out
}

/// A fresh controller for a listed source id: a built-in source's controller, or the controller of the plugin that
/// names the id. None for an id neither provides.
pub fn controller_for(id: &str, plugins: &[Descriptor]) -> Option<Box<dyn InputController>> {
    new_controller(id).or_else(|| plugins.iter().find(|p| p.id == id).and_then(controller))
}

/// The input source plugins the installed packages declare (`input_sources` in each manifest), in install order. An
/// id an earlier package already gave is skipped, and a package whose list does not parse contributes none.
pub fn installed(apps: &[crate::shell::store::install::InstalledApp]) -> Vec<Descriptor> {
    let mut seen: Vec<Descriptor> = Vec::new();
    for app in apps {
        let Ok(text) = std::fs::read_to_string(app.dir.join("manifest.toml")) else { continue };
        let Ok(manifest) = crate::shell::store::install::parse_manifest(&text) else { continue };
        let Ok(found) = parse(&manifest.input_sources.join("\n")) else { continue };
        for d in found {
            if !seen.iter().any(|s| s.id == d.id) {
                seen.push(d);
            }
        }
    }
    seen
}

/// Reads a plugin list. Every entry needs four non-empty fields, a known engine, and an id that no earlier entry uses.
pub fn parse(text: &str) -> Result<Vec<Descriptor>, ParseError> {
    let mut plugins: Vec<Descriptor> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let entry = raw.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let fail = |reason: &str| ParseError { line, reason: reason.to_string() };
        let fields: Vec<&str> = entry.split('|').map(str::trim).collect();
        let [id, name, language, engine] = fields.as_slice() else {
            return Err(fail("expected four fields: id|name|language|engine"));
        };
        if [id, name, language, engine].iter().any(|f| f.is_empty()) {
            return Err(fail("a field is empty"));
        }
        if engine_source(engine).is_none() {
            return Err(fail("unknown engine"));
        }
        if plugins.iter().any(|p| p.id == *id) {
            return Err(fail("duplicate id"));
        }
        plugins.push(Descriptor {
            id: id.to_string(),
            name: name.to_string(),
            language: language.to_string(),
            engine: engine.to_string(),
        });
    }
    Ok(plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_reads_one_plugin_per_line_and_skips_comments() {
        let text = "# installed plugins\n\nbifrost.plugin.kana|Kana (JIS)|ja|kana-typing\n  bifrost.plugin.tamil|Tamil|ta|tamil  \n";
        let list = parse(text).expect("valid list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Kana (JIS)");
        assert_eq!(list[1].engine, "tamil");
    }

    #[test]
    fn an_entry_needs_four_fields() {
        let err = parse("a|b|c").unwrap_err();
        assert_eq!(err.line, 1);
        assert!(err.reason.contains("four fields"));
    }

    #[test]
    fn an_empty_field_is_refused() {
        let err = parse("a||ja|kana-typing").unwrap_err();
        assert_eq!(err.reason, "a field is empty");
    }

    #[test]
    fn an_unknown_engine_is_refused() {
        let err = parse("a|A|ja|some-engine").unwrap_err();
        assert_eq!(err.reason, "unknown engine");
        assert_eq!(err.line, 1);
    }

    #[test]
    fn a_duplicate_id_is_refused_on_its_line() {
        let err = parse("a|A|ja|kana-typing\n\na|B|ja|kana-typing").unwrap_err();
        assert_eq!(err.line, 3);
        assert_eq!(err.reason, "duplicate id");
    }

    #[test]
    fn an_engine_resolves_to_its_source_and_builds_a_controller() {
        assert_eq!(engine_source("pinyin"), Some(PINYIN));
        assert_eq!(engine_source("nope"), None);
        let plugin = Descriptor {
            id: "x".into(),
            name: "X".into(),
            language: "zh".into(),
            engine: "zhuyin".into(),
        };
        assert!(controller(&plugin).is_some());
    }

    #[test]
    fn listed_sources_add_plugins_after_the_built_in_ones() {
        let plugins = vec![
            Descriptor { id: "x.pin2".into(), name: "Pinyin 2".into(), language: "zh-Hans".into(), engine: "pinyin".into() },
            Descriptor { id: KOREAN_2SET.into(), name: "Clash".into(), language: "ko".into(), engine: "hangul-2set".into() },
        ];
        let listed = listed_sources(&plugins);
        assert_eq!(listed.len(), super::super::sources().len() + 1, "a plugin reusing a built-in id is not listed");
        let last = listed.last().unwrap();
        assert_eq!((last.id.as_str(), last.name.as_str()), ("x.pin2", "Pinyin 2"));
        assert_eq!(listed[0].name, "2-Set Korean", "built-in sources keep their names");
    }

    #[test]
    fn controller_for_resolves_built_in_and_plugin_ids() {
        let plugins = vec![Descriptor { id: "x.pin2".into(), name: "P".into(), language: "zh".into(), engine: "pinyin".into() }];
        assert!(controller_for(PINYIN, &plugins).is_some());
        assert!(controller_for("x.pin2", &plugins).is_some());
        assert!(controller_for("x.missing", &plugins).is_none());
    }

    #[test]
    fn installed_packages_add_their_sources_once_each() {
        use crate::shell::store::install::InstalledApp;
        let root = std::env::temp_dir().join(format!("input-plugins-{}-installed", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = |id: &str, sources: &str| {
            let dir = root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.toml"), format!("id = \"{id}\"\nname = \"N\"\nversion = \"1.0.0\"\n{sources}")).unwrap();
            InstalledApp { id: id.to_string(), dir, ..Default::default() }
        };
        let apps = [
            app("is.x.one", "input_sources = [\"x.kana|Kana|ja|kana-typing\", \"x.pin|Pinyin|zh|pinyin\"]\n"),
            app("is.x.two", "input_sources = [\"x.kana|Other|ja|kana-typing\", \"x.viet|Viet|vi|vietnamese-telex\"]\n"),
            app("is.x.none", ""),
        ];
        let got = installed(&apps);
        let ids: Vec<&str> = got.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["x.kana", "x.pin", "x.viet"]);
        assert_eq!(got[0].name, "Kana", "the first package keeps the id");
        assert!(controller(&got[2]).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
