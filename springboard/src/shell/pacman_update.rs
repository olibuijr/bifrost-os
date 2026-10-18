//! Software Update on an Arch desktop: the shell is the `springboard-shell` package, so the
//! installed version comes from `pacman -Q` and updates from pacman instead of the OTA server.
//!
//! Checking is read-only. Installing invokes the independent recovery guard,
//! which preserves the shell package and rejects base-system upgrades until a
//! whole-root recovery path is available. The UI confirms before installation.

use super::update::Manifest;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

pub const PACKAGE: &str = "springboard-shell";

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Found {
    /// the installed package version ("0.1.0-1"), when the package is installed
    pub installed: Option<String>,
    pub update: Option<Manifest>,
}

/// "springboard-shell 0.1.0-1" (`pacman -Q`) to "0.1.0-1".
pub fn parse_installed(out: &str) -> Option<String> {
    let mut f = out.split_whitespace();
    (f.next()? == PACKAGE).then(|| f.next().map(String::from)).flatten()
}

/// The new version from `checkupdates` or `pacman -Qu` output, whose lines read
/// "springboard-shell 0.1.0-1 -> 0.2.0-1".
pub fn parse_update(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[0] == PACKAGE && f[2] == "->").then(|| f[3].to_string())
    })
}

/// "Download Size  : 12.30 MiB" from `pacman -Si` in bytes; 0 when absent.
pub fn parse_download_size(si: &str) -> u64 {
    let Some(line) = si.lines().find(|l| l.starts_with("Download Size")) else { return 0 };
    let mut f = line.split(':').nth(1).unwrap_or("").split_whitespace();
    let (Some(n), Some(unit)) = (f.next().and_then(|n| n.parse::<f64>().ok()), f.next()) else { return 0 };
    let mult = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return 0,
    };
    (n * mult) as u64
}

/// The "Description" and "Version" style value of a `pacman -Si` field.
fn field(si: &str, name: &str) -> Option<String> {
    si.lines().find(|l| l.starts_with(name))?.split_once(':').map(|(_, v)| v.trim().to_string())
}

fn manifest(version: String, si: &str) -> Manifest {
    Manifest { version, build: None, sha256: String::new(), size: parse_download_size(si), url: String::new(), notes: field(si, "Description"), changelog: vec![] }
}

fn run(cmd: &str, args: &[&str]) -> Result<(i32, String), String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    Ok((out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned()))
}

fn on_path(name: &str) -> bool {
    std::env::var("PATH").is_ok_and(|p| p.split(':').any(|d| std::path::Path::new(d).join(name).is_file()))
}

/// Asks what is installed and whether the package has an update; blocking. A user session asks privd
/// (which refreshes the sync databases as root); the root shell, the installer and host tests run it here.
pub fn check() -> Result<Found, String> {
    let c = crate::privd_client::connect();
    if c.is_local() {
        return check_local();
    }
    let i = c.update_check().map_err(|e| e.to_string())?;
    let update = i.version.map(|v| Manifest { version: v, build: None, sha256: String::new(), size: i.size, url: String::new(), notes: i.notes, changelog: vec![] });
    Ok(Found { installed: i.installed, update })
}

/// pacman's answer, run in this process (privd and root callers).
pub fn check_local() -> Result<Found, String> {
    if !on_path("pacman") {
        return Err("pacman is not installed".into());
    }
    let installed = run("pacman", &["-Q", PACKAGE]).ok().and_then(|(_, o)| parse_installed(&o));
    if installed.is_none() {
        return Err(format!("{PACKAGE} is not installed as a package"));
    }
    // checkupdates exits 2 when nothing is pending, 0 with a list
    // `checkupdates` needs fakeroot; without it (or `checkupdates`) a root shell refreshes the sync databases itself
    let pending = if on_path("checkupdates") && on_path("fakeroot") {
        run("checkupdates", &[])?.1
    } else {
        if is_root() {
            let _ = run("pacman", &["-Sy", "--noconfirm"]);
        }
        run("pacman", &["-Qu", PACKAGE])?.1
    };
    let update = parse_update(&pending).map(|v| manifest(v, &run("pacman", &["-Si", PACKAGE]).map(|r| r.1).unwrap_or_default()));
    Ok(Found { installed, update })
}

/// Whether this process is root (the OS shell is; a session shell is not).
fn is_root() -> bool {
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// The command line of the install, as (program, args).
pub fn install_command(root: bool) -> (&'static str, Vec<&'static str>) {
    if root {
        ("/usr/bin/bifrost-update-guard", vec!["install"])
    } else {
        ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "install"])
    }
}

/// The packages the person installed explicitly that Storage may offer to remove: the list `pacman -Qqe` prints,
/// without the ones `remove_args` refuses (the shell, the base system, names that read as flags). Blocking: it reads
/// the local package database, so it runs on a worker.
pub fn removable_packages() -> Vec<String> {
    match run("pacman", &["-Qqe"]) {
        Ok((0, out)) => removable_from(&out),
        _ => Vec::new(),
    }
}

/// The removable names in `pacman -Qqe` output: one name per line, sorted, without duplicates.
pub fn removable_from(listing: &str) -> Vec<String> {
    let mut names: Vec<String> = listing.lines().map(str::trim).filter(|n| remove_args(n).is_ok()).map(str::to_string).collect();
    names.sort();
    names.dedup();
    names
}

/// Packages that cannot be removed from Software Update: the shell itself and the base system that the recovery
/// guard protects.
pub const PROTECTED_PACKAGES: [&str; 9] = ["springboard-shell", "base", "linux", "linux-rt", "linux-firmware", "pacman", "systemd", "glibc", "sudo"];

/// A plain pacman package name: lower-case letters, digits and `@._+-`, not starting with `-` (so it cannot read as a flag).
pub fn plain_name(package: &str) -> bool {
    !package.is_empty() && package.len() <= 128 && !package.starts_with('-') && package.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "@._+-".contains(c))
}

/// The archive endings pacman installs from a file.
pub const PACKAGE_EXTENSIONS: [&str; 2] = [".pkg.tar.zst", ".pkg.tar.xz"];

/// The package name in a package archive's file name (`name-version-release-arch.pkg.tar.zst`), or `None` when the file is
/// not named like a package. The name is checked like a name given to `remove_args`.
pub fn package_file_name(file: &str) -> Option<String> {
    package_file_info(file).map(|(name, _, _)| name)
}

/// The name, version (with the release, `3.3.0-1`) and architecture a package archive's file name carries
/// (`name-version-release-arch.pkg.tar.zst`), or `None` when the file is not named like a package.
pub fn package_file_info(file: &str) -> Option<(String, String, String)> {
    let base = PACKAGE_EXTENSIONS.iter().find_map(|e| file.strip_suffix(e))?;
    let mut parts = base.rsplitn(4, '-');
    let (arch, release, version, name) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (!arch.is_empty() && !release.is_empty() && !version.is_empty() && plain_name(name)).then(|| (name.to_string(), format!("{version}-{release}"), arch.to_string()))
}

/// Whether a file is named like a package archive.
pub fn is_package_file(file: &str) -> bool {
    package_file_name(file).is_some()
}

/// Most packages one install may name.
pub const MAX_INSTALL_NAMES: usize = 16;

/// The pacman arguments that install packages from the sync databases: `-S --needed --noconfirm` and the names. The shell
/// has already asked the person. Refuses an empty list, too many names, and a name that is not a plain package name.
pub fn install_repo_args(names: &[String]) -> Result<Vec<String>, String> {
    if names.is_empty() || names.len() > MAX_INSTALL_NAMES {
        return Err(format!("an install names 1 to {MAX_INSTALL_NAMES} packages"));
    }
    if let Some(bad) = names.iter().find(|n| !plain_name(n)) {
        return Err(format!("{bad:?} is not a package name"));
    }
    let mut args: Vec<String> = ["-S", "--needed", "--noconfirm"].iter().map(|s| s.to_string()).collect();
    args.extend(names.iter().cloned());
    Ok(args)
}

/// The pacman arguments that install one package archive from `file`, without a prompt (the shell has asked).
pub fn install_file_args(file: &str) -> Vec<String> {
    vec!["-U".into(), "--noconfirm".into(), file.into()]
}

/// The package names in a store link, `bifrost-package:install/name,name` (a `//` after the colon is allowed), or `None` when
/// the text is not such a link or a name is not a plain package name.
pub fn package_link(url: &str) -> Option<Vec<String>> {
    let (scheme, rest) = url.trim().split_once(':')?;
    if !scheme.eq_ignore_ascii_case("bifrost-package") {
        return None;
    }
    let names: Vec<String> = rest.trim_start_matches('/').strip_prefix("install/")?.split(',').map(str::to_string).collect();
    install_repo_args(&names).ok()?;
    Some(names)
}

/// What the person asked to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A package archive on disk (`.pkg.tar.zst`), for instance one downloaded in Files.
    File(std::path::PathBuf),
    /// Packages from the sync databases, named by a store link.
    Repo(Vec<String>),
}

impl Source {
    /// The packages this install adds, for the confirmation sheet, or why it cannot be offered.
    pub fn packages(&self) -> Result<Vec<String>, String> {
        match self {
            Source::File(path) => {
                let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                package_file_name(file).map(|n| vec![n]).ok_or_else(|| format!("{file:?} is not a package file"))
            }
            Source::Repo(names) => install_repo_args(names).map(|_| names.clone()),
        }
    }

    /// What the confirmation sheet tells the person about the source before they authenticate: for a package file its
    /// name, version and that it is unsigned (a local file is not checked against a trusted signature); for repository
    /// packages that pacman checks their signatures.
    pub fn describe(&self) -> String {
        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
        match self {
            Source::File(path) => {
                let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                match package_file_info(file) {
                    Some((name, version, arch)) => format!("{name} {} ({}). Unsigned: a package file you opened is not checked against a trusted signature (pacman -U).", clean(&version), clean(&arch)),
                    None => "Unknown package file.".to_string(),
                }
            }
            Source::Repo(names) => format!("From the package repository: {}. pacman checks their signatures.", names.join(", ")),
        }
    }

    /// Runs the install: through privd for a user session (with the `grant` of a fresh administrator password), in this
    /// process for the root shell and host tests (which ignore the grant). Blocking;
    /// the caller has asked the person.
    pub fn run(&self, grant: &str) -> Result<(), String> {
        self.packages()?;
        let c = crate::privd_client::connect();
        if !c.is_local() {
            return match self {
                Source::File(path) => c.package_install_file(&path.to_string_lossy(), grant),
                Source::Repo(names) => c.package_install_repo(names, grant),
            }
            .map_err(|e| e.to_string());
        }
        let args = match self {
            Source::File(path) => install_file_args(&path.to_string_lossy()),
            Source::Repo(names) => install_repo_args(names)?,
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run("pacman", &args)? {
            (0, _) => Ok(()),
            (code, out) => Err(format!("pacman exited with {code}: {}", last_line(&out))),
        }
    }
}

/// The pacman arguments that remove one package, without a prompt because the shell has already confirmed the
/// removal. Plain `-R` only: pacman refuses to remove a package that others need, and no dependencies are removed
/// with it. Refuses a name that is not a plain package name, a name that reads as a flag, and a protected package.
pub fn remove_args(package: &str) -> Result<Vec<String>, String> {
    if !plain_name(package) {
        return Err(format!("{package:?} is not a package name"));
    }
    if PROTECTED_PACKAGES.contains(&package) {
        return Err(format!("{package} is part of the base system and cannot be removed"));
    }
    Ok(vec!["-R".into(), "--noconfirm".into(), package.into()])
}

/// A package removal the user is asked to confirm. The shell shows `title()`, `body()` and `confirm_label()` in a sheet;
/// only `run()` removes anything, and only after the user chose the confirm button. `ask` refuses a package that
/// `remove_args` would refuse, so the sheet never offers a removal that cannot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removal {
    package: String,
}

impl Removal {
    /// The removal of `package`, offered to the user, or the reason it cannot be offered.
    pub fn ask(package: &str) -> Result<Removal, String> {
        remove_args(package)?;
        Ok(Removal { package: package.to_string() })
    }

    /// The package being removed.
    pub fn package(&self) -> &str {
        &self.package
    }

    /// The sheet's title.
    pub fn title(&self) -> String {
        format!("Remove {}?", self.package)
    }

    /// The sheet's explanation: what leaves the computer and what stays.
    pub fn body(&self) -> String {
        format!("{} and its program files are removed from this computer. Files in your home folder are kept.", self.package)
    }

    /// The label of the button that runs the removal.
    pub fn confirm_label(&self) -> &'static str {
        "Remove"
    }

    /// Runs the removal: through privd for a user session, in this process for the root shell and host tests.
    pub fn run(self) -> Result<(), String> {
        let c = crate::privd_client::connect();
        if !c.is_local() {
            return c.package_remove(&self.package).map_err(|e| e.to_string());
        }
        let args = remove_args(&self.package)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run("pacman", &args)? {
            (0, _) => Ok(()),
            (code, out) => Err(format!("pacman exited with {code}: {}", last_line(&out))),
        }
    }
}

/// The last non-empty line of pacman's output, as the failure text.
fn last_line(s: &str) -> String {
    s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("pacman failed").trim().to_string()
}

/// The command line that starts the rollback, as (program, args). It only starts a separate unit,
/// so it returns at once; the shell is then stopped by the rollback itself.
pub fn rollback_command(root: bool) -> (&'static str, Vec<&'static str>) {
    if root {
        ("/usr/bin/bifrost-update-guard", vec!["rollback-start"])
    } else {
        ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "rollback-start"])
    }
}

fn run_guard(prog: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(prog).args(args).output().map_err(|e| format!("{prog}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let (err, stdout) = (String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
        // the page shows one line; the journal keeps the whole run for diagnosis
        eprintln!("springboard: {prog} {} failed ({}):\n{stdout}{err}", args.join(" "), out.status);
        Err(last_line(if err.trim().is_empty() { &stdout } else { &err }))
    }
}

/// Upgrades the system including the shell package; blocking. The caller has asked the user.
pub fn install() -> Result<(), String> {
    let c = crate::privd_client::connect();
    if c.is_local() {
        return install_local();
    }
    c.update_apply("os").map_err(|e| e.to_string())
}

/// The guard run itself (privd, or the root shell). The caller exits 3 afterwards so the session script applies it.
pub fn install_local() -> Result<(), String> {
    let (prog, args) = install_command(is_root());
    run_guard(prog, &args)
}

/// Starts the rollback to the previous system; the caller has asked the user.
pub fn rollback() -> Result<(), String> {
    let c = crate::privd_client::connect();
    if c.is_local() {
        return rollback_local();
    }
    c.update_rollback().map_err(|e| e.to_string())
}

pub fn rollback_local() -> Result<(), String> {
    let (prog, args) = rollback_command(is_root());
    run_guard(prog, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_file_name_gives_the_package_name() {
        assert_eq!(package_file_name("foo-1.0-1-x86_64.pkg.tar.zst").as_deref(), Some("foo"));
        assert_eq!(package_file_name("gtk4-demo-4.12.0-2-any.pkg.tar.xz").as_deref(), Some("gtk4-demo"));
        assert_eq!(package_file_name("python@3.12-3.12.1-1-x86_64.pkg.tar.zst").as_deref(), Some("python@3.12"));
        for bad in ["foo.pkg.tar.zst", "foo-1.0.pkg.tar.zst", "foo-1.0-1-x86_64.tar.gz", "Foo-1-1-any.pkg.tar.zst", "-foo-1-1-any.pkg.tar.zst", "foo-1-1-.pkg.tar.zst", "notes.txt", ""] {
            assert!(!is_package_file(bad), "{bad:?}");
        }
        assert!(is_package_file("a-1-1-any.pkg.tar.zst"));
    }

    #[test]
    fn repo_installs_name_plain_packages_only() {
        assert_eq!(install_repo_args(&["htop".into(), "gtk4-demo".into()]).unwrap(), vec!["-S", "--needed", "--noconfirm", "htop", "gtk4-demo"]);
        assert!(install_repo_args(&[]).is_err());
        assert!(install_repo_args(&["--overwrite=*".into()]).is_err(), "a flag is not a name");
        assert!(install_repo_args(&["a b".into()]).is_err());
        assert!(install_repo_args(&vec!["a".to_string(); MAX_INSTALL_NAMES + 1]).is_err());
        assert_eq!(install_file_args("/var/x/a-1-1-any.pkg.tar.zst"), vec!["-U", "--noconfirm", "/var/x/a-1-1-any.pkg.tar.zst"]);
    }

    #[test]
    fn a_store_link_names_the_packages_to_install() {
        assert_eq!(package_link("bifrost-package:install/htop"), Some(vec!["htop".to_string()]));
        assert_eq!(package_link("BIFROST-PACKAGE://install/htop,gtk4-demo"), Some(vec!["htop".to_string(), "gtk4-demo".to_string()]));
        for bad in ["https://install/htop", "bifrost-package:remove/htop", "bifrost-package:install/", "bifrost-package:install/htop,", "bifrost-package:install/--force", "bifrost-package:install/ht op", "htop"] {
            assert_eq!(package_link(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_source_lists_the_packages_the_sheet_asks_about() {
        use std::path::PathBuf;
        assert_eq!(Source::File(PathBuf::from("/home/o/Downloads/foo-1.0-1-x86_64.pkg.tar.zst")).packages().unwrap(), vec!["foo"]);
        assert!(Source::File(PathBuf::from("/home/o/Downloads/notes.txt")).packages().is_err());
        assert_eq!(Source::Repo(vec!["a".into(), "b".into()]).packages().unwrap().len(), 2);
        assert!(Source::Repo(vec![]).packages().is_err());
    }

    #[test]
    fn remove_args_name_one_package_without_dependencies() {
        assert_eq!(remove_args("firefox").unwrap(), vec!["-R", "--noconfirm", "firefox"]);
        assert_eq!(remove_args("gtk4-demo+x").unwrap()[2], "gtk4-demo+x");
        assert_eq!(remove_args("python@3.12").unwrap()[2], "python@3.12");
    }

    #[test]
    fn remove_args_refuse_flags_empty_and_unusual_names() {
        assert!(remove_args("").is_err());
        assert!(remove_args("--assume-installed=x").is_err());
        assert!(remove_args("Firefox").is_err(), "package names are lower case");
        assert!(remove_args("foo bar").is_err());
        assert!(remove_args("foo;rm").is_err());
    }

    #[test]
    fn removable_from_keeps_plain_names_once_and_drops_the_base_system() {
        let listing = "zed\nbase\nfirefox\n\n  gimp \nfirefox\nlinux\n--flag\nspringboard-shell\n";
        assert_eq!(removable_from(listing), vec!["firefox", "gimp", "zed"]);
        assert!(removable_from("").is_empty());
    }

    #[test]
    fn a_removal_is_only_offered_for_a_package_that_can_run() {
        let r = Removal::ask("firefox").unwrap();
        assert_eq!(r.package(), "firefox");
        assert_eq!(r.title(), "Remove firefox?");
        assert!(r.body().contains("firefox"));
        assert!(r.body().contains("home folder are kept"));
        assert_eq!(r.confirm_label(), "Remove");
        assert!(Removal::ask("linux").is_err(), "the base system is never offered");
        assert!(Removal::ask("-Syu").is_err());
    }

    #[test]
    fn remove_args_refuse_the_shell_and_the_base_system() {
        for p in PROTECTED_PACKAGES {
            assert!(remove_args(p).is_err(), "{p} must not be removable");
        }
    }

    #[test]
    fn installed_version_is_the_second_word() {
        assert_eq!(parse_installed("springboard-shell 0.1.0-1\n").as_deref(), Some("0.1.0-1"));
        assert_eq!(parse_installed("other 1-1"), None);
        assert_eq!(parse_installed(""), None);
    }

    #[test]
    fn update_lines_name_the_new_version() {
        let out = "linux 7.2.4-1 -> 7.2.5-1\nspringboard-shell 0.1.0-1 -> 0.2.0-1\n";
        assert_eq!(parse_update(out).as_deref(), Some("0.2.0-1"));
        assert_eq!(parse_update("linux 1-1 -> 2-1\n"), None);
        assert_eq!(parse_update(""), None);
    }

    #[test]
    fn download_size_converts_units() {
        assert_eq!(parse_download_size("Name : x\nDownload Size  : 12.00 MiB\nInstalled Size : 1 MiB"), 12 * 1024 * 1024);
        assert_eq!(parse_download_size("Download Size  : 512.00 KiB"), 512 * 1024);
        assert_eq!(parse_download_size("Name : x"), 0);
        assert_eq!(parse_download_size("Download Size  : 1 parsec"), 0);
    }

    #[test]
    fn manifest_carries_size_and_description() {
        let m = manifest("0.2.0-1".into(), "Description    : SpringBoard shell\nDownload Size  : 2.00 MiB\n");
        assert_eq!((m.version.as_str(), m.size, m.notes.as_deref()), ("0.2.0-1", 2 * 1024 * 1024, Some("SpringBoard shell")));
    }

    #[test]
    fn rollback_starts_the_guard_unit_directly_or_through_sudo() {
        assert_eq!(rollback_command(true), ("/usr/bin/bifrost-update-guard", vec!["rollback-start"]));
        assert_eq!(rollback_command(false), ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "rollback-start"]));
    }

    #[test]
    fn root_installs_directly_and_others_through_sudo() {
        assert_eq!(install_command(true), ("/usr/bin/bifrost-update-guard", vec!["install"]));
        assert_eq!(install_command(false), ("sudo", vec!["-n", "/usr/bin/bifrost-update-guard", "install"]));
    }

    #[test]
    fn failure_text_is_the_last_line() {
        assert_eq!(last_line("a\n\nerror: failed to init transaction\n\n"), "error: failed to init transaction");
        assert_eq!(last_line(""), "pacman failed");
    }
}
