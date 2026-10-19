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
    ("gsfonts", &["D050000L", "Standard Symbols PS"]),
    ("noto-fonts", &["Noto Sans", "Noto Serif", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math"]),
];

#[test]
fn runtime_packages_list_the_font_set() {
    let list = arch("runtime-packages.txt");
    let pkgs: Vec<&str> = list.lines().map(str::trim).collect();
    for want in [
        "noto-fonts", "noto-fonts-extra", "ttf-liberation", "ttf-croscore", "gsfonts",
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

/// The Source Sans 3 italic face is pinned in the font library (Settings > Fonts), with a 64-hex digest.
#[test]
fn library_pins_the_source_sans_italic_face() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/pack.py");
    let pack = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let line = pack
        .lines()
        .find(|l| l.contains("\"SourceSans3-Italic-Variable.ttf\""))
        .expect("pack.py LIBRARY_FONTS lacks the Source Sans 3 italic face");
    assert!(line.contains("sourcesans3/SourceSans3-Italic[wght].ttf"), "wrong upstream path: {line}");
    let digest = line.rsplit('"').nth(1).unwrap_or("");
    assert_eq!(digest.len(), 64, "digest is not sha256: {line}");
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()), "digest is not hex: {line}");
}

/// The Browser engine's fallback list is fixed and its last family, "Droid Sans Fallback", is the only one that can answer the
/// CJK punctuation marks (script Common). A scan rule gives that name to Noto Sans CJK SC, the package installs the rule and
/// the scriptlet rebuilds the font cache, which a scan rule needs.
#[test]
fn cjk_punctuation_falls_back_to_the_installed_cjk_font() {
    let conf = arch("fontconfig/61-bifrost-cjk-fallback.conf");
    assert!(conf.contains("target=\"scan\""), "the engine lists families from the font cache, so the rule must be a scan rule");
    assert!(conf.contains("<string>Noto Sans CJK SC</string>"), "the rule must test the Noto Sans CJK SC family");
    assert!(conf.contains("mode=\"append\""), "the family is added, the original names stay");
    assert!(conf.contains("<string>Droid Sans Fallback</string>"), "the name is the last entry of the engine's fallback list");
    let list = arch("runtime-packages.txt");
    assert!(list.lines().any(|l| l.trim() == "noto-fonts-cjk"), "noto-fonts-cjk provides the family the rule extends");
    let sh = arch("package-files.sh");
    assert!(sh.contains("fontconfig/61-bifrost-cjk-fallback.conf"), "package-files.sh does not install the CJK fallback rule");
    assert!(sh.contains("/etc/fonts/conf.d/61-bifrost-cjk-fallback.conf"));
    let scriptlet = arch("springboard.install");
    assert!(scriptlet.contains("bifrost_fontcache"), "springboard.install does not rebuild the font cache");
    assert!(scriptlet.contains("fc-cache -sf"), "a cache built before the rule needs a forced rebuild");
}

#[test]
fn installer_ships_the_alias_file() {
    let sh = arch("package-files.sh");
    assert!(sh.contains("fontconfig/60-bifrost-aliases.conf"), "package-files.sh does not install the alias file");
    assert!(sh.contains("/etc/fonts/conf.d/60-bifrost-aliases.conf"));
}
