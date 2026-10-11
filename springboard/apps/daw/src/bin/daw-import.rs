//! `daw-import <mobile-DAW-dir> <desktop-store-dir>`
//!
//! Converts the mobile DAW's `data/projects` and `data/revisions` into the desktop store and prints a report.
//! `<mobile-DAW-dir>` is the DAW repo root (the directory that contains `data/`).
use daw::store;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [src, dst] = args.as_slice() else {
        eprintln!("usage: daw-import <mobile-DAW-dir> <desktop-store-dir>");
        std::process::exit(2);
    };
    let data = PathBuf::from(src).join("data");
    let report = match store::import(&data, &PathBuf::from(dst), &data.join("plugins")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("daw-import: {e}");
            std::process::exit(1);
        }
    };
    for p in &report.projects {
        println!("{:<12} {:<40} tracks {:>2}  {}", p.id, p.name, p.tracks, p.warnings.join("; "));
    }
    println!("revisions copied: {}", report.revisions_copied);
    println!("instrument types: {}", report.instrument_types.join(", "));
    println!("effect types: {}", report.effect_types.join(", "));
    if !report.unknown_instruments.is_empty() {
        println!("instruments without a plugin: {}", report.unknown_instruments.join(", "));
    }
    for f in &report.failures {
        eprintln!("failed: {f}");
    }
    if !report.failures.is_empty() {
        std::process::exit(1);
    }
}
