//! The app's Dock tile through the `dock` system service (`springboard/src/shell/services/dock_tile.rs`): the badge
//! (`NativeDockTile.badgeLabel`), a progress bar (a `contentView` with a progress indicator) and an attention bounce
//! (`NativeApplication.requestUserAttention`), and the app's own items in the Dock's menu (`set_menu`; choosing one
//! arrives as `Input::ContextAction` with the item's id). Each call returns the request id; the reply arrives in
//! `App::service_reply`, or can be taken with `service::take_reply`.
//!
//! ```ignore
//! dock_tile::set_badge(ctx, "3");
//! dock_tile::set_progress(ctx, Some(0.5));
//! dock_tile::bounce(ctx, false);
//! ```
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The shell service name.
pub const SERVICE: &str = "dock";

/// The params of `badge`: the text the Dock shows (the shell keeps four characters; empty clears the badge).
pub fn badge_params(text: &str) -> Json {
    Json::Obj(vec![("text".into(), Json::Str(text.into()))])
}

/// The params of `progress`: a fraction from 0 to 1 (the shell clamps it), or no key at all to clear the bar.
pub fn progress_params(fraction: Option<f32>) -> Json {
    match fraction.filter(|f| f.is_finite()) {
        Some(f) => Json::Obj(vec![("fraction".into(), Json::Num(f as f64))]),
        None => Json::Obj(Vec::new()),
    }
}

/// The params of an indeterminate `progress`: a sweeping bar.
pub fn indeterminate_params() -> Json {
    Json::Obj(vec![("indeterminate".into(), Json::Bool(true))])
}

/// The params of `bounce`: `critical` keeps the icon bouncing for longer.
pub fn bounce_params(critical: bool) -> Json {
    Json::Obj(vec![("critical".into(), Json::Bool(critical))])
}

/// Sets the app's Dock badge text. Needs the `notifications` permission; empty clears it.
pub fn set_badge(ctx: &Ctx, text: &str) -> u32 {
    service::call(ctx, SERVICE, "badge", &badge_params(text))
}

/// Shows a determinate progress bar at `fraction` (0 to 1), or clears it with `None`.
pub fn set_progress(ctx: &Ctx, fraction: Option<f32>) -> u32 {
    service::call(ctx, SERVICE, "progress", &progress_params(fraction))
}

/// Shows an indeterminate (sweeping) progress bar.
pub fn set_indeterminate(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "progress", &indeterminate_params())
}

/// Bounces the app's Dock icon once (`critical` false) or for longer (`critical` true).
pub fn bounce(ctx: &Ctx, critical: bool) -> u32 {
    service::call(ctx, SERVICE, "bounce", &bounce_params(critical))
}

/// One item of the app's Dock menu. Choosing it arrives as `Input::ContextAction(id)`.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub id: u32,
    pub title: String,
    /// A disabled item shows, greyed out, and cannot be chosen.
    pub enabled: bool,
}

/// The params of `menu`: the list of items, at most eight, each with its id, title and enabled flag.
pub fn menu_params(items: &[MenuItem]) -> Json {
    let list = items
        .iter()
        .map(|i| {
            Json::Obj(vec![
                ("id".into(), Json::Num(f64::from(i.id))),
                ("title".into(), Json::Str(i.title.clone())),
                ("enabled".into(), Json::Bool(i.enabled)),
            ])
        })
        .collect();
    Json::Obj(vec![("items".into(), Json::Arr(list))])
}

/// Replaces the app's Dock menu items (the shell keeps eight, with titles cut to 64 characters). An empty slice clears them.
pub fn set_menu(ctx: &Ctx, items: &[MenuItem]) -> u32 {
    service::call(ctx, SERVICE, "menu", &menu_params(items))
}

/// Reads back the badge, progress, indeterminate and bouncing state of the app's Dock tile.
pub fn state(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "state", &Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn test_ctx() -> Ctx {
        use crate::kit::{Font, Ui};
        use crate::proto::Configure;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    #[test]
    fn the_params_name_the_keys_the_shell_reads() {
        assert_eq!(badge_params("3").encode(), Json::Obj(vec![("text".into(), Json::Str("3".into()))]).encode());
        assert_eq!(progress_params(Some(0.25)).get("fraction"), Some(&Json::Num(0.25)));
        assert_eq!(progress_params(None).get("fraction"), None, "no key clears the bar");
        assert_eq!(progress_params(Some(f32::NAN)).get("fraction"), None, "not finite: cleared");
        assert_eq!(indeterminate_params().get("indeterminate"), Some(&Json::Bool(true)));
        assert_eq!(bounce_params(true).get("critical"), Some(&Json::Bool(true)));
    }

    #[test]
    fn the_menu_params_list_each_item_with_its_id_title_and_flag() {
        let items = [MenuItem { id: 4, title: "Open Recent".into(), enabled: false }];
        let params = menu_params(&items);
        let Some(Json::Arr(list)) = params.get("items") else { panic!("items must be a list") };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("id"), Some(&Json::Num(4.0)));
        assert_eq!(list[0].get("title"), Some(&Json::Str("Open Recent".into())));
        assert_eq!(list[0].get("enabled"), Some(&Json::Bool(false)));
        assert_eq!(menu_params(&[]).get("items"), Some(&Json::Arr(Vec::new())), "an empty list clears the menu");
    }

    #[test]
    fn each_call_goes_to_the_dock_service_with_its_method() {
        let ctx = test_ctx();
        let a = set_badge(&ctx, "7");
        let b = set_progress(&ctx, Some(0.5));
        let c = bounce(&ctx, false);
        let d = state(&ctx);
        let sent = ctx.take_sent();
        assert_eq!(sent[0], ToShell::ServiceCall { request: a, service: "dock".into(), method: "badge".into(), body: badge_params("7").encode() });
        assert_eq!(sent[1], ToShell::ServiceCall { request: b, service: "dock".into(), method: "progress".into(), body: progress_params(Some(0.5)).encode() });
        assert_eq!(sent[2], ToShell::ServiceCall { request: c, service: "dock".into(), method: "bounce".into(), body: bounce_params(false).encode() });
        assert_eq!(sent[3], ToShell::ServiceCall { request: d, service: "dock".into(), method: "state".into(), body: "null".into() });
    }
}
