//! The installed local account for apps: the user name, numeric ids, home folder and shell of the person who runs the
//! desktop. `NSUserName`, `NSHomeDirectory` and `getpwuid` parity for an app that needs to know whose account it runs as.
//!
//! The shell records the account name in `/var/lib/springboard/account` and the account itself lives in `/etc/passwd`.
//! [`LocalAccount::parse`] reads one passwd line for that name and applies the same policy as the shell: the name must be
//! plain, the entry must have seven fields, and the account must be a non-root user whose home is inside `/home`.
//! [`LocalAccount::installed`] reads the two files. Nothing here writes to the account.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// Where the shell records the installed account name.
pub const ACCOUNT_RECORD: &str = "/var/lib/springboard/account";
/// The passwd database the account entry is read from.
pub const PASSWD_PATH: &str = "/etc/passwd";

/// Why the installed account cannot be used by an app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalAccountError {
    /// The account record is missing its `username=` line.
    NoRecord,
    /// The name is empty or holds characters outside letters, digits, `_` and `-`.
    InvalidName,
    /// No passwd entry carries the account name.
    UnknownAccount,
    /// The passwd entry does not have the seven fields of the format.
    MalformedEntry,
    /// A numeric field is not a number.
    MalformedId,
    /// The account is root, or its home is not a folder inside `/home`.
    NotAnAppAccount,
    /// The account record or passwd database could not be read.
    Unreadable(io::ErrorKind),
}

impl fmt::Display for LocalAccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            LocalAccountError::NoRecord => "the account record has no username",
            LocalAccountError::InvalidName => "the account name is not valid",
            LocalAccountError::UnknownAccount => "no passwd entry for the account",
            LocalAccountError::MalformedEntry => "the passwd entry is malformed",
            LocalAccountError::MalformedId => "the passwd entry has a bad numeric id",
            LocalAccountError::NotAnAppAccount => "apps need a non-root account with a home under /home",
            LocalAccountError::Unreadable(_) => "the account files cannot be read",
        };
        f.write_str(text)
    }
}

/// The local account (`NSUserName`, `NSHomeDirectory`, the passwd fields an app reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAccount {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// The GECOS field: the full name, when the account has one.
    pub full_name: String,
    pub home: PathBuf,
    pub shell: String,
}

impl LocalAccount {
    /// Reads the installed account from the account record and the passwd database.
    pub fn installed() -> Result<LocalAccount, LocalAccountError> {
        let record = fs::read_to_string(ACCOUNT_RECORD).map_err(|e| LocalAccountError::Unreadable(e.kind()))?;
        let name = record
            .lines()
            .find_map(|l| l.strip_prefix("username="))
            .ok_or(LocalAccountError::NoRecord)?;
        let passwd = fs::read_to_string(PASSWD_PATH).map_err(|e| LocalAccountError::Unreadable(e.kind()))?;
        LocalAccount::parse(name, &passwd)
    }

    /// Finds `name` in passwd text and checks it the way the shell does.
    pub fn parse(name: &str, passwd: &str) -> Result<LocalAccount, LocalAccountError> {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(LocalAccountError::InvalidName);
        }
        let line = passwd
            .lines()
            .find(|l| l.split(':').next() == Some(name))
            .ok_or(LocalAccountError::UnknownAccount)?;
        let f: Vec<&str> = line.split(':').collect();
        if f.len() != 7 {
            return Err(LocalAccountError::MalformedEntry);
        }
        let uid = f[2].parse::<u32>().map_err(|_| LocalAccountError::MalformedId)?;
        let gid = f[3].parse::<u32>().map_err(|_| LocalAccountError::MalformedId)?;
        let home = PathBuf::from(f[5]);
        if uid == 0 || gid == 0 || !home.starts_with("/home") || home == Path::new("/home") {
            return Err(LocalAccountError::NotAnAppAccount);
        }
        Ok(LocalAccount {
            name: name.to_string(),
            uid,
            gid,
            full_name: f[4].split(',').next().unwrap_or("").to_string(),
            home,
            shell: f[6].to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
alice:x:1000:1000:Alice Example,,,:/home/alice:/bin/bash\n\
bob:x:1001:1001::/home/bob:/bin/sh\n\
svc:x:999:999:service:/var/lib/svc:/usr/bin/nologin\n\
broken:x:1002:1002:only four\n";

    #[test]
    fn a_normal_account_is_read_with_its_fields() {
        let a = LocalAccount::parse("alice", PASSWD).unwrap();
        assert_eq!(a.name, "alice");
        assert_eq!(a.uid, 1000);
        assert_eq!(a.gid, 1000);
        assert_eq!(a.full_name, "Alice Example");
        assert_eq!(a.home, PathBuf::from("/home/alice"));
        assert_eq!(a.shell, "/bin/bash");
    }

    #[test]
    fn an_empty_gecos_gives_an_empty_full_name() {
        let a = LocalAccount::parse("bob", PASSWD).unwrap();
        assert_eq!(a.full_name, "");
        assert_eq!(a.shell, "/bin/sh");
    }

    #[test]
    fn root_and_system_accounts_are_refused() {
        assert_eq!(LocalAccount::parse("root", PASSWD), Err(LocalAccountError::NotAnAppAccount));
        assert_eq!(LocalAccount::parse("svc", PASSWD), Err(LocalAccountError::NotAnAppAccount));
    }

    #[test]
    fn a_bad_name_or_missing_entry_is_refused() {
        assert_eq!(LocalAccount::parse("", PASSWD), Err(LocalAccountError::InvalidName));
        assert_eq!(LocalAccount::parse("al ice", PASSWD), Err(LocalAccountError::InvalidName));
        assert_eq!(LocalAccount::parse("nobody", PASSWD), Err(LocalAccountError::UnknownAccount));
    }

    #[test]
    fn a_short_entry_is_refused() {
        assert_eq!(LocalAccount::parse("broken", PASSWD), Err(LocalAccountError::MalformedEntry));
    }

    #[test]
    fn a_non_numeric_id_is_refused() {
        let passwd = "carol:x:abc:1000:Carol:/home/carol:/bin/sh\n";
        assert_eq!(LocalAccount::parse("carol", passwd), Err(LocalAccountError::MalformedId));
    }
}
