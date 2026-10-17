//! Account, group, host-name and sharing-service methods (Settings > Users & Groups and Sharing). Every parameter is
//! validated before a fixed argv runs as root: names are plain lowercase identifiers, the installed account and the
//! system accounts (uid below 1000) are never touched, and the last administrator cannot be demoted or removed.
use super::{obj, s, PErr};
use bifrost_core::json::Json;
use std::io::Write;
use std::process::{Command, Stdio};

/// Lowest uid of a person's account.
pub const FIRST_UID: u32 = 1000;
/// Highest uid of a person's account (above it are service identities such as nobody).
pub const LAST_UID: u32 = 59_999;
/// The group whose members may use sudo: an administrator.
pub const ADMIN_GROUP: &str = "wheel";
/// Groups every new account gets; administrators also get `ADMIN_GROUP`.
const STANDARD_GROUPS: &str = "audio,video,storage";
const ADMIN_GROUPS: &str = "wheel,audio,video,input,storage";

/// One `/etc/passwd` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passwd {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// the full name (the first comma-separated GECOS field)
    pub full_name: String,
    pub home: String,
    pub shell: String,
}

/// One `/etc/group` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

pub fn parse_passwd(text: &str) -> Vec<Passwd> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() != 7 {
                return None;
            }
            Some(Passwd { name: f[0].to_string(), uid: f[2].parse().ok()?, gid: f[3].parse().ok()?, full_name: f[4].split(',').next().unwrap_or("").to_string(), home: f[5].to_string(), shell: f[6].to_string() })
        })
        .collect()
}

pub fn parse_group(text: &str) -> Vec<Group> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() != 4 {
                return None;
            }
            Some(Group { name: f[0].to_string(), gid: f[2].parse().ok()?, members: f[3].split(',').filter(|m| !m.is_empty()).map(str::to_string).collect() })
        })
        .collect()
}

/// A person's account: a uid in the human range and a shell that can log in.
pub fn is_human(p: &Passwd) -> bool {
    (FIRST_UID..=LAST_UID).contains(&p.uid) && !p.shell.ends_with("/nologin") && !p.shell.ends_with("/false")
}

/// Whether `user` is an administrator (a member of `ADMIN_GROUP`, by listing or by primary group).
pub fn is_admin(user: &Passwd, groups: &[Group]) -> bool {
    groups.iter().any(|g| g.name == ADMIN_GROUP && (g.members.iter().any(|m| *m == user.name) || g.gid == user.gid))
}

/// `[a-z_][a-z0-9_-]{0,31}`: what useradd and groupadd accept without a warning.
pub fn valid_user(name: &str) -> bool {
    let mut chars = name.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    first && name.len() <= 32 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A full name goes in the GECOS field: no separators, controls or empty text.
pub fn valid_full_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && t.chars().count() <= 64 && !t.chars().any(|c| c.is_control() || matches!(c, ':' | ',' | '=' | '\\' | '"'))
}

/// A host name label: letters, digits and hyphens, 1..=63 long, no hyphen at either end.
pub fn valid_hostname(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && !name.starts_with('-') && !name.ends_with('-') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A computer name shown to people: any text without controls, at most 64 characters.
pub fn valid_computer_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && t.chars().count() <= 64 && !t.chars().any(char::is_control)
}

/// What chpasswd reads on one line.
pub fn valid_password(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && !p.chars().any(char::is_control)
}

/// The local host name a computer name suggests: lowercase letters and digits joined by hyphens.
pub fn hostname_for(computer_name: &str) -> String {
    let mut out = String::new();
    for c in computer_name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') && (c.is_whitespace() || matches!(c, '-' | '_' | '.' | '\'')) {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    out.chars().take(63).collect::<String>().trim_end_matches('-').to_string()
}

/// The sharing switches Settings may flip, and the units behind each. Nothing else can be enabled through privd.
/// File Sharing is the SMB server and the NetBIOS name service beside it; Printer Sharing is the LPD listener that hands
/// each connection to the `cups-lpd` the cups package ships.
pub fn service_units(id: &str) -> Option<&'static [&'static str]> {
    match id {
        "remote_login" => Some(&["sshd.service"]),
        "file_sharing" => Some(&["smb.service", "nmb.service"]),
        "screen_sharing" => Some(&["bifrost-wayvnc.service"]),
        "printer_sharing" => Some(&["cups-lpd.socket"]),
        _ => None,
    }
}

/// The unit whose state a sharing switch shows: the first of its units.
pub fn service_unit(id: &str) -> Option<&'static str> {
    service_units(id).and_then(|u| u.first().copied())
}

/// Why `name` may not be deleted (None: it may).
pub fn delete_refusal(name: &str, users: &[Passwd], groups: &[Group], installed: Option<&str>) -> Option<&'static str> {
    let Some(u) = users.iter().find(|u| u.name == name) else { return Some("no such account") };
    if !is_human(u) {
        return Some("not a user account");
    }
    if installed == Some(name) {
        return Some("this is the account BifrOSt was installed for");
    }
    if is_admin(u, groups) && last_admin(name, users, groups) {
        return Some("the last administrator cannot be deleted");
    }
    None
}

/// Whether `name` is the only administrator among the human accounts.
fn last_admin(name: &str, users: &[Passwd], groups: &[Group]) -> bool {
    !users.iter().any(|u| u.name != name && is_human(u) && is_admin(u, groups))
}

/// Why `name` may not change administrator status to `admin`.
pub fn admin_refusal(name: &str, admin: bool, users: &[Passwd], groups: &[Group], installed: Option<&str>) -> Option<&'static str> {
    let Some(u) = users.iter().find(|u| u.name == name) else { return Some("no such account") };
    if !is_human(u) {
        return Some("not a user account");
    }
    if !admin && (installed == Some(name) || last_admin(name, users, groups)) {
        return Some("this account must stay an administrator");
    }
    None
}

/// The argv that creates an account (without its password).
pub fn add_argv(name: &str, full_name: &str, admin: bool) -> Vec<String> {
    ["useradd", "-m", "-G", if admin { ADMIN_GROUPS } else { STANDARD_GROUPS }, "-s", "/bin/bash", "-c", full_name.trim(), name].iter().map(|s| s.to_string()).collect()
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn db() -> (Vec<Passwd>, Vec<Group>) {
    (parse_passwd(&read("/etc/passwd")), parse_group(&read("/etc/group")))
}

/// The account BifrOSt was installed for, which can be neither deleted nor demoted.
fn installed() -> Option<String> {
    std::fs::read_to_string(format!("{}/account", super::STATE_DIR)).ok()?.lines().find_map(|l| l.strip_prefix("username=")).map(str::to_string)
}

fn run(argv: &[String], stdin: Option<&str>) -> Result<(), PErr> {
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]).stdout(Stdio::null()).stderr(Stdio::piped());
    if stdin.is_some() {
        c.stdin(Stdio::piped());
    }
    let mut child = c.spawn().map_err(|e| PErr::internal(format!("{}: {e}", argv[0])))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).map_err(|e| PErr::internal(format!("{}: {e}", argv[0])))?;
    }
    let out = child.wait_with_output().map_err(|e| PErr::internal(format!("{}: {e}", argv[0])))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(PErr::internal(format!("{}: {}", argv[0], err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim())))
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn text<'a>(p: &'a Json, name: &str) -> Result<&'a str, PErr> {
    p.get(name).and_then(Json::as_str).ok_or_else(|| PErr::bad(format!("missing {name}")))
}

fn flag(p: &Json, name: &str) -> Result<bool, PErr> {
    match p.get(name) {
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err(PErr::bad(format!("{name} must be a boolean"))),
    }
}

fn set_password(user: &str, password: &str) -> Result<(), PErr> {
    if !valid_password(password) {
        return Err(PErr::bad("invalid password"));
    }
    run(&strs(&["/usr/bin/chpasswd"]), Some(&format!("{user}:{password}\n")))
}

fn str_list(p: &Json, name: &str) -> Result<Vec<String>, PErr> {
    match p.get(name) {
        Some(Json::Arr(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| PErr::bad(format!("{name} must hold strings"))))
            .collect(),
        _ => Err(PErr::bad(format!("{name} must be a list"))),
    }
}

/// Settings > Users & Groups > content limits: replaces one managed person's app blocks and feature switches in the
/// account policy file the launcher reads. The installed account and system accounts have no limits to set.
fn set_policy(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "account")?;
    let (users, _) = db();
    if !users.iter().any(|u| u.name == name && is_human(u)) {
        return Err(PErr::bad("not a person's account"));
    }
    if installed().as_deref() == Some(name) {
        return Err(PErr::bad("the account BifrOSt was installed for has no limits"));
    }
    let blocked = str_list(p, "blocked")?;
    let disabled = str_list(p, "disabled")?;
    let blocked: Vec<&str> = blocked.iter().map(String::as_str).collect();
    let disabled: Vec<&str> = disabled.iter().map(String::as_str).collect();
    crate::bare::account_policy::validate_rules(&blocked, &disabled).map_err(PErr::bad)?;
    crate::bare::account_policy::set_account_rules(std::path::Path::new(crate::bare::account_policy::POLICY_FILE), name, &blocked, &disabled)
        .map(|_| obj(vec![]))
        .map_err(PErr::internal)
}

/// Dispatches the account methods; None when `method` is not one of them.
pub fn call(method: &str, p: &Json) -> Option<Result<Json, PErr>> {
    Some(match method {
        "account.set_policy" => set_policy(p),
        "user.add" => user_add(p),
        "user.delete" => user_delete(p),
        "user.set_admin" => user_set_admin(p),
        "user.set_password" => user_set_password(p),
        "group.add" => group_add(p),
        "group.delete" => group_delete(p),
        "group.member" => group_member(p),
        "system.set_names" => set_names(p),
        "service.set" => service_set(p),
        "screen_sharing.configure" => screen_sharing_configure(p),
        _ => return None,
    })
}

fn user_add(p: &Json) -> Result<Json, PErr> {
    let (name, full) = (text(p, "name")?, text(p, "full_name")?);
    if !valid_user(name) {
        return Err(PErr::bad("invalid account name"));
    }
    if !valid_full_name(full) {
        return Err(PErr::bad("invalid full name"));
    }
    let admin = flag(p, "admin")?;
    let password = text(p, "password")?;
    if !valid_password(password) {
        return Err(PErr::bad("invalid password"));
    }
    let (users, groups) = db();
    if users.iter().any(|u| u.name == name) || groups.iter().any(|g| g.name == name) {
        return Err(PErr::bad("that name is already taken"));
    }
    run(&add_argv(name, full, admin), None)?;
    if let Err(e) = set_password(name, password) {
        // never leave an account without the password the person typed
        let _ = run(&strs(&["userdel", "-r", name]), None);
        return Err(e);
    }
    Ok(obj(vec![("name", s(name))]))
}

fn user_delete(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "name")?;
    let remove_home = flag(p, "remove_home")?;
    if !valid_user(name) {
        return Err(PErr::bad("invalid account name"));
    }
    let (users, groups) = db();
    if let Some(why) = delete_refusal(name, &users, &groups, installed().as_deref()) {
        return Err(PErr::bad(why));
    }
    let mut argv = strs(&["userdel"]);
    if remove_home {
        argv.push("-r".into());
    }
    argv.push(name.into());
    run(&argv, None).map(|_| obj(vec![]))
}

fn user_set_admin(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "name")?;
    let admin = flag(p, "admin")?;
    if !valid_user(name) {
        return Err(PErr::bad("invalid account name"));
    }
    let (users, groups) = db();
    if let Some(why) = admin_refusal(name, admin, &users, &groups, installed().as_deref()) {
        return Err(PErr::bad(why));
    }
    run(&strs(&["gpasswd", if admin { "-a" } else { "-d" }, name, ADMIN_GROUP]), None).map(|_| obj(vec![]))
}

/// Another account's password. The installed account changes its own through `account.set_password`, which
/// checks the old one.
fn user_set_password(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "name")?;
    if !valid_user(name) {
        return Err(PErr::bad("invalid account name"));
    }
    let (users, _) = db();
    match users.iter().find(|u| u.name == name) {
        Some(u) if is_human(u) && installed().as_deref() != Some(name) => set_password(name, text(p, "new")?).map(|_| obj(vec![])),
        Some(_) => Err(PErr::bad("this account changes its password from Passwords")),
        None => Err(PErr::bad("no such account")),
    }
}

fn group_add(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "name")?;
    let (users, groups) = db();
    if !valid_user(name) || groups.iter().any(|g| g.name == name) || users.iter().any(|u| u.name == name) {
        return Err(PErr::bad("invalid or taken group name"));
    }
    run(&strs(&["groupadd", name]), None).map(|_| obj(vec![]))
}

/// A group Settings may manage: created for people (gid in the human range), not the admin group.
fn editable_group<'a>(groups: &'a [Group], name: &str) -> Result<&'a Group, PErr> {
    match groups.iter().find(|g| g.name == name) {
        Some(g) if (FIRST_UID..=LAST_UID).contains(&g.gid) && g.name != ADMIN_GROUP => Ok(g),
        Some(_) => Err(PErr::bad("that group is managed by the system")),
        None => Err(PErr::bad("no such group")),
    }
}

fn group_delete(p: &Json) -> Result<Json, PErr> {
    let name = text(p, "name")?;
    let (users, groups) = db();
    let g = editable_group(&groups, name)?;
    if users.iter().any(|u| u.gid == g.gid) {
        return Err(PErr::bad("it is an account's primary group"));
    }
    run(&strs(&["groupdel", name]), None).map(|_| obj(vec![]))
}

fn group_member(p: &Json) -> Result<Json, PErr> {
    let (group, user) = (text(p, "group")?, text(p, "user")?);
    let add = flag(p, "add")?;
    let (users, groups) = db();
    editable_group(&groups, group)?;
    if !users.iter().any(|u| u.name == user && is_human(u)) {
        return Err(PErr::bad("no such account"));
    }
    run(&strs(&["gpasswd", if add { "-a" } else { "-d" }, user, group]), None).map(|_| obj(vec![]))
}

/// `system.set_names {computer?, local?}`: the computer name people see (pretty host name) and the local host
/// name other computers use. Either may be given.
fn set_names(p: &Json) -> Result<Json, PErr> {
    let computer = p.get("computer").and_then(Json::as_str);
    let local = p.get("local").and_then(Json::as_str);
    if computer.is_none() && local.is_none() {
        return Err(PErr::bad("computer or local"));
    }
    if computer.is_some_and(|c| !valid_computer_name(c)) {
        return Err(PErr::bad("invalid computer name"));
    }
    if local.is_some_and(|l| !valid_hostname(l)) {
        return Err(PErr::bad("invalid local host name"));
    }
    if let Some(c) = computer {
        run(&strs(&["hostnamectl", "set-hostname", "--pretty", c.trim()]), None)?;
    }
    if let Some(l) = local {
        run(&strs(&["hostnamectl", "set-hostname", "--static", "--transient", l]), None)?;
    }
    Ok(obj(vec![]))
}

fn service_set(p: &Json) -> Result<Json, PErr> {
    let id = text(p, "id")?;
    let units = service_units(id).ok_or_else(|| PErr::bad("unknown service"))?;
    let on = flag(p, "on")?;
    if on && id == "screen_sharing" {
        // never start the server without its account, key and a current local-network address
        screen_sharing_prepare()?;
    }
    let mut argv = vec!["systemctl", if on { "enable" } else { "disable" }, "--now"];
    argv.extend_from_slice(units);
    run(&strs(&argv), None).map(|_| obj(vec![]))
}

/// Writes the wayvnc configuration for the address this computer has on the local network now.
fn screen_sharing_write_config() -> Result<(), PErr> {
    use super::screen_sharing as ss;
    let out = Command::new("ip").args(["-4", "-o", "addr", "show", "scope", "global"]).output().map_err(|e| PErr::internal(format!("ip: {e}")))?;
    let address = ss::lan_address(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| PErr::bad("this computer has no local network address, so Screen Sharing stays off"))?;
    super::methods::write_nofollow(ss::CONF, &ss::config_text(&address, ss::PORT, ss::KEY)).map_err(|e| PErr::internal(e.to_string()))
}

/// What must exist before the server starts: the account that may sign in and the key. The address is refreshed.
fn screen_sharing_prepare() -> Result<(), PErr> {
    use super::screen_sharing as ss;
    let ready = std::fs::read_to_string(ss::USERS).ok().and_then(|t| ss::user_from(&t)).is_some() && std::path::Path::new(ss::KEY).is_file();
    if !ready {
        return Err(PErr::bad("choose the account for Screen Sharing first"));
    }
    screen_sharing_write_config()
}

/// `screen_sharing.configure {user}`: the account whose password lets a viewer in. The first call makes this computer's
/// RSA key; the account must be a person's account. The server restarts when it runs, so the change takes effect.
fn screen_sharing_configure(p: &Json) -> Result<Json, PErr> {
    use super::screen_sharing as ss;
    let user = text(p, "user")?;
    if !valid_user(user) {
        return Err(PErr::bad("invalid account name"));
    }
    let (users, _) = db();
    if !users.iter().any(|u| u.name == user && is_human(u)) {
        return Err(PErr::bad("not a person's account"));
    }
    std::fs::create_dir_all(ss::DIR).map_err(|e| PErr::internal(e.to_string()))?;
    if !std::path::Path::new(ss::KEY).exists() {
        run(&ss::keygen_argv(ss::KEY), None)?;
    }
    super::methods::write_nofollow(ss::USERS, &ss::users_text(user)).map_err(|e| PErr::internal(e.to_string()))?;
    // the account name is not a secret and Settings reads it to show it; the key and the configuration stay root-only
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ss::USERS, std::fs::Permissions::from_mode(0o644)).map_err(|e| PErr::internal(e.to_string()))?;
    }
    screen_sharing_write_config()?;
    // a running server keeps the account it started with: restart it only when it is running
    let _ = run(&strs(&["systemctl", "try-restart", "bifrost-wayvnc.service"]), None);
    Ok(obj(vec![("user", s(user))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:Nobody:/:/usr/bin/nologin\nolof:x:1000:1000:Olof Dev,,,:/home/olof:/bin/bash\nkid:x:1001:1001:Kid:/home/kid:/bin/bash\nsvc:x:970:970::/var/svc:/usr/bin/nologin\nbad line\n";
    const GROUP: &str = "root:x:0:\nwheel:x:998:olof\naudio:x:996:olof,kid\nolof:x:1000:\nkid:x:1001:\nfriends:x:1002:kid\n";

    #[test]
    fn passwd_and_group_parse_and_skip_bad_lines() {
        let users = parse_passwd(PASSWD);
        assert_eq!(users.len(), 5);
        assert_eq!(users[2], Passwd { name: "olof".into(), uid: 1000, gid: 1000, full_name: "Olof Dev".into(), home: "/home/olof".into(), shell: "/bin/bash".into() });
        let groups = parse_group(GROUP);
        assert_eq!(groups[1].members, ["olof"]);
        assert_eq!(groups[2].members, ["olof", "kid"]);
        assert!(groups[0].members.is_empty());
    }

    #[test]
    fn people_are_the_human_uids_with_a_login_shell() {
        let humans: Vec<_> = parse_passwd(PASSWD).into_iter().filter(is_human).map(|u| u.name).collect();
        assert_eq!(humans, ["olof", "kid"]);
    }

    #[test]
    fn administrators_are_the_wheel_members() {
        let (users, groups) = (parse_passwd(PASSWD), parse_group(GROUP));
        assert!(is_admin(&users[2], &groups));
        assert!(!is_admin(&users[3], &groups));
    }

    #[test]
    fn names_follow_the_useradd_rules() {
        for ok in ["olof", "_x", "a-b_c9", "a"] {
            assert!(valid_user(ok), "{ok}");
        }
        let long = "a".repeat(33);
        for bad in ["", "Olof", "9a", "a b", "a:b", "-a", "a/b", "../x", long.as_str()] {
            assert!(!valid_user(bad), "{bad}");
        }
        assert!(valid_full_name("Ása Jónsdóttir") && !valid_full_name("a:b") && !valid_full_name("  ") && !valid_full_name("a\nb"));
        assert!(valid_hostname("my-pc1") && !valid_hostname("-pc") && !valid_hostname("pc-") && !valid_hostname("a b") && !valid_hostname("a.b"));
        assert!(valid_computer_name("Óli's Computer") && !valid_computer_name("x\ny"));
        assert!(valid_password("pa:ss word") && !valid_password("") && !valid_password("a\nroot:b"));
    }

    #[test]
    fn a_computer_name_suggests_a_local_host_name() {
        assert_eq!(hostname_for("Óli's MacBook Pro"), "li-s-macbook-pro");
        assert_eq!(hostname_for("  My   PC! "), "my-pc");
        assert_eq!(hostname_for("???"), "");
        assert!(valid_hostname(&hostname_for("Living Room TV")));
    }

    #[test]
    fn only_the_listed_services_can_be_switched() {
        assert_eq!(service_unit("remote_login"), Some("sshd.service"));
        assert_eq!(service_unit("sshd.service"), None);
        assert_eq!(service_unit("../x"), None);
    }

    #[test]
    fn screen_sharing_switches_the_packaged_unit() {
        assert_eq!(service_units("screen_sharing"), Some(&["bifrost-wayvnc.service"][..]));
    }

    #[test]
    fn file_sharing_brings_the_netbios_name_service_with_it() {
        assert_eq!(service_units("file_sharing"), Some(&["smb.service", "nmb.service"][..]));
        assert_eq!(service_unit("file_sharing"), Some("smb.service"));
        assert_eq!(service_units("printer_sharing"), Some(&["cups-lpd.socket"][..]));
        assert_eq!(service_units("nmb.service"), None);
        for id in ["remote_login", "file_sharing", "screen_sharing", "printer_sharing"] {
            assert!(service_units(id).unwrap().iter().all(|u| u.ends_with(".service") || u.ends_with(".socket")));
        }
    }

    #[test]
    fn delete_keeps_system_accounts_the_installed_account_and_the_last_admin() {
        let (users, groups) = (parse_passwd(PASSWD), parse_group(GROUP));
        assert_eq!(delete_refusal("kid", &users, &groups, Some("olof")), None);
        assert!(delete_refusal("olof", &users, &groups, Some("olof")).is_some(), "installed account");
        assert!(delete_refusal("olof", &users, &groups, None).is_some(), "the only administrator");
        assert!(delete_refusal("nobody", &users, &groups, None).is_some());
        assert!(delete_refusal("root", &users, &groups, None).is_some());
        assert!(delete_refusal("ghost", &users, &groups, None).is_some());
    }

    #[test]
    fn demotion_needs_another_administrator_and_not_the_installed_account() {
        let (users, mut groups) = (parse_passwd(PASSWD), parse_group(GROUP));
        assert!(admin_refusal("olof", false, &users, &groups, None).is_some(), "last administrator");
        assert_eq!(admin_refusal("kid", true, &users, &groups, Some("olof")), None);
        groups[1].members.push("kid".into());
        assert_eq!(admin_refusal("olof", false, &users, &groups, None), None);
        assert!(admin_refusal("olof", false, &users, &groups, Some("olof")).is_some(), "the installed account stays an administrator");
    }

    #[test]
    fn the_add_argv_depends_on_the_kind_of_account() {
        let std = add_argv("kid", " Kid ", false);
        assert_eq!(std, ["useradd", "-m", "-G", "audio,video,storage", "-s", "/bin/bash", "-c", "Kid", "kid"]);
        assert!(add_argv("a", "A", true).contains(&"wheel,audio,video,input,storage".to_string()));
    }

    #[test]
    fn bad_parameters_are_refused_before_anything_runs() {
        let bad = |m: &str, fields: Vec<(&str, Json)>| call(m, &obj(fields)).expect("a known method").unwrap_err().code;
        assert_eq!(bad("user.add", vec![("name", s("Bad Name")), ("full_name", s("x")), ("admin", Json::Bool(false)), ("password", s("pw"))]), "bad_params");
        assert_eq!(bad("user.add", vec![("name", s("ok")), ("full_name", s("x")), ("admin", Json::Bool(false)), ("password", s(""))]), "bad_params");
        assert_eq!(bad("user.delete", vec![("name", s("root")), ("remove_home", Json::Bool(true))]), "bad_params");
        assert_eq!(bad("service.set", vec![("id", s("telnet")), ("on", Json::Bool(true))]), "bad_params");
        assert_eq!(bad("screen_sharing.configure", vec![]), "bad_params");
        assert_eq!(bad("screen_sharing.configure", vec![("user", s("Bad Name"))]), "bad_params");
        assert_eq!(bad("screen_sharing.configure", vec![("user", s("root"))]), "bad_params", "root is not a person's account");
        assert_eq!(bad("system.set_names", vec![]), "bad_params");
        assert_eq!(bad("system.set_names", vec![("local", s("a b"))]), "bad_params");
        assert!(call("time.set_ntp", &obj(vec![])).is_none());
    }
}
