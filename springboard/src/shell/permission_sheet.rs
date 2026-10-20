//! The permission sheet: when an app asks for location, the camera, the microphone or screen recording through the
//! `privacy` service, the shell asks the person Allow or Don't Allow on an alert card.
//!
//! The sheet is the permission question model (`bifrost_ui::permission_question`). It sends one question about the
//! privacy topic to its own `AskCenter`, and the person's button is the answer. The default choice is Don't Allow, so a
//! question nobody answers never grants. The answer is written with `privacy_permissions::decide`, the same store the
//! Settings switches use. The decisions are pure (`Sheet::new`, `Sheet::finish`), so they are testable without a
//! screen; `Sheet` only draws its card.
//!
//! The queue of requests is in `services::privacy_permissions` (`queue_prompt`, `take_prompt`). The shell takes one
//! request at a time, from its frame tick, when no sheet is on screen.

use super::services::privacy_permissions::{self, Grant, Kind, Prompt};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use crate::gfx::Surface;
use bifrost_ui::permission_question::{AskCenter, PermissionQuestion, QuestionTopic};

/// The choice ids a media question offers. Don't Allow is the default, the second one.
pub const ALLOW: &str = "allow";
pub const DONT_ALLOW: &str = "dont_allow";
/// The choice ids a location question offers: Allow Once, Allow While Using App, Don't Allow (the default).
pub const ALLOW_ONCE: &str = "allow_once";
pub const ALLOW_WHILE_USING: &str = "allow_while_using";
/// The choice id of a device question's Always Allow (it maps to the `WhileUsing` grant, which the device store keeps).
pub const ALWAYS_ALLOW: &str = "always_allow";
const USB_CHOICES: [&str; 3] = [ALLOW_ONCE, ALWAYS_ALLOW, DONT_ALLOW];
const MEDIA_CHOICES: [&str; 2] = [ALLOW, DONT_ALLOW];
const LOCATION_CHOICES: [&str; 3] = [ALLOW_ONCE, ALLOW_WHILE_USING, DONT_ALLOW];

/// The question's choices for a request: the ids in button order, and the index of the default (Don't Allow).
pub fn choices_for(kind: Kind) -> (&'static [&'static str], usize) {
    match kind {
        Kind::Location => (&LOCATION_CHOICES[..], 2),
        Kind::Usb => (&USB_CHOICES[..], 2),
        Kind::Media(_) | Kind::Contacts | Kind::Calendars { .. } => (&MEDIA_CHOICES[..], 1),
    }
}

/// The grant a location choice means. `None` for an id the location question does not offer.
pub fn grant_for(choice: &str) -> Option<Grant> {
    match choice {
        ALLOW_ONCE => Some(Grant::Once),
        ALLOW_WHILE_USING => Some(Grant::WhileUsing),
        DONT_ALLOW => Some(Grant::Deny),
        _ => None,
    }
}

/// The one question a sheet asks. Each sheet has its own queue, so the number only needs to be unique there.
const QUESTION_ID: u64 = 1;

/// Where the person changes the answer later, as the sheet's text says.
pub const SETTINGS_PATH: &str = "Settings > Privacy & Security > App Permissions";
/// Where the person changes a location answer later: the global switch, the fixed location and each app's access.
pub const LOCATION_SETTINGS_PATH: &str = "Settings > Privacy & Security > Location Services";

/// The privacy topic: which permission the question is about, by its wire name (`location`, `camera`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyTopic(pub &'static str);

impl QuestionTopic for PrivacyTopic {
    fn topic_name(&self) -> &'static str {
        self.0
    }
}

/// The buttons: Allow Once, Allow While Using App and Don't Allow for location (left to right); Don't Allow and Allow
/// for the media permissions. The bold one is the allow that keeps working while the app is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    AllowOnce,
    AllowWhileUsing,
    /// A device question's remembered answer.
    AlwaysAllow,
    Allow,
    DontAllow,
}

const USB_BUTTONS: [Button; 3] = [Button::AllowOnce, Button::AlwaysAllow, Button::DontAllow];
const LOCATION_BUTTONS: [Button; 3] = [Button::AllowOnce, Button::AllowWhileUsing, Button::DontAllow];
const MEDIA_BUTTONS: [Button; 2] = [Button::DontAllow, Button::Allow];

/// The buttons a request shows, left to right.
pub fn buttons_for(kind: Kind) -> &'static [Button] {
    match kind {
        Kind::Location => &LOCATION_BUTTONS[..],
        Kind::Usb => &USB_BUTTONS[..],
        Kind::Media(_) | Kind::Contacts | Kind::Calendars { .. } => &MEDIA_BUTTONS[..],
    }
}

impl Button {
    /// The choice id this button sends.
    pub fn choice(self) -> &'static str {
        match self {
            Button::AllowOnce => ALLOW_ONCE,
            Button::AllowWhileUsing => ALLOW_WHILE_USING,
            Button::AlwaysAllow => ALWAYS_ALLOW,
            Button::Allow => ALLOW,
            Button::DontAllow => DONT_ALLOW,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::AllowOnce => "Allow Once",
            Button::AllowWhileUsing => "Allow While Using App",
            Button::AlwaysAllow => "Always Allow",
            Button::Allow => "Allow",
            Button::DontAllow => "Don't Allow",
        }
    }

    fn bold(self) -> bool {
        matches!(self, Button::AllowWhileUsing | Button::AlwaysAllow | Button::Allow)
    }
}

/// The text under the title. For location it is the app's purpose string (its manifest's `[privacy] location`), then
/// where to change the answer; media keeps the short explanation.
pub fn detail(prompt: &Prompt) -> String {
    match prompt.kind {
        Kind::Location => format!("{} You can change this at any time in {LOCATION_SETTINGS_PATH}.", prompt.purpose),
        Kind::Media(_) => format!("Allowing lets the app use this while it is open. You can change this at any time in {SETTINGS_PATH}."),
        Kind::Contacts => format!("Allowing lets the app read the contacts you choose, while it is open. You can change this at any time in {SETTINGS_PATH}."),
        Kind::Calendars { write_only: true } => format!("Allowing lets the app add events to your calendar. It cannot read the events you already have. You can change this at any time in {SETTINGS_PATH}."),
        Kind::Calendars { write_only: false } => format!("Allowing lets the app read, add, change and delete your calendar events, while it is open. You can change this at any time in {SETTINGS_PATH}."),
        Kind::Usb => bifrost_core::usb_grants::sheet_detail(&super::services::usb_access::label_of(&prompt.purpose)),
    }
}

/// One request on screen: its question, the centre that holds it, and the card that draws it.
pub struct Sheet {
    prompt: Prompt,
    center: AskCenter,
    card: AlertCard,
}

impl Sheet {
    /// The sheet for one request, sent to a fresh queue at `now` (seconds). None only if the model refuses the question,
    /// which the fixed choices here never do.
    pub fn new(ui: &Ui, prompt: Prompt, now: u64) -> Option<Self> {
        let (choices, default) = choices_for(prompt.kind);
        let question = PermissionQuestion::for_topic(PrivacyTopic(prompt.kind.name()), QUESTION_ID, None).with_choices(choices, default).ok()?;
        let mut center = AskCenter::new();
        center.ask(&question, now).ok()?;
        let labels = buttons_for(prompt.kind)
            .iter()
            .map(|b| ui.text(b.label(), MAC_BODY_PT, if b.bold() { Weight::Semibold } else { Weight::Regular }, LINK))
            .collect();
        let card = AlertCard::new(ui, &prompt.kind.title(&prompt.app_name), &detail(&prompt), labels);
        Some(Sheet { prompt, center, card })
    }

    /// The request this sheet answers.
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// The button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        self.card.hit(ui, w, h, x, y).map(|i| buttons_for(self.prompt.kind)[i])
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, which: Button) -> Option<(f32, f32)> {
        let i = buttons_for(self.prompt.kind).iter().position(|b| *b == which)?;
        Some(self.card.button_centre(ui, w, h, i))
    }

    /// The person pressed a button at `now`. The question takes the answer and the store is written. Returns whether
    /// the request was allowed (any Allow), or None when the question refused the answer.
    pub fn finish(mut self, button: Button, now: u64) -> Option<bool> {
        let response = self.center.answer(QUESTION_ID, button.choice(), now).ok()?;
        let grant = match response.choice.as_str() {
            ALLOW_ONCE => Grant::Once,
            ALLOW_WHILE_USING | ALWAYS_ALLOW => Grant::WhileUsing,
            ALLOW => Grant::WhileUsing,
            _ => Grant::Deny,
        };
        privacy_permissions::decide_grant(&self.prompt, grant);
        Some(grant != Grant::Deny)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::services::privacy_permissions::{self as store};
    use bifrost_ui::capture_access::Media;
    use bifrost_ui::location_auth::AuthorizationStatus;

    /// A request for `kind` from `id`, queued and taken the way the shell takes it.
    fn request(id: &str, name: &str, kind: Kind) -> Prompt {
        store::queue_prompt(id, name, kind, "Finna næstu stöð.");
        store::take_prompt().expect("a request the person has not answered")
    }

    fn at(sheet: &Sheet, ui: &Ui, which: Button) -> (f32, f32) {
        sheet.button(ui, 1200, 1920, which).unwrap()
    }

    #[test]
    fn allow_while_using_writes_the_location_switch_on() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.allow";
        let sheet = Sheet::new(&ui, request(id, "Maps", Kind::Location), 0).unwrap();
        assert_eq!(sheet.prompt().app_name, "Maps");
        let (x, y) = at(&sheet, &ui, Button::AllowWhileUsing);
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some(Button::AllowWhileUsing));
        assert_eq!(sheet.finish(Button::AllowWhileUsing, 5), Some(true));
        assert_eq!(store::location_status(id), AuthorizationStatus::AuthorizedWhenInUse);
    }

    #[test]
    fn allow_once_grants_until_the_app_quits() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.once";
        let sheet = Sheet::new(&ui, request(id, "Maps", Kind::Location), 0).unwrap();
        assert_eq!(sheet.finish(Button::AllowOnce, 5), Some(true));
        assert_eq!(store::location_status(id), AuthorizationStatus::AuthorizedWhenInUse);
        assert!(store::expire_once(|a| a == id).is_empty(), "the app is still running");
        assert_eq!(store::expire_once(|_| false), [id.to_string()]);
        assert_eq!(store::location_status(id), AuthorizationStatus::NotDetermined, "asked again next time");
    }

    #[test]
    fn dont_allow_writes_the_location_switch_off() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.deny";
        let sheet = Sheet::new(&ui, request(id, "Maps", Kind::Location), 0).unwrap();
        let (x, y) = at(&sheet, &ui, Button::DontAllow);
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some(Button::DontAllow));
        assert_eq!(sheet.finish(Button::DontAllow, 5), Some(false));
        assert_eq!(store::location_status(id), AuthorizationStatus::Denied);
    }

    #[test]
    fn a_camera_sheet_grants_only_the_camera() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.cam";
        let sheet = Sheet::new(&ui, request(id, "Camera", Kind::Media(Media::Video)), 0).unwrap();
        assert_eq!(sheet.finish(Button::Allow, 1), Some(true));
        assert!(store::access(id).can_capture(Media::Video));
        assert!(!store::access(id).can_capture(Media::Audio));
    }

    #[test]
    fn a_tap_outside_the_buttons_hits_nothing() {
        let ui = crate::shell::tests::ui();
        let sheet = Sheet::new(&ui, request("is.example.sheet.miss", "Miss", Kind::Location), 0).unwrap();
        assert_eq!(sheet.hit(&ui, 1200, 1920, 2.0, 2.0), None);
    }

    #[test]
    fn each_request_offers_its_own_buttons_in_order() {
        assert_eq!(buttons_for(Kind::Location), &[Button::AllowOnce, Button::AllowWhileUsing, Button::DontAllow]);
        assert_eq!(buttons_for(Kind::Media(Media::Audio)), &[Button::DontAllow, Button::Allow]);
        assert_eq!(choices_for(Kind::Location), (&[ALLOW_ONCE, ALLOW_WHILE_USING, DONT_ALLOW][..], 2));
        assert_eq!(choices_for(Kind::Media(Media::Audio)), (&[ALLOW, DONT_ALLOW][..], 1));
    }

    #[test]
    fn a_calendar_request_offers_dont_allow_and_allow_and_says_what_allowing_lets_it_do() {
        let ask = |write_only| Prompt { app_id: "a".into(), app_name: "A".into(), kind: Kind::Calendars { write_only }, purpose: String::new() };
        assert_eq!(buttons_for(Kind::Calendars { write_only: true }), &[Button::DontAllow, Button::Allow]);
        assert_eq!(choices_for(Kind::Calendars { write_only: false }), (&[ALLOW, DONT_ALLOW][..], 1));
        assert!(detail(&ask(true)).contains("cannot read"));
        assert!(detail(&ask(false)).contains("read, add, change and delete"));
    }

    #[test]
    fn a_device_request_offers_allow_once_always_allow_and_dont_allow() {
        assert_eq!(buttons_for(Kind::Usb), &[Button::AllowOnce, Button::AlwaysAllow, Button::DontAllow]);
        assert_eq!(choices_for(Kind::Usb), (&[ALLOW_ONCE, ALWAYS_ALLOW, DONT_ALLOW][..], 2), "Don't Allow is the default");
        assert_eq!(Button::AlwaysAllow.choice(), "always_allow");
        assert_eq!(Kind::Usb.name(), "usb");
        assert_eq!(Kind::Usb.title("Scope"), "\u{201c}Scope\u{201d} would like to use a device connected to this computer.");
    }

    #[test]
    fn a_location_choice_maps_to_its_grant_and_an_unknown_one_to_none() {
        assert_eq!(grant_for(ALLOW_ONCE), Some(Grant::Once));
        assert_eq!(grant_for(ALLOW_WHILE_USING), Some(Grant::WhileUsing));
        assert_eq!(grant_for(DONT_ALLOW), Some(Grant::Deny));
        assert_eq!(grant_for("allow"), None, "the media answer is not a location choice");
    }

    #[test]
    fn the_detail_shows_the_purpose_and_the_matching_settings_path() {
        let p = request("is.example.sheet.text", "Maps", Kind::Location);
        let d = detail(&p);
        assert!(d.starts_with("Finna næstu stöð."));
        assert!(d.contains("Settings > Privacy & Security > Location Services"));
        let media = request("is.example.sheet.text2", "Camera", Kind::Media(Media::Video));
        assert!(detail(&media).contains("App Permissions"));
        assert_eq!(Button::Allow.choice(), "allow");
        assert_eq!(Button::DontAllow.choice(), "dont_allow");
    }
    /// MA6: Mail asked for the address book when a message was composed, and the sheet was drawn into the shell's own frame, which
    /// sits under every window on the desktop: no card to see, a veil left on the wallpaper that grew darker with each frame
    /// and stayed after the sheet was gone. On the desktop the sheet is a surface of its own above the windows.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn on_the_desktop_a_sheet_is_an_overlay_above_the_windows_and_leaves_no_veil_behind() {
        use crate::shell::desktop_bar::MODAL_CHROME_ID;
        let _lock = crate::test_support::global_lock();
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mid = (h / 2) * w + w / 2;
        let mut px = vec![0u32; w * h];
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 500.0);
        let bare = px[mid];
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != MODAL_CHROME_ID), "no sheet, no overlay");
        sh.permission_sheet = Sheet::new(&sh.ui, request("is.example.sheet.veil", "Mail", Kind::Contacts), 0);
        assert!(sh.permission_sheet.is_some());
        for k in 0..3 {
            sh.dirty = true;
            sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0 + k as f64 * 100.0);
        }
        assert_eq!(px[mid], bare, "the sheet does not draw into the frame under the windows: no veil, however many frames");
        let frames = sh.desktop_chrome_frames(&px, w, h);
        let f = frames.iter().find(|f| f.id == MODAL_CHROME_ID).expect("the sheet is a surface above the windows");
        assert_eq!((f.x, f.y, f.width, f.height), (0, 0, w, h), "the whole screen, so the sheet is modal");
        assert_eq!(f.input, None, "it takes the pointer");
        assert!(f.pixels[0] >> 24 != 0, "the veil covers the corner of the screen");
        assert!(f.pixels[mid] != f.pixels[0], "and the card is on it");
        // answered: the surface goes and the desktop is as it was
        sh.permission_sheet = None;
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 2000.0);
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != MODAL_CHROME_ID), "the overlay is dropped with the sheet");
        assert_eq!(px[mid], bare, "nothing of the sheet is left on the desktop");
    }
}
