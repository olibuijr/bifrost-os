//! The accounts file in the app's data directory. It holds the servers and addresses only; passwords live in the keychain
//! (see [`crate::account`]). The file is replaced whole, through a temporary file, so a crash never leaves half of it.

use crate::account::{self, Account};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FILE: &str = "accounts.ini";

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The saved accounts. A missing file is no accounts; a damaged one is an error the window shows.
pub fn load(dir: &Path) -> Result<Vec<Account>, String> {
    match fs::read_to_string(path(dir)) {
        Ok(text) => account::decode(&text).map_err(|e| format!("{FILE}: {e}")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{FILE}: {e}")),
    }
}

/// Writes the accounts file.
pub fn save(dir: &Path, accounts: &[Account]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let target = path(dir);
    let temp = dir.join(format!("{FILE}.tmp"));
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(account::encode(accounts).as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&temp, &target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Security, Server};

    #[test]
    fn accounts_survive_a_save_and_load() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-store-{}", std::process::id()));
        let account = Account {
            id: "ana".into(),
            name: "Ana".into(),
            email: "ana@example.com".into(),
            username: "ana".into(),
            imap: Server { host: "imap.example.com".into(), port: 993, security: Security::Tls },
            smtp: Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls },
        };
        assert_eq!(load(&dir).unwrap(), Vec::new(), "no file yet");
        save(&dir, std::slice::from_ref(&account)).unwrap();
        assert_eq!(load(&dir).unwrap(), vec![account]);
        let _ = fs::remove_dir_all(&dir);
    }
}
