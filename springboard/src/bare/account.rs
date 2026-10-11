//! The installed local account (`/var/lib/springboard/account` plus its passwd entry). Not feature-gated: the
//! app launch policy and privd's login both use it.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}
impl Account {
    pub fn installed() -> io::Result<Self> {
        let account = fs::read_to_string("/var/lib/springboard/account")?;
        let name = account
            .lines()
            .find_map(|l| l.strip_prefix("username="))
            .ok_or_else(|| io::Error::other("Missing local account"))?;
        Self::parse(name, &fs::read_to_string("/etc/passwd")?)
    }
    pub(crate) fn parse(name: &str, passwd: &str) -> io::Result<Self> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(io::Error::other("Invalid local account"));
        }
        let f: Vec<_> = passwd
            .lines()
            .find(|l| l.split(':').next() == Some(name))
            .ok_or_else(|| io::Error::other("Unknown local account"))?
            .split(':')
            .collect();
        if f.len() != 7 {
            return Err(io::Error::other("Invalid account entry"));
        }
        let uid = f[2].parse::<u32>().map_err(io::Error::other)?;
        let gid = f[3].parse::<u32>().map_err(io::Error::other)?;
        let home = PathBuf::from(f[5]);
        if uid == 0 || gid == 0 || !home.starts_with("/home") || home == Path::new("/home") {
            return Err(io::Error::other("Apps require a non-root local account"));
        }
        Ok(Self {
            name: name.into(),
            uid,
            gid,
            home,
        })
    }
}
