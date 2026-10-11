//! Who a terminal shell runs as: the logged-in desktop user, never root unless the user is root.

#[cfg(feature = "desktop-wayland")]
use crate::bare::app_policy::Account;
use std::io;
use std::path::PathBuf;

/// The identity a pty child switches to before `execve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
    pub home: PathBuf,
    pub shell: String,
}

extern "C" {
    fn geteuid() -> u32;
}

/// The shell and supplementary groups of the account from passwd/group text.
#[cfg(feature = "desktop-wayland")]
fn lookup(account: &Account, passwd: &str, group: &str) -> Login {
    let shell = passwd
        .lines()
        .find(|l| l.split(':').next() == Some(account.name.as_str()))
        .and_then(|l| l.split(':').nth(6))
        .filter(|s| s.starts_with('/') && !s.ends_with("/nologin") && !s.ends_with("/false"))
        .unwrap_or("/bin/bash")
        .to_string();
    let mut groups = vec![account.gid];
    for line in group.lines() {
        let f: Vec<_> = line.split(':').collect();
        if f.len() == 4 && f[3].split(',').any(|u| u == account.name) {
            if let Some(g) = f[2].parse::<u32>().ok().filter(|g| !groups.contains(g)) {
                groups.push(g);
            }
        }
    }
    Login { name: account.name.clone(), uid: account.uid, gid: account.gid, groups, home: account.home.clone(), shell }
}

/// The privilege decision: `None` keeps the current identity (we are not root, so the
/// shell cannot be root either); a root process must have a non-root account to drop to.
pub fn decide(euid: u32, account: io::Result<Login>) -> io::Result<Option<Login>> {
    if euid != 0 {
        return Ok(None);
    }
    match account {
        Ok(l) if l.uid != 0 => Ok(Some(l)),
        Ok(_) => Err(io::Error::other("the session account is root")),
        Err(e) => Err(io::Error::other(format!("no desktop account to run the shell as: {e}"))),
    }
}

/// The login to use for a new terminal on this machine.
/// Without the desktop there is no installed account to switch to: keep the process identity.
#[cfg(not(feature = "desktop-wayland"))]
pub fn current() -> io::Result<Option<Login>> {
    Ok(None)
}

#[cfg(feature = "desktop-wayland")]
pub fn current() -> io::Result<Option<Login>> {
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { geteuid() };
    if euid != 0 {
        return Ok(None);
    }
    let account = Account::installed().and_then(|a| {
        let passwd = std::fs::read_to_string("/etc/passwd")?;
        let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
        Ok(lookup(&a, &passwd, &group))
    });
    decide(euid, account)
}

/// The pty environment. With a `login` it is built from that account; otherwise from
/// the process's own `HOME`, `USER`, `SHELL` and `PATH`.
pub fn env(login: Option<&Login>, var: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let path = var("PATH").filter(|p| !p.is_empty()).unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    let (home, user, shell) = match login {
        Some(l) => (l.home.to_string_lossy().into_owned(), l.name.clone(), l.shell.clone()),
        None => (
            var("HOME").filter(|h| h.starts_with('/')).unwrap_or_else(|| "/".into()),
            var("USER").or_else(|| var("LOGNAME")).unwrap_or_default(),
            var("SHELL").filter(|s| s.starts_with('/')).unwrap_or_else(|| "/bin/bash".into()),
        ),
    };
    let mut v = vec![
        "TERM=xterm-256color".to_string(),
        "COLORTERM=truecolor".into(),
        "LANG=C.UTF-8".into(),
        format!("HOME={home}"),
        format!("PATH={path}"),
        format!("SHELL={shell}"),
    ];
    if !user.is_empty() {
        v.push(format!("USER={user}"));
        v.push(format!("LOGNAME={user}"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same login without the desktop's account reader.
    #[cfg(not(feature = "desktop-wayland"))]
    fn acct() -> Login {
        Login { name: "ola".into(), uid: 1000, gid: 1000, groups: vec![1000, 10], home: "/home/ola".into(), shell: "/bin/zsh".into() }
    }

    #[cfg(feature = "desktop-wayland")]
    fn acct() -> Login {
        let a = Account { name: "ola".into(), uid: 1000, gid: 1000, home: "/home/ola".into() };
        lookup(&a, "root:x:0:0::/root:/bin/bash\nola:x:1000:1000::/home/ola:/bin/zsh\n", "wheel:x:10:ola,bob\naudio:x:92:bob\nola:x:1000:\n")
    }

    #[test]
    #[cfg(feature = "desktop-wayland")]
    fn lookup_reads_shell_and_groups() {
        let l = acct();
        assert_eq!(l.shell, "/bin/zsh");
        assert_eq!(l.groups, vec![1000, 10]);
    }

    #[test]
    #[cfg(feature = "desktop-wayland")]
    fn nologin_shell_falls_back_to_bash() {
        let a = Account { name: "ola".into(), uid: 1000, gid: 1000, home: "/home/ola".into() };
        assert_eq!(lookup(&a, "ola:x:1000:1000::/home/ola:/usr/bin/nologin\n", "").shell, "/bin/bash");
    }

    #[test]
    fn root_drops_to_the_account_and_never_stays_root() {
        assert_eq!(decide(0, Ok(acct())).unwrap(), Some(acct()));
        assert!(decide(0, Err(io::Error::other("x"))).is_err());
        let mut r = acct();
        r.uid = 0;
        assert!(decide(0, Ok(r)).is_err());
    }

    #[test]
    fn a_normal_user_keeps_their_identity() {
        assert_eq!(decide(1000, Err(io::Error::other("x"))).unwrap(), None);
        assert_eq!(decide(1000, Ok(acct())).unwrap(), None);
    }

    #[test]
    fn env_for_a_login_uses_the_account() {
        let e = env(Some(&acct()), &|k| match k {
            "PATH" => Some("/usr/bin".to_string()),
            "HOME" => Some("/root".to_string()),
            _ => None,
        });
        for want in ["HOME=/home/ola", "USER=ola", "LOGNAME=ola", "SHELL=/bin/zsh", "PATH=/usr/bin", "TERM=xterm-256color", "COLORTERM=truecolor"] {
            assert!(e.iter().any(|x| x == want), "{want} in {e:?}");
        }
        assert!(!e.iter().any(|x| x.contains("/root")));
    }

    #[test]
    fn env_without_a_login_falls_back_safely() {
        let e = env(None, &|k| (k == "HOME").then(|| "relative".to_string()));
        assert!(e.contains(&"HOME=/".to_string()));
        assert!(e.contains(&"PATH=/usr/local/bin:/usr/bin:/bin".to_string()));
        assert!(!e.iter().any(|x| x.starts_with("USER=")));
    }
}
