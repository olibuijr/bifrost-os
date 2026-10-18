//! Kerberos 5 client through the MIT tools of the `krb5` package: `kinit` gets a ticket-granting ticket and `klist`
//! lists the cache. The credential cache is the one [`crate::gss_krb5`] names (`KRB5CCNAME`), so an app that set its
//! cache with `gss_krb5::Krb5Settings::set_ccache_name` sees the same tickets here.
//!
//! The tools are run with `std::process::Command`, never through a shell, so a principal or a password cannot inject
//! arguments. A password goes to `kinit` on standard input. The parsers are pure and tested; the two runners need the
//! `krb5` package installed (`kinit`, `klist`).

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

use crate::gss_krb5::{CacheKind, CacheName};

/// Why a Kerberos call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Krb5ClientError {
    /// The principal or password is empty or holds a control character, space or newline.
    InvalidInput(&'static str),
    /// The tool could not be started (for example the `krb5` package is not installed).
    Spawn(String),
    /// The tool ran and failed; the text is its standard error.
    Failed(String),
    /// The tool's output did not have the shape `klist` prints.
    Malformed,
    /// A credential cache could not be read or written.
    Io(String),
}

impl fmt::Display for Krb5ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Krb5ClientError::InvalidInput(what) => write!(f, "invalid Kerberos {what}"),
            Krb5ClientError::Spawn(reason) => write!(f, "could not run the Kerberos tool: {reason}"),
            Krb5ClientError::Failed(text) => f.write_str(text),
            Krb5ClientError::Malformed => f.write_str("unexpected klist output"),
            Krb5ClientError::Io(reason) => write!(f, "credential cache: {reason}"),
        }
    }
}

impl std::error::Error for Krb5ClientError {}

/// One line of the ticket table: when it became valid, when it expires, and its service principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub valid_from: String,
    pub expires: String,
    pub service: String,
}

/// What `klist` reports: the cache, the default principal and the tickets in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Klist {
    /// The cache name as `klist` prints it (`FILE:/tmp/krb5cc_1000`), when it printed one.
    pub cache: Option<String>,
    pub principal: String,
    pub tickets: Vec<Ticket>,
}

impl Klist {
    /// True when the cache holds a ticket for the given service principal, such as `krbtgt/EXAMPLE.COM@EXAMPLE.COM`.
    pub fn has_service(&self, service: &str) -> bool {
        self.tickets.iter().any(|t| t.service == service)
    }
}

/// Checks a principal or password. Both are non-empty and hold no whitespace or control characters.
pub fn validate(text: &str, what: &'static str) -> Result<(), Krb5ClientError> {
    if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Krb5ClientError::InvalidInput(what));
    }
    Ok(())
}

/// The `KRB5CCNAME` value for a parsed cache name: its type prefix, a colon, and the residual.
pub fn ccache_env(name: &CacheName) -> String {
    let prefix = match name.kind() {
        CacheKind::File => "FILE",
        CacheKind::Directory => "DIR",
        CacheKind::Keyring => "KEYRING",
        CacheKind::Kcm => "KCM",
        CacheKind::Memory => "MEMORY",
    };
    format!("{prefix}:{}", name.residual())
}

/// The `klist` command for a cache (the default cache when None).
pub fn klist_command(cache: Option<&CacheName>) -> Command {
    let mut cmd = Command::new("klist");
    if let Some(name) = cache {
        cmd.env("KRB5CCNAME", ccache_env(name));
    }
    cmd
}

/// The `kinit` command for a principal and cache. The password is written to its standard input by [`kinit`].
pub fn kinit_command(principal: &str, cache: Option<&CacheName>) -> Result<Command, Krb5ClientError> {
    validate(principal, "principal")?;
    let mut cmd = Command::new("kinit");
    cmd.arg(principal);
    if let Some(name) = cache {
        cmd.env("KRB5CCNAME", ccache_env(name));
    }
    Ok(cmd)
}

/// Reads `klist` output. The default principal line is required; the ticket table is optional.
pub fn parse_klist(text: &str) -> Result<Klist, Krb5ClientError> {
    let mut cache = None;
    let mut principal = None;
    let mut tickets = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Ticket cache:") {
            cache = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Default principal:") {
            principal = Some(rest.trim().to_string());
        } else if line.starts_with("Valid starting") {
            in_table = true;
        } else if in_table && !line.trim().is_empty() && !line.starts_with(char::is_whitespace) {
            // a ticket row: three columns separated by runs of spaces
            let fields: Vec<&str> = line.split("  ").map(str::trim).filter(|f| !f.is_empty()).collect();
            if let [valid_from, expires, service] = fields[..] {
                tickets.push(Ticket { valid_from: valid_from.into(), expires: expires.into(), service: service.into() });
            }
        }
    }
    let principal = principal.filter(|p| !p.is_empty()).ok_or(Krb5ClientError::Malformed)?;
    Ok(Klist { cache, principal, tickets })
}

/// Runs `klist` on a cache and parses its output.
pub fn klist(cache: Option<&CacheName>) -> Result<Klist, Krb5ClientError> {
    let out = klist_command(cache).output().map_err(|e| Krb5ClientError::Spawn(e.to_string()))?;
    if !out.status.success() {
        return Err(Krb5ClientError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    parse_klist(&String::from_utf8_lossy(&out.stdout))
}

/// Runs `kinit` for a principal with its password on standard input, storing the ticket in the cache.
pub fn kinit(principal: &str, password: &str, cache: Option<&CacheName>) -> Result<(), Krb5ClientError> {
    validate(password, "password")?;
    let mut child = kinit_command(principal, cache)?
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Krb5ClientError::Spawn(e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // a failed write means kinit exited early; its exit status below says why
        let _ = writeln!(stdin, "{password}");
    }
    let out = child.wait_with_output().map_err(|e| Krb5ClientError::Spawn(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Krb5ClientError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// Copies one FILE credential cache to another, the file form of `gss_krb5_copy_ccache`. The copy is written beside
/// the target with mode 0600 and renamed into place, so a failed copy leaves the target as it was. Returns the number of
/// bytes copied. Only FILE caches can be copied; the other types live in the kernel or a service.
pub fn copy_ccache(src: &CacheName, dst: &CacheName) -> Result<u64, Krb5ClientError> {
    if src.kind() != CacheKind::File || dst.kind() != CacheKind::File {
        return Err(Krb5ClientError::InvalidInput("cache type: only FILE caches can be copied"));
    }
    let data = fs::read(src.residual()).map_err(|e| Krb5ClientError::Io(e.to_string()))?;
    let target = Path::new(dst.residual());
    let tmp = PathBuf::from(format!("{}.copy-{}", dst.residual(), process::id()));
    let _ = fs::remove_file(&tmp);
    let written = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(Krb5ClientError::Io(e.to_string()));
    }
    Ok(data.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gss_krb5::Krb5Settings;

    const SAMPLE: &str = "Ticket cache: FILE:/tmp/krb5cc_1000\n\
Default principal: alice@EXAMPLE.COM\n\
\n\
Valid starting       Expires              Service principal\n\
10/17/2026 09:00:00  10/17/2026 19:00:00  krbtgt/EXAMPLE.COM@EXAMPLE.COM\n\
\trenew until 10/24/2026 09:00:00\n\
10/17/2026 09:05:00  10/17/2026 19:00:00  host/printer.example.com@EXAMPLE.COM\n";

    fn cache(text: &str) -> CacheName {
        let mut s = Krb5Settings::new();
        // The setter returns the cache it replaces (None on a fresh settings), so read the new one back.
        s.set_ccache_name(text).unwrap();
        s.ccache().cloned().unwrap()
    }

    #[test]
    fn klist_output_yields_the_principal_and_tickets() {
        let k = parse_klist(SAMPLE).unwrap();
        assert_eq!(k.cache.as_deref(), Some("FILE:/tmp/krb5cc_1000"));
        assert_eq!(k.principal, "alice@EXAMPLE.COM");
        assert_eq!(k.tickets.len(), 2, "the renew line is not a ticket");
        assert_eq!(k.tickets[0].service, "krbtgt/EXAMPLE.COM@EXAMPLE.COM");
        assert_eq!(k.tickets[0].expires, "10/17/2026 19:00:00");
        assert!(k.has_service("host/printer.example.com@EXAMPLE.COM"));
        assert!(!k.has_service("ldap/dc.example.com@EXAMPLE.COM"));
    }

    #[test]
    fn klist_without_a_principal_is_malformed() {
        assert_eq!(parse_klist("klist: nothing here\n"), Err(Krb5ClientError::Malformed));
        assert_eq!(parse_klist("Default principal: \n"), Err(Krb5ClientError::Malformed));
    }

    #[test]
    fn klist_with_no_tickets_still_parses() {
        let k = parse_klist("Default principal: bob@EXAMPLE.COM\n").unwrap();
        assert_eq!(k.principal, "bob@EXAMPLE.COM");
        assert!(k.tickets.is_empty());
        assert_eq!(k.cache, None);
    }

    #[test]
    fn cache_names_become_krb5ccname_values() {
        assert_eq!(ccache_env(&cache("FILE:/tmp/cc")), "FILE:/tmp/cc");
        assert_eq!(ccache_env(&cache("DIR:/run/user/1000/krb")), "DIR:/run/user/1000/krb");
        assert_eq!(ccache_env(&cache("KCM:1000")), "KCM:1000");
        assert_eq!(ccache_env(&cache("/tmp/plain")), "FILE:/tmp/plain", "a name with no prefix is a file");
    }

    #[test]
    fn principals_and_passwords_are_checked() {
        assert_eq!(validate("alice@EXAMPLE.COM", "principal"), Ok(()));
        assert_eq!(validate("", "principal"), Err(Krb5ClientError::InvalidInput("principal")));
        assert_eq!(validate("a b", "principal"), Err(Krb5ClientError::InvalidInput("principal")));
        assert_eq!(validate("pw\nrm -rf", "password"), Err(Krb5ClientError::InvalidInput("password")));
        assert!(kinit_command("x;rm", None).is_ok(), "no shell, so ';' is only an odd principal");
        assert!(kinit_command("", None).is_err());
    }

    #[test]
    fn kinit_command_passes_the_principal_as_one_argument_and_sets_the_cache() {
        let name = cache("FILE:/tmp/cc-test");
        let cmd = kinit_command("alice@EXAMPLE.COM", Some(&name)).unwrap();
        assert_eq!(cmd.get_program(), "kinit");
        assert_eq!(cmd.get_args().count(), 1);
        assert_eq!(cmd.get_args().next().unwrap(), "alice@EXAMPLE.COM");
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        assert!(env.iter().any(|(k, v)| k == "KRB5CCNAME" && v.as_deref() == Some("FILE:/tmp/cc-test")));
    }

    #[test]
    fn klist_command_without_a_cache_leaves_the_environment_alone() {
        let cmd = klist_command(None);
        assert_eq!(cmd.get_program(), "klist");
        assert_eq!(cmd.get_envs().count(), 0);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-krb5-{tag}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_ccache_copies_the_bytes_with_a_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("copy");
        let src = dir.join("src-cc");
        let dst = dir.join("dst-cc");
        fs::write(&src, b"ticket-bytes").unwrap();
        let n = copy_ccache(&cache(&format!("FILE:{}", src.display())), &cache(&format!("FILE:{}", dst.display()))).unwrap();
        assert_eq!(n, 12);
        assert_eq!(fs::read(&dst).unwrap(), b"ticket-bytes");
        assert_eq!(fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!dir.join("dst-cc.copy-".to_string() + &process::id().to_string()).exists(), "no temporary file is left");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_ccache_refuses_non_file_caches_and_reports_a_missing_source() {
        let dir = scratch("refuse");
        let dst = cache(&format!("FILE:{}", dir.join("dst").display()));
        assert_eq!(
            copy_ccache(&cache("KCM:1000"), &dst),
            Err(Krb5ClientError::InvalidInput("cache type: only FILE caches can be copied"))
        );
        let missing = cache(&format!("FILE:{}", dir.join("nope").display()));
        assert!(matches!(copy_ccache(&missing, &dst), Err(Krb5ClientError::Io(_))));
        assert!(!dir.join("dst").exists(), "a failed copy writes nothing");
        let _ = fs::remove_dir_all(&dir);
    }
}
