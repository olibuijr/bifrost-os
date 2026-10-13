//! `sw_vers`: prints the operating system's name, version and build. The facts come from the os-release file the system
//! ships with: `NAME` for the product name, `VERSION_ID` for the version, `BUILD_ID` for the build (or `VERSION_ID` when
//! there is no build id), and `VARIANT_ID` for the extra version line (empty when absent).

use bifrost_core::sysinfo::{parse_os_release, Roots};
use std::fs;

pub const USAGE: &str = "usage: sw_vers [-productName | -productVersion | -productVersionExtra | -buildVersion]";
/// The product name when the os-release file names none.
pub const DEFAULT_PRODUCT: &str = "BifrOSt";

/// The facts the tool prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versions {
    pub product_name: String,
    pub product_version: String,
    /// The extra version line; empty when the system has none.
    pub product_version_extra: String,
    pub build_version: String,
}

/// One field to print, or all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    All,
    ProductName,
    ProductVersion,
    ProductVersionExtra,
    BuildVersion,
}

/// What the command line asks for: the usage text, or a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Help,
    Show(Field),
}

/// The value of one `KEY=value` line of os-release text, without quotes. Empty when the key is missing.
pub fn field(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

/// The facts from os-release text.
pub fn from_os_release(text: &str) -> Versions {
    let (_, version) = parse_os_release(text);
    let name = field(text, "NAME");
    let build = field(text, "BUILD_ID");
    Versions {
        product_name: if name.is_empty() { DEFAULT_PRODUCT.to_string() } else { name },
        product_version: version.clone(),
        product_version_extra: field(text, "VARIANT_ID"),
        build_version: if build.is_empty() { version } else { build },
    }
}

/// The facts of this system, read from its os-release file.
pub fn current() -> Versions {
    let roots = Roots::system();
    from_os_release(&fs::read_to_string(&roots.os_release).unwrap_or_default())
}

/// Reads the arguments (without the program name). Long options may also be written in lower case.
pub fn parse(args: &[String]) -> Result<Request, String> {
    let [arg] = args else {
        return match args {
            [] => Ok(Request::Show(Field::All)),
            _ => Err("sw_vers takes one option at a time".to_string()),
        };
    };
    let key = arg.trim_start_matches('-').to_ascii_lowercase();
    match key.as_str() {
        "help" | "h" => Ok(Request::Help),
        "productname" => Ok(Request::Show(Field::ProductName)),
        "productversion" => Ok(Request::Show(Field::ProductVersion)),
        "productversionextra" => Ok(Request::Show(Field::ProductVersionExtra)),
        "buildversion" => Ok(Request::Show(Field::BuildVersion)),
        _ => Err(format!("unknown option {arg}")),
    }
}

/// The text for a field, with its line ending.
pub fn render(v: &Versions, field: Field) -> String {
    match field {
        Field::All => {
            let mut out = format!("ProductName:\t{}\nProductVersion:\t{}\n", v.product_name, v.product_version);
            if !v.product_version_extra.is_empty() {
                out.push_str(&format!("ProductVersionExtra:\t{}\n", v.product_version_extra));
            }
            out.push_str(&format!("BuildVersion:\t{}\n", v.build_version));
            out
        }
        Field::ProductName => format!("{}\n", v.product_name),
        Field::ProductVersion => format!("{}\n", v.product_version),
        Field::ProductVersionExtra => format!("{}\n", v.product_version_extra),
        Field::BuildVersion => format!("{}\n", v.build_version),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS_RELEASE: &str = "NAME=\"BifrOSt\"\nPRETTY_NAME=\"BifrOSt 0.6\"\nVERSION_ID=0.6.4\nBUILD_ID=2026.10\n";

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn facts_come_from_the_os_release_fields() {
        let v = from_os_release(OS_RELEASE);
        assert_eq!(v.product_name, "BifrOSt");
        assert_eq!(v.product_version, "0.6.4");
        assert_eq!(v.build_version, "2026.10");
        assert_eq!(v.product_version_extra, "");
    }

    #[test]
    fn missing_fields_fall_back() {
        let v = from_os_release("VERSION_ID=1.2\n");
        assert_eq!(v.product_name, DEFAULT_PRODUCT);
        assert_eq!(v.build_version, "1.2", "no build id means the version is the build");
    }

    #[test]
    fn the_full_listing_has_the_extra_line_only_when_present() {
        let mut v = from_os_release(OS_RELEASE);
        assert_eq!(render(&v, Field::All), "ProductName:\tBifrOSt\nProductVersion:\t0.6.4\nBuildVersion:\t2026.10\n");
        v.product_version_extra = "(a)".into();
        assert!(render(&v, Field::All).contains("ProductVersionExtra:\t(a)\n"));
    }

    #[test]
    fn one_field_prints_only_its_value() {
        let v = from_os_release(OS_RELEASE);
        assert_eq!(render(&v, Field::ProductVersion), "0.6.4\n");
        assert_eq!(render(&v, Field::BuildVersion), "2026.10\n");
    }

    #[test]
    fn options_are_read_in_either_case() {
        assert_eq!(parse(&args(&[])), Ok(Request::Show(Field::All)));
        assert_eq!(parse(&args(&["--productName"])), Ok(Request::Show(Field::ProductName)));
        assert_eq!(parse(&args(&["-productversion"])), Ok(Request::Show(Field::ProductVersion)));
        assert_eq!(parse(&args(&["-h"])), Ok(Request::Help));
        assert!(parse(&args(&["-bogus"])).unwrap_err().contains("unknown option"));
        assert!(parse(&args(&["-productName", "-buildVersion"])).is_err());
    }
}
