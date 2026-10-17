//! Document grants, phase 1 of `docs/file-portal-design.md` (the broker behind `NativeOpenPanel`/`NativeSavePanel`).
//!
//! The shell is the only component that resolves a user path. It opens the file the user chose *as the account*
//! (fsuid/fsgid on the calling thread), beneath the account's home, refusing symlinks and `..` escapes, and hands
//! the app a descriptor. The app never receives a path. A grant is per app and per file; reopening it checks that
//! the file is still the one that was granted (device and inode).
//!
//! Known limit: the shell keeps root's supplementary groups while it acts as the account, so a file readable only
//! by group 0 would open. The account owns its home, so this matters only for files root placed there.

use bifrost_core::{fsutil::write_atomic, json};
use bifrost_proto::{DocErrorCode, DocMode, GrantId};
use std::ffi::{c_int, c_long, c_uint, CString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

const SYS_OPENAT2: c_long = 437;
const O_RDWR: u64 = 0o2;
const O_CREAT: u64 = 0o100;
const O_EXCL: u64 = 0o200;
const O_NONBLOCK: u64 = 0o4000;
const O_DIRECTORY: u64 = 0o200000;
const O_NOFOLLOW: u64 = 0o400000;
const O_CLOEXEC: u64 = 0o2000000;
const O_PATH: u64 = 0o10000000;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;
const RESOLVE: u64 = RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

extern "C" {
    fn syscall(n: c_long, ...) -> c_long;
    fn setfsuid(uid: c_uint) -> c_int;
    fn setfsgid(gid: c_uint) -> c_int;
    fn renameat(olddir: c_int, old: *const std::ffi::c_char, newdir: c_int, new: *const std::ffi::c_char) -> c_int;
    fn mkdirat(dir: c_int, path: *const std::ffi::c_char, mode: c_uint) -> c_int;
    fn unlinkat(dir: c_int, path: *const std::ffi::c_char, flags: c_int) -> c_int;
}

fn code_of(e: &io::Error) -> DocErrorCode {
    match e.kind() {
        io::ErrorKind::NotFound => DocErrorCode::NotFound,
        // EACCES/EPERM, and ELOOP/EXDEV (a symlink or an escape refused by RESOLVE_*), all read as Denied
        _ => DocErrorCode::Denied,
    }
}

/// A file name the user typed into a save panel: one path component, not hidden by accident, no NUL.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// A path relative to the home: plain components only (no root, `.` or `..`).
fn relative(rel: &Path) -> Result<CString, DocErrorCode> {
    if rel.as_os_str().is_empty() {
        return CString::new(".").map_err(|_| DocErrorCode::Denied);
    }
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(DocErrorCode::Denied);
    }
    CString::new(rel.as_os_str().as_bytes()).map_err(|_| DocErrorCode::Denied)
}

/// Opens files as the account, beneath its home.
pub struct Opener {
    root: OwnedFd,
    home: PathBuf,
    uid: u32,
    gid: u32,
}

impl Opener {
    pub fn new(home: &Path, uid: u32, gid: u32) -> io::Result<Self> {
        let c = CString::new(home.as_os_str().as_bytes()).map_err(io::Error::other)?;
        // SAFETY: c is NUL-terminated; O_PATH opens no file contents.
        let fd = unsafe { super::sys::open(c.as_ptr(), (O_PATH | O_DIRECTORY | O_CLOEXEC) as c_int) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just returned by open and is owned here.
        Ok(Opener { root: unsafe { OwnedFd::from_raw_fd(fd) }, home: home.to_path_buf(), uid, gid })
    }

    /// The account's home: what the panel lists (paths given to `open` are relative to it).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Runs `f` with this thread's filesystem identity set to the account, then restores it.
    fn as_account<T>(&self, f: impl FnOnce() -> T) -> T {
        // SAFETY: setfsuid/setfsgid change only this thread's filesystem credentials; both are restored below.
        let (old_gid, old_uid) = unsafe { (setfsgid(self.gid), setfsuid(self.uid)) };
        let out = f();
        // SAFETY: as above.
        unsafe {
            setfsuid(old_uid as c_uint);
            setfsgid(old_gid as c_uint);
        }
        out
    }

    fn openat2(&self, dir: c_int, path: &CString, flags: u64, mode: u64) -> io::Result<OwnedFd> {
        let how = OpenHow { flags: flags | O_CLOEXEC | O_NOFOLLOW, mode, resolve: RESOLVE };
        // SAFETY: path is NUL-terminated and how is a live open_how of the size passed.
        let fd = self.as_account(|| unsafe { syscall(SYS_OPENAT2, dir, path.as_ptr(), &how as *const OpenHow, std::mem::size_of::<OpenHow>()) });
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just returned by openat2 and is owned here.
        Ok(unsafe { OwnedFd::from_raw_fd(fd as c_int) })
    }

    /// Opens a regular file the user chose. `O_NONBLOCK` keeps a FIFO from blocking the shell; it is refused after.
    pub fn open(&self, rel: &Path, mode: DocMode) -> Result<File, DocErrorCode> {
        let access = if mode == DocMode::Write { O_RDWR } else { 0 };
        let fd = self.openat2(self.root.as_raw_fd(), &relative(rel)?, access | O_NONBLOCK, 0).map_err(|e| code_of(&e))?;
        let file = File::from(fd);
        if !file.metadata().map_err(|e| code_of(&e))?.is_file() {
            return Err(DocErrorCode::Denied);
        }
        Ok(file)
    }

    /// Opens a folder the user chose (`canChooseDirectories`), read-only; the descriptor is the folder itself.
    pub fn open_dir(&self, rel: &Path) -> Result<File, DocErrorCode> {
        let fd = self.openat2(self.root.as_raw_fd(), &relative(rel)?, O_DIRECTORY, 0).map_err(|e| code_of(&e))?;
        Ok(File::from(fd))
    }

    /// A file or a folder, whichever `rel` is (a choice from a panel that allows both).
    pub fn open_any(&self, rel: &Path, mode: DocMode) -> Result<File, DocErrorCode> {
        match self.open(rel, mode) {
            Err(DocErrorCode::Denied) => self.open_dir(rel),
            other => other,
        }
    }

    /// `NativeSavePanel.canCreateDirectories`: makes the folder `name` inside `parent` (one plain component, as the
    /// account, mode 0755). Fails with `Denied` when it exists or the name is not valid.
    pub fn make_dir(&self, parent: &Path, name: &str) -> Result<(), DocErrorCode> {
        if !valid_name(name) {
            return Err(DocErrorCode::Denied);
        }
        let dir = self.openat2(self.root.as_raw_fd(), &relative(parent)?, O_DIRECTORY, 0).map_err(|e| code_of(&e))?;
        let c = CString::new(name.as_bytes()).map_err(|_| DocErrorCode::Denied)?;
        // SAFETY: c is a NUL-terminated single component, dir is a live folder descriptor.
        if self.as_account(|| unsafe { mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o755) }) < 0 {
            return Err(code_of(&io::Error::last_os_error()));
        }
        Ok(())
    }

    /// The folder `rel` as the account sees it: (name, is a folder) for plain files and folders. Symlinks and other
    /// kinds are left out (`open` refuses them anyway). The folder is opened with the same rules as a file, then read
    /// through its descriptor, so the listing is the account's, not root's.
    pub fn list(&self, rel: &Path) -> Result<Vec<(String, bool)>, DocErrorCode> {
        let dir = self.openat2(self.root.as_raw_fd(), &relative(rel)?, O_DIRECTORY, 0).map_err(|e| code_of(&e))?;
        let at = format!("/proc/self/fd/{}", dir.as_raw_fd());
        let entries = self.as_account(|| fs::read_dir(&at)).map_err(|e| code_of(&e))?;
        let mut out = Vec::new();
        for e in entries.flatten() {
            let (Ok(kind), Ok(name)) = (e.file_type(), e.file_name().into_string()) else { continue };
            if kind.is_dir() || kind.is_file() {
                out.push((name, kind.is_dir()));
            }
        }
        Ok(out)
    }

    /// Creates a temporary file next to `name` in folder `dir` for a save. Nothing is replaced until `commit`.
    /// A package name (`Notes.rtfd`) gets a temporary folder instead: the app writes the package's files into it, and
    /// `commit` puts the folder in place of the package.
    pub fn begin_save(&self, dir: &Path, name: &str) -> Result<(PendingSave, File), DocErrorCode> {
        if !valid_name(name) {
            return Err(DocErrorCode::Denied);
        }
        let package = bifrost_ui::uttype::is_package_name(name);
        let dir_fd = self.openat2(self.root.as_raw_fd(), &relative(dir)?, O_DIRECTORY, 0).map_err(|e| code_of(&e))?;
        let tmp = format!(".{}.bifrost-{}.tmp", name.chars().take(64).collect::<String>(), hex(&random::<4>()));
        let c = CString::new(tmp.as_bytes()).map_err(|_| DocErrorCode::Denied)?;
        let fd = if package {
            // SAFETY: c is a NUL-terminated single component, dir_fd is a live folder descriptor.
            let made = self.as_account(|| unsafe { mkdirat(dir_fd.as_raw_fd(), c.as_ptr(), 0o755) });
            if made < 0 {
                return Err(code_of(&io::Error::last_os_error()));
            }
            self.openat2(dir_fd.as_raw_fd(), &c, O_DIRECTORY, 0).map_err(|e| code_of(&e))?
        } else {
            self.openat2(dir_fd.as_raw_fd(), &c, O_RDWR | O_CREAT | O_EXCL, 0o644).map_err(|e| code_of(&e))?
        };
        let file = File::from(fd);
        let keep = file.try_clone().map_err(|e| code_of(&e))?;
        Ok((PendingSave { dir: dir_fd, dir_rel: dir.to_path_buf(), tmp, name: name.into(), file: keep, package }, file))
    }
}

/// A save in progress: the temporary file the app writes, and where it goes.
pub struct PendingSave {
    dir: OwnedFd,
    dir_rel: PathBuf,
    tmp: String,
    name: String,
    file: File,
    /// the temporary is a folder for a package (see `begin_save`)
    package: bool,
}

impl PendingSave {
    /// Puts the file in place: fsync, rename over the destination, fsync the folder. Returns its path in the home.
    pub fn commit(self, opener: &Opener) -> Result<(PathBuf, File), DocErrorCode> {
        if self.package {
            return self.commit_package(opener);
        }
        self.file.sync_all().map_err(|e| code_of(&e))?;
        let (tmp, name) = (CString::new(self.tmp.as_bytes()).unwrap(), CString::new(self.name.as_bytes()).unwrap());
        let d = self.dir.as_raw_fd();
        // SAFETY: both names are NUL-terminated single components inside the folder `d`.
        if opener.as_account(|| unsafe { renameat(d, tmp.as_ptr(), d, name.as_ptr()) }) < 0 {
            let e = io::Error::last_os_error();
            self.discard(opener);
            return Err(code_of(&e));
        }
        // fsync of the folder makes the rename durable; a failure here does not undo the save
        if let Ok(dir) = File::from(self.dir).try_clone() {
            let _ = dir.sync_all();
        }
        Ok((self.dir_rel.join(&self.name), self.file))
    }

    /// Puts the temporary folder in place of the package. A package already there is moved aside, the new folder is
    /// renamed into its place, and the old one is removed; when the new folder cannot be put in place the old one is
    /// moved back.
    fn commit_package(self, opener: &Opener) -> Result<(PathBuf, File), DocErrorCode> {
        let _ = self.file.sync_all();
        let d = self.dir.as_raw_fd();
        let rename = |from: &str, to: &str| -> io::Result<()> {
            let (from, to) = (CString::new(from).unwrap(), CString::new(to).unwrap());
            // SAFETY: both names are NUL-terminated single components inside the folder d.
            opener.as_account(|| {
                // SAFETY: as above; errno is read before the account's credentials are restored.
                let r = unsafe { renameat(d, from.as_ptr(), d, to.as_ptr()) };
                if r < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            })
        };
        if rename(&self.tmp, &self.name).is_err() {
            // the package is there already (a folder cannot replace a non-empty one): move it aside first
            let aside = format!(".{}.bifrost-old-{}", self.name.chars().take(64).collect::<String>(), hex(&random::<4>()));
            if let Err(e) = rename(&self.name, &aside) {
                self.discard(opener);
                return Err(code_of(&e));
            }
            if let Err(e) = rename(&self.tmp, &self.name) {
                let _ = rename(&aside, &self.name);
                self.discard(opener);
                return Err(code_of(&e));
            }
            let old = opener.home().join(&self.dir_rel).join(&aside);
            let _ = opener.as_account(|| fs::remove_dir_all(&old).or_else(|_| fs::remove_file(&old)));
        }
        // fsync of the folder makes the rename durable; a failure here does not undo the save
        if let Ok(dir) = File::from(self.dir).try_clone() {
            let _ = dir.sync_all();
        }
        Ok((self.dir_rel.join(&self.name), self.file))
    }

    /// Throws the temporary file (or folder) away.
    pub fn abort(self, opener: &Opener) {
        self.discard(opener);
    }

    fn discard(&self, opener: &Opener) {
        if self.package {
            let tmp = opener.home().join(&self.dir_rel).join(&self.tmp);
            let _ = opener.as_account(|| fs::remove_dir_all(&tmp));
            return;
        }
        let tmp = CString::new(self.tmp.as_bytes()).unwrap();
        // SAFETY: tmp is a NUL-terminated single component inside the folder fd.
        opener.as_account(|| unsafe { unlinkat(self.dir.as_raw_fd(), tmp.as_ptr(), 0) });
    }
}

/// A fresh random grant id.
pub fn new_grant_id() -> GrantId {
    random::<16>()
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("/dev/urandom");
    b
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<GrantId> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub id: GrantId,
    pub app_id: String,
    pub mode: DocMode,
    /// relative to the account's home
    pub path: PathBuf,
    pub dev: u64,
    pub ino: u64,
    pub created: u64,
}

/// The grant records, kept in a root-owned file (mode 0600) and written atomically on every change.
pub struct Grants {
    file: PathBuf,
    items: Vec<Grant>,
}

impl Grants {
    /// Reads the records; a missing or damaged file starts empty (grants are a convenience, not data).
    pub fn load(file: &Path) -> Self {
        let mut items = Vec::new();
        if let Some(list) = fs::read_to_string(file).ok().and_then(|t| json::parse(&t).ok()) {
            for g in list.as_array().unwrap_or(&[]) {
                let mode = match g.get("mode").and_then(|m| m.as_str()) {
                    Some("write") => DocMode::Write,
                    Some("read") => DocMode::Read,
                    _ => continue,
                };
                let (Some(id), Some(app_id), Some(path), Some(dev), Some(ino)) = (
                    g.get("id").and_then(|v| v.as_str()).and_then(unhex),
                    g.str_field("app"),
                    g.str_field("path"),
                    g.get("dev").and_then(|v| v.as_u64()),
                    g.get("ino").and_then(|v| v.as_u64()),
                ) else {
                    continue;
                };
                let created = g.get("created").and_then(|v| v.as_u64()).unwrap_or(0);
                items.push(Grant { id, app_id, mode, path: path.into(), dev, ino, created });
            }
        }
        Grants { file: file.into(), items }
    }

    fn save(&self) -> io::Result<()> {
        let rows: Vec<String> = self
            .items
            .iter()
            .map(|g| {
                format!(
                    "{{\"id\":{},\"app\":{},\"mode\":{},\"path\":{},\"dev\":{},\"ino\":{},\"created\":{}}}",
                    json::quote(&hex(&g.id)),
                    json::quote(&g.app_id),
                    json::quote(if g.mode == DocMode::Write { "write" } else { "read" }),
                    json::quote(g.path.to_str().unwrap_or_default()),
                    g.dev,
                    g.ino,
                    g.created
                )
            })
            .collect();
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        write_atomic(&self.file, format!("[{}]\n", rows.join(",\n")))?;
        fs::set_permissions(&self.file, fs::Permissions::from_mode(0o600))
    }

    /// Records a grant for the file the user chose, opened as `file`. Paths that are not UTF-8 are not recorded
    /// (the app still gets the descriptor, but cannot reopen it later).
    pub fn add(&mut self, app_id: &str, path: &Path, mode: DocMode, file: &File) -> io::Result<GrantId> {
        let id = new_grant_id();
        self.insert(id, app_id, path, mode, file)?;
        Ok(id)
    }

    /// `add` under an id chosen earlier: a save tells the app its grant with the temporary file, and records it on commit.
    pub fn insert(&mut self, id: GrantId, app_id: &str, path: &Path, mode: DocMode, file: &File) -> io::Result<()> {
        let m = file.metadata()?;
        if path.to_str().is_some() {
            let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            self.items.push(Grant { id, app_id: app_id.into(), mode, path: path.into(), dev: m.dev(), ino: m.ino(), created });
            self.save()?;
        }
        Ok(())
    }

    /// The grant `id` of `app_id`. Another app's id and an unknown id answer the same (`Denied`), so an app
    /// cannot probe for grants it does not hold.
    pub fn get(&self, app_id: &str, id: &GrantId) -> Result<&Grant, DocErrorCode> {
        self.items.iter().find(|g| &g.id == id && g.app_id == app_id).ok_or(DocErrorCode::Denied)
    }

    /// Opens a grant again and checks it is the same file (`Stale` when it was replaced).
    pub fn reopen(&self, opener: &Opener, app_id: &str, id: &GrantId) -> Result<(File, &Grant), DocErrorCode> {
        let g = self.get(app_id, id)?;
        let f = opener.open_any(&g.path, g.mode)?;
        let m = f.metadata().map_err(|e| code_of(&e))?;
        if (m.dev(), m.ino()) != (g.dev, g.ino) {
            return Err(DocErrorCode::Stale);
        }
        Ok((f, g))
    }

    /// Drops one grant of `app_id` (`DocRelease`); a grant of another app is left alone.
    pub fn release(&mut self, app_id: &str, id: &GrantId) -> io::Result<()> {
        let n = self.items.len();
        self.items.retain(|g| !(&g.id == id && g.app_id == app_id));
        if self.items.len() != n {
            self.save()?;
        }
        Ok(())
    }

    /// Drops every grant of an app (the app was removed, or the user revoked it in Settings).
    pub fn revoke_app(&mut self, app_id: &str) -> io::Result<()> {
        let n = self.items.len();
        self.items.retain(|g| g.app_id != app_id);
        if self.items.len() != n {
            self.save()?;
        }
        Ok(())
    }

    pub fn of_app<'a>(&'a self, app_id: &'a str) -> impl Iterator<Item = &'a Grant> {
        self.items.iter().filter(move |g| g.app_id == app_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};
    use std::os::unix::fs::symlink;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("docbroker-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/Documents")).unwrap();
        d
    }

    fn opener(d: &Path) -> Opener {
        let m = fs::metadata(d).unwrap();
        Opener::new(&d.join("home"), m.uid(), m.gid()).unwrap()
    }

    #[test]
    fn opens_beneath_the_home_only_and_refuses_symlinks_and_escapes() {
        let d = scratch("open");
        fs::write(d.join("home/Documents/a.txt"), "hello").unwrap();
        fs::write(d.join("secret"), "no").unwrap();
        symlink(d.join("secret"), d.join("home/link")).unwrap();
        symlink(d.join("home/Documents"), d.join("home/docs")).unwrap();
        let o = opener(&d);
        let mut s = String::new();
        o.open(Path::new("Documents/a.txt"), DocMode::Read).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(o.open(Path::new("link"), DocMode::Read).unwrap_err(), DocErrorCode::Denied, "a symlink to a file");
        assert_eq!(o.open(Path::new("docs/a.txt"), DocMode::Read).unwrap_err(), DocErrorCode::Denied, "a symlink in the path");
        assert_eq!(o.open(Path::new("../secret"), DocMode::Read).unwrap_err(), DocErrorCode::Denied);
        assert_eq!(o.open(&d.join("secret"), DocMode::Read).unwrap_err(), DocErrorCode::Denied, "absolute path");
        assert_eq!(o.open(Path::new("Documents"), DocMode::Read).unwrap_err(), DocErrorCode::Denied, "a folder");
        assert_eq!(o.open(Path::new("Documents/none.txt"), DocMode::Read).unwrap_err(), DocErrorCode::NotFound);

        let mut home = o.list(Path::new("")).unwrap();
        home.sort();
        assert_eq!(home, [("Documents".to_string(), true)], "symlinks are not listed");
        assert_eq!(o.list(Path::new("Documents")).unwrap(), [("a.txt".to_string(), false)]);
        assert_eq!(o.list(Path::new("docs")).unwrap_err(), DocErrorCode::Denied, "a symlinked folder");
        assert_eq!(o.list(Path::new("..")).unwrap_err(), DocErrorCode::Denied);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn grants_belong_to_one_app_survive_a_restart_and_notice_a_replaced_file() {
        let d = scratch("grant");
        let p = d.join("home/Documents/a.txt");
        fs::write(&p, "one").unwrap();
        let o = opener(&d);
        let store = d.join("state/grants.json");
        let mut g = Grants::load(&store);
        let f = o.open(Path::new("Documents/a.txt"), DocMode::Write).unwrap();
        let id = g.add("is.example.editor", Path::new("Documents/a.txt"), DocMode::Write, &f).unwrap();
        assert_eq!(fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o600);

        let g = Grants::load(&store);
        assert!(g.reopen(&o, "is.example.editor", &id).is_ok(), "reloaded from disk");
        assert_eq!(g.reopen(&o, "is.example.other", &id).unwrap_err(), DocErrorCode::Denied, "another app's grant");
        assert_eq!(g.reopen(&o, "is.example.editor", &[0; 16]).unwrap_err(), DocErrorCode::Denied, "unknown id");

        // replaced by a different file with the same name: the identity check catches it
        fs::remove_file(&p).unwrap();
        fs::write(d.join("other"), "two").unwrap();
        fs::rename(d.join("other"), &p).unwrap();
        assert_eq!(g.reopen(&o, "is.example.editor", &id).unwrap_err(), DocErrorCode::Stale);

        let mut g = g;
        g.release("is.example.other", &id).unwrap();
        assert_eq!(g.of_app("is.example.editor").count(), 1, "another app cannot release it");
        g.revoke_app("is.example.editor").unwrap();
        assert_eq!(Grants::load(&store).of_app("is.example.editor").count(), 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_save_replaces_the_file_only_on_commit_and_abort_leaves_nothing() {
        let d = scratch("save");
        let dest = d.join("home/Documents/n.md");
        fs::write(&dest, "old").unwrap();
        let o = opener(&d);
        let listing = || {
            let mut v: Vec<String> = fs::read_dir(d.join("home/Documents")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
            v.sort();
            v
        };

        let (pending, mut f) = o.begin_save(Path::new("Documents"), "n.md").unwrap();
        f.write_all(b"new").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old", "untouched before commit");
        assert_eq!(listing().len(), 2, "the temp file sits next to it");
        let (path, mut kept) = pending.commit(&o).unwrap();
        assert_eq!(path, Path::new("Documents/n.md"));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "new");
        assert_eq!(listing(), ["n.md"]);
        kept.rewind().unwrap();
        let mut s = String::new();
        kept.read_to_string(&mut s).unwrap();
        assert_eq!(s, "new", "the descriptor kept for the grant is the saved file");

        let (pending, mut f) = o.begin_save(Path::new("Documents"), "n.md").unwrap();
        f.write_all(b"discarded").unwrap();
        pending.abort(&o);
        assert_eq!((listing(), fs::read_to_string(&dest).unwrap()), (vec!["n.md".to_string()], "new".to_string()));

        for bad in ["", ".", "..", "a/b", "x\0y"] {
            assert_eq!(o.begin_save(Path::new("Documents"), bad).err(), Some(DocErrorCode::Denied), "{bad:?}");
        }
        assert_eq!(o.begin_save(Path::new(".."), "x").err(), Some(DocErrorCode::Denied));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_package_is_saved_as_a_folder_that_replaces_the_old_one_only_on_commit() {
        let d = scratch("pkgsave");
        let docs = d.join("home/Documents");
        fs::create_dir_all(docs.join("Notes.rtfd")).unwrap();
        fs::write(docs.join("Notes.rtfd/TXT.rtf"), "old").unwrap();
        let o = opener(&d);
        let temp_of = |name: &str| {
            let prefix = format!(".{name}.bifrost-");
            fs::read_dir(&docs).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with(&prefix))
        };

        // a save hands over a folder, and the app writes the package's files into it
        let (pending, f) = o.begin_save(Path::new("Documents"), "Notes.rtfd").unwrap();
        assert!(f.metadata().unwrap().is_dir());
        fs::write(temp_of("Notes.rtfd").unwrap().join("TXT.rtf"), "new").unwrap();
        assert_eq!(fs::read_to_string(docs.join("Notes.rtfd/TXT.rtf")).unwrap(), "old", "nothing is replaced before commit");
        let (path, _) = pending.commit(&o).unwrap();
        assert_eq!(path, Path::new("Documents/Notes.rtfd"));
        assert_eq!(fs::read_to_string(docs.join("Notes.rtfd/TXT.rtf")).unwrap(), "new");
        let names: Vec<String> = fs::read_dir(&docs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Notes.rtfd"], "the old package and the temporary folder are gone");

        // an abort removes the temporary folder and leaves the package alone
        let (pending, _f) = o.begin_save(Path::new("Documents"), "Notes.rtfd").unwrap();
        fs::write(temp_of("Notes.rtfd").unwrap().join("TXT.rtf"), "lost").unwrap();
        pending.abort(&o);
        assert!(temp_of("Notes.rtfd").is_none());
        assert_eq!(fs::read_to_string(docs.join("Notes.rtfd/TXT.rtf")).unwrap(), "new");

        // a new name needs no old package to move aside
        let (pending, _f) = o.begin_save(Path::new("Documents"), "Copy.rtfd").unwrap();
        fs::write(temp_of("Copy.rtfd").unwrap().join("TXT.rtf"), "copy").unwrap();
        assert_eq!(pending.commit(&o).unwrap().0, Path::new("Documents/Copy.rtfd"));
        assert_eq!(fs::read_to_string(docs.join("Copy.rtfd/TXT.rtf")).unwrap(), "copy");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn folders_open_read_only_and_new_folders_are_made_inside_the_home() {
        let d = scratch("dirs");
        let o = opener(&d);
        fs::write(d.join("home/Documents/a.txt"), "a").unwrap();
        assert!(o.open_dir(Path::new("Documents")).unwrap().metadata().unwrap().is_dir());
        assert!(o.open_any(Path::new("Documents"), DocMode::Read).unwrap().metadata().unwrap().is_dir());
        assert!(o.open_any(Path::new("Documents/a.txt"), DocMode::Read).unwrap().metadata().unwrap().is_file());
        assert!(o.open(Path::new("Documents"), DocMode::Read).is_err(), "open alone still refuses folders");
        o.make_dir(Path::new("Documents"), "Drafts").unwrap();
        assert!(d.join("home/Documents/Drafts").is_dir());
        assert_eq!(o.make_dir(Path::new("Documents"), "Drafts").err(), Some(DocErrorCode::Denied), "exists");
        for bad in ["", ".", "..", "a/b"] {
            assert_eq!(o.make_dir(Path::new("Documents"), bad).err(), Some(DocErrorCode::Denied), "{bad:?}");
        }
        assert_eq!(o.make_dir(Path::new(".."), "x").err(), Some(DocErrorCode::Denied));
        fs::remove_dir_all(&d).unwrap();
    }
}
