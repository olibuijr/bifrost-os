//! `smbutil`: looks at SMB servers. It maps the reference tool's subcommands onto the Samba client programs in the
//! runtime package list: `view //[user@]host` lists the shares (`smbclient -L`), `lookup NAME` resolves a NetBIOS name
//! (`nmblookup`) and `status HOST` shows the NetBIOS names a host answers to (`nmblookup -A`).
//! Refused with a message: `statshares`, `identity`, `multichannel`, `dfs` and the other subcommands that read the
//! reference client's mount state, which the Linux client keeps in the kernel's CIFS mounts instead. A password in the
//! URL (`//user:secret@host`) is refused so it never reaches a command line another user can read; `smbclient`
//! prompts for it.

pub const USAGE: &str = "usage: smbutil view //[user@]host | lookup name | status host";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Help,
    /// Run `program` with `args`.
    Run { program: &'static str, args: Vec<String> },
}

/// Splits `//[user@]host[/share]` into the user and the host. Refuses an embedded password.
pub fn split_url(url: &str) -> Result<(Option<String>, String), String> {
    let rest = url.strip_prefix("//").or_else(|| url.strip_prefix("smb://")).unwrap_or(url);
    let rest = rest.split('/').next().unwrap_or("");
    let (user, host) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, rest),
    };
    if user.is_some_and(|u| u.contains(':')) {
        return Err("a password in the URL is not accepted; smbclient asks for it".into());
    }
    if host.is_empty() || host.contains(char::is_whitespace) || host.starts_with('-') {
        return Err(format!("{url}: not a server address"));
    }
    Ok((user.filter(|u| !u.is_empty()).map(str::to_string), host.to_string()))
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Plan, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "-N" && *a != "-G").collect();
    match words.as_slice() {
        [] | ["-h" | "-help" | "--help" | "help"] => Ok(Plan::Help),
        ["view", url] => {
            let (user, host) = split_url(url)?;
            let mut out = vec!["-L".to_string(), host];
            match user {
                Some(u) => out.extend(["-U".to_string(), u]),
                None => out.push("-N".to_string()),
            }
            Ok(Plan::Run { program: "smbclient", args: out })
        }
        ["lookup", name] if !name.starts_with('-') => Ok(Plan::Run { program: "nmblookup", args: vec![name.to_string()] }),
        ["status", host] if !host.starts_with('-') => Ok(Plan::Run { program: "nmblookup", args: vec!["-A".to_string(), host.to_string()] }),
        ["view" | "lookup" | "status", ..] => Err("expected exactly one argument".into()),
        [other, ..] => Err(format!("{other} is not supported: it reads the reference client's mount state")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn view_lists_shares_anonymously_or_as_the_named_user() {
        assert_eq!(parse(&args(&["view", "//nas.local"])), Ok(Plan::Run { program: "smbclient", args: args(&["-L", "nas.local", "-N"]) }));
        assert_eq!(
            parse(&args(&["view", "//olafur@nas.local"])),
            Ok(Plan::Run { program: "smbclient", args: args(&["-L", "nas.local", "-U", "olafur"]) })
        );
        assert_eq!(parse(&args(&["view", "//nas.local/share"])), parse(&args(&["view", "//nas.local"])));
    }

    #[test]
    fn lookup_and_status_use_the_netbios_client() {
        assert_eq!(parse(&args(&["lookup", "NAS"])), Ok(Plan::Run { program: "nmblookup", args: args(&["NAS"]) }));
        assert_eq!(parse(&args(&["status", "192.168.1.9"])), Ok(Plan::Run { program: "nmblookup", args: args(&["-A", "192.168.1.9"]) }));
    }

    #[test]
    fn a_password_in_the_url_and_other_subcommands_are_refused() {
        assert!(parse(&args(&["view", "//me:secret@nas"])).unwrap_err().contains("password in the URL"));
        assert!(parse(&args(&["statshares", "-a"])).unwrap_err().contains("statshares is not supported"));
        assert!(parse(&args(&["view", "//-oops"])).is_err());
        assert!(parse(&args(&["view"])).is_err());
        assert_eq!(parse(&[]), Ok(Plan::Help));
    }
}
