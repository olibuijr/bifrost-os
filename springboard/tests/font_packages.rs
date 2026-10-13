//! The runtime font payload: packages listed, aliases map to packaged families, installer ships the alias file.
use std::fs;
use std::path::PathBuf;

fn arch(file: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../packaging/arch").join(file);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Family names a package provides (the ones the alias file may point at).
const PROVIDES: &[(&str, &[&str])] = &[
    ("ttf-liberation", &["Liberation Sans", "Liberation Serif", "Liberation Mono"]),
    ("ttf-croscore", &["Arimo", "Tinos", "Cousine"]),
    ("ttf-dejavu", &["DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono"]),
    ("otf-stix", &["STIX Two Math", "STIX Two Text"]),
    ("gsfonts", &["D050000L", "Standard Symbols PS"]),
    ("unifont", &["Unifont", "Unifont Upper"]),
    ("noto-fonts", &["Noto Sans", "Noto Serif", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math"]),
];

#[test]
fn runtime_packages_list_the_font_set() {
    let list = arch("runtime-packages.txt");
    let pkgs: Vec<&str> = list.lines().map(str::trim).collect();
    for want in [
        "noto-fonts", "noto-fonts-extra", "ttf-liberation", "ttf-croscore", "otf-stix", "gsfonts", "unifont",
        "noto-fonts-cjk", "noto-fonts-emoji", "ttf-dejavu",
    ] {
        assert!(pkgs.contains(&want), "runtime-packages.txt lacks {want}");
    }
}

fn aliases() -> Vec<(String, Vec<String>)> {
    let conf = arch("fontconfig/60-bifrost-aliases.conf");
    let tags = |s: &str| -> Vec<String> {
        s.split("<family>").skip(1).filter_map(|p| p.split("</family>").next()).map(str::to_string).collect()
    };
    conf.split("<alias").skip(1).map(|block| {
        let block = block.split("</alias>").next().unwrap();
        assert!(block.contains("binding=\"same\""), "alias without binding=same: {block}");
        let (from, to) = block.split_once("<accept>").expect("alias without accept");
        (tags(from).remove(0), tags(to))
    }).collect()
}

#[test]
fn every_compat_alias_maps_to_a_packaged_family() {
    let found = aliases();
    for name in [
        "Arial", "Helvetica", "Helvetica Neue", "Times New Roman", "Times", "Courier New", "Courier", "Menlo", "Monaco",
        "Symbol", "Zapf Dingbats",
    ] {
        let (_, targets) = found.iter().find(|(f, _)| f == name).unwrap_or_else(|| panic!("no alias for {name}"));
        assert!(!targets.is_empty(), "{name} accepts nothing");
        for t in targets {
            assert!(PROVIDES.iter().any(|(_, fams)| fams.contains(&t.as_str())), "{name} -> {t}: no package provides it");
        }
    }
    let list = arch("runtime-packages.txt");
    for (pkg, _) in PROVIDES {
        assert!(list.lines().any(|l| l.trim() == *pkg), "{pkg} provides alias targets but is not listed");
    }
}

#[test]
fn metric_compatible_targets_come_first() {
    let found = aliases();
    let first = |n: &str| found.iter().find(|(f, _)| f == n).unwrap().1[0].clone();
    assert_eq!(first("Arial"), "Liberation Sans");
    assert_eq!(first("Times New Roman"), "Liberation Serif");
    assert_eq!(first("Courier New"), "Liberation Mono");
}

#[test]
fn installer_ships_the_alias_file() {
    let sh = arch("package-files.sh");
    assert!(sh.contains("fontconfig/60-bifrost-aliases.conf"), "package-files.sh does not install the alias file");
    assert!(sh.contains("/etc/fonts/conf.d/60-bifrost-aliases.conf"));
}
