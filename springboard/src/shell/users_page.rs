//! Settings > Users & Groups, backed by the real system: the accounts and groups are read from `/etc/passwd` and
//! `/etc/group`, and every change goes to `bifrost-privd` (`privd::accounts`), which validates it and runs the fixed
//! command as root. The installed account can be neither deleted nor demoted; its own password changes through the
//! login service, which checks the old one.
use super::forms::{Prompt, Purpose};
use super::list::{Group, Item, Spec, Tint};
use super::options::Options;
use super::pages::{Action, PageId, Toggle};
use crate::privd::accounts::{self, is_admin, is_human, Passwd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub full_name: String,
    pub uid: u32,
    pub admin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRow {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
    /// Settings may add and remove members and delete it
    pub editable: bool,
}

/// The accounts and groups as the system has them now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub users: Vec<User>,
    pub groups: Vec<GroupRow>,
    /// the account BifrOSt was installed for
    pub me: String,
}

impl View {
    pub fn load() -> View {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        View::from_text(&read("/etc/passwd"), &read("/etc/group"), &super::account::load().username)
    }

    pub fn from_text(passwd: &str, group: &str, me: &str) -> View {
        let pw = accounts::parse_passwd(passwd);
        let gr = accounts::parse_group(group);
        let mut users: Vec<User> = pw.iter().filter(|p| is_human(p)).map(|p: &Passwd| User { name: p.name.clone(), full_name: if p.full_name.is_empty() { p.name.clone() } else { p.full_name.clone() }, uid: p.uid, admin: is_admin(p, &gr) }).collect();
        users.sort_by_key(|u| u.uid);
        let groups = gr
            .into_iter()
            .filter(|g| g.name == accounts::ADMIN_GROUP || (accounts::FIRST_UID..=accounts::LAST_UID).contains(&g.gid))
            // a user's private group is not interesting: same name, same single member or none
            .filter(|g| g.name == accounts::ADMIN_GROUP || !users.iter().any(|u| u.name == g.name && g.members.iter().all(|m| *m == u.name)))
            .map(|g| GroupRow { editable: g.name != accounts::ADMIN_GROUP, name: g.name, gid: g.gid, members: g.members })
            .collect();
        View { users, groups, me: me.to_string() }
    }

    fn user(&self, name: &str) -> Option<&User> {
        self.users.iter().find(|u| u.name == name)
    }

    fn group(&self, name: &str) -> Option<&GroupRow> {
        self.groups.iter().find(|g| g.name == name)
    }
}

/// What a row on these pages does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// opens the new account prompts (true: an administrator)
    Add(bool),
    SetAdmin(String, bool),
    /// opens the password prompt(s)
    Password(String),
    AddGroup,
    Member { group: String, user: String, add: bool },
}

/// A deletion, which asks first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Danger {
    DeleteUser(String),
    DeleteGroup(String),
}

impl Danger {
    /// Alert title, message, confirm button.
    pub fn alert(&self) -> (&'static str, String, &'static str) {
        match self {
            Danger::DeleteUser(n) => ("Delete User?", format!("“{n}” and the home folder of this account are deleted. This cannot be undone."), "Delete"),
            Danger::DeleteGroup(n) => ("Delete Group?", format!("The group “{n}” is removed. Its members and their files are kept."), "Delete"),
        }
    }
}

fn client_err(e: crate::privd_client::ClientError) -> String {
    e.message
}

/// Runs a confirmed deletion.
pub fn run(danger: &Danger) -> Result<(), String> {
    let c = crate::privd_client::connect();
    match danger {
        Danger::DeleteUser(n) => c.user_delete(n, true),
        Danger::DeleteGroup(n) => c.group_delete(n),
    }
    .map_err(client_err)
}

/// Applies an edit. Immediate ones run now (None); the others open their prompt.
pub fn apply(edit: &Edit, view: &View) -> Result<Option<Prompt>, String> {
    let c = crate::privd_client::connect();
    match edit {
        Edit::Add(admin) => Ok(Some(Prompt::new_user(*admin))),
        Edit::AddGroup => Ok(Some(Prompt::new_group())),
        Edit::Password(name) => Ok(Some(if *name == view.me { Prompt::current_password(name) } else { Prompt::change_password(name) })),
        Edit::SetAdmin(name, admin) => c.user_set_admin(name, *admin).map(|_| None).map_err(client_err),
        Edit::Member { group, user, add } => c.group_member(group, user, *add).map(|_| None).map_err(client_err),
    }
}

/// Finishes a prompt: the next prompt of its flow, or the change itself. A new password is typed twice (the confirm
/// prompt), and it reaches privd only when both entries are the same.
pub fn finish(purpose: &Purpose, text: &str) -> Result<Option<Prompt>, String> {
    if let Purpose::ConfirmPassword { first, then } = purpose {
        if text != first.as_str() {
            return Err("The passwords do not match. Type the password again.".into());
        }
        return submit(then, first).map(|_| None);
    }
    if let Some(next) = Prompt::next(purpose, text) {
        return Ok(Some(next));
    }
    submit(purpose, text).map(|_| None)
}

/// Sends the change that `purpose` asks for, with the password `text`, to privd.
fn submit(purpose: &Purpose, text: &str) -> Result<(), String> {
    let c = crate::privd_client::connect();
    match purpose {
        Purpose::NewUserPassword { admin, full, name } => c.user_add(name, full, *admin, text),
        Purpose::ChangePassword { name, old: None } => c.user_set_password(name, text),
        Purpose::ChangePassword { old: Some(old), .. } => c.account_set_password(old, text),
        Purpose::NewGroup => c.group_add(text),
        _ => return Ok(()),
    }
    .map_err(client_err)
}

fn role(u: &User) -> &'static str {
    if u.admin {
        "Admin"
    } else {
        "Standard"
    }
}

fn user_row(u: &User) -> Item {
    // the label is the account name: opening the row hands it to the detail page
    Item::new(u.name.as_str()).value(format!("{} · {}", u.full_name, role(u))).link(PageId::UserDetail)
}

/// The pages of Settings > Users & Groups. `note` is the last error or result, shown under the list.
pub fn spec(id: PageId, v: &View, detail: &str, note: &str, o: &Options) -> Spec {
    let page = |title: &str, groups: Vec<Group>| Spec { id, title: title.into(), large: false, groups };
    let with_note = |mut groups: Vec<Group>| {
        if !note.is_empty() {
            groups.push(Group::new(vec![]).footer(note));
        }
        groups
    };
    match id {
        PageId::UserDetail => {
            let Some(u) = v.user(detail) else { return page("User", with_note(vec![Group::new(vec![Item::new("This account no longer exists").tint(Tint::Dim)])])) };
            let installed = u.name == v.me;
            let admin = Item::new("Allow this user to administer this computer").switch(u.admin, Action::Users(Edit::SetAdmin(u.name.clone(), !u.admin)));
            let mut groups = vec![
                Group::new(vec![Item::new("Name").value(u.full_name.as_str()), Item::new("Account").value(u.name.as_str()), Item::new("Kind").value(role(u))]),
                if installed { Group::new(vec![admin.tint(Tint::Dim).action(Action::None)]).footer("The account this computer was set up with always stays an administrator.") } else { Group::new(vec![admin]) },
                Group::new(vec![Item::new("Change Password…").tint(Tint::Blue).action(Action::Users(Edit::Password(u.name.clone())))]),
            ];
            if !installed {
                groups.push(Group::new(vec![Item::new("Delete User…").tint(Tint::Red).action(Action::Run(super::pages::Command::Account(Danger::DeleteUser(u.name.clone()))))]));
            }
            page(&u.full_name, with_note(groups))
        }
        PageId::Groups => {
            let mut rows: Vec<Item> = v.groups.iter().map(|g| Item::new(g.name.as_str()).value(format!("{} member{}", g.members.len(), if g.members.len() == 1 { "" } else { "s" })).link(PageId::GroupDetail)).collect();
            if rows.is_empty() {
                rows.push(Item::new("No Groups").tint(Tint::Dim));
            }
            page("Groups", with_note(vec![Group::new(rows), Group::new(vec![Item::new("New Group…").tint(Tint::Blue).action(Action::Users(Edit::AddGroup))])]))
        }
        PageId::GroupDetail => {
            let Some(g) = v.group(detail) else { return page("Group", with_note(vec![Group::new(vec![Item::new("This group no longer exists").tint(Tint::Dim)])])) };
            let rows: Vec<Item> = v
                .users
                .iter()
                .map(|u| {
                    let member = g.members.contains(&u.name);
                    let item = Item::new(format!("{} ({})", u.full_name, u.name));
                    if g.editable {
                        item.switch(member, Action::Users(Edit::Member { group: g.name.clone(), user: u.name.clone(), add: !member }))
                    } else {
                        item.value(if member { "Member" } else { "" })
                    }
                })
                .collect();
            let mut groups = vec![Group::new(rows).header("MEMBERS")];
            if g.editable {
                groups.push(Group::new(vec![Item::new("Delete Group…").tint(Tint::Red).action(Action::Run(super::pages::Command::Account(Danger::DeleteGroup(g.name.clone()))))]));
            } else {
                groups.push(Group::new(vec![]).footer("Administrators can change settings and install software. Use a user's page to change this."));
            }
            page(&g.name, with_note(groups))
        }
        _ => {
            let me = v.user(&v.me);
            let card = match me {
                Some(u) => Item::new(u.full_name.as_str()).value(format!("{} · {}", u.name, role(u))).link(PageId::UserDetail),
                None => Item::new(v.me.as_str()).value("Account"),
            };
            let others: Vec<Item> = v.users.iter().filter(|u| u.name != v.me).map(user_row).collect();
            let mut groups = vec![Group::new(vec![card]).header("THIS ACCOUNT")];
            if !others.is_empty() {
                groups.push(Group::new(others).header("OTHER USERS"));
            }
            groups.push(Group::new(vec![Item::new("Add Standard User…").tint(Tint::Blue).action(Action::Users(Edit::Add(false))), Item::new("Add Administrator…").tint(Tint::Blue).action(Action::Users(Edit::Add(true)))]));
            groups.push(Group::new(vec![Item::new("Groups").value(v.groups.len().to_string()).link(PageId::Groups)]));
            groups.push(
                Group::new(vec![
                    Item::new("Name and Password").switch(o.login_name_password, Action::Toggle(Toggle::LoginNamePassword)),
                    Item::new("Password Hints").switch(o.login_hints, Action::Toggle(Toggle::LoginHints)),
                    Item::new("Automatic Login").value("Disk unlock").tint(Tint::Dim),
                    Item::new("Guest User").value("Off").tint(Tint::Dim),
                ])
                .header("LOGIN OPTIONS")
                .footer("Automatic login happens when the disk passphrase unlocks the session at start-up. A guest user is not available yet."),
            );
            page("Users & Groups", with_note(groups))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nolof:x:1000:1000:Olof Dev:/home/olof:/bin/bash\nkid:x:1001:1001:Kid:/home/kid:/bin/bash\nnobody:x:65534:65534:Nobody:/:/usr/bin/nologin\n";
    const GROUP: &str = "root:x:0:\nwheel:x:998:olof\nolof:x:1000:\nkid:x:1001:\nfriends:x:1002:kid,olof\n";

    fn view() -> View {
        View::from_text(PASSWD, GROUP, "olof")
    }

    #[test]
    fn the_view_lists_people_and_shared_groups_only() {
        let v = view();
        assert_eq!(v.users.iter().map(|u| (u.name.as_str(), u.admin)).collect::<Vec<_>>(), [("olof", true), ("kid", false)]);
        assert_eq!(v.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["wheel", "friends"], "private groups are hidden");
        assert!(!v.groups[0].editable && v.groups[1].editable);
    }

    #[test]
    fn the_root_page_shows_the_account_others_and_the_actions() {
        let s = spec(PageId::Users, &view(), "", "", &Options::default());
        let headers: Vec<_> = s.groups.iter().filter_map(|g| g.header.as_deref()).collect();
        assert_eq!(headers, ["THIS ACCOUNT", "OTHER USERS", "LOGIN OPTIONS"]);
        assert_eq!(s.groups[1].items[0].label, "kid");
        assert_eq!(s.groups[1].items[0].action, Action::Push(PageId::UserDetail));
        assert_eq!(s.groups[2].items[0].action, Action::Users(Edit::Add(false)));
        assert_eq!(s.groups[2].items[1].action, Action::Users(Edit::Add(true)));
    }

    #[test]
    fn a_standard_user_can_be_promoted_and_deleted_but_the_installed_account_cannot() {
        let s = spec(PageId::UserDetail, &view(), "kid", "", &Options::default());
        assert_eq!(s.groups[1].items[0].action, Action::Users(Edit::SetAdmin("kid".into(), true)));
        assert!(s.groups.iter().any(|g| g.items.iter().any(|i| i.action == Action::Run(super::super::pages::Command::Account(Danger::DeleteUser("kid".into()))))));
        let me = spec(PageId::UserDetail, &view(), "olof", "", &Options::default());
        assert!(!me.groups.iter().any(|g| g.items.iter().any(|i| i.label.starts_with("Delete"))));
        assert_eq!(me.groups[1].items[0].action, Action::None, "cannot demote the installed account");
    }

    #[test]
    fn group_members_are_switches_for_editable_groups() {
        let s = spec(PageId::GroupDetail, &view(), "friends", "", &Options::default());
        let rows = &s.groups[0].items;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].action, Action::Users(Edit::Member { group: "friends".into(), user: "olof".into(), add: false }));
        let wheel = spec(PageId::GroupDetail, &view(), "wheel", "", &Options::default());
        assert!(wheel.groups[0].items.iter().all(|i| i.action == Action::None));
    }

    #[test]
    fn an_error_note_shows_under_the_list() {
        let s = spec(PageId::Groups, &view(), "", "That name is taken", &Options::default());
        assert_eq!(s.groups.last().and_then(|g| g.footer.as_deref()), Some("That name is taken"));
    }

    #[test]
    fn a_password_that_does_not_match_its_confirmation_is_refused_before_privd() {
        let typed_again = Purpose::ConfirmPassword { first: "s3cret-one".into(), then: Box::new(Purpose::NewGroup) };
        assert!(finish(&typed_again, "s3cret-two").unwrap_err().contains("do not match"));
    }

    #[test]
    fn opening_an_edit_asks_before_changing_anything() {
        // these never reach privd: they open prompts
        assert!(matches!(apply(&Edit::Add(true), &view()), Ok(Some(p)) if p.title == "New Administrator"));
        assert!(matches!(apply(&Edit::AddGroup, &view()), Ok(Some(p)) if p.title == "New Group"));
        assert!(matches!(apply(&Edit::Password("olof".into()), &view()), Ok(Some(p)) if matches!(p.purpose, Purpose::CurrentPassword { .. })));
        assert!(matches!(apply(&Edit::Password("kid".into()), &view()), Ok(Some(p)) if p.title == "Change Password"));
    }

    #[test]
    fn the_password_flow_of_the_installed_account_asks_the_old_one_first() {
        let first = Prompt::current_password("olof");
        let next = Prompt::next(&first.purpose, "old-pass").expect("then the new one");
        assert_eq!(next.purpose, Purpose::ChangePassword { name: "olof".into(), old: Some("old-pass".into()) });
    }
}
