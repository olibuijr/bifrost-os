//! The computer's readable names, for Settings > General > About and the About This Computer window: the model out
//! of the firmware's DMI strings and the chip out of the processor's name. One place, so the pane and the window agree.
//!
//! A PC's DMI `product_name` is often a type code (a Lenovo ThinkCentre M900 reports `10FLS2WK00`) and the human name
//! sits in `product_version` or `product_family`; boards with a bare desktop board fill every field with a placeholder
//! ("To Be Filled By O.E.M."). `model_name` takes the first string that reads as a name.

/// The firmware strings that can name the machine (`/sys/class/dmi/id/*`), each trimmed, empty when missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dmi {
    pub product_name: String,
    pub product_version: String,
    pub product_family: String,
    pub board_name: String,
}

/// Where the firmware strings are.
const DMI_DIR: &str = "/sys/class/dmi/id";

/// Reads the firmware strings from `dir` (missing files are empty strings).
pub fn read_dmi_from(dir: &std::path::Path) -> Dmi {
    let get = |name: &str| std::fs::read_to_string(dir.join(name)).map(|s| s.trim_end_matches('\0').trim().to_string()).unwrap_or_default();
    Dmi { product_name: get("product_name"), product_version: get("product_version"), product_family: get("product_family"), board_name: get("board_name") }
}

/// The firmware strings of this machine.
pub fn read_dmi() -> Dmi {
    read_dmi_from(std::path::Path::new(DMI_DIR))
}

/// Strings firmware makers leave where they never filled a field in, compared in lower case.
const PLACEHOLDERS: &[&str] = &[
    "to be filled by o.e.m.",
    "to be filled by o.e.m",
    "system product name",
    "system version",
    "system name",
    "default string",
    "not specified",
    "not applicable",
    "o.e.m.",
    "oem",
    "none",
    "n/a",
    "unknown",
    "undefined",
    "type1productconfigid",
    "type1familyconfig",
    "desktop",
    "notebook",
    "laptop",
    "server",
    "computer",
    "all series",
    "pc",
];

/// A version string such as "1.0", "V1.2", "Rev 3" or "0.1": it says nothing of the machine.
fn looks_like_version(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    // an optional prefix word, then only digits and separators
    let rest = ["version", "ver.", "ver", "rev.", "rev", "v"].iter().find_map(|p| l.strip_prefix(p)).unwrap_or(&l).trim();
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// A type code such as `10FLS2WK00` or `20XW0001US`: one word of capitals and digits, at least seven characters, at least
/// three of them digits. A human name has a space or lower case letters in it.
pub fn is_type_code(s: &str) -> bool {
    let s = s.trim();
    s.len() >= 7 && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && s.chars().filter(char::is_ascii_digit).count() >= 3
}

/// Whether `s` can stand as a name at all: not empty, not a placeholder, not a version.
fn is_usable(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !PLACEHOLDERS.contains(&t.to_ascii_lowercase().as_str()) && !looks_like_version(t)
}

/// The machine's name from its firmware strings: a readable `product_name` first, else the human name a type-code
/// `product_name` leaves in `product_version` or `product_family`, else the board's name, else the type code itself.
/// None when the firmware names nothing.
pub fn model_name(d: &Dmi) -> Option<String> {
    let name = d.product_name.trim();
    if is_usable(name) && !is_type_code(name) {
        return Some(name.to_string());
    }
    [d.product_version.as_str(), d.product_family.as_str(), d.board_name.as_str()]
        .into_iter()
        .map(str::trim)
        .find(|s| is_usable(s) && !is_type_code(s))
        .map(str::to_string)
        .or_else(|| is_usable(name).then(|| name.to_string()))
}

/// A processor's name as a person says it: the trademark marks, the word "CPU" or "Processor", the core count and the
/// clock speed go ("Intel(R) Core(TM) i7-6700T CPU @ 2.80GHz" is "Intel Core i7-6700T"). Only whole words are dropped, so
/// the name never ends in the middle of a word.
pub fn chip_name(raw: &str) -> String {
    let mut s = raw.replace("(R)", "").replace("(TM)", "").replace("(r)", "").replace("(tm)", "");
    // the clock speed and what an integrated part adds ("with Radeon Graphics") come after the name
    for cut in [" @ ", " with "] {
        if let Some(at) = s.find(cut) {
            s.truncate(at);
        }
    }
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| {
            let l = w.to_ascii_lowercase();
            l != "cpu" && l != "processor" && !(l.ends_with("-core") && l.trim_end_matches("-core").chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    words.join(" ")
}

/// The physical cores in a `/proc/cpuinfo` text: the unique (physical id, core id) pairs, so the two hardware threads of
/// a hyper-threaded core count once. A processor list without core ids (the ARM layout, where every processor is a
/// core) counts its processors. 0 when the text lists none.
pub fn physical_cores(cpuinfo: &str) -> usize {
    let mut cores: Vec<(&str, &str)> = Vec::new();
    let mut processors = 0;
    for block in cpuinfo.split("\n\n") {
        let field = |name: &str| block.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == name).map(|(_, v)| v.trim()));
        if field("processor").is_some() {
            processors += 1;
        }
        if let Some(core) = field("core id") {
            let pair = (field("physical id").unwrap_or("0"), core);
            if !cores.contains(&pair) {
                cores.push(pair);
            }
        }
    }
    if cores.is_empty() { processors } else { cores.len() }
}

/// About's processor line: the chip's name and its core count ("Intel Core i7-6700T, 4 cores"). Either part is left out
/// when it is unknown (`soc` empty, `cores` 0).
pub fn chip_line(soc: &str, cores: usize) -> String {
    let count = match cores {
        0 => String::new(),
        1 => "1 core".to_string(),
        n => format!("{n} cores"),
    };
    match (soc.is_empty(), count.is_empty()) {
        (_, true) => soc.to_string(),
        (true, false) => count,
        (false, false) => format!("{soc}, {count}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dmi(name: &str, version: &str, family: &str, board: &str) -> Dmi {
        Dmi { product_name: name.into(), product_version: version.into(), product_family: family.into(), board_name: board.into() }
    }

    #[test]
    fn a_type_code_gives_way_to_the_human_name_in_product_version() {
        // the Midget lab machine: a ThinkCentre M900 (E2E AB1)
        let d = dmi("10FLS2WK00", "ThinkCentre M900", "ThinkCentre M900", "SKYBAY");
        assert_eq!(model_name(&d).as_deref(), Some("ThinkCentre M900"));
        // the family names it when the version is a version
        assert_eq!(model_name(&dmi("20XW0001US", "1.0", "ThinkPad X1 Carbon", "")).as_deref(), Some("ThinkPad X1 Carbon"));
    }

    #[test]
    fn a_readable_product_name_is_kept() {
        assert_eq!(model_name(&dmi("OptiPlex 7040", "", "OptiPlex", "0Y7WYT")).as_deref(), Some("OptiPlex 7040"));
        assert_eq!(model_name(&dmi("HP EliteDesk 800 G2 DM 35W", "Rev A", "103C_53307F", "")).as_deref(), Some("HP EliteDesk 800 G2 DM 35W"));
    }

    #[test]
    fn placeholders_and_versions_never_name_a_machine() {
        let d = dmi("System Product Name", "System Version", "To Be Filled By O.E.M.", "PRIME B250M-A");
        assert_eq!(model_name(&d).as_deref(), Some("PRIME B250M-A"), "the board names it");
        assert_eq!(model_name(&dmi("Default string", "V1.0", "Desktop", "")), None);
        assert_eq!(model_name(&Dmi::default()), None);
    }

    #[test]
    fn the_type_code_is_the_last_resort_not_a_blank() {
        assert_eq!(model_name(&dmi("10FLS2WK00", "", "", "")).as_deref(), Some("10FLS2WK00"));
    }

    #[test]
    fn type_codes_and_versions_are_told_from_names() {
        assert!(is_type_code("10FLS2WK00"));
        assert!(is_type_code("20XW0001US"));
        assert!(!is_type_code("ThinkCentre M900"));
        assert!(!is_type_code("OptiPlex"));
        assert!(!is_type_code("B450M"), "short board names are names");
        for v in ["1.0", "V1.2", "Rev 3", "0.1", "version 2", "v 4.5"] {
            assert!(looks_like_version(v), "{v}");
        }
        for n in ["ThinkCentre M900", "Version One", "Vostro 3470", "Rev Pro"] {
            assert!(!looks_like_version(n), "{n}");
        }
    }

    #[test]
    fn a_chip_loses_its_marks_clock_and_core_count_but_never_half_a_word() {
        assert_eq!(chip_name("Intel(R) Core(TM) i7-6700T CPU @ 2.80GHz"), "Intel Core i7-6700T");
        assert_eq!(chip_name("Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz"), "Intel Xeon E5-2680 v4");
        assert_eq!(chip_name("AMD Ryzen 7 5800X 8-Core Processor"), "AMD Ryzen 7 5800X");
        assert_eq!(chip_name("AMD Ryzen 5 5600G with Radeon Graphics"), "AMD Ryzen 5 5600G");
        assert_eq!(chip_name("Cortex-A53"), "Cortex-A53");
        assert_eq!(chip_name(""), "");
    }

    #[test]
    fn the_firmware_strings_are_read_from_a_directory() {
        let dir = std::env::temp_dir().join(format!("bifrost-dmi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("product_name"), "10FLS2WK00\n").unwrap();
        std::fs::write(dir.join("product_version"), "ThinkCentre M900\n").unwrap();
        let d = read_dmi_from(&dir);
        assert_eq!(d.product_version, "ThinkCentre M900");
        assert_eq!(d.board_name, "", "a missing file is empty");
        assert_eq!(model_name(&d).as_deref(), Some("ThinkCentre M900"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The i7-6700T of the Midget lab machine: four cores, two hardware threads each (eight processors).
    fn skylake_cpuinfo() -> String {
        (0..8).map(|n| format!("processor\t: {n}\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Core(TM) i7-6700T CPU @ 2.80GHz\nphysical id\t: 0\nsiblings\t: 8\ncore id\t\t: {}\ncpu cores\t: 4\n\n", n % 4)).collect()
    }

    #[test]
    fn hardware_threads_are_not_counted_as_cores() {
        assert_eq!(physical_cores(&skylake_cpuinfo()), 4, "8 processors, 4 cores");
    }

    #[test]
    fn cores_are_unique_per_socket_and_core_id() {
        let two_sockets: String = (0..4).map(|n| format!("processor\t: {n}\nphysical id\t: {}\ncore id\t\t: {}\n\n", n / 2, n % 2)).collect();
        assert_eq!(physical_cores(&two_sockets), 4, "core 0 of each socket is its own core");
        let no_socket: String = (0..4).map(|n| format!("processor\t: {n}\ncore id\t\t: {}\n\n", n % 2)).collect();
        assert_eq!(physical_cores(&no_socket), 2, "a missing physical id is socket 0");
    }

    #[test]
    fn a_layout_without_core_ids_counts_processors_and_nothing_counts_zero() {
        let arm: String = (0..8).map(|n| format!("processor\t: {n}\nBogoMIPS\t: 38.40\n\n")).collect();
        assert_eq!(physical_cores(&arm), 8);
        assert_eq!(physical_cores(""), 0);
        assert_eq!(physical_cores("model name\t: Mystery\n"), 0);
    }

    #[test]
    fn the_chip_line_names_the_chip_and_the_cores_it_knows() {
        assert_eq!(chip_line("Intel Core i7-6700T", 4), "Intel Core i7-6700T, 4 cores");
        assert_eq!(chip_line("Intel Core i7-6700T", 0), "Intel Core i7-6700T", "no count when unknown");
        assert_eq!(chip_line("", 4), "4 cores");
        assert_eq!(chip_line("Tiny", 1), "Tiny, 1 core");
        assert_eq!(chip_line("", 0), "");
    }
}
