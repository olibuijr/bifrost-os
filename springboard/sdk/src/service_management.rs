//! Login items (`SMAppService`): `SMAppService.mainApp` registers the app to start when the user's session starts. The
//! shell keeps the list (`Settings > General > Login Items` shows and removes entries) and launches the apps in it
//! after the desktop is up. Needs no manifest permission: the user sees and controls the list.
//!
//! `status()` is synchronous like and reads the last status the shell reported; the shell reports it once when the
//! app starts and after every `register`, `unregister` and `refresh`, which also arrives in `App::login_item_status`.

pub use crate::proto::LoginStatus as Status;
use crate::proto::{LoginOp, ToShell};
use crate::Ctx;
use std::sync::atomic::{AtomicU8, Ordering};

static LAST: AtomicU8 = AtomicU8::new(0);

/// `SMAppService`; only `mainApp` exists (BifrOSt has no embedded login-item helpers, launch agents or daemons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SMAppService(());

impl SMAppService {
    /// `SMAppService.mainApp`.
    pub fn main_app() -> Self {
        SMAppService(())
    }
    /// `register()`: the answer arrives as `App::login_item_status` with the returned request id.
    pub fn register(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Register)
    }
    /// `unregister()`.
    pub fn unregister(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Unregister)
    }
    /// Asks the shell for the current status again (it can change in Settings).
    pub fn refresh(&self, ctx: &Ctx) -> u32 {
        send(ctx, LoginOp::Query)
    }
    /// `status`: the last status the shell reported (`NotRegistered` until the first report).
    pub fn status(&self) -> Status {
        Status::from_wire(LAST.load(Ordering::Relaxed)).unwrap_or(Status::NotRegistered)
    }
}

fn send(ctx: &Ctx, op: LoginOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::LoginItem { request, op });
    request
}

pub(crate) fn mirror(status: Status) {
    LAST.store(status as u8, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_go_to_the_shell_and_status_follows_the_report() {
        let ctx = crate::runtime::test_ctx();
        let s = SMAppService::main_app();
        assert_eq!(s.status(), Status::NotRegistered);
        let a = s.register(&ctx);
        let b = s.refresh(&ctx);
        let c = s.unregister(&ctx);
        assert_eq!(
            ctx.take_sent(),
            [ToShell::LoginItem { request: a, op: LoginOp::Register }, ToShell::LoginItem { request: b, op: LoginOp::Query }, ToShell::LoginItem { request: c, op: LoginOp::Unregister }]
        );
        mirror(Status::Enabled);
        assert_eq!(s.status(), Status::Enabled);
        mirror(Status::NotRegistered);
    }
}
