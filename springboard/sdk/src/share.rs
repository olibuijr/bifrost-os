//! Sharing (`NativeSharingServicePicker`): ask the shell to show the share menu for some items under a rect, and receive items
//! other apps share with you.
//!
//! Sending: [`show_picker`] with text, URLs and files (a file is named by the document grant the app already holds). The
//! shell lists the installed apps whose manifest `handles` (UTTypes) and `schemes` accept every item, plus Copy, and answers
//! `App::share_done` with the chosen app id (`"copy"` for Copy) or `None` when dismissed.
//!
//! Receiving: declare what you open in the manifest (`handles = ["public.png"]`, `schemes = ["https"]`). A shared URL arrives
//! as `Input::Open(url)`; shared text as `Input::Open` with a `data:` URL, which [`text_from_url`] decodes; shared files as
//! `App::document` events (`DocumentEvent::Opened` / `OpenedMany`) whose `request` has [`INBOUND`] set, each with a read-only
//! grant made for this app.

pub use crate::proto::Payload;
pub use springboard::shell::sharing::{text_from_url, Candidate, COMPOSE, COPY, INBOUND};

use crate::proto::{ToShell, UiRect};
use crate::Ctx;
use bifrost_ui::compose::{ComposeDraft, ComposeError, ComposeResult};
use springboard::shell::sharing::{copy_text, handles, targets, with_compose};

/// The compose draft that shared `items` start with: text parts joined by line breaks, each file as an image and each
/// URL as a link, in the order given. Fails with the error of the first item the draft refuses (a URL that is not
/// `http` or `https`, or more images or links than a post holds).
pub fn compose_draft_from_items(items: &[Payload]) -> Result<ComposeDraft, ComposeError> {
    let mut draft = ComposeDraft::new();
    let mut text: Vec<&str> = Vec::new();
    for item in items {
        match item {
            Payload::Text(part) => text.push(part.as_str()),
            Payload::Url(url) => draft.add_url(url.as_str())?,
            Payload::File { name, .. } => draft.add_image(name.as_str())?,
        }
    }
    if !text.is_empty() {
        draft.set_initial_text(&text.join("\n"));
    }
    Ok(draft)
}

/// `NativeSharingServicePicker.show(relativeTo:of:preferredEdge:)` for `items`, anchored under `anchor` (window points). The
/// answer arrives in `App::share_done` with the returned request id. At most 8 items.
pub fn show_picker(ctx: &Ctx, items: &[Payload], anchor: UiRect) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::Share { request, items: items.to_vec(), anchor });
    request
}

/// `ShareLink(item:)` for a `Transferable`: shows the share menu for the text or link `value` exports to another app. `None`
/// (nothing shown) when no visible representation can be shared.
pub fn show_transferable<T>(ctx: &Ctx, reps: &crate::transfer::Representations<T>, value: &T, anchor: UiRect) -> Option<u32> {
    let items = crate::transfer::share_items(reps, value);
    if items.is_empty() {
        return None;
    }
    Some(show_picker(ctx, &items, anchor))
}

/// True when a document `request` was not asked for by this app: a file somebody shared or dropped on it.
pub fn is_inbound(request: u32) -> bool {
    request & INBOUND != 0
}

/// One target of the share menu for a set of items (`NativeSharingService`). Built-in services come from
/// [`SharingService::named`]; installed apps and their order come from [`SharingService::sharing_services_for_items`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharingService {
    name: String,
    title: String,
    kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Copy,
    Compose,
    /// an installed app, with what its manifest says it opens
    App { handles: Vec<String>, schemes: Vec<String> },
}

impl SharingService {
    /// `NativeSharingService(named:)` for the built-in services (Copy, Compose Post). An app is not named; it comes from
    /// [`SharingService::sharing_services_for_items`]. `None` for any other name.
    pub fn named(name: &str) -> Option<SharingService> {
        match name {
            COPY => Some(SharingService { name: COPY.into(), title: "Copy".into(), kind: Kind::Copy }),
            COMPOSE => Some(SharingService { name: COMPOSE.into(), title: "Compose Post".into(), kind: Kind::Compose }),
            _ => None,
        }
    }

    /// `sharingServices(forItems:)`: the services that take every item, in the menu's order: the apps by name, then Compose
    /// Post and Copy. `installed` is what the caller may offer (its own app left out). Nothing shared offers nothing.
    pub fn sharing_services_for_items(items: &[Payload], installed: &[Candidate]) -> Vec<SharingService> {
        if items.is_empty() {
            return Vec::new();
        }
        with_compose(targets(installed, "", items), items)
            .into_iter()
            .filter_map(|t| match t.id.as_str() {
                COPY => SharingService::named(COPY),
                COMPOSE => SharingService::named(COMPOSE),
                id => {
                    let c = installed.iter().find(|c| c.id == id)?;
                    let kind = Kind::App { handles: c.handles.to_vec(), schemes: c.schemes.to_vec() };
                    Some(SharingService { name: t.id.clone(), title: t.name.clone(), kind })
                }
            })
            .collect()
    }

    /// The services that take what a `Transferable` exports ([`crate::transfer::share_items`]), in menu order.
    pub fn sharing_services_for_transferable<T>(reps: &crate::transfer::Representations<T>, value: &T, installed: &[Candidate]) -> Vec<SharingService> {
        Self::sharing_services_for_items(&crate::transfer::share_items(reps, value), installed)
    }

    /// The service's identifier: an app id, `COPY` or `COMPOSE`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The name the user sees in the menu.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// `canPerform(withItems:)`: true when the service takes every item. Nothing to share is never performable.
    pub fn can_perform(&self, items: &[Payload]) -> bool {
        !items.is_empty() && items.iter().all(|item| self.accepts(item))
    }

    fn accepts(&self, item: &Payload) -> bool {
        match &self.kind {
            Kind::Copy | Kind::Compose => true,
            Kind::App { handles: h, schemes } => handles(&Candidate { id: &self.name, name: &self.title, handles: h, schemes }, item),
        }
    }

    /// `perform(withItems:)` for Copy: the items go on the clipboard as the menu's Copy puts them. Returns false when the
    /// service cannot take the items. Apps and Compose Post are reached through the menu ([`SharingServicePicker::show`]),
    /// because the shell has no message that hands items to one named target yet.
    pub fn perform(&self, ctx: &Ctx, items: &[Payload]) -> bool {
        if !matches!(self.kind, Kind::Copy) || !self.can_perform(items) {
            return false;
        }
        ctx.send_to_shell(ToShell::SetClipboard { text: copy_text(items) });
        true
    }
}

/// Why a chosen service did not take the items (`sharingService(_:didFailToShareItems:)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// the shell answered with a target the picker never offered
    NotOffered(String),
    /// the compose sheet was closed without a post
    Cancelled,
}

/// `NativeSharingServicePickerDelegate`.
pub trait SharingServicePickerDelegate {
    /// `sharingServicePicker(_:didChoose:)`: the service the user chose, or `None` when the menu was dismissed.
    fn did_choose(&mut self, service: Option<&SharingService>);
}

/// `NativeSharingServiceDelegate`: how a chosen service ended.
pub trait SharingServiceDelegate {
    /// `sharingService(_:didShareItems:)`.
    fn did_share_items(&mut self, items: &[Payload]);
    /// `sharingService(_:didFailToShareItems:)`.
    fn did_fail_to_share_items(&mut self, items: &[Payload], error: ShareError);
}

/// `NativeSharingServicePicker` for one set of items: the services it offers, the menu it is showing, and the routing of
/// the shell's answer to the delegates.
pub struct SharingServicePicker {
    items: Vec<Payload>,
    services: Vec<SharingService>,
    /// the request of the menu on screen, until it is answered
    request: Option<u32>,
    /// true after Compose Post was chosen, until the compose sheet reports how it closed
    composing: bool,
}

impl SharingServicePicker {
    /// The picker for `items`, offering the services that take them (see [`SharingService::sharing_services_for_items`]).
    pub fn new(items: &[Payload], installed: &[Candidate]) -> Self {
        SharingServicePicker {
            items: items.to_vec(),
            services: SharingService::sharing_services_for_items(items, installed),
            request: None,
            composing: false,
        }
    }

    /// True between choosing Compose Post and the compose sheet's result (see [`Self::compose_finished`]).
    pub fn awaiting_compose(&self) -> bool {
        self.composing
    }

    /// The services offered, in menu order.
    pub fn services(&self) -> &[SharingService] {
        &self.services
    }

    /// `show(relativeTo:of:preferredEdge:)`: shows the menu under `anchor`. The answer comes to [`Self::share_done`].
    pub fn show(&mut self, ctx: &Ctx, anchor: UiRect) -> u32 {
        let request = show_picker(ctx, &self.items, anchor);
        self.begin(request);
        request
    }

    fn begin(&mut self, request: u32) {
        self.request = Some(request);
    }

    /// Feeds the answer of `App::share_done`. A chosen app or Copy gets `did_choose(Some)` then `did_share_items`. Compose
    /// Post gets `did_choose(Some)` only: the post is shared when the compose sheet reports `Done` to
    /// [`Self::compose_finished`]. A dismissed menu gets `did_choose(None)`; a target the picker did not offer gets
    /// `did_choose(None)` and a `NotOffered` failure. Returns false when `request` is not the menu on screen (or was
    /// answered already).
    pub fn share_done(
        &mut self,
        request: u32,
        target: Option<&str>,
        picker: &mut impl SharingServicePickerDelegate,
        sharer: &mut impl SharingServiceDelegate,
    ) -> bool {
        if self.request != Some(request) {
            return false;
        }
        self.request = None;
        match target {
            None => picker.did_choose(None),
            Some(id) => match self.services.iter().find(|s| s.name() == id) {
                Some(service) if service.name() == COMPOSE => {
                    picker.did_choose(Some(service));
                    self.composing = true;
                }
                Some(service) => {
                    picker.did_choose(Some(service));
                    sharer.did_share_items(&self.items);
                }
                None => {
                    picker.did_choose(None);
                    sharer.did_fail_to_share_items(&self.items, ShareError::NotOffered(id.to_string()));
                }
            },
        }
        true
    }

    /// The compose sheet's result for a Compose Post share (`ComposeSheet::finish`): `Done` shares the items, `Cancelled`
    /// fails the share with [`ShareError::Cancelled`]. Returns false when no Compose Post share is waiting.
    pub fn compose_finished(&mut self, result: ComposeResult, sharer: &mut impl SharingServiceDelegate) -> bool {
        if !self.composing {
            return false;
        }
        self.composing = false;
        match result {
            ComposeResult::Done => sharer.did_share_items(&self.items),
            ComposeResult::Cancelled => sharer.did_fail_to_share_items(&self.items, ShareError::Cancelled),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_requests_are_recognised() {
        assert!(is_inbound(INBOUND | 5));
        assert!(!is_inbound(5));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// Installed apps as the shell lists them (the sender left out): id, name, handles, schemes.
    fn table() -> Vec<(&'static str, &'static str, Vec<String>, Vec<String>)> {
        vec![
            ("notes", "Notes", s(&["public.plain-text:default"]), vec![]),
            ("photos", "Photos", s(&["public.png:owner", "public.jpeg"]), vec![]),
            ("browser", "Browser", s(&["public.html:owner"]), s(&["https"])),
        ]
    }

    fn candidates<'a>(table: &'a [(&'a str, &'a str, Vec<String>, Vec<String>)]) -> Vec<Candidate<'a>> {
        table.iter().map(|(id, name, handles, schemes)| Candidate { id, name, handles, schemes }).collect()
    }

    fn titles(services: &[SharingService]) -> Vec<&str> {
        services.iter().map(|s| s.title()).collect()
    }

    fn png() -> Payload {
        Payload::File { grant: [1; 16], name: "a.png".into() }
    }

    #[test]
    fn services_for_text_are_the_apps_that_open_text_then_compose_and_copy() {
        let table = table();
        let services = SharingService::sharing_services_for_items(&[Payload::Text("hi".into())], &candidates(&table));
        assert_eq!(titles(&services), ["Notes", "Compose Post", "Copy"]);
    }

    #[test]
    fn services_for_a_file_match_by_conformance() {
        let table = table();
        let services = SharingService::sharing_services_for_items(&[png()], &candidates(&table));
        assert_eq!(titles(&services), ["Photos", "Compose Post", "Copy"]);
    }

    #[test]
    fn a_service_must_accept_every_item_and_nothing_shared_offers_nothing() {
        let table = table();
        let mixed = [png(), Payload::Text("x".into())];
        assert_eq!(titles(&SharingService::sharing_services_for_items(&mixed, &candidates(&table))), ["Compose Post", "Copy"]);
        assert!(SharingService::sharing_services_for_items(&[], &candidates(&table)).is_empty());
    }

    #[test]
    fn web_urls_go_to_the_app_that_handles_the_scheme() {
        let table = table();
        let url = [Payload::Url("https://example.com/x".into())];
        assert_eq!(titles(&SharingService::sharing_services_for_items(&url, &candidates(&table))), ["Browser", "Compose Post", "Copy"]);
    }

    #[test]
    fn services_for_a_transferable_follow_its_exported_text() {
        let table = table();
        let reps = crate::transfer::Representations::<String>::new().data("public.utf8-plain-text", crate::transfer::Visibility::All, |s| s.clone().into_bytes(), |b| String::from_utf8(b.to_vec()).ok());
        let services = SharingService::sharing_services_for_transferable(&reps, &"hi".to_string(), &candidates(&table));
        assert_eq!(titles(&services), ["Notes", "Compose Post", "Copy"]);
        let none = crate::transfer::Representations::<String>::new();
        assert!(SharingService::sharing_services_for_transferable(&none, &"hi".to_string(), &candidates(&table)).is_empty());
    }

    #[test]
    fn can_perform_is_false_for_items_a_service_does_not_accept() {
        let table = table();
        let services = SharingService::sharing_services_for_items(&[Payload::Text("hi".into())], &candidates(&table));
        let notes = services.iter().find(|s| s.name() == "notes").expect("Notes takes text");
        let copy = services.iter().find(|s| s.name() == COPY).expect("Copy is always offered");
        assert!(notes.can_perform(&[Payload::Text("again".into())]));
        assert!(!notes.can_perform(&[png()]), "Notes does not open images");
        assert!(!notes.can_perform(&[]), "nothing to share");
        assert!(copy.can_perform(&[png(), Payload::Url("https://x.test".into())]), "Copy takes every kind");
    }

    #[test]
    fn named_builds_the_built_in_services_only() {
        assert_eq!(SharingService::named(COPY).expect("Copy").title(), "Copy");
        assert_eq!(SharingService::named(COMPOSE).expect("Compose").title(), "Compose Post");
        assert!(SharingService::named("ghost").is_none());
    }

    #[test]
    fn shared_items_round_trip_through_the_protocol() {
        let items = vec![Payload::Text("héllo".into()), Payload::Url("https://x.test/a".into()), Payload::File { grant: [9; 16], name: "a.png".into() }];
        let msg = ToShell::Share { request: 4, items: items.clone(), anchor: UiRect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 } };
        match ToShell::decode(&msg.encode()) {
            Ok(ToShell::Share { request, items: back, .. }) => {
                assert_eq!(request, 4);
                assert_eq!(back, items);
            }
            _ => panic!("the share did not decode"),
        }
    }

    #[derive(Default)]
    struct PickerLog {
        chose: Vec<Option<String>>,
    }

    impl SharingServicePickerDelegate for PickerLog {
        fn did_choose(&mut self, service: Option<&SharingService>) {
            self.chose.push(service.map(|s| s.name().to_string()));
        }
    }

    #[derive(Default)]
    struct SharerLog {
        shared: usize,
        failed: Vec<ShareError>,
    }

    impl SharingServiceDelegate for SharerLog {
        fn did_share_items(&mut self, items: &[Payload]) {
            self.shared += items.len();
        }
        fn did_fail_to_share_items(&mut self, _items: &[Payload], error: ShareError) {
            self.failed.push(error);
        }
    }

    /// A picker that has shown its menu as `request` (the part of `show` that needs a shell).
    fn shown(request: u32) -> SharingServicePicker {
        let table = table();
        let items = [Payload::Text("hi".into())];
        let mut picker = SharingServicePicker::new(&items, &candidates(&table));
        picker.begin(request);
        picker
    }

    #[test]
    fn a_chosen_service_reports_did_choose_then_did_share_items() {
        let mut picker = shown(7);
        let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
        assert!(picker.share_done(7, Some(COPY), &mut chooser, &mut sharer));
        assert_eq!(chooser.chose, vec![Some(COPY.to_string())]);
        assert_eq!(sharer.shared, 1);
        assert!(sharer.failed.is_empty());
    }

    #[test]
    fn a_dismissed_menu_reports_did_choose_nil_and_shares_nothing() {
        let mut picker = shown(8);
        let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
        assert!(picker.share_done(8, None, &mut chooser, &mut sharer));
        assert_eq!(chooser.chose, vec![None]);
        assert_eq!(sharer.shared, 0);
        assert!(sharer.failed.is_empty());
    }

    #[test]
    fn a_target_the_picker_did_not_offer_is_a_failed_share() {
        let mut picker = shown(9);
        let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
        assert!(picker.share_done(9, Some("ghost"), &mut chooser, &mut sharer));
        assert_eq!(chooser.chose, vec![None]);
        assert_eq!(sharer.shared, 0);
        assert_eq!(sharer.failed, vec![ShareError::NotOffered("ghost".into())]);
    }

    #[test]
    fn answers_for_other_requests_or_answered_ones_are_ignored() {
        let mut picker = shown(10);
        let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
        assert!(!picker.share_done(11, Some(COPY), &mut chooser, &mut sharer), "another request");
        assert!(picker.share_done(10, Some(COPY), &mut chooser, &mut sharer));
        assert!(!picker.share_done(10, Some(COPY), &mut chooser, &mut sharer), "answered once");
        assert_eq!(chooser.chose.len(), 1);
        assert_eq!(sharer.shared, 1);
    }
}
