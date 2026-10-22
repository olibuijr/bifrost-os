//! Family controls for apps (`FamilyControlsAgent` parity): the authorization an app needs before it may restrict other
//! apps, and the restrictions an authorized app has set. Screen Time itself (limits, Downtime, the shield) lives in the
//! shell (`shell::screentime`); this is the app-facing gate in front of it, all local.
//!
//! An app asks with [`FamilyControls::request`]. Nothing changes until the device owner decides, so the request waits in
//! [`FamilyControls::pending`]; the shell shows the prompt, checks the Screen Time passcode when one is set, and calls
//! [`FamilyControls::decide`]. An approved app may [`set_blocked_apps`](FamilyControls::set_blocked_apps) for apps other
//! than itself; revoking the authorization removes everything that app restricted, so a removed or distrusted app can
//! never leave the device locked down. A denied app stays denied until the owner [`reset`](FamilyControls::reset)s it.
//!
//! The whole state is one small text file ([`encode`](FamilyControls::encode) and [`parse`](FamilyControls::parse)) so it
//! survives a restart; the lines are `auth.<app>=approved:individual|approved:child|denied`, `pending.<app>=individual|child`
//! and `block.<app>=<id>,<id>`.
//!
//! Not here: the shell reading [`is_blocked`](FamilyControls::is_blocked) next to `Config::block`, and the prompt itself.
//! The remote side of family management (a parent managing a child's device from another device through an account) is
//! not mirrored: there is no family account service.

use std::collections::{BTreeMap, BTreeSet};

/// Who the app wants to restrict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Member {
    /// The person using this device restricts themselves.
    Individual,
    /// A guardian restricts a child's use of this device.
    Child,
}

impl Member {
    fn name(self) -> &'static str {
        match self {
            Member::Individual => "individual",
            Member::Child => "child",
        }
    }
    fn parse(text: &str) -> Option<Member> {
        match text {
            "individual" => Some(Member::Individual),
            "child" => Some(Member::Child),
            _ => None,
        }
    }
}

/// What an app may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The owner has not been asked, or has not answered yet.
    NotDetermined,
    Denied,
    Approved(Member),
}

/// What [`FamilyControls::request`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requested {
    /// The app already has this answer; no prompt is needed.
    Settled(Status),
    /// The request is waiting for the owner.
    Waiting,
}

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The app id is empty or has characters other than letters, digits, `.`, `-` and `_`.
    BadAppId(String),
    /// The app is not approved.
    NotAuthorized,
    /// There is no waiting request from that app.
    NothingPending,
    /// An app may not restrict itself or the Settings app.
    ProtectedApp(String),
    /// More blocked apps than [`MAX_BLOCKED`].
    TooMany,
}

/// The most apps one authorized app may block.
pub const MAX_BLOCKED: usize = 200;
/// The app that can never be blocked, so the owner can always undo a restriction.
pub const SETTINGS_APP: &str = "settings";

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Authorizations, waiting requests and the restrictions of the approved apps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FamilyControls {
    decided: BTreeMap<String, Status>,
    pending: BTreeMap<String, Member>,
    blocked: BTreeMap<String, BTreeSet<String>>,
}

impl FamilyControls {
    pub fn new() -> Self {
        Self::default()
    }

    /// What `app` may do now.
    pub fn status(&self, app: &str) -> Status {
        self.decided.get(app).copied().unwrap_or(Status::NotDetermined)
    }

    /// An app asks to restrict `member`. An answered app gets its answer back; a new or waiting one waits (asking again
    /// keeps one request, with the newest member).
    pub fn request(&mut self, app: &str, member: Member) -> Result<Requested, Error> {
        if !valid_id(app) {
            return Err(Error::BadAppId(app.to_string()));
        }
        match self.status(app) {
            Status::NotDetermined => {
                self.pending.insert(app.to_string(), member);
                Ok(Requested::Waiting)
            }
            settled => Ok(Requested::Settled(settled)),
        }
    }

    /// The apps waiting for the owner, with who they want to restrict, in app id order.
    pub fn pending(&self) -> Vec<(&str, Member)> {
        self.pending.iter().map(|(app, member)| (app.as_str(), *member)).collect()
    }

    /// The owner's answer to a waiting request. The caller has already asked for the Screen Time passcode.
    pub fn decide(&mut self, app: &str, approve: bool) -> Result<Status, Error> {
        let member = self.pending.remove(app).ok_or(Error::NothingPending)?;
        let status = if approve { Status::Approved(member) } else { Status::Denied };
        self.decided.insert(app.to_string(), status);
        Ok(status)
    }

    /// Takes the authorization away and removes every restriction the app set. Returns whether the app had any state.
    pub fn revoke(&mut self, app: &str) -> bool {
        let had = self.decided.remove(app).is_some() | self.pending.remove(app).is_some();
        self.blocked.remove(app);
        had
    }

    /// Forgets a denial (or an approval) so the app can ask again. Same effect as [`revoke`](Self::revoke).
    pub fn reset(&mut self, app: &str) -> bool {
        self.revoke(app)
    }

    /// Replaces the apps `setter` blocks. `setter` must be approved; it cannot block itself or the Settings app. An
    /// empty list clears its restrictions.
    pub fn set_blocked_apps(&mut self, setter: &str, apps: &[&str]) -> Result<(), Error> {
        if !matches!(self.status(setter), Status::Approved(_)) {
            return Err(Error::NotAuthorized);
        }
        let mut set = BTreeSet::new();
        for &app in apps {
            if !valid_id(app) {
                return Err(Error::BadAppId(app.to_string()));
            }
            if app == setter || app == SETTINGS_APP {
                return Err(Error::ProtectedApp(app.to_string()));
            }
            set.insert(app.to_string());
        }
        if set.len() > MAX_BLOCKED {
            return Err(Error::TooMany);
        }
        if set.is_empty() {
            self.blocked.remove(setter);
        } else {
            self.blocked.insert(setter.to_string(), set);
        }
        Ok(())
    }

    /// The apps `setter` blocks, in id order.
    pub fn blocked_by(&self, setter: &str) -> Vec<&str> {
        self.blocked.get(setter).map(|s| s.iter().map(String::as_str).collect()).unwrap_or_default()
    }

    /// Whether any approved app blocks `app`. The shell asks this next to its own Screen Time rules.
    pub fn is_blocked(&self, app: &str) -> bool {
        app != SETTINGS_APP && self.blocked.values().any(|s| s.contains(app))
    }

    /// The state as text, in a stable order.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for (app, status) in &self.decided {
            let value = match status {
                Status::Approved(member) => format!("approved:{}", member.name()),
                Status::Denied => "denied".to_string(),
                Status::NotDetermined => continue,
            };
            out.push_str(&format!("auth.{app}={value}\n"));
        }
        for (app, member) in &self.pending {
            out.push_str(&format!("pending.{app}={}\n", member.name()));
        }
        for (app, set) in &self.blocked {
            out.push_str(&format!("block.{app}={}\n", set.iter().cloned().collect::<Vec<_>>().join(",")));
        }
        out
    }

    /// Reads [`encode`](Self::encode). Lines that are malformed are skipped, and the rules are applied again on the way
    /// in: restrictions of an app that is not approved are dropped, and a waiting request of an app that has an answer
    /// is dropped, so a hand-edited file cannot grant more than the owner did.
    pub fn parse(text: &str) -> FamilyControls {
        let mut out = FamilyControls::default();
        let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.trim().split_once('=') else { continue };
            if let Some(app) = key.strip_prefix("auth.").filter(|a| valid_id(a)) {
                let status = match value.split_once(':') {
                    Some(("approved", member)) => Member::parse(member).map(Status::Approved),
                    None if value == "denied" => Some(Status::Denied),
                    _ => None,
                };
                if let Some(status) = status {
                    out.decided.insert(app.to_string(), status);
                }
            } else if let Some(app) = key.strip_prefix("pending.").filter(|a| valid_id(a)) {
                if let Some(member) = Member::parse(value) {
                    out.pending.insert(app.to_string(), member);
                }
            } else if let Some(app) = key.strip_prefix("block.").filter(|a| valid_id(a)) {
                blocks.push((app.to_string(), value.split(',').map(str::to_string).collect()));
            }
        }
        out.pending.retain(|app, _| !out.decided.contains_key(app));
        for (setter, apps) in blocks {
            let refs: Vec<&str> = apps.iter().map(String::as_str).filter(|a| !a.is_empty()).collect();
            let _ = out.set_blocked_apps(&setter, &refs);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved(app: &str, member: Member) -> FamilyControls {
        let mut f = FamilyControls::new();
        f.request(app, member).unwrap();
        f.decide(app, true).unwrap();
        f
    }

    #[test]
    fn an_app_starts_undetermined_and_its_request_waits_for_the_owner() {
        let mut f = FamilyControls::new();
        assert_eq!(f.status("com.x.guard"), Status::NotDetermined);
        assert_eq!(f.request("com.x.guard", Member::Child), Ok(Requested::Waiting));
        assert_eq!(f.status("com.x.guard"), Status::NotDetermined, "asking grants nothing");
        assert_eq!(f.pending(), vec![("com.x.guard", Member::Child)]);
    }

    #[test]
    fn approving_records_who_the_app_may_restrict() {
        let mut f = FamilyControls::new();
        f.request("a", Member::Individual).unwrap();
        assert_eq!(f.decide("a", true), Ok(Status::Approved(Member::Individual)));
        assert!(f.pending().is_empty());
        assert_eq!(f.request("a", Member::Child), Ok(Requested::Settled(Status::Approved(Member::Individual))));
    }

    #[test]
    fn a_denied_app_stays_denied_until_reset() {
        let mut f = FamilyControls::new();
        f.request("a", Member::Child).unwrap();
        assert_eq!(f.decide("a", false), Ok(Status::Denied));
        assert_eq!(f.request("a", Member::Child), Ok(Requested::Settled(Status::Denied)));
        assert!(f.reset("a"));
        assert_eq!(f.request("a", Member::Child), Ok(Requested::Waiting));
    }

    #[test]
    fn deciding_without_a_request_is_refused() {
        assert_eq!(FamilyControls::new().decide("a", true), Err(Error::NothingPending));
    }

    #[test]
    fn bad_app_ids_are_refused() {
        let mut f = FamilyControls::new();
        for bad in ["", "a b", "../x", "a=b", "a\nb"] {
            assert_eq!(f.request(bad, Member::Individual), Err(Error::BadAppId(bad.to_string())), "{bad:?}");
        }
    }

    #[test]
    fn only_an_approved_app_may_set_restrictions() {
        let mut f = FamilyControls::new();
        assert_eq!(f.set_blocked_apps("a", &["games"]), Err(Error::NotAuthorized));
        f.request("a", Member::Child).unwrap();
        assert_eq!(f.set_blocked_apps("a", &["games"]), Err(Error::NotAuthorized), "a waiting app is not approved");
        f.decide("a", true).unwrap();
        f.set_blocked_apps("a", &["games", "chat"]).unwrap();
        assert_eq!(f.blocked_by("a"), vec!["chat", "games"]);
        assert!(f.is_blocked("games") && f.is_blocked("chat") && !f.is_blocked("notes"));
    }

    #[test]
    fn an_app_cannot_block_itself_or_settings() {
        let mut f = approved("a", Member::Individual);
        assert_eq!(f.set_blocked_apps("a", &["a"]), Err(Error::ProtectedApp("a".into())));
        assert_eq!(f.set_blocked_apps("a", &["games", SETTINGS_APP]), Err(Error::ProtectedApp(SETTINGS_APP.into())));
        assert!(f.blocked_by("a").is_empty(), "a refused call changes nothing");
        assert!(!f.is_blocked(SETTINGS_APP));
    }

    #[test]
    fn a_new_list_replaces_the_old_and_an_empty_one_clears() {
        let mut f = approved("a", Member::Individual);
        f.set_blocked_apps("a", &["x", "y"]).unwrap();
        f.set_blocked_apps("a", &["z"]).unwrap();
        assert!(!f.is_blocked("x") && f.is_blocked("z"));
        f.set_blocked_apps("a", &[]).unwrap();
        assert!(!f.is_blocked("z"));
    }

    #[test]
    fn two_apps_can_block_the_same_app_and_each_unblocks_only_its_own() {
        let mut f = approved("a", Member::Individual);
        f.request("b", Member::Individual).unwrap();
        f.decide("b", true).unwrap();
        f.set_blocked_apps("a", &["games"]).unwrap();
        f.set_blocked_apps("b", &["games"]).unwrap();
        f.revoke("a");
        assert!(f.is_blocked("games"), "b still blocks it");
        f.revoke("b");
        assert!(!f.is_blocked("games"));
    }

    #[test]
    fn revoking_removes_the_authorization_and_every_restriction() {
        let mut f = approved("a", Member::Child);
        f.set_blocked_apps("a", &["games"]).unwrap();
        assert!(f.revoke("a"));
        assert_eq!(f.status("a"), Status::NotDetermined);
        assert!(!f.is_blocked("games"));
        assert!(!f.revoke("a"), "nothing left to revoke");
    }

    #[test]
    fn too_many_blocked_apps_are_refused() {
        let mut f = approved("a", Member::Individual);
        let ids: Vec<String> = (0..=MAX_BLOCKED).map(|i| format!("app{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(f.set_blocked_apps("a", &refs), Err(Error::TooMany));
    }

    #[test]
    fn the_state_round_trips_through_text() {
        let mut f = approved("a", Member::Child);
        f.set_blocked_apps("a", &["games", "chat"]).unwrap();
        f.request("denied.app", Member::Individual).unwrap();
        f.decide("denied.app", false).unwrap();
        f.request("waiting", Member::Individual).unwrap();
        let text = f.encode();
        assert!(text.contains("auth.a=approved:child\n") && text.contains("auth.denied.app=denied\n"));
        assert!(text.contains("pending.waiting=individual\n") && text.contains("block.a=chat,games\n"));
        assert_eq!(FamilyControls::parse(&text), f);
    }

    #[test]
    fn a_hand_edited_file_cannot_grant_more_than_the_owner_did() {
        let text = "block.rogue=games\n\
                    auth.ok=approved:individual\n\
                    block.ok=settings,notes,ok\n\
                    pending.ok=child\n\
                    auth.bad=approved:everyone\n\
                    auth.=denied\n\
                    nonsense\n";
        let f = FamilyControls::parse(text);
        assert!(!f.is_blocked("games"), "an unapproved app's restrictions are dropped");
        assert_eq!(f.status("ok"), Status::Approved(Member::Individual));
        assert!(f.pending().is_empty(), "an answered app has no waiting request");
        assert_eq!(f.status("bad"), Status::NotDetermined);
        assert!(f.blocked_by("ok").is_empty(), "the whole list is refused when it names settings or the app itself");
    }
}
