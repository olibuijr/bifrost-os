use super::*;
use crate::privd::pam::{ct_eq, shadow_entry, Hash, Verifier, Verify};
use std::sync::{Arc, Mutex};

struct Fake(Arc<Mutex<Vec<Verify>>>, &'static str);
impl Verifier for Fake {
    fn verify(&self, _: &str, pw: &str) -> Verify {
        if let Some(v) = self.0.lock().unwrap().pop() {
            return v;
        }
        if pw == self.1 { Verify::Ok } else { Verify::Wrong }
    }
}

fn setup(tag: &str) -> (Auth, Arc<Mutex<Vec<Verify>>>, PathBuf) {
    let d = std::env::temp_dir().join(format!("privd-auth-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("shadow"), "olafur:$6$salt$hash:1::::::\n").unwrap();
    std::fs::write(d.join("account"), "full_name=Olafur B\nusername=olafur\n").unwrap();
    let cfg = Config {
        account_file: d.join("account"),
        shadow: d.join("shadow"),
        fails_file: d.join("fails"),
        bypass_flag: d.join("no-login"),
    };
    let q = Arc::new(Mutex::new(vec![]));
    let acct: AccountFn = Box::new(|| Ok(Account { name: "olafur".into(), uid: 1000, gid: 1000, home: "/home/olafur".into() }));
    (Auth::with(cfg, Box::new(Fake(q.clone(), "pw")), Some(acct)), q, d)
}

#[test]
fn lockout_curve() {
    assert_eq!([0, 4, 5, 6, 7, 8, 50].map(lockout_secs), [0, 0, 60, 300, 900, 3600, 3600]);
}

#[test]
fn token_compare_and_len() {
    assert!(ct_eq(b"abc", b"abc"));
    assert!(!ct_eq(b"abc", b"abd"));
    assert!(!ct_eq(b"abc", b"abcd"));
    let t = new_token().unwrap();
    assert_eq!(t.len(), 32);
    assert_ne!(t, new_token().unwrap());
}

#[test]
fn shadow_parsing() {
    let s = "root:!:1::::::\nbob:*:1::::::\nann::1::::::\ncy:$y$j9T$x$y:1::::::\n";
    assert_eq!(shadow_entry(s, "root").0, Hash::Locked);
    assert_eq!(shadow_entry(s, "bob").0, Hash::Locked);
    assert_eq!(shadow_entry(s, "ann").0, Hash::Empty);
    assert_eq!(shadow_entry(s, "cy"), (Hash::Set, "$y$j9T$x$y"));
    assert_eq!(shadow_entry(s, "zed").0, Hash::Missing);
}

#[test]
fn login_unlock_lock_cycle() {
    let (mut a, _, _) = setup("cycle");
    assert_eq!(a.require_unlocked(Some("x")), Err(AuthError::Locked));
    let t = a.authenticate_at("pw", 100).unwrap();
    assert_eq!(a.require_unlocked(Some(&t)), Ok(()));
    assert_eq!(a.require_unlocked(Some("bad")), Err(AuthError::Unauthorized));
    assert_eq!(a.require_unlocked(None), Err(AuthError::Unauthorized));
    a.lock();
    assert_eq!(a.require_unlocked(Some(&t)), Err(AuthError::Locked));
}

#[test]
fn wrong_passwords_lock_out_and_persist() {
    let (mut a, _, d) = setup("fail");
    for i in 1..=4 {
        assert_eq!(a.authenticate_at("no", 10), Err(AuthError::WrongPassword { fails: i, retry_secs: 0 }));
    }
    assert_eq!(a.authenticate_at("no", 10), Err(AuthError::WrongPassword { fails: 5, retry_secs: 60 }));
    assert_eq!(a.authenticate_at("pw", 20), Err(AuthError::LockedOut { retry_secs: 50 }));
    assert_eq!(parse_fails(&std::fs::read_to_string(d.join("fails")).unwrap()), (5, 70));
    assert!(a.authenticate_at("pw", 71).is_ok());
    assert_eq!(parse_fails(&std::fs::read_to_string(d.join("fails")).unwrap()), (0, 0));
}

#[test]
fn locked_hash_means_no_password() {
    let (mut a, _, d) = setup("nopw");
    std::fs::write(d.join("shadow"), "olafur:!:1::::::\n").unwrap();
    assert_eq!(a.authenticate_at("pw", 1), Err(AuthError::NoPassword));
    assert!(!a.account_info().unwrap().has_password);
}

#[test]
fn flag_bypass_accepts_empty_only() {
    let (mut a, _, d) = setup("flag");
    std::fs::write(d.join("no-login"), "").unwrap();
    assert!(!a.account_info().unwrap().has_password);
    assert!(a.authenticate_at("", 1).is_ok());
    assert!(matches!(a.authenticate_at("x", 1), Err(AuthError::BadParams(_))));
}

#[test]
fn runtime_pam_errors_never_bypass() {
    let (mut a, q, _) = setup("auto");
    for _ in 0..10 {
        q.lock().unwrap().push(Verify::Internal("boom".into()));
        assert!(matches!(a.authenticate_at("pw", 1), Err(AuthError::Internal(_))));
    }
    assert!(!a.bypass_active());
    assert!(!a.is_unlocked());
    // an empty password is just a wrong password
    assert!(matches!(a.authenticate_at("", 1), Err(AuthError::WrongPassword { .. })));
}

struct Dead;
impl Verifier for Dead {
    fn verify(&self, _: &str, _: &str) -> Verify {
        Verify::Internal("dead".into())
    }
    fn probe(&self) -> Result<(), String> {
        Err("pam_start failed (4)".into())
    }
}

#[test]
fn bypass_only_from_flag_or_failed_boot_probe() {
    let (mut a, _, d) = setup("probe");
    a.probe_boot();
    assert!(!a.bypass_active(), "a working backend never bypasses");
    let cfg = Config { account_file: d.join("account"), shadow: d.join("shadow"), fails_file: d.join("fails2"), bypass_flag: d.join("no-login") };
    let acct: AccountFn = Box::new(|| Ok(Account { name: "olafur".into(), uid: 1000, gid: 1000, home: "/home/olafur".into() }));
    let mut b = Auth::with(cfg, Box::new(Dead), Some(acct));
    assert!(!b.bypass_active(), "no probe, no bypass: runtime errors alone cannot open the lock");
    assert!(matches!(b.authenticate_at("", 1), Err(AuthError::WrongPassword { .. }) | Err(AuthError::Internal(_))));
    b.probe_boot();
    assert!(!b.bypass_active(), "a failed boot probe fails closed: no automatic bypass");
    std::fs::write(d.join("no-login"), "").unwrap();
    assert!(b.bypass_active(), "only root's explicit flag opens it");
    assert!(b.authenticate_at("", 1).is_ok());
}

#[test]
fn password_is_not_checked_under_the_lock() {
    let (mut a, _, _) = setup("split");
    let Begin::Verify(u) = a.begin_authenticate("pw", 1).unwrap() else { panic!("expected verify") };
    assert_eq!(u, "olafur");
    // a second attempt while the first verifies is refused, not queued behind the lock
    assert!(matches!(a.begin_authenticate("pw", 1), Err(AuthError::LockedOut { .. })));
    assert!(a.finish_authenticate(Verify::Ok, 1).is_ok());
    assert!(matches!(a.begin_authenticate("pw", 1), Ok(Begin::Verify(_))));
}

#[test]
fn set_password_requires_unlock_and_valid_new() {
    let (mut a, _, _) = setup("setpw");
    assert_eq!(a.set_password(None, true, "pw", "longenough"), Err(AuthError::Locked));
    let t = a.authenticate_at("pw", 1).unwrap();
    assert!(matches!(a.set_password(Some(&t), true, "pw", "a:bcdefgh"), Err(AuthError::BadParams(_))));
    assert!(matches!(a.set_password(Some(&t), true, "pw", "short"), Err(AuthError::BadParams(_))));
    assert!(matches!(a.set_password(Some(&t), true, "wrong", "longenough"), Err(AuthError::WrongPassword { .. })));
    // a wrong old password counts toward the lockout
    assert_eq!(a.fails, 1);
}

#[test]
fn set_password_without_token_only_for_an_empty_hash_from_the_shell() {
    let (mut a, _, d) = setup("setpw2");
    // usable hash, no token: refused whoever asks
    assert_eq!(a.set_password(None, true, "", "longenough"), Err(AuthError::Locked));
    // locked hash (passwd -l): never, not even from the shell
    std::fs::write(d.join("shadow"), "olafur:!$6$x:1::::::\n").unwrap();
    assert_eq!(a.set_password(None, true, "", "longenough"), Err(AuthError::NoPassword));
    std::fs::write(d.join("shadow"), "olafur:!:1::::::\n").unwrap();
    assert_eq!(a.set_password(None, true, "", "longenough"), Err(AuthError::NoPassword));
    // empty hash: not from anyone but the console shell
    std::fs::write(d.join("shadow"), "olafur::1::::::\n").unwrap();
    assert_eq!(a.set_password(None, false, "", "longenough"), Err(AuthError::Unauthorized));
    // from the shell the plan is allowed, but a short password is still refused
    assert!(matches!(a.set_password(None, true, "", "short"), Err(AuthError::BadParams(_))));
    assert_eq!(a.plan_set_password(None, true, "longenough", 1), Ok(PwPlan::First("olafur".into())));
    // a missing entry fails closed
    std::fs::write(d.join("shadow"), "other:$6$x:1::::::\n").unwrap();
    assert!(matches!(a.set_password(None, true, "", "longenough"), Err(AuthError::Internal(_))));
}

#[test]
fn the_password_hint_is_read_from_the_account_file_and_cleaned() {
    let (a, _, d) = setup("hint");
    assert_eq!(a.account_info().unwrap().password_hint, "");
    std::fs::write(d.join("account"), "full_name=Olafur B\nusername=olafur\npassword_hint=  first  pet\u{7}  \n").unwrap();
    assert_eq!(a.account_info().unwrap().password_hint, "first  pet");
}

#[test]
fn hint_parsing_drops_control_characters_blank_text_and_long_text() {
    assert_eq!(parse_hint("full_name=x\n"), "");
    assert_eq!(parse_hint("password_hint=   \n"), "");
    assert_eq!(parse_hint("password_hint=a\u{0}b\n"), "ab");
    let long = format!("password_hint={}\n", "h".repeat(HINT_MAX + 50));
    assert_eq!(parse_hint(&long).chars().count(), HINT_MAX);
}
