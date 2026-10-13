//! Password verification for privd: libpam FFI, plus shadow + crypt_r (feature `shadow-auth`).
//! The user name always comes from the installed account, never from a client.
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;

/// Outcome of a verification attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Verify {
    Ok,
    Wrong,
    /// libpam or the shadow file broke. Never a login and never a bypass: the session stays locked.
    Internal(String),
}

/// Something that can check a password for a user. Tests use fakes.
pub trait Verifier: Send + Sync {
    fn verify(&self, user: &str, password: &str) -> Verify;
    /// Called once at boot. `Err` means the backend cannot start at all (libpam or the service file is
    /// missing): privd logs it loudly and keeps the lock shut (see `Auth::probe_boot`).
    fn probe(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Constant-time byte comparison (length leak only).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0
}

/// Overwrites a buffer in a way the optimiser may not drop.
pub fn zeroise(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// What the second field of an /etc/shadow line means.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Hash {
    /// A usable crypt(3) hash.
    Set,
    /// `!`, `*`, `!!` or `!hash`: no password login.
    Locked,
    /// Empty field: passwordless.
    Empty,
    Missing,
}

/// Looks `user` up in shadow text and returns the hash class and field.
pub fn shadow_entry<'a>(shadow: &'a str, user: &str) -> (Hash, &'a str) {
    for line in shadow.lines() {
        let mut f = line.split(':');
        if f.next() != Some(user) {
            continue;
        }
        let h = f.next().unwrap_or("");
        let class = if h.is_empty() {
            Hash::Empty
        } else if h.starts_with('!') || h.starts_with('*') {
            Hash::Locked
        } else {
            Hash::Set
        };
        return (class, h);
    }
    (Hash::Missing, "")
}

// ---- libpam ----
#[repr(C)]
struct PamMessage {
    style: c_int,
    msg: *const c_char,
}
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    retcode: c_int,
}
#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    data: *mut c_void,
}
// libpam is loaded with dlopen so that only `bifrost-privd` ever maps it: no other target in this crate
// links it, and a missing library is a clean `pam_start`-class failure at boot instead of a loader error.
extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, sym: *const c_char) -> *mut c_void;
    fn calloc(n: usize, s: usize) -> *mut c_void;
    fn strdup(s: *const c_char) -> *mut c_char;
}
type StartFn = unsafe extern "C" fn(*const c_char, *const c_char, *const PamConv, *mut *mut c_void) -> c_int;
type AuthFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type EndFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
struct PamLib {
    start: StartFn,
    authenticate: AuthFn,
    acct_mgmt: AuthFn,
    end: EndFn,
}
// SAFETY: plain function pointers into libpam, which is thread-safe per handle.
unsafe impl Send for PamLib {}
unsafe impl Sync for PamLib {}

fn pam_lib() -> Result<&'static PamLib, String> {
    static LIB: std::sync::OnceLock<Result<PamLib, String>> = std::sync::OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let h = dlopen(c"libpam.so.0".as_ptr(), 2 /* RTLD_NOW */);
        if h.is_null() {
            return Err("libpam.so.0 cannot be loaded".into());
        }
        let sym = |n: &std::ffi::CStr| {
            let p = dlsym(h, n.as_ptr());
            if p.is_null() { Err(format!("libpam has no {n:?}")) } else { Ok(p) }
        };
        Ok(PamLib {
            start: std::mem::transmute::<*mut c_void, StartFn>(sym(c"pam_start")?),
            authenticate: std::mem::transmute::<*mut c_void, AuthFn>(sym(c"pam_authenticate")?),
            acct_mgmt: std::mem::transmute::<*mut c_void, AuthFn>(sym(c"pam_acct_mgmt")?),
            end: std::mem::transmute::<*mut c_void, EndFn>(sym(c"pam_end")?),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}
const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_PERM_DENIED: c_int = 6;
const PAM_AUTH_ERR: c_int = 7;
const PAM_AUTHINFO_UNAVAIL: c_int = 9;
const PAM_USER_UNKNOWN: c_int = 10;
const PAM_MAXTRIES: c_int = 11;
const PAM_NEW_AUTHTOK_REQD: c_int = 12;
const PAM_ACCT_EXPIRED: c_int = 13;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// Conversation: answers every password prompt with a fresh copy of the password
/// (libpam frees and wipes its copy).
extern "C" fn converse(n: c_int, msgs: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || data.is_null() {
        return PAM_CONV_ERR;
    }
    let pw = unsafe { &*(data as *const CString) };
    let out = unsafe { calloc(n as usize, std::mem::size_of::<PamResponse>()) } as *mut PamResponse;
    if out.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..n as isize {
        let m = unsafe { &**msgs.offset(i) };
        if m.style == PAM_PROMPT_ECHO_OFF || m.style == PAM_PROMPT_ECHO_ON {
            unsafe { (*out.offset(i)).resp = strdup(pw.as_ptr()) };
        }
    }
    unsafe { *resp = out };
    PAM_SUCCESS
}

/// libpam backend using the `bifrost-login` service.
pub struct Pam {
    pub service: &'static str,
}
impl Default for Pam {
    fn default() -> Self {
        Pam { service: "bifrost-login" }
    }
}
impl Verifier for Pam {
    fn probe(&self) -> Result<(), String> {
        let lib = pam_lib()?;
        let (svc, usr) = (CString::new(self.service).map_err(|e| e.to_string())?, c"root");
        let conv = PamConv { conv: converse, data: ptr::null_mut() };
        let mut h: *mut c_void = ptr::null_mut();
        // SAFETY: valid NUL-terminated strings and a conversation that is never called.
        let rc = unsafe { (lib.start)(svc.as_ptr(), usr.as_ptr(), &conv, &mut h) };
        if rc != PAM_SUCCESS {
            return Err(format!("pam_start failed ({rc})"));
        }
        unsafe { (lib.end)(h, PAM_SUCCESS) };
        Ok(())
    }

    fn verify(&self, user: &str, password: &str) -> Verify {
        let lib = match pam_lib() {
            Ok(l) => l,
            Err(e) => return Verify::Internal(e),
        };
        let (Ok(svc), Ok(usr)) = (CString::new(self.service), CString::new(user)) else {
            return Verify::Internal("bad name".into());
        };
        let Ok(pw) = CString::new(password) else {
            return Verify::Wrong; // NUL can never match
        };
        let conv = PamConv { conv: converse, data: &pw as *const CString as *mut c_void };
        let mut h: *mut c_void = ptr::null_mut();
        let mut rc = unsafe { (lib.start)(svc.as_ptr(), usr.as_ptr(), &conv, &mut h) };
        if rc != PAM_SUCCESS {
            return Verify::Internal(format!("pam_start {rc}"));
        }
        rc = unsafe { (lib.authenticate)(h, 0) };
        if rc == PAM_SUCCESS {
            rc = unsafe { (lib.acct_mgmt)(h, 0) };
            // An expired token still proves the password; the user can change it later.
            if rc == PAM_NEW_AUTHTOK_REQD {
                rc = PAM_SUCCESS;
            }
        }
        unsafe { (lib.end)(h, rc) };
        let mut raw = pw.into_bytes();
        zeroise(&mut raw);
        match rc {
            PAM_SUCCESS => Verify::Ok,
            PAM_AUTH_ERR | PAM_AUTHINFO_UNAVAIL | PAM_USER_UNKNOWN | PAM_MAXTRIES | PAM_PERM_DENIED | PAM_ACCT_EXPIRED => Verify::Wrong,
            other => Verify::Internal(format!("pam {other}")),
        }
    }
}

#[cfg(feature = "shadow-auth")]
mod shadow_impl {
    use super::*;
    #[repr(C)]
    struct CryptData {
        buf: [u8; 32768],
    }
    #[link(name = "crypt")]
    extern "C" {
        fn crypt_r(key: *const c_char, salt: *const c_char, data: *mut CryptData) -> *mut c_char;
    }
    /// /etc/shadow + crypt_r backend.
    pub struct Shadow;
    impl Verifier for Shadow {
        fn verify(&self, user: &str, password: &str) -> Verify {
            let Ok(text) = std::fs::read_to_string("/etc/shadow") else {
                return Verify::Internal("shadow unreadable".into());
            };
            let (class, hash) = shadow_entry(&text, user);
            match class {
                Hash::Set => {}
                Hash::Missing => return Verify::Internal("no shadow entry".into()),
                _ => return Verify::Wrong,
            }
            let (Ok(h), Ok(pw)) = (CString::new(hash), CString::new(password)) else {
                return Verify::Wrong;
            };
            let mut d = Box::new(CryptData { buf: [0; 32768] });
            let r = unsafe { crypt_r(pw.as_ptr(), h.as_ptr(), &mut *d) };
            let ok = !r.is_null() && ct_eq(unsafe { std::ffi::CStr::from_ptr(r) }.to_bytes(), hash.as_bytes());
            zeroise(&mut d.buf);
            let mut raw = pw.into_bytes();
            zeroise(&mut raw);
            if ok { Verify::Ok } else { Verify::Wrong }
        }
    }
}
#[cfg(feature = "shadow-auth")]
pub use shadow_impl::Shadow;
