//! Desktop song store and the importer for the mobile DAW's data directory.
//!
//! Layout written by the importer (and read by the app):
//! `<dst>/projects/<id>.json`, `<dst>/revisions/<id>/rN.json` (and the revision README/PNGs), `<dst>/import.json`.
//! Originals are only read, never modified.

use crate::model::Project;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One row of the song list.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub bpm: f64,
    pub tracks: usize,
    pub scenes: usize,
    pub path: PathBuf,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub projects: Vec<ImportedProject>,
    pub revisions_copied: usize,
    pub instrument_types: Vec<String>,
    pub effect_types: Vec<String>,
    pub unknown_instruments: Vec<String>,
    pub sample_refs: Vec<String>,
    pub failures: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportedProject {
    pub id: String,
    pub name: String,
    pub tracks: usize,
    pub warnings: Vec<String>,
}

/// Instrument type names the mobile DAW ships as plugin directories (`data/plugins/<type>`).
/// The importer reports any song that uses a type outside this list; the instrument still imports.
pub fn plugin_ids(plugins_dir: &Path) -> BTreeSet<String> {
    fs::read_dir(plugins_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Writes `text` to `path` through a temporary file and a rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads one project file.
pub fn load(path: &Path) -> Result<Project, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Project::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Lists the projects in `<dir>/projects`, newest name order, skipping files that do not parse.
pub fn list(dir: &Path) -> Vec<Summary> {
    let mut out: Vec<Summary> = fs::read_dir(dir.join("projects"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .filter_map(|path| {
                    let project = load(&path).ok()?;
                    Some(Summary {
                        id: project.id.clone(),
                        name: project.name.clone(),
                        bpm: project.bpm,
                        tracks: project.tracks.len(),
                        scenes: project.scenes.len(),
                        path,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    out
}

/// Copies a directory tree, returning the number of files copied.
fn copy_tree(from: &Path, to: &Path) -> Result<usize, String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut count = 0;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            count += copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| format!("{}: {e}", entry.path().display()))?;
            count += 1;
        }
    }
    Ok(count)
}

/// Imports every project and revision from the mobile DAW data directory `src` into `dst`.
///
/// Each project is parsed into the typed model, validated, and written back in the desktop layout.
/// Projects that fail to parse are reported and skipped; nothing under `src` is modified.
/// `plugins` is the mobile DAW's `data/plugins` directory, used to report unknown instrument types.
pub fn import(src: &Path, dst: &Path, plugins: &Path) -> Result<ImportReport, String> {
    let mut report = ImportReport::default();
    let known = plugin_ids(plugins);
    let mut instruments = BTreeSet::new();
    let mut effects = BTreeSet::new();
    let mut unknown = BTreeSet::new();
    let mut samples = BTreeSet::new();

    let projects = src.join("projects");
    let mut files: Vec<PathBuf> = fs::read_dir(&projects)
        .map_err(|e| format!("{}: {e}", projects.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();

    for path in files {
        let project = match load(&path) {
            Ok(p) => p,
            Err(e) => {
                report.failures.push(e);
                continue;
            }
        };
        let warnings = project.validate();
        for track in &project.tracks {
            instruments.insert(track.instrument.kind.clone());
            if !known.is_empty() && !known.contains(&track.instrument.kind) {
                unknown.insert(track.instrument.kind.clone());
            }
            for fx in track.effects.iter().chain(project.master.iter().flat_map(|m| m.effects.iter())) {
                effects.insert(fx.kind.clone());
            }
            if let Some(sample) = &track.instrument.sample {
                samples.insert(sample.to_string());
            }
        }
        let out = dst.join("projects").join(format!("{}.json", project.id));
        if let Err(e) = write_atomic(&out, &project.to_json()) {
            report.failures.push(e);
            continue;
        }
        report.projects.push(ImportedProject {
            id: project.id.clone(),
            name: project.name.clone(),
            tracks: project.tracks.len(),
            warnings,
        });
    }

    let revisions = src.join("revisions");
    if revisions.is_dir() {
        for entry in fs::read_dir(&revisions).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                report.revisions_copied += copy_tree(&entry.path(), &dst.join("revisions").join(entry.file_name()))?;
            }
        }
    }

    report.instrument_types = instruments.into_iter().collect();
    report.effect_types = effects.into_iter().collect();
    report.unknown_instruments = unknown.into_iter().collect();
    report.sample_refs = samples.into_iter().collect();
    write_atomic(&dst.join("import.json"), &serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_orders_by_name_and_skips_broken_files() {
        let dir = std::env::temp_dir().join(format!("daw-store-{}", std::process::id()));
        let projects = dir.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let song = |id: &str, name: &str| {
            format!(r#"{{"id":"{id}","name":"{name}","bpm":120,"tracks":[],"scenes":[]}}"#)
        };
        fs::write(projects.join("b.json"), song("b", "Zed")).unwrap();
        fs::write(projects.join("a.json"), song("a", "alpha")).unwrap();
        fs::write(projects.join("bad.json"), "{nope").unwrap();
        let names: Vec<String> = list(&dir).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["alpha", "Zed"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
