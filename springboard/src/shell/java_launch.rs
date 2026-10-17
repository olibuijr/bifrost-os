//! Launching Java programs from the desktop (`JavaLaunching` parity: the code that starts a Java application bundle).
//!
//! A Java program reaches the desktop as a `.jar` file (Open With, a double click in Files, a Dock item) or as a
//! description that names a main class and a class path. This module turns either into the command line of the Java
//! runtime (`jdk-openjdk` in `packaging/arch/runtime-packages.txt` provides `java`):
//!
//! - [`parse_manifest`] reads the main section of a jar's `META-INF/MANIFEST.MF` (`Main-Class`, `Class-Path`), with the
//!   72-byte line wrapping of the format; [`manifest_from_jar`] reads it out of the archive.
//! - [`JavaSpec`] is what to run: a jar or a main class, the class path, runtime options, system properties and
//!   program arguments. [`JavaSpec::argv`] builds the command line; it never goes through a shell.
//! - [`find_java`] picks the runtime (`JAVA_HOME`, then `PATH`, then the Arch default), [`parse_java_version`] reads the
//!   version a runtime prints, and [`version_ok`] checks a requirement such as `17+` against it.
//! - Runtime options are limited to the safe set ([`option_allowed`]): memory and stack sizes, system properties,
//!   assertions. Options that load native code or open the runtime's internals (`-agentlib`, `-javaagent`,
//!   `--add-opens`, ...) are refused, because a jar from the internet must not choose them.
//!
//! The Java program runs as the person, in the app sandbox the shell gives any external program; nothing here widens it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The main section of a jar manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub main_class: Option<String>,
    /// `Class-Path` entries: relative URLs, separated by spaces in the file.
    pub class_path: Vec<String>,
    /// `Multi-Release: true`.
    pub multi_release: bool,
}

/// Reads the main section (up to the first blank line). Lines wrapped with a leading space continue the previous
/// line. Attribute names compare without regard to case. Text without `Main-Class` still parses (a library jar).
pub fn parse_manifest(text: &str) -> Manifest {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if raw.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        match (raw.strip_prefix(' '), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    let mut m = Manifest::default();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "main-class" if !value.is_empty() => m.main_class = Some(value.to_string()),
            "class-path" => m.class_path = value.split_whitespace().map(str::to_string).collect(),
            "multi-release" => m.multi_release = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    m
}

/// The manifest inside a jar (a ZIP archive). A jar without `META-INF/MANIFEST.MF` has an empty manifest.
pub fn manifest_from_jar(path: &Path) -> std::io::Result<Manifest> {
    let file = std::fs::File::open(path)?;
    let mut archive = crate::files::zip::Archive::open(file)?;
    let member = archive.members.iter().find(|m| m.name.eq_ignore_ascii_case("META-INF/MANIFEST.MF")).cloned();
    match member {
        Some(m) => Ok(parse_manifest(&String::from_utf8_lossy(&archive.read(&m)?))),
        None => Ok(Manifest::default()),
    }
}

/// Why a Java program cannot be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// The jar names no `Main-Class`: it is a library, not a program.
    NoMainClass,
    /// A runtime option outside the safe set.
    OptionRefused(String),
    /// A system property name or a class name that cannot be passed safely.
    BadName(String),
    /// No Java runtime is installed.
    NoRuntime,
    /// The runtime found is older than the program needs.
    RuntimeTooOld { need: String, have: u32 },
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::NoMainClass => write!(f, "This archive is a library, not a program: it names no main class."),
            LaunchError::OptionRefused(o) => write!(f, "The option {o} is not allowed for programs opened from the desktop."),
            LaunchError::BadName(n) => write!(f, "{n} is not a valid name."),
            LaunchError::NoRuntime => write!(f, "No Java runtime is installed."),
            LaunchError::RuntimeTooOld { need, have } => write!(f, "This program needs Java {need}; Java {have} is installed."),
        }
    }
}

impl std::error::Error for LaunchError {}

/// A Java program to start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JavaSpec {
    /// Run `-jar` this archive (its manifest names the main class and extends the class path).
    pub jar: Option<PathBuf>,
    /// Run this class instead, with `class_path`.
    pub main_class: Option<String>,
    pub class_path: Vec<PathBuf>,
    /// Runtime options such as `-Xmx512m`, checked by [`option_allowed`].
    pub vm_options: Vec<String>,
    /// System properties, passed as `-Dname=value`.
    pub properties: BTreeMap<String, String>,
    /// Arguments of the program itself.
    pub args: Vec<String>,
    /// Required runtime: `17` (exactly), `17+` (at least).
    pub requires: Option<String>,
}

/// Whether a runtime option may be passed. Allowed: `-Xmx`/`-Xms`/`-Xss` with a size, `-ea`, `-da`, `-enableassertions`,
/// `-disableassertions`, `-Dname=value`, `-XX:+UseG1GC`-style flags from a short list, and `-Dfile.encoding`.
pub fn option_allowed(opt: &str) -> bool {
    if let Some(size) = ["-Xmx", "-Xms", "-Xss"].iter().find_map(|p| opt.strip_prefix(p)) {
        let digits = size.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
        return !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && size.len() - digits.len() <= 1;
    }
    if let Some(prop) = opt.strip_prefix("-D") {
        return valid_property_name(prop.split('=').next().unwrap_or(""));
    }
    matches!(
        opt,
        "-ea" | "-da" | "-enableassertions" | "-disableassertions" | "-Xshare:auto" | "-XX:+UseG1GC" | "-XX:+UseSerialGC" | "-XX:+UseZGC" | "-XX:+UseParallelGC" | "-XX:+HeapDumpOnOutOfMemoryError"
    )
}

fn valid_property_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A Java class name such as `com.example.Main` (dots and identifier characters; `$` for nested classes).
pub fn valid_class_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$') && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        })
}

impl JavaSpec {
    /// The program a jar names. A jar without `Main-Class` is a library and is refused.
    pub fn for_jar(jar: &Path, manifest: &Manifest) -> Result<JavaSpec, LaunchError> {
        if manifest.main_class.is_none() {
            return Err(LaunchError::NoMainClass);
        }
        Ok(JavaSpec { jar: Some(jar.to_path_buf()), ..JavaSpec::default() })
    }

    /// Checks every part of the spec; the first problem is returned.
    pub fn validate(&self) -> Result<(), LaunchError> {
        for o in &self.vm_options {
            if !option_allowed(o) {
                return Err(LaunchError::OptionRefused(o.clone()));
            }
        }
        for k in self.properties.keys() {
            if !valid_property_name(k) {
                return Err(LaunchError::BadName(k.clone()));
            }
        }
        match (&self.jar, &self.main_class) {
            (None, None) => return Err(LaunchError::NoMainClass),
            (None, Some(c)) if !valid_class_name(c) => return Err(LaunchError::BadName(c.clone())),
            _ => {}
        }
        Ok(())
    }

    /// The command line, `java` first. Arguments are separate strings (no shell). Options never start with the
    /// program's own arguments, and `--` is not needed because the program's arguments follow the jar or class name.
    pub fn argv(&self, java: &Path) -> Result<Vec<String>, LaunchError> {
        self.validate()?;
        let mut out = vec![java.to_string_lossy().into_owned()];
        out.extend(self.vm_options.iter().cloned());
        for (k, v) in &self.properties {
            out.push(format!("-D{k}={v}"));
        }
        match (&self.jar, &self.main_class) {
            (Some(jar), _) => {
                out.push("-jar".into());
                out.push(jar.to_string_lossy().into_owned());
            }
            (None, Some(class)) => {
                if !self.class_path.is_empty() {
                    out.push("-cp".into());
                    out.push(self.class_path.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(":"));
                }
                out.push(class.clone());
            }
            (None, None) => return Err(LaunchError::NoMainClass),
        }
        out.extend(self.args.iter().cloned());
        Ok(out)
    }
}

/// The Java runtime to use: `$JAVA_HOME/bin/java`, then `java` in a `PATH` directory, then the Arch default link
/// (`/usr/lib/jvm/default/bin/java`). `is_file` answers whether a path is a file (a closure so tests need no disk).
pub fn find_java(java_home: Option<&str>, path_var: &str, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = java_home.filter(|h| !h.is_empty()) {
        candidates.push(Path::new(home).join("bin/java"));
    }
    candidates.extend(path_var.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("java")));
    candidates.push(PathBuf::from("/usr/lib/jvm/default/bin/java"));
    candidates.into_iter().find(|p| p.is_absolute() && is_file(p))
}

/// The major version in the text `java -version` prints: `openjdk version "21.0.2" ...` is 21; the old scheme
/// `"1.8.0_392"` is 8.
pub fn parse_java_version(text: &str) -> Option<u32> {
    let start = text.find("version \"")? + "version \"".len();
    let v = &text[start..start + text[start..].find('"')?];
    let mut parts = v.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// Whether a runtime of major version `have` meets `requires`: `17` means exactly 17, `17+` at least 17, and the old
/// `1.8+` form means 8 or later. `None` or an empty requirement is always met.
pub fn version_ok(requires: Option<&str>, have: u32) -> bool {
    let Some(req) = requires.map(str::trim).filter(|r| !r.is_empty()) else { return true };
    let (num, plus) = match req.strip_suffix('+') {
        Some(n) => (n, true),
        None => (req, false),
    };
    let num = num.strip_prefix("1.").unwrap_or(num);
    match num.parse::<u32>() {
        Ok(n) if plus => have >= n,
        Ok(n) => have == n,
        Err(_) => false,
    }
}

/// Starts to finish a launch: checks the runtime against the requirement and builds the command line.
pub fn plan(spec: &JavaSpec, java: Option<&Path>, java_major: Option<u32>) -> Result<Vec<String>, LaunchError> {
    let java = java.ok_or(LaunchError::NoRuntime)?;
    if let (Some(need), Some(have)) = (spec.requires.as_deref(), java_major) {
        if !version_ok(Some(need), have) {
            return Err(LaunchError::RuntimeTooOld { need: need.to_string(), have });
        }
    }
    spec.argv(java)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_main_section_is_read_with_wrapped_lines() {
        let text = "Manifest-Version: 1.0\r\nMain-Class: com.example.app.Mai\r\n n\r\nclass-path: lib/a.jar lib/b.\r\n jar lib/c.jar\r\nMulti-Release: TRUE\r\n\r\nName: other/Section\r\nMain-Class: not.this.One\r\n";
        let m = parse_manifest(text);
        assert_eq!(m.main_class.as_deref(), Some("com.example.app.Main"));
        assert_eq!(m.class_path, vec!["lib/a.jar", "lib/b.jar", "lib/c.jar"]);
        assert!(m.multi_release, "only the main section counts, and the value is case blind");
        assert_eq!(parse_manifest("Manifest-Version: 1.0\n"), Manifest::default());
        assert_eq!(parse_manifest("\n\nMain-Class: a.B\n").main_class.as_deref(), Some("a.B"));
    }

    #[test]
    fn a_jar_is_read_from_its_archive() {
        let dir = std::env::temp_dir().join(format!("java-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("app.jar");
        let mut w = crate::files::zip::Writer::new(std::fs::File::create(&jar).unwrap());
        w.add_bytes("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\nMain-Class: demo.Main\n", 0).unwrap();
        w.add_bytes("demo/Main.class", b"\xca\xfe\xba\xbe", 0).unwrap();
        w.finish().unwrap();
        let m = manifest_from_jar(&jar).unwrap();
        assert_eq!(m.main_class.as_deref(), Some("demo.Main"));
        let spec = JavaSpec::for_jar(&jar, &m).unwrap();
        let argv = spec.argv(Path::new("/usr/bin/java")).unwrap();
        assert_eq!(argv[..2], ["/usr/bin/java".to_string(), "-jar".to_string()]);
        assert!(argv[2].ends_with("app.jar"));

        let lib = dir.join("lib.jar");
        let mut w = crate::files::zip::Writer::new(std::fs::File::create(&lib).unwrap());
        w.add_bytes("x/Y.class", b"1234", 0).unwrap();
        w.finish().unwrap();
        let m = manifest_from_jar(&lib).unwrap();
        assert_eq!(JavaSpec::for_jar(&lib, &m), Err(LaunchError::NoMainClass));
        assert!(manifest_from_jar(&dir.join("missing.jar")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_command_line_orders_options_properties_target_and_arguments() {
        let mut spec = JavaSpec {
            main_class: Some("com.example.Main".into()),
            class_path: vec!["/opt/app/a.jar".into(), "/opt/app/lib/b.jar".into()],
            vm_options: vec!["-Xmx512m".into(), "-ea".into()],
            args: vec!["--file".into(), "my file.txt".into()],
            ..JavaSpec::default()
        };
        spec.properties.insert("user.theme".into(), "dark".into());
        let argv = spec.argv(Path::new("/usr/bin/java")).unwrap();
        assert_eq!(
            argv,
            ["/usr/bin/java", "-Xmx512m", "-ea", "-Duser.theme=dark", "-cp", "/opt/app/a.jar:/opt/app/lib/b.jar", "com.example.Main", "--file", "my file.txt"]
        );
    }

    #[test]
    fn unsafe_options_and_names_are_refused() {
        for bad in ["-javaagent:/tmp/x.jar", "-agentlib:jdwp=transport=dt_socket", "--add-opens=java.base/java.lang=ALL-UNNAMED", "-Xrunhprof", "-Xmx", "-Xmx12q", "-Xbootclasspath/a:/tmp/x", "-D=x", "-Da b=1"] {
            let spec = JavaSpec { main_class: Some("a.B".into()), vm_options: vec![bad.to_string()], ..JavaSpec::default() };
            assert_eq!(spec.validate(), Err(LaunchError::OptionRefused(bad.to_string())), "{bad}");
        }
        assert!(option_allowed("-Xss1m") && option_allowed("-Dfile.encoding=UTF-8") && option_allowed("-XX:+UseZGC"));
        let spec = JavaSpec { main_class: Some("a.B; rm -rf /".into()), ..JavaSpec::default() };
        assert!(matches!(spec.validate(), Err(LaunchError::BadName(_))));
        let spec = JavaSpec { main_class: Some("a.B".into()), properties: [("bad key".to_string(), "v".to_string())].into(), ..JavaSpec::default() };
        assert!(matches!(spec.validate(), Err(LaunchError::BadName(_))));
        assert_eq!(JavaSpec::default().validate(), Err(LaunchError::NoMainClass));
        assert!(valid_class_name("com.example.Outer$Inner") && !valid_class_name("com..Main") && !valid_class_name("1abc.Main"));
    }

    #[test]
    fn the_runtime_comes_from_java_home_then_path_then_the_arch_default() {
        fn has(present: &'static [&'static str]) -> impl Fn(&Path) -> bool {
            move |p| present.contains(&p.to_str().unwrap())
        }
        assert_eq!(find_java(Some("/opt/jdk"), "/usr/bin", has(&["/opt/jdk/bin/java", "/usr/bin/java"])), Some(PathBuf::from("/opt/jdk/bin/java")));
        assert_eq!(find_java(Some("/opt/jdk"), "/bin:/usr/bin", has(&["/usr/bin/java"])), Some(PathBuf::from("/usr/bin/java")));
        assert_eq!(find_java(None, "", has(&["/usr/lib/jvm/default/bin/java"])), Some(PathBuf::from("/usr/lib/jvm/default/bin/java")));
        assert_eq!(find_java(None, "relative/dir", has(&["relative/dir/java"])), None, "a relative PATH entry is never run");
        assert_eq!(find_java(Some(""), "/usr/bin", has(&[])), None);
    }

    #[test]
    fn versions_are_read_and_requirements_checked() {
        assert_eq!(parse_java_version("openjdk version \"21.0.2\" 2024-01-16\nOpenJDK Runtime Environment"), Some(21));
        assert_eq!(parse_java_version("java version \"1.8.0_392\""), Some(8));
        assert_eq!(parse_java_version("openjdk version \"17\" 2021-09-14"), Some(17));
        assert_eq!(parse_java_version("not java"), None);
        assert!(version_ok(None, 8) && version_ok(Some(""), 8));
        assert!(version_ok(Some("17+"), 21) && version_ok(Some("17+"), 17) && !version_ok(Some("17+"), 11));
        assert!(version_ok(Some("17"), 17) && !version_ok(Some("17"), 21));
        assert!(version_ok(Some("1.8+"), 11) && !version_ok(Some("banana"), 11));
    }

    #[test]
    fn plan_reports_a_missing_or_old_runtime() {
        let spec = JavaSpec { main_class: Some("a.B".into()), requires: Some("17+".into()), ..JavaSpec::default() };
        assert_eq!(plan(&spec, None, None), Err(LaunchError::NoRuntime));
        assert_eq!(plan(&spec, Some(Path::new("/usr/bin/java")), Some(11)), Err(LaunchError::RuntimeTooOld { need: "17+".into(), have: 11 }));
        assert_eq!(plan(&spec, Some(Path::new("/usr/bin/java")), Some(21)).unwrap(), ["/usr/bin/java", "a.B"]);
        assert!(LaunchError::NoRuntime.to_string().contains("No Java runtime"));
    }
}
