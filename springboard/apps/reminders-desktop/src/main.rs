//! Reminders as an installed BifrOSt app: a full-size Golden Gate window (sidebar of smart lists and My Lists,
//! unified toolbar), hosted through the SDK. Text input comes from the hardware keyboard.

mod alerts;
mod app;
mod archive;
mod attach;
mod model;
mod organize;
mod quick;
mod format;
// the location watch is not called by the window yet (the form has no place fields), so its items are unused for now
#[allow(dead_code)]
mod place;
mod repeat;
mod scripting;
mod search;
mod smartdef;
mod settings;
mod store;
mod system;
mod zones;

use bifrost_app::app_events::AppEvent;
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::share::INBOUND;
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use bifrost_app::status_item::{MenuItem as StatusMenuItem, StatusBar, StatusItem, StatusItemLength};
use bifrost_app::{dock_tile, notification_schedule, widgets_timeline};
use bifrost_core::json::Json;
use std::path::Path;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;

/// `AppMenuItem` ids for `menu_action` (0 is reserved for separators).
const MENU_NEW: u32 = 1;
const MENU_NEW_LIST: u32 = 2;
const MENU_DELETE: u32 = 3;
const MENU_FIND: u32 = 4;
const MENU_COMPLETE: u32 = 5;
const MENU_INDENT: u32 = 6;
const MENU_OUTDENT: u32 = 7;
const MENU_OVERDUE: u32 = 8;
const MENU_PIN: u32 = 9;
const MENU_NEW_SECTION: u32 = 10;
const MENU_AUTOCATEGORIZE: u32 = 11;
const MENU_NEW_GROUP: u32 = 12;
const MENU_SAVE_TEMPLATE: u32 = 13;
const MENU_IMPORT: u32 = 14;
const MENU_PRINT: u32 = 15;
const MENU_SUBTASKS: u32 = 16;
const MENU_NO_SECTION: u32 = 17;
const MENU_NEW_SMART: u32 = 18;
const MENU_SETTINGS: u32 = 19;
const MENU_URGENT: u32 = 24;
const MENU_SAVE_AS: u32 = 25;
const MENU_BOLD: u32 = 26;
const MENU_BULLETS: u32 = 27;
const MENU_NUMBERS: u32 = 28;
/// Item ids of the menus that list things: the base plus the position in the list.
const MENU_MARK_BASE: u32 = 20;
const MENU_SORT_BASE: u32 = 30;
const MENU_SMART_BASE: u32 = 40;
const MENU_SECTION_BASE: u32 = 100;
const MENU_GROUP_BASE: u32 = 200;
const MENU_TEMPLATE_NEW_BASE: u32 = 300;
const MENU_TEMPLATE_DELETE_BASE: u32 = 400;
/// How many sections, groups and templates a submenu lists.
const SHOWN: usize = 10;
/// The menu bar extra's entries (their own id space, from `status_item_action`).
const QUICK_NEW: u32 = 1;
const QUICK_CLIPBOARD: u32 = 2;
/// The largest checklist file an import reads, in bytes.
const IMPORT_MAX: u64 = 4 << 20;

struct RemindersApp {
    app: Option<app::Reminders>,
    kb: bool,
    alerts: alerts::AlertState,
    wall: Wall,
    /// The count last sent as the dock badge, and the badge request still waiting for its reply (with its label, for
    /// the fallback to the older badge message).
    badge: Option<usize>,
    badge_req: Option<(u32, String)>,
    /// Whether the shell has the `notification-schedule` service: `None` until the probe answers. Alerts wait for it.
    schedule: Option<bool>,
    /// The request id of the probe for `notification-schedule`.
    probe: Option<u32>,
    /// The store generation the Today widget timeline was last sent for.
    widget_gen: Option<u32>,
    /// Whether the app is currently opted into sudden termination (it starts disabled, as the count does).
    terminable: bool,
    /// The open panel asked for by File > Import List.
    import: Option<u32>,
    /// The Quick Reminder menu bar extra.
    quick: Option<StatusItem>,
    /// The open panel asked for by a reminder form's Add Image.
    pick: Option<u32>,
    /// The reminders as the Core Search index holds them (see `search.rs`), and the store generation they were read at.
    indexed: std::collections::HashMap<u64, String>,
    indexed_gen: Option<u32>,
    /// The save panel asked for by File > Save List As, with the checklist it will write.
    save: Option<(u32, String)>,
}

/// The menu bar extra: New Quick Reminder opens the new reminder form (the title takes the quick-add words, such as
/// "call mum tomorrow 9am"); Add Reminder from Clipboard turns the copied text into a reminder at once.
fn quick_reminder_item(ctx: &Ctx) -> StatusItem {
    let item = StatusBar::system().status_item(StatusItemLength::Variable);
    item.set_symbol(ctx, "checklist");
    item.set_menu(ctx, &[StatusMenuItem::new(QUICK_NEW, "New Quick Reminder"), StatusMenuItem::new(QUICK_CLIPBOARD, "Add Reminder from Clipboard")]);
    item
}

impl RemindersApp {
    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let on = self.app.as_ref().is_some_and(|a| a.keyboard_shown());
        if on != self.kb {
            self.kb = on;
            ctx.wants_keyboard(on);
        }
        self.sync_termination(ctx);
        // the reminder form asked for a picture: the open panel answers in `document`
        if self.app.as_mut().is_some_and(|a| a.take_image_request()) {
            self.pick = Some(ctx.open_document(DocMode::Read, "Add Image", &["png", "jpg", "jpeg"]));
        }
    }

    /// Keeps the Core Search index in step with the reminders: runs when the store generation changed.
    fn sync_index(&mut self, ctx: &Ctx) {
        let Some(a) = &self.app else { return };
        if self.indexed_gen == Some(a.generation()) {
            return;
        }
        self.indexed_gen = Some(a.generation());
        let (changed, gone) = search::sync(&mut self.indexed, a.items());
        let index = bifrost_app::quicksearch::CSSearchableIndex::default_index();
        if !changed.is_empty() {
            index.index_searchable_items(ctx, &changed);
        }
        if !gone.is_empty() {
            let refs: Vec<&str> = gone.iter().map(String::as_str).collect();
            index.delete_searchable_items(ctx, &refs);
        }
    }

    /// Every change is saved as it is made, so the app may be ended at once (sudden termination) except while an
    /// editor holds a draft that is not saved yet.
    fn sync_termination(&mut self, ctx: &Ctx) {
        let terminable = self.app.as_ref().is_some_and(|a| !a.has_draft());
        if terminable != self.terminable {
            self.terminable = terminable;
            if terminable {
                ctx.enable_sudden_termination();
            } else {
                ctx.disable_sudden_termination();
            }
        }
    }
}

impl RemindersApp {
    /// The AppEvents Reminders answers (`scripting`): `make new reminder` and `make new list`, which go through the
    /// Add Reminder and Create List intents, and the core `getd` read of a reminder property. Setting properties and
    /// deleting reminders from a script are refused for now.
    fn script_event(&mut self, ctx: Option<&Ctx>, data_dir: &Path, ev: &AppEvent) -> Result<Json, RpcError> {
        match (ev.class.as_str(), ev.id.as_str()) {
            ("reminders", "make") => {
                let params = scripting::reminder_params(&ev.params)?;
                Ok(self.perform_intent(ctx, data_dir, "add-reminder", &params)?.to_json())
            }
            ("reminders", "makelist") => {
                let params = scripting::list_params(&ev.params)?;
                Ok(self.perform_intent(ctx, data_dir, "create-list", &params)?.to_json())
            }
            ("core", "getd") => {
                let object = ev.params.get("object").cloned().ok_or_else(|| RpcError::new(INVALID_PARAMS, "getd needs an object"))?;
                let property = object.str_field("property");
                let Some(a) = self.app.as_ref() else { return Err(RpcError::new(INVALID_PARAMS, "Reminders is not running yet")) };
                let rows = scripting::rows(a.items(), a.lists());
                scripting::get_property(&rows, &object, property.as_deref())
            }
            ("core", "setd") | ("core", "delt") => {
                Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, "Reminders cannot change reminders from a script yet"))
            }
            _ => Err(scripting::unhandled(&ev.method())),
        }
    }
}

impl App for RemindersApp {
    fn id(&self) -> &str {
        "is.olibuijr.reminders.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.app {
            Some(a) => {
                a.set_size(w, h);
                a.set_top_inset(bifrost_app::layout::Layout::new(&ctx.cfg).top);
            }
            None => {
                self.wall = Wall::sample(bifrost_proto::sys::now_ms());
                let mut r = app::Reminders::new(&ctx.ui, w, h, ctx.data_dir.join("reminders.tsv"), self.wall)
                    .with_tips(tip_store(&ctx.data_dir));
                r.set_top_inset(bifrost_app::layout::Layout::new(&ctx.cfg).top);
                r.enable_file_export();
                self.app = Some(r);
                self.quick = Some(quick_reminder_item(ctx));
            }
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(a) = &self.app else { return Vec::new() };
        let on_items = a.on_items();
        let selected = on_items && a.selected_item().is_some();
        let list = a.open_list().is_some();
        // Reminder and List are the app's own menus. File, Edit, View, Window and Help are the shell's; an app menu
        // with one of those titles adds its items to the shell's menu, so File and View below hold only Reminders'.
        let mut section_items = vec![AppMenuItem::submenu("Move to Section").enabled(a.can_move_to_section())];
        if a.can_move_to_section() {
            section_items.push(AppMenuItem::new(MENU_NO_SECTION, "No Section").checked(a.selected_section().is_none()).depth(1));
            for (i, (id, name)) in a.open_list_sections().into_iter().take(SHOWN).enumerate() {
                section_items.push(AppMenuItem::new(MENU_SECTION_BASE + i as u32, &name).checked(a.selected_section() == Some(id)).depth(1));
            }
        }
        let mut mark = vec![AppMenuItem::submenu("Mark Due Date As").enabled(selected)];
        for (i, m) in organize::MarkDue::ALL.iter().enumerate() {
            mark.push(AppMenuItem::new(MENU_MARK_BASE + i as u32, m.label()).enabled(selected).depth(1));
        }
        let mut reminder = vec![
            AppMenuItem::new(MENU_NEW, "New Reminder").key("cmd+n").enabled(on_items),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_DELETE, "Delete Reminder").key("cmd+backspace").enabled(selected),
            AppMenuItem::new(MENU_COMPLETE, "Mark as Completed").enabled(selected),
            AppMenuItem::new(MENU_URGENT, if a.selected_urgent() { "Remove Urgent Mark" } else { "Mark as Urgent" }).enabled(selected),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_INDENT, "Indent").key("cmd+]").enabled(a.can_indent()),
            AppMenuItem::new(MENU_OUTDENT, "Outdent").key("cmd+[").enabled(a.can_outdent()),
        ];
        reminder.extend(section_items);
        reminder.push(AppMenuItem::separator());
        reminder.extend(mark);
        reminder.push(AppMenuItem::new(MENU_OVERDUE, "Mark All Overdue as Today").enabled(on_items && a.has_overdue()));
        reminder.push(AppMenuItem::separator());
        reminder.push(AppMenuItem::new(MENU_FIND, "Find").key("cmd+f").enabled(on_items));
        reminder.push(AppMenuItem::separator());
        reminder.push(AppMenuItem::new(MENU_SETTINGS, "Settings...").key("cmd+,").enabled(on_items));

        let groups = a.groups();
        let mut edit_group = vec![AppMenuItem::submenu("Edit Group").enabled(!groups.is_empty() && on_items)];
        for (i, (_, name)) in groups.iter().take(SHOWN).enumerate() {
            edit_group.push(AppMenuItem::new(MENU_GROUP_BASE + i as u32, name).depth(1));
        }
        let templates = a.templates();
        let mut from_template = vec![AppMenuItem::submenu("New List from Template").enabled(!templates.is_empty() && on_items)];
        let mut delete_template = vec![AppMenuItem::submenu("Delete Template").enabled(!templates.is_empty() && on_items)];
        for (i, (_, name)) in templates.iter().take(SHOWN).enumerate() {
            from_template.push(AppMenuItem::new(MENU_TEMPLATE_NEW_BASE + i as u32, name).depth(1));
            delete_template.push(AppMenuItem::new(MENU_TEMPLATE_DELETE_BASE + i as u32, name).depth(1));
        }
        let mut lists = vec![
            AppMenuItem::new(MENU_NEW_LIST, "New List").key("cmd+shift+n").enabled(on_items),
            AppMenuItem::new(MENU_NEW_GROUP, "New List Group").enabled(on_items),
        ];
        lists.extend(edit_group);
        lists.push(AppMenuItem::separator());
        lists.push(AppMenuItem::new(MENU_PIN, if a.open_list_pinned() { "Unpin List" } else { "Pin List" }).enabled(list));
        lists.push(AppMenuItem::new(MENU_NEW_SECTION, "New Section").enabled(list));
        lists.push(AppMenuItem::new(MENU_AUTOCATEGORIZE, "Auto-Categorize").enabled(list));
        lists.push(AppMenuItem::separator());
        lists.push(AppMenuItem::new(MENU_SAVE_TEMPLATE, "Save List as Template").enabled(list));
        lists.extend(from_template);
        lists.extend(delete_template);

        let mut sort_by = vec![AppMenuItem::submenu("Sort By")];
        for (i, sort) in model::Sort::ALL.iter().enumerate() {
            sort_by.push(AppMenuItem::new(MENU_SORT_BASE + i as u32, sort.label()).checked(a.sort() == *sort).depth(1));
        }
        let mut smart_lists = vec![AppMenuItem::submenu("Show Smart List")];
        for (i, smart) in model::Smart::ALL.iter().enumerate() {
            smart_lists.push(AppMenuItem::new(MENU_SMART_BASE + i as u32, smart.label()).checked(a.smart_shown(*smart)).depth(1));
        }
        let mut view = sort_by;
        view.extend(smart_lists);
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::new(MENU_SUBTASKS, if a.subtasks_hidden() { "Show All Subtasks" } else { "Hide All Subtasks" }).enabled(a.has_subtasks()));
        vec![
            AppMenu { title: "Reminder".into(), items: reminder },
            AppMenu { title: "List".into(), items: lists },
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_NEW_SMART, "New Smart List...").enabled(on_items),
                    AppMenuItem::new(MENU_IMPORT, "Import List...").key("cmd+o").enabled(on_items),
                    AppMenuItem::new(MENU_SAVE_AS, "Save List As...").enabled(list),
                    AppMenuItem::new(MENU_PRINT, "Print List...").key("cmd+p").enabled(a.can_print()),
                ],
            },
            AppMenu { title: "View".into(), items: view },
            AppMenu {
                title: "Format".into(),
                items: vec![
                    AppMenuItem::new(MENU_BOLD, "Bold").key("cmd+b").enabled(a.notes_focused()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_BULLETS, "Bullet List").enabled(a.notes_focused()),
                    AppMenuItem::new(MENU_NUMBERS, "Numbered List").enabled(a.notes_focused()),
                ],
            },
        ]
    }

    /// A chosen entry of the menu bar extra.
    fn status_item_action(&mut self, ctx: &Ctx, item: u32, id: u32) -> bool {
        if self.quick.as_ref().map(StatusItem::id) != Some(item) {
            return false;
        }
        let t = bifrost_proto::sys::now_ms();
        let Some(a) = &mut self.app else { return false };
        let handled = match id {
            QUICK_NEW => {
                a.new_reminder(&ctx.ui, t);
                true
            }
            QUICK_CLIPBOARD => {
                let text = bifrost_app::kit::clipboard::text().unwrap_or_default();
                match a.quick_add(&ctx.ui, &text, t) {
                    Ok(_) => true,
                    Err(e) => {
                        ctx.log(&format!("quick reminder: {e}"));
                        false
                    }
                }
            }
            _ => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let t = bifrost_proto::sys::now_ms();
        if id == MENU_IMPORT {
            if self.app.as_ref().is_some_and(|a| a.on_items()) {
                self.import = Some(ctx.open_document(DocMode::Read, "Import List", &["public.text"]));
            }
            return false;
        }
        if id == MENU_SAVE_AS {
            if let Some((name, text)) = self.app.as_ref().and_then(|a| a.list_export()) {
                let request = ctx.save_document(&format!("{name}.md"), &["md", "txt"]);
                self.save = Some((request, text));
            }
            return false;
        }
        let Some(a) = &mut self.app else { return false };
        let ui = &ctx.ui;
        let handled = match id {
            MENU_NEW => {
                a.new_reminder(ui, t);
                true
            }
            MENU_NEW_LIST => {
                a.new_list(ui, t);
                true
            }
            MENU_DELETE => a.delete_selected(ui, t),
            MENU_FIND => {
                a.find(ui, t);
                true
            }
            MENU_COMPLETE => a.toggle_selected(ui),
            MENU_INDENT => a.indent_selected(ui),
            MENU_OUTDENT => a.outdent_selected(ui),
            MENU_OVERDUE => a.mark_overdue_today(ui),
            MENU_PIN => a.toggle_pin(),
            MENU_NEW_SECTION => a.new_section(ui, t),
            MENU_AUTOCATEGORIZE => a.auto_categorize(ui),
            MENU_NEW_GROUP => {
                a.new_group(ui, t);
                true
            }
            MENU_SAVE_TEMPLATE => a.save_as_template(),
            MENU_NEW_SMART => a.new_smart_list(ui, t),
            MENU_SETTINGS => a.open_settings(t),
            MENU_URGENT => a.toggle_urgent_selected(ui),
            MENU_BOLD => a.format_notes(ui, format::Style::Bold),
            MENU_BULLETS => a.format_notes(ui, format::Style::Bullets),
            MENU_NUMBERS => a.format_notes(ui, format::Style::Numbers),
            MENU_SUBTASKS => {
                a.toggle_all_subtasks(ui);
                true
            }
            MENU_NO_SECTION => a.move_selected_to_section(ui, None),
            MENU_PRINT => {
                if let Some((name, text)) = a.printable_list() {
                    // the job goes to CUPS on its own thread, so the window keeps drawing while the scheduler answers
                    let job = bifrost_app::print::PrintOperation::text(name, text);
                    bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                        if let Err(e) = job.run() {
                            eprintln!("Reminders print: {e:?}");
                        }
                    });
                }
                false
            }
            n if (MENU_MARK_BASE..MENU_MARK_BASE + organize::MarkDue::ALL.len() as u32).contains(&n) => {
                a.mark_selected_due(ui, organize::MarkDue::ALL[(n - MENU_MARK_BASE) as usize])
            }
            n if (MENU_SORT_BASE..MENU_SORT_BASE + model::Sort::ALL.len() as u32).contains(&n) => {
                a.set_sort(ui, model::Sort::ALL[(n - MENU_SORT_BASE) as usize]);
                true
            }
            n if (MENU_SMART_BASE..MENU_SMART_BASE + model::Smart::ALL.len() as u32).contains(&n) => {
                a.toggle_smart(model::Smart::ALL[(n - MENU_SMART_BASE) as usize]);
                true
            }
            n if (MENU_SECTION_BASE..MENU_SECTION_BASE + SHOWN as u32).contains(&n) => {
                let section = a.open_list_sections().get((n - MENU_SECTION_BASE) as usize).map(|(id, _)| *id);
                section.is_some_and(|section| a.move_selected_to_section(ui, Some(section)))
            }
            n if (MENU_GROUP_BASE..MENU_GROUP_BASE + SHOWN as u32).contains(&n) => {
                if let Some((group, _)) = a.groups().get((n - MENU_GROUP_BASE) as usize) {
                    a.edit_group(ui, *group, t);
                }
                true
            }
            n if (MENU_TEMPLATE_NEW_BASE..MENU_TEMPLATE_NEW_BASE + SHOWN as u32).contains(&n) => {
                let template = a.templates().get((n - MENU_TEMPLATE_NEW_BASE) as usize).map(|(id, _)| *id);
                template.is_some_and(|template| a.new_list_from_template(ui, template))
            }
            n if (MENU_TEMPLATE_DELETE_BASE..MENU_TEMPLATE_DELETE_BASE + SHOWN as u32).contains(&n) => {
                let template = a.templates().get((n - MENU_TEMPLATE_DELETE_BASE) as usize).map(|(id, _)| *id);
                template.is_some_and(|template| a.delete_template(template))
            }
            _ => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn intents(&self) -> Vec<Intent> {
        vec![
            Intent::new("add-reminder", "Add Reminder", "Adds a reminder to the default list, with an optional due date.")
                .param(Param::new("title", "Title", ParamType::String))
                .param(Param::new("due", "Due", ParamType::Date).optional()),
            Intent::new("create-list-item", "Create List Item", "Adds a reminder to the named list, with an optional due date.")
                .param(Param::new("title", "Title", ParamType::String))
                .param(Param::new("list", "List", ParamType::String))
                .param(Param::new("due", "Due", ParamType::Date).optional()),
            Intent::new("quick-reminder", "Add Quick Reminder", "Adds a reminder from a line such as \"call mum tomorrow 9am\".")
                .param(Param::new("text", "Text", ParamType::String)),
            Intent::new("create-list", "Create List", "Creates a new list.").param(Param::new("name", "Name", ParamType::String)),
            Intent::new("open-list", "Open List", "Shows a list in Reminders.").param(Param::new("list", "List", ParamType::String)).foreground(),
            Intent::new("open-smart-list", "Open Smart List", "Shows Today, Scheduled, Flagged, All or Completed in Reminders.")
                .param(Param::new("smart", "Smart List", ParamType::Enum(model::Smart::ALL.iter().map(|s| s.label().to_string()).collect())))
                .foreground(),
            Intent::new("search-reminders", "Search Reminders", "Searches the reminders and shows the results.")
                .param(Param::new("query", "Search", ParamType::String))
                .foreground(),
        ]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if !matches!(id, "add-reminder" | "create-list-item" | "quick-reminder" | "create-list" | "open-list" | "open-smart-list" | "search-reminders") {
            return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
        }
        let (Some(ctx), Some(a)) = (ctx, self.app.as_mut()) else { return Err(intents::failed("Reminders is not running yet")) };
        let ui = &ctx.ui;
        let result = match id {
            "add-reminder" => {
                let title = intents::str_param(params, "title")?;
                let due = params.get("due").and_then(Json::as_str);
                let id = a.add_reminder(ui, title, due).map_err(intents::failed)?;
                IntentResult::value(Json::Num(id as f64)).with_dialog(&format!("Added \"{title}\""))
            }
            "create-list-item" => {
                let title = intents::str_param(params, "title")?;
                let list = intents::str_param(params, "list")?;
                let due = params.get("due").and_then(Json::as_str);
                let id = a.add_reminder_to_named(ui, title, due, list).map_err(intents::failed)?;
                IntentResult::value(Json::Num(id as f64)).with_dialog(&format!("Added \"{title}\" to {}", list.trim()))
            }
            "quick-reminder" => {
                let text = intents::str_param(params, "text")?;
                let id = a.quick_add(ui, text, bifrost_proto::sys::now_ms()).map_err(intents::failed)?;
                IntentResult::value(Json::Num(id as f64)).with_dialog("Added the reminder")
            }
            "create-list" => {
                let name = intents::str_param(params, "name")?;
                let id = a.create_list(ui, name).map_err(intents::failed)?;
                IntentResult::value(Json::Num(id as f64)).with_dialog(&format!("Created the list \"{}\"", name.trim()))
            }
            "open-list" => {
                let name = a.open_list_named(ui, intents::str_param(params, "list")?).map_err(intents::failed)?;
                IntentResult::value(Json::Str(name.clone())).with_dialog(&format!("Opened \"{name}\"")).continue_in_foreground()
            }
            "open-smart-list" => {
                let smart = a.open_smart_named(ui, intents::str_param(params, "smart")?).map_err(intents::failed)?;
                IntentResult::value(Json::Str(smart.label().to_string())).with_dialog(&format!("Opened {}", smart.label())).continue_in_foreground()
            }
            _ => {
                let query = intents::str_param(params, "query")?;
                let found = a.search_for(ui, query, bifrost_proto::sys::now_ms()).map_err(intents::failed)?;
                let noun = if found == 1 { "reminder" } else { "reminders" };
                IntentResult::value(Json::Num(found as f64)).with_dialog(&format!("Found {found} {noun}")).continue_in_foreground()
            }
        };
        self.sync_keyboard(ctx);
        Ok(result)
    }

    fn rpc(&mut self, ctx: Option<&Ctx>, data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        // scripting: `tell app "Reminders"` commands arrive as AppEvents (see `scripting` and the manifest's [[script.*]])
        match AppEvent::from_method(method, params) {
            Some(ev) => self.script_event(ctx, data_dir, &ev),
            None => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.app.as_ref().map_or_else(Vec::new, |a| a.controls(&ctx.ui))
    }

    /// Edit > Undo and Redo are the shell's items; they are enabled from the Reminders history.
    fn commands(&self, _ctx: &Ctx) -> Commands {
        self.app.as_ref().map_or_else(Commands::default, |a| a.edit_commands())
    }

    fn undo_titles(&self, _ctx: &Ctx) -> (String, String) {
        self.app.as_ref().map(|a| a.edit_titles()).unwrap_or_default()
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(a) = &mut self.app else { return false };
        let handled = match ev {
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = a.controls(&ctx.ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
                let (x, y) = (ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0), ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0));
                let t = bifrost_proto::sys::now_ms();
                a.touch(&ctx.ui, Event::Down(x, y), t);
                a.touch(&ctx.ui, Event::Up(x, y), t);
                if let Some((mx, my, items)) = a.take_menu_request() {
                    ctx.context_menu(mx, my, items);
                }
                true
            }
            Input::Touch { kind, x, y, t, .. } => {
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                a.touch(&ctx.ui, e, t);
                // the toolbar's "..." asks for the list's menu
                if let Some((mx, my, items)) = a.take_menu_request() {
                    ctx.context_menu(mx, my, items);
                }
                true
            }
            // a right click on a list in the sidebar opens its menu; the choice comes back as a ContextAction
            Input::Context { x, y } => match a.context_menu_at(&ctx.ui, x, y) {
                Some(items) => {
                    ctx.context_menu(x, y, items);
                    true
                }
                None => false,
            },
            Input::ContextAction(id) => a.context_action(&ctx.ui, id, bifrost_proto::sys::now_ms()),
            Input::Key(k) => {
                if let Some((key, mods)) = k.as_chord() {
                    a.chord(&ctx.ui, key, mods, bifrost_proto::sys::now_ms());
                    self.sync_keyboard(ctx);
                    return true;
                }
                if k == KeyInput::Code(1) {
                    let used = a.escape(&ctx.ui, bifrost_proto::sys::now_ms());
                    self.sync_keyboard(ctx);
                    return used;
                }
                let out = match k {
                    KeyInput::Text(s) => Some(Out::Text(s)),
                    KeyInput::Code(8) => Some(Out::Backspace),
                    KeyInput::Code(13) => Some(Out::Enter),
                    KeyInput::Code(37) => Some(Out::Named("Left")),
                    KeyInput::Code(38) => Some(Out::Named("Up")),
                    KeyInput::Code(39) => Some(Out::Named("Right")),
                    KeyInput::Code(40) => Some(Out::Named("Down")),
                    KeyInput::Code(_) => None,
                };
                if let Some(out) = out {
                    a.key(&ctx.ui, out, bifrost_proto::sys::now_ms());
                }
                true
            }
            // Edit > Undo and Redo (the shell's Edit menu, Cmd+Z and Cmd+Shift+Z) step the Reminders history.
            Input::Command(Command::Undo) => a.step_history(&ctx.ui, false),
            Input::Command(Command::Redo) => a.step_history(&ctx.ui, true),
            Input::Open(_) => false,
            _ => false,
        };
        self.sync_keyboard(ctx);
        handled
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        use std::io::{Read, Write};
        match ev {
            // File > Save List As: the chosen file gets the checklist, and the commit makes it the saved file
            DocumentEvent::SaveTo(mut doc) => {
                let Some((_, text)) = self.save.take_if(|(r, _)| *r == doc.request) else {
                    ctx.abort_document(doc.request);
                    return false;
                };
                match doc.file.write_all(text.as_bytes()) {
                    Ok(()) => ctx.commit_document(doc.request),
                    Err(e) => {
                        ctx.abort_document(doc.request);
                        ctx.log(&format!("save list: {e}"));
                    }
                }
                false
            }
            // the app keeps no grant to a file it only wrote
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                false
            }
            // a checklist chosen in File > Import List, or a Markdown file shared or opened from Files (INBOUND)
            DocumentEvent::Opened(mut doc) if self.import == Some(doc.request) || doc.request & INBOUND != 0 => {
                if self.import == Some(doc.request) {
                    self.import = None;
                }
                if doc.size > IMPORT_MAX {
                    ctx.log(&format!("import: {} is too large ({} bytes)", doc.name, doc.size));
                    ctx.release_document(doc.grant);
                    return false;
                }
                let mut bytes = Vec::new();
                if let Err(e) = (&mut doc.file).take(IMPORT_MAX).read_to_end(&mut bytes) {
                    ctx.log(&format!("import: {e}"));
                    ctx.release_document(doc.grant);
                    return false;
                }
                // the list is named by the heading, or by the file's name without its extension
                let fallback = Path::new(&doc.name).file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                let imported = self.app.as_mut().is_some_and(|a| a.import_checklist(&ctx.ui, &String::from_utf8_lossy(&bytes), &fallback));
                // one import is one use of the file: the app keeps no grant to it
                ctx.release_document(doc.grant);
                imported
            }
            DocumentEvent::Opened(mut doc) if self.pick == Some(doc.request) => {
                self.pick = None;
                let mut bytes = Vec::new();
                let read = if doc.size > attach::MAX_BYTES as u64 { Err("the picture is too large".to_string()) } else { (&mut doc.file).take(attach::MAX_BYTES as u64).read_to_end(&mut bytes).map_err(|e| e.to_string()) };
                ctx.release_document(doc.grant);
                match read.and_then(|_| self.app.as_mut().map_or(Err("Reminders is not ready".to_string()), |a| a.attach_image(&bytes))) {
                    Ok(()) => true,
                    Err(e) => {
                        ctx.log(&format!("attach image: {e}"));
                        false
                    }
                }
            }
            DocumentEvent::Failed { request, code } => {
                if self.import == Some(request) {
                    self.import = None;
                }
                if self.pick == Some(request) {
                    self.pick = None;
                }
                if self.save.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.save = None;
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document request {request}: {code:?}"));
                }
                false
            }
            _ => false,
        }
    }

    fn continue_activity(&mut self, _ctx: &Ctx, activity: bifrost_app::quicksearch::UserActivity) -> bool {
        let Ok(id) = activity.unique_identifier.parse::<u64>() else { return false };
        activity.activity_type == bifrost_app::quicksearch::ACTION_TYPE && self.app.as_mut().is_some_and(|a| a.show_item(id))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(a) = &mut self.app else { return false };
        self.wall = Wall::sample(t);
        a.set_wall(self.wall);
        let redraw = a.tick(&ctx.ui, t);
        let due = a.badge_count();
        if self.badge != Some(due) {
            self.badge = Some(due);
            // The Dock badge goes through the `dock` service; the reply falls back to the older message.
            let label = model::badge_label(due);
            let request = dock_tile::set_badge(ctx, &label);
            self.badge_req = Some((request, label));
        }
        let wall_offset = self.wall.offset;
        let offset_at = |t: i64| springboard::bare::timezone::offset_at(t).unwrap_or(wall_offset);
        let now = self.wall.now(t);
        // The Today widget is sent when the reminders change, and again when the shell asks for a reload.
        if self.widget_gen != Some(a.generation()) {
            self.widget_gen = Some(a.generation());
            system::reload_today(ctx, a.items(), a.today(), now, a.all_day_min(), &offset_at);
        }
        if self.schedule.is_none() && self.probe.is_none() {
            self.probe = Some(notification_schedule::pending(ctx));
        }
        let path = ctx.data_dir.join("reminders.tsv");
        // Alerts are shell triggers: only the plan changes are sent (see alerts.rs). They wait for the probe.
        if let Some(via_service) = self.schedule {
            let change = alerts::step(&mut self.alerts, now, a.generation(), a.all_day_min(), || store::load(&path).items, &offset_at);
            if let Some(change) = change {
                alerts::apply(ctx, &change, via_service);
            }
        }
        self.sync_index(ctx);
        self.sync_keyboard(ctx);
        redraw
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let a = self.app.as_ref()?;
        let alert = self.alerts.wake(self.wall.unix, self.wall.at);
        match (a.next_wake(t), alert) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.probe == Some(request) {
            self.probe = None;
            self.schedule = Some(result.is_ok());
            return true;
        }
        if let Some((sent, label)) = self.badge_req.take() {
            if sent == request {
                // An older shell has no `dock` service: the badge then goes through the message it has always had.
                if result.is_err() {
                    ctx.set_badge(&label);
                }
                return true;
            }
            self.badge_req = Some((sent, label));
        }
        false
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        // The shell asks for a new Today timeline when its reload date passes.
        let Some(widgets_timeline::Event::Reload { kind }) = widgets_timeline::parse_event(service, event, body) else { return false };
        if kind != system::WIDGET_KIND {
            return false;
        }
        let Some(a) = &self.app else { return false };
        let wall_offset = self.wall.offset;
        let offset_at = |t: i64| springboard::bare::timezone::offset_at(t).unwrap_or(wall_offset);
        system::reload_today(ctx, a.items(), a.today(), self.wall.unix, a.all_day_min(), &offset_at);
        self.widget_gen = Some(a.generation());
        false
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(a) = &mut self.app {
            a.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

/// The first-use tip state, in the app's data folder. When that file cannot be read the tips live in memory only.
fn tip_store(data_dir: &Path) -> bifrost_app::controls::tips::Datastore {
    use bifrost_app::controls::tips::{configure, ConfigOption};
    match configure(&[ConfigOption::DatastorePath(data_dir.join("tips.txt"))]) {
        Ok(store) => store,
        Err(_) => configure(&[]).expect("an in-memory tip store always loads"),
    }
}

fn main() {
    bifrost_app::run(RemindersApp { app: None, kb: false, alerts: alerts::AlertState::default(), wall: Wall::default(), badge: None, badge_req: None, schedule: None, probe: None, widget_gen: None, terminable: false, import: None, quick: None, pick: None, save: None, indexed: Default::default(), indexed_gen: None });
}
