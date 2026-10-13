//! `defaults`: reads and writes the preference store that apps and settings use. Each domain is one file,
//! `<root>/appdata/<domain>/defaults.txt`, the same file an app's `UserDefaults` keeps (see `bifrost_core::defaults_file`).
//! The global domain is `GlobalDomain`. Values are strings, booleans, integers, doubles, data, arrays and dictionaries.
//!
//! Supported: `read`, `read-type`, `write` (untyped, typed with `-string -int -float -bool -data -date`, and the
//! collection forms `-array -array-add -dict -dict-add`), `write domain 'plist'`, `rename`, `delete` (a key or a whole
//! domain), `delete-all` (the same here, since there is one host and one container), `import` and `export` (XML text,
//! to a file or `-`), `domains`, `find`, `help`. The options `-currentHost`, `-host` and `-verbose` are accepted; this
//! machine is the one host. Refused with a message rather than run wrongly: `-app` and `-container` (there is no app
//! bundle lookup or container store yet), and a plist file given as the domain. Export writes XML, not a binary plist.

use bifrost_core::defaults_file::{self as store, Value};
use bifrost_core::fsutil;
use bifrost_core::plist::{self, Plist};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: defaults [-currentHost | -host hostname] [-verbose] read [domain [key]]\n\
       defaults [-currentHost | -host hostname] [-verbose] read-type domain key\n\
       defaults [-currentHost | -host hostname] [-verbose] write domain { 'plist' | key 'value' }\n\
       defaults [-currentHost | -host hostname] [-verbose] rename domain old_key new_key\n\
       defaults [-currentHost | -host hostname] [-verbose] delete [domain [key]]\n\
       defaults [-currentHost | -host hostname] [-verbose] { import | export } domain { path | - }\n\
       defaults [-currentHost | -host hostname] [-verbose] { domains | find word | help }";

/// The domain `-g` and `-globalDomain` name.
pub const GLOBAL_DOMAIN: &str = "GlobalDomain";
/// The directory under the store root that holds one folder per domain.
pub const APPDATA: &str = "appdata";

/// The value types a `write` flag gives.
pub const TYPE_FLAGS: &[&str] = &["-string", "-data", "-int", "-integer", "-float", "-bool", "-boolean", "-date", "-array", "-array-add", "-dict", "-dict-add"];

/// How a `write` changes its key.
#[derive(Clone, Debug, PartialEq)]
pub enum Set {
    /// Replaces the value (every typed form, and an untyped one).
    Value(Plist),
    /// Adds elements to the end of an array (`-array-add`), creating the array if the key is absent.
    Append(Vec<Plist>),
    /// Adds or replaces entries of a dictionary (`-dict-add`), creating it if the key is absent.
    Merge(Vec<(String, Plist)>),
}

/// One parsed command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Help,
    Read { domain: Option<String>, key: Option<String> },
    ReadType { domain: String, key: String },
    Write { domain: String, key: String, set: Set },
    /// `write domain 'plist'`: the whole domain is replaced by the dictionary.
    WriteAll { domain: String, value: Plist },
    Rename { domain: String, old: String, new: String },
    Delete { domain: String, key: Option<String> },
    Import { domain: String, path: String },
    Export { domain: String, path: String },
    Domains,
    Find(String),
}

/// Reads the arguments into a command.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-currentHost" | "-verbose" => i += 1,
            "-host" => {
                if i + 1 >= args.len() {
                    return Err("-host needs a host name".into());
                }
                i += 2;
            }
            "-container" | "-app" => return Err(format!("{} is not supported yet", args[i])),
            _ => break,
        }
    }
    let Some(cmd) = args.get(i) else { return Err(USAGE.into()) };
    let rest = &args[i + 1..];
    let want = |n: usize| -> Result<(), String> {
        if rest.len() == n {
            Ok(())
        } else {
            Err(format!("{cmd} takes {n} argument{}\n{USAGE}", if n == 1 { "" } else { "s" }))
        }
    };
    match cmd.as_str() {
        "help" | "-h" | "-help" | "--help" => Ok(Command::Help),
        "read" => match rest.len() {
            0 => Ok(Command::Read { domain: None, key: None }),
            1 => Ok(Command::Read { domain: Some(domain_arg(&rest[0])?), key: None }),
            2 => Ok(Command::Read { domain: Some(domain_arg(&rest[0])?), key: Some(rest[1].clone()) }),
            _ => Err(format!("read takes up to 2 arguments\n{USAGE}")),
        },
        "read-type" => {
            want(2)?;
            Ok(Command::ReadType { domain: domain_arg(&rest[0])?, key: rest[1].clone() })
        }
        "write" => parse_write(rest),
        "rename" => {
            want(3)?;
            Ok(Command::Rename { domain: domain_arg(&rest[0])?, old: rest[1].clone(), new: rest[2].clone() })
        }
        "delete" | "delete-all" => match rest.len() {
            0 => Err("delete needs a domain".into()),
            1 => Ok(Command::Delete { domain: domain_arg(&rest[0])?, key: None }),
            2 => Ok(Command::Delete { domain: domain_arg(&rest[0])?, key: Some(rest[1].clone()) }),
            _ => Err(format!("delete takes up to 2 arguments\n{USAGE}")),
        },
        "import" => {
            want(2)?;
            Ok(Command::Import { domain: domain_arg(&rest[0])?, path: rest[1].clone() })
        }
        "export" => {
            want(2)?;
            Ok(Command::Export { domain: domain_arg(&rest[0])?, path: rest[1].clone() })
        }
        "domains" => {
            want(0)?;
            Ok(Command::Domains)
        }
        "find" => {
            want(1)?;
            Ok(Command::Find(rest[0].clone()))
        }
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

fn parse_write(rest: &[String]) -> Result<Command, String> {
    let domain = domain_arg(rest.first().ok_or("write needs a domain")?)?;
    match rest.len() {
        1 => Err("write needs a key and a value, or a dictionary".into()),
        2 => {
            let value = plist::parse_ascii(&rest[1]).map_err(|e| format!("the value is not a property list: {e}"))?;
            match value {
                Plist::Dict(_) => Ok(Command::WriteAll { domain, value }),
                _ => Err("a whole-domain write needs a dictionary, in the form '{ key = value; }'".into()),
            }
        }
        3 if !TYPE_FLAGS.contains(&rest[2].as_str()) => Ok(Command::Write {
            domain,
            key: rest[1].clone(),
            // an untyped value is a property list when it reads as one, else plain text
            set: Set::Value(plist::parse_ascii(&rest[2]).unwrap_or_else(|_| Plist::Str(rest[2].clone()))),
        }),
        _ if !TYPE_FLAGS.contains(&rest[2].as_str()) => Err(format!("unknown type flag or too many values\n{USAGE}")),
        _ => {
            let flag = rest[2].as_str();
            let values = &rest[3..];
            let set = typed_set(flag, values)?;
            Ok(Command::Write { domain, key: rest[1].clone(), set })
        }
    }
}

/// The value a typed `write` flag makes from its operands.
pub fn typed_set(flag: &str, values: &[String]) -> Result<Set, String> {
    Ok(match flag {
        "-string" => Set::Value(Plist::Str(one_value(flag, values)?.to_string())),
        "-int" | "-integer" => {
            let v = one_value(flag, values)?;
            Set::Value(Plist::Int(v.trim().parse().map_err(|_| format!("{v} is not an integer"))?))
        }
        "-float" => Set::Value(Plist::Real(one_value(flag, values)?.trim().parse().map_err(|_| "the value is not a number".to_string())?)),
        "-bool" | "-boolean" => {
            let v = one_value(flag, values)?;
            Set::Value(Plist::Bool(match v.to_ascii_uppercase().as_str() {
                "YES" | "TRUE" => true,
                "NO" | "FALSE" => false,
                _ => return Err(format!("{v}: -bool needs TRUE, FALSE, YES or NO")),
            }))
        }
        "-data" => Set::Value(Plist::Data(plist::hex_decode(one_value(flag, values)?).ok_or("the data must be hex digits")?)),
        "-date" => {
            let v = one_value(flag, values)?;
            if v.is_empty() {
                return Err("the date is empty".into());
            }
            Set::Value(Plist::Date(v.to_string()))
        }
        "-array" => Set::Value(Plist::Array(values.iter().map(|v| Plist::Str(v.clone())).collect())),
        "-array-add" => Set::Append(values.iter().map(|v| Plist::Str(v.clone())).collect()),
        "-dict" => Set::Value(Plist::Dict(pairs(values)?)),
        "-dict-add" => Set::Merge(pairs(values)?),
        other => return Err(format!("unknown type flag {other}")),
    })
}

/// The single operand of a typed flag.
fn one_value<'a>(flag: &str, values: &'a [String]) -> Result<&'a str, String> {
    match values {
        [v] => Ok(v.as_str()),
        _ => Err(format!("{flag} takes one value")),
    }
}

fn pairs(values: &[String]) -> Result<Vec<(String, Plist)>, String> {
    if values.len() % 2 != 0 {
        return Err("a dictionary needs key and value pairs".into());
    }
    let mut out: Vec<(String, Plist)> = Vec::new();
    for pair in values.chunks(2) {
        let (k, v) = (pair[0].clone(), Plist::Str(pair[1].clone()));
        match out.iter_mut().find(|e| e.0 == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    Ok(out)
}

/// A domain operand: `-g` and `-globalDomain` name the global domain. A name with a slash, or a plist path, is refused.
pub fn domain_arg(arg: &str) -> Result<String, String> {
    if arg == "-g" || arg == "-globalDomain" {
        return Ok(GLOBAL_DOMAIN.to_string());
    }
    if arg.contains('/') || arg.ends_with(".plist") {
        return Err(format!("{arg}: a plist file as a domain is not supported yet"));
    }
    if arg.is_empty() || arg == "." || arg == ".." || arg.chars().any(|c| c.is_control() || c == '\\') {
        return Err(format!("{arg:?} is not a domain name"));
    }
    Ok(arg.to_string())
}

/// The store root: `SPRINGBOARD_DATA` when set, else the user's data directory, the place a user session keeps app data.
pub fn root_from_env(data: Option<&str>, xdg_data_home: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(d) = data.filter(|s| !s.is_empty()) {
        return PathBuf::from(d);
    }
    match (xdg_data_home.filter(|s| !s.is_empty()), home.filter(|s| !s.is_empty())) {
        (Some(x), _) => PathBuf::from(x).join("springboard"),
        (None, Some(h)) => PathBuf::from(h).join(".local/share/springboard"),
        (None, None) => PathBuf::from("/var/lib/springboard"),
    }
}

/// The store root for this process, from the environment.
pub fn root() -> PathBuf {
    root_from_env(
        std::env::var("SPRINGBOARD_DATA").ok().as_deref(),
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// The domain files under one store root.
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    fn dir(&self, domain: &str) -> PathBuf {
        self.root.join(APPDATA).join(domain)
    }

    fn file(&self, domain: &str) -> PathBuf {
        self.dir(domain).join(store::FILE)
    }

    pub fn exists(&self, domain: &str) -> bool {
        self.file(domain).is_file()
    }

    /// The values of a domain. A domain with no file is empty.
    pub fn load(&self, domain: &str) -> Result<BTreeMap<String, Value>, String> {
        match std::fs::read_to_string(self.file(domain)) {
            Ok(text) => Ok(store::parse(&text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(format!("{domain}: {e}")),
        }
    }

    pub fn save(&self, domain: &str, values: &BTreeMap<String, Value>) -> Result<(), String> {
        fsutil::write_atomic(&self.file(domain), store::serialise(values)).map_err(|e| format!("{domain}: {e}"))
    }

    /// Deletes a whole domain. False when it did not exist.
    pub fn remove_domain(&self, domain: &str) -> Result<bool, String> {
        match std::fs::remove_file(self.file(domain)) {
            Ok(()) => {
                let _ = std::fs::remove_dir(self.dir(domain));
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("{domain}: {e}")),
        }
    }

    /// The domains that have a file, sorted.
    pub fn domains(&self) -> Vec<String> {
        let mut out: Vec<String> = match std::fs::read_dir(self.root.join(APPDATA)) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().join(store::FILE).is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        out.sort();
        out
    }
}

/// The text `defaults read` prints for one value: strings raw, booleans as 1 or 0, data as hex, and collections in
/// the ASCII form.
pub fn value_text(p: &Plist) -> String {
    match p {
        Plist::Str(s) | Plist::Date(s) => s.clone(),
        Plist::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
        Plist::Int(n) => n.to_string(),
        Plist::Real(f) => f.to_string(),
        Plist::Data(d) => format!("<{}>", plist::hex_encode(d)),
        Plist::Array(_) | Plist::Dict(_) => plist::to_ascii(p),
    }
}

/// The type word `read-type` prints.
pub fn type_word(p: &Plist) -> &'static str {
    match p {
        Plist::Bool(_) => "boolean",
        other => other.type_name(),
    }
}

fn as_dict(values: &BTreeMap<String, Value>) -> Plist {
    Plist::Dict(values.iter().map(|(k, v)| (k.clone(), v.to_plist())).collect())
}

fn missing_pair(domain: &str, key: &str) -> String {
    format!("The domain/default pair of ({domain}, {key}) does not exist")
}

/// Applies a `write` to a domain's values.
pub fn apply(values: &mut BTreeMap<String, Value>, key: &str, set: Set) -> Result<(), String> {
    match set {
        Set::Value(p) => {
            values.insert(key.to_string(), Value::from_plist(p));
        }
        Set::Append(items) => {
            let mut arr = match values.get(key).map(Value::to_plist) {
                None => Vec::new(),
                Some(Plist::Array(a)) => a,
                Some(_) => return Err(format!("{key}: the value is not an array")),
            };
            arr.extend(items);
            values.insert(key.to_string(), Value::from_plist(Plist::Array(arr)));
        }
        Set::Merge(pairs) => {
            let mut dict = match values.get(key).map(Value::to_plist) {
                None => Vec::new(),
                Some(Plist::Dict(d)) => d,
                Some(_) => return Err(format!("{key}: the value is not a dictionary")),
            };
            for (k, v) in pairs {
                match dict.iter_mut().find(|e| e.0 == k) {
                    Some(slot) => slot.1 = v,
                    None => dict.push((k, v)),
                }
            }
            values.insert(key.to_string(), Value::from_plist(Plist::Dict(dict)));
        }
    }
    Ok(())
}

/// Runs a command against the store and returns the text to print. `stdin` is read only for `import domain -`.
pub fn run(cmd: &Command, store: &Store, stdin: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<String, String> {
    match cmd {
        Command::Help => Ok(format!("{USAGE}\n")),
        Command::Read { domain: None, .. } => {
            let mut out = String::new();
            for d in store.domains() {
                let values = store.load(&d)?;
                out.push_str(&format!("{d} = {};\n", plist::to_ascii(&as_dict(&values))));
            }
            Ok(out)
        }
        Command::Read { domain: Some(d), key: None } => {
            if !store.exists(d) {
                return Err(format!("Domain {d} does not exist"));
            }
            Ok(format!("{}\n", plist::to_ascii(&as_dict(&store.load(d)?))))
        }
        Command::Read { domain: Some(d), key: Some(k) } => {
            let values = store.load(d)?;
            let v = values.get(k).ok_or_else(|| missing_pair(d, k))?;
            Ok(format!("{}\n", value_text(&v.to_plist())))
        }
        Command::ReadType { domain, key } => {
            let values = store.load(domain)?;
            let v = values.get(key).ok_or_else(|| missing_pair(domain, key))?;
            Ok(format!("Type is {}\n", type_word(&v.to_plist())))
        }
        Command::Write { domain, key, set } => {
            let mut values = store.load(domain)?;
            apply(&mut values, key, set.clone())?;
            store.save(domain, &values)?;
            Ok(String::new())
        }
        Command::WriteAll { domain, value } => {
            let Plist::Dict(items) = value else { return Err("a whole-domain write needs a dictionary".into()) };
            let values: BTreeMap<String, Value> = items.iter().map(|(k, v)| (k.clone(), Value::from_plist(v.clone()))).collect();
            store.save(domain, &values)?;
            Ok(String::new())
        }
        Command::Rename { domain, old, new } => {
            let mut values = store.load(domain)?;
            let v = values.remove(old).ok_or_else(|| missing_pair(domain, old))?;
            values.insert(new.clone(), v);
            store.save(domain, &values)?;
            Ok(String::new())
        }
        Command::Delete { domain, key: None } => {
            if store.remove_domain(domain)? {
                Ok(String::new())
            } else {
                Err(format!("Domain {domain} does not exist"))
            }
        }
        Command::Delete { domain, key: Some(k) } => {
            let mut values = store.load(domain)?;
            if values.remove(k).is_some() {
                store.save(domain, &values)?;
            }
            Ok(String::new())
        }
        Command::Import { domain, path } => {
            let bytes = if path == "-" { stdin()? } else { std::fs::read(path).map_err(|e| format!("{path}: {e}"))? };
            let Plist::Dict(items) = plist::parse_bytes(&bytes)? else {
                return Err("the property list must be a dictionary".into());
            };
            let values: BTreeMap<String, Value> = items.into_iter().map(|(k, v)| (k, Value::from_plist(v))).collect();
            store.save(domain, &values)?;
            Ok(String::new())
        }
        Command::Export { domain, path } => {
            if !store.exists(domain) {
                return Err(format!("Domain {domain} does not exist"));
            }
            let text = plist::to_xml(&as_dict(&store.load(domain)?));
            if path == "-" {
                Ok(text)
            } else {
                fsutil::write_atomic(Path::new(path), &text).map_err(|e| format!("{path}: {e}"))?;
                Ok(String::new())
            }
        }
        Command::Domains => {
            let names = store.domains();
            Ok(format!("{}\n", names.join(", ")))
        }
        Command::Find(word) => {
            let needle = word.to_lowercase();
            let mut out = String::new();
            for d in store.domains() {
                if d.to_lowercase().contains(&needle) {
                    out.push_str(&format!("{d}\n"));
                }
                for (k, v) in store.load(&d)? {
                    let text = value_text(&v.to_plist());
                    if k.to_lowercase().contains(&needle) || text.to_lowercase().contains(&needle) {
                        out.push_str(&format!("{d}: {k} = {}\n", text.replace('\n', " ")));
                    }
                }
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-defaults-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn go(store: &Store, v: &[&str]) -> Result<String, String> {
        let cmd = parse(&args(v))?;
        run(&cmd, store, || Err("no stdin in tests".into()))
    }

    #[test]
    fn globals_and_hosts_are_accepted_and_containers_refused() {
        assert_eq!(parse(&args(&["-currentHost", "-verbose", "domains"])), Ok(Command::Domains));
        assert_eq!(parse(&args(&["-host", "box", "domains"])), Ok(Command::Domains));
        assert!(parse(&args(&["-container", "x", "domains"])).unwrap_err().contains("not supported yet"));
        assert!(parse(&args(&["-app", "TextEdit", "read"])).is_err());
        assert!(parse(&args(&[])).unwrap_err().starts_with("usage: defaults"));
        assert!(parse(&args(&["frobnicate"])).unwrap_err().contains("unknown command"));
        assert_eq!(parse(&args(&["help"])), Ok(Command::Help));
    }

    #[test]
    fn domain_names_are_checked() {
        assert_eq!(domain_arg("-g"), Ok(GLOBAL_DOMAIN.to_string()));
        assert_eq!(domain_arg("is.olibuijr.notes"), Ok("is.olibuijr.notes".to_string()));
        assert!(domain_arg("a/b").unwrap_err().contains("not supported yet"));
        assert!(domain_arg("x.plist").is_err());
        assert!(domain_arg("..").is_err());
        assert!(domain_arg("").is_err());
    }

    #[test]
    fn root_comes_from_the_environment_in_order() {
        assert_eq!(root_from_env(Some("/d"), Some("/x"), Some("/h")), PathBuf::from("/d"));
        assert_eq!(root_from_env(Some(""), Some("/x"), Some("/h")), PathBuf::from("/x/springboard"));
        assert_eq!(root_from_env(None, None, Some("/h")), PathBuf::from("/h/.local/share/springboard"));
        assert_eq!(root_from_env(None, None, None), PathBuf::from("/var/lib/springboard"));
    }

    #[test]
    fn write_forms_pick_the_right_set() {
        assert_eq!(
            parse(&args(&["write", "d", "k", "-int", "7"])),
            Ok(Command::Write { domain: "d".into(), key: "k".into(), set: Set::Value(Plist::Int(7)) })
        );
        assert_eq!(
            parse(&args(&["write", "d", "k", "(255, 0, 0)"])),
            Ok(Command::Write {
                domain: "d".into(),
                key: "k".into(),
                set: Set::Value(Plist::Array(vec![Plist::Str("255".into()), Plist::Str("0".into()), Plist::Str("0".into())])),
            })
        );
        assert_eq!(
            parse(&args(&["write", "d", "k", "hello world"])),
            Ok(Command::Write { domain: "d".into(), key: "k".into(), set: Set::Value(Plist::Str("hello world".into())) })
        );
        assert!(matches!(parse(&args(&["write", "d", "{ a = b; }"])), Ok(Command::WriteAll { .. })));
        assert!(parse(&args(&["write", "d", "(1)"])).is_err());
        assert!(parse(&args(&["write", "d", "k", "a", "b"])).is_err());
    }

    #[test]
    fn typed_values_are_checked() {
        assert_eq!(typed_set("-bool", &args(&["YES"])), Ok(Set::Value(Plist::Bool(true))));
        assert_eq!(typed_set("-bool", &args(&["no"])), Ok(Set::Value(Plist::Bool(false))));
        assert!(typed_set("-bool", &args(&["1"])).is_err());
        assert!(typed_set("-int", &args(&["x"])).is_err());
        assert_eq!(typed_set("-float", &args(&["2.5"])), Ok(Set::Value(Plist::Real(2.5))));
        assert_eq!(typed_set("-data", &args(&["00ff"])), Ok(Set::Value(Plist::Data(vec![0, 255]))));
        assert!(typed_set("-data", &args(&["zz"])).is_err());
        assert!(typed_set("-string", &args(&["a", "b"])).is_err());
        assert_eq!(
            typed_set("-dict", &args(&["a", "1", "b", "2", "a", "3"])),
            Ok(Set::Value(Plist::Dict(vec![("a".into(), Plist::Str("3".into())), ("b".into(), Plist::Str("2".into()))])))
        );
        assert!(typed_set("-dict", &args(&["odd"])).is_err());
        assert_eq!(typed_set("-array-add", &args(&["x"])), Ok(Set::Append(vec![Plist::Str("x".into())])));
    }

    #[test]
    fn write_read_and_delete_round_trip_through_the_store() {
        let dir = scratch("roundtrip");
        let s = Store::new(&dir);
        go(&s, &["write", "is.test.app", "Name", "Ada"]).unwrap();
        go(&s, &["write", "is.test.app", "Count", "-int", "3"]).unwrap();
        go(&s, &["write", "is.test.app", "Tags", "-array", "a", "b"]).unwrap();
        go(&s, &["write", "is.test.app", "Tags", "-array-add", "c"]).unwrap();
        go(&s, &["write", "is.test.app", "Opts", "-dict", "x", "1"]).unwrap();
        go(&s, &["write", "is.test.app", "Opts", "-dict-add", "y", "2"]).unwrap();
        assert_eq!(go(&s, &["read", "is.test.app", "Name"]), Ok("Ada\n".into()));
        assert_eq!(go(&s, &["read", "is.test.app", "Count"]), Ok("3\n".into()));
        assert_eq!(go(&s, &["read", "is.test.app", "Tags"]).unwrap(), "(\n    a,\n    b,\n    c,\n)\n");
        assert_eq!(go(&s, &["read-type", "is.test.app", "Tags"]), Ok("Type is array\n".into()));
        assert_eq!(go(&s, &["read-type", "is.test.app", "Count"]), Ok("Type is integer\n".into()));
        assert_eq!(go(&s, &["domains"]), Ok("is.test.app\n".into()));
        assert!(go(&s, &["read", "is.test.app", "Nope"]).unwrap_err().contains("does not exist"));
        assert!(go(&s, &["read", "missing.domain"]).unwrap_err().contains("does not exist"));
        assert!(go(&s, &["write", "is.test.app", "Name", "-int", "1"]).is_ok());
        assert!(go(&s, &["write", "is.test.app", "Tags", "-array-add", "d"]).is_ok());
        assert!(go(&s, &["write", "is.test.app", "Name", "-array-add", "d"]).unwrap_err().contains("not an array"));

        go(&s, &["rename", "is.test.app", "Count", "Total"]).unwrap();
        assert_eq!(go(&s, &["read", "is.test.app", "Total"]), Ok("3\n".into()));
        go(&s, &["delete", "is.test.app", "Total"]).unwrap();
        assert!(go(&s, &["read", "is.test.app", "Total"]).is_err());
        go(&s, &["delete", "is.test.app"]).unwrap();
        assert!(s.domains().is_empty());
        assert!(go(&s, &["delete", "is.test.app"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whole_domain_write_import_and_export_use_xml_text() {
        let dir = scratch("export");
        let s = Store::new(&dir);
        go(&s, &["write", "d.one", "{ \"Default Color\" = (255, 0, 0); \"Default Font\" = Helvetica; }"]).unwrap();
        assert_eq!(go(&s, &["read", "d.one", "Default Font"]), Ok("Helvetica\n".into()));
        let xml = go(&s, &["export", "d.one", "-"]).unwrap();
        assert!(xml.contains("<key>Default Color</key>"), "{xml}");

        let file = dir.join("in.plist");
        std::fs::write(&file, &xml).unwrap();
        go(&s, &["import", "d.two", file.to_str().unwrap()]).unwrap();
        assert_eq!(go(&s, &["read", "d.two", "Default Font"]), Ok("Helvetica\n".into()));
        let out = dir.join("out.plist");
        go(&s, &["export", "d.two", out.to_str().unwrap()]).unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().starts_with("<?xml"));
        assert!(go(&s, &["import", "d.three", "-"]).is_err(), "stdin of tests is refused");
        std::fs::write(&file, "<string>not a dictionary</string>").unwrap();
        assert!(go(&s, &["import", "d.three", file.to_str().unwrap()]).unwrap_err().contains("dictionary"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_matches_domains_keys_and_values() {
        let dir = scratch("find");
        let s = Store::new(&dir);
        go(&s, &["write", "is.a.app", "Theme", "Dark"]).unwrap();
        go(&s, &["write", "is.b.app", "Font", "dark roast"]).unwrap();
        let hits = go(&s, &["find", "DARK"]).unwrap();
        assert!(hits.contains("is.a.app: Theme = Dark"), "{hits}");
        assert!(hits.contains("is.b.app: Font = dark roast"), "{hits}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_value_reads_back_as_the_same_plist() {
        let dir = scratch("nested");
        let s = Store::new(&dir);
        go(&s, &["write", "d", "k", "-data", "0a0b"]).unwrap();
        assert_eq!(go(&s, &["read", "d", "k"]), Ok("<0a0b>\n".into()));
        go(&s, &["write", "d", "b", "-bool", "NO"]).unwrap();
        assert_eq!(go(&s, &["read", "d", "b"]), Ok("0\n".into()));
        let values = s.load("d").unwrap();
        assert_eq!(values.get("b"), Some(&Value::Bool(false)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
