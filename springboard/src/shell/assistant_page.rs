//! Settings > Assistant: the assistant's switch, the shortcut it answers to (switched in Keyboard Shortcuts), the
//! backend (Off by default), whether conversations are kept, and per-app access to intents. Every row saves on tap
//! through `apply`; the apps listed are the installed ones that declare `[[intent]]` tables.

use super::assistant::{Backend, Settings as AssistantSettings};
use super::list::{Group, Item, Spec};
use super::pages::{Action, PageId};
use super::prefs::Prefs;
use std::io;

/// One change the page makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// the assistant on or off
    Enabled(bool),
    /// the backend a question goes to
    Backend(Backend),
    /// conversations kept and sent with later questions
    History(bool),
    /// answers read aloud
    Speak(bool),
    /// the app `id` may (or may not) have its intents used
    App { id: String, on: bool },
}

/// The installed apps that declare intents, as (id, name), in install order.
pub fn intent_apps() -> Vec<(String, String)> {
    super::apphost::installed().iter().filter(|a| !a.manifest.intents.is_empty()).map(|a| (a.manifest.id.clone(), a.manifest.name.clone())).collect()
}

/// Saves one change to the assistant's prefs.
pub fn apply(prefs: &mut Prefs, edit: &Edit) -> io::Result<()> {
    let mut s = AssistantSettings::load(prefs);
    match edit {
        Edit::Enabled(on) => s.enabled = *on,
        Edit::Backend(b) => s.backend = *b,
        Edit::History(on) => s.history = *on,
        Edit::Speak(on) => s.speak = *on,
        Edit::App { id, on } => {
            if *on {
                s.allowed.insert(id.clone());
            } else {
                s.allowed.remove(id);
            }
        }
    }
    s.save(prefs)
}

/// Settings > Assistant as the saved settings and the apps with intents show it.
pub fn spec(s: &AssistantSettings, apps: &[(String, String)]) -> Spec {
    let mut groups = vec![
        Group::new(vec![Item::new("Assistant").switch(s.enabled, Action::Assistant(Edit::Enabled(!s.enabled)))])
            .footer("Cmd+Shift+A opens the assistant over the desktop. While it is off, that shortcut opens this page and nothing is sent."),
        Group::new(vec![Item::new("Shortcut").value(s.chord.label())]).footer("Switch the shortcut off or back on in Keyboard Shortcuts."),
        Group::new(vec![
            Item::new(Backend::Off.label()).check(s.backend == Backend::Off, Action::Assistant(Edit::Backend(Backend::Off))),
            Item::new(Backend::Anthropic.label()).check(s.backend == Backend::Anthropic, Action::Assistant(Edit::Backend(Backend::Anthropic))),
        ])
        .header("BACKEND")
        .footer("Anthropic API sends each question, and the actions of the apps you allow, to api.anthropic.com. Its key is read from the keychain item is.olibuijr.assistant."),
        Group::new(vec![Item::new("Keep Conversation History").switch(s.history, Action::Assistant(Edit::History(!s.history)))])
            .footer("Off: each question stands alone and nothing is kept. On: earlier turns are kept here and sent with new questions."),
        Group::new(vec![Item::new("Speak Answers").switch(s.speak, Action::Assistant(Edit::Speak(!s.speak)))])
            .footer("The answer is read aloud by the speech engine when it arrives. Nothing is read while the setting is off."),
    ];
    let rows: Vec<Item> = if apps.is_empty() {
        vec![Item::new("No installed app declares actions.")]
    } else {
        apps.iter()
            .map(|(id, name)| {
                let on = s.allowed.contains(id);
                Item::new(name.as_str()).switch(on, Action::Assistant(Edit::App { id: id.clone(), on: !on }))
            })
            .collect()
    };
    groups.push(Group::new(rows).header("ALLOW APP ACTIONS").footer("The assistant runs an app's action only after you confirm it on the card it shows."));
    Spec { id: PageId::Assistant, title: "Assistant".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::super::list::Accessory;
    use super::*;

    fn apps() -> Vec<(String, String)> {
        vec![("is.olibuijr.notes".into(), "Notes".into()), ("is.olibuijr.calendar".into(), "Calendar".into())]
    }

    fn switch_on(i: &Item) -> Option<bool> {
        match i.accessory {
            Accessory::Switch(on) => Some(on),
            _ => None,
        }
    }

    #[test]
    fn the_defaults_show_the_assistant_off_with_no_backend_and_no_history() {
        let s = spec(&AssistantSettings::default(), &apps());
        assert_eq!(s.id, PageId::Assistant);
        assert_eq!(s.title, "Assistant");
        assert_eq!(switch_on(&s.groups[0].items[0]), Some(false));
        assert_eq!(s.groups[1].items[0].value, "Cmd+Shift+A");
        let backend = &s.groups[2].items;
        assert_eq!(backend[0].label, "Off");
        assert_eq!(backend[0].accessory, Accessory::Check(true), "Off is the chosen backend");
        assert_eq!(backend[1].accessory, Accessory::Check(false));
        assert_eq!(switch_on(&s.groups[3].items[0]), Some(false), "history is off");
        assert_eq!(switch_on(&s.groups[4].items[0]), Some(false), "answers are not spoken");
    }

    #[test]
    fn each_installed_app_with_intents_gets_an_access_switch() {
        let allowed = AssistantSettings { allowed: ["is.olibuijr.calendar".to_string()].into_iter().collect(), ..AssistantSettings::default() };
        let s = spec(&allowed, &apps());
        let rows = &s.groups[5].items;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Notes");
        assert_eq!(switch_on(&rows[0]), Some(false));
        assert_eq!(rows[0].action, Action::Assistant(Edit::App { id: "is.olibuijr.notes".into(), on: true }));
        assert_eq!(switch_on(&rows[1]), Some(true));
        assert_eq!(rows[1].action, Action::Assistant(Edit::App { id: "is.olibuijr.calendar".into(), on: false }));
    }

    #[test]
    fn with_no_app_declaring_intents_the_page_says_so() {
        let s = spec(&AssistantSettings::default(), &[]);
        assert_eq!(s.groups[5].items[0].label, "No installed app declares actions.");
    }

    #[test]
    fn the_switches_carry_the_change_they_make() {
        let s = spec(&AssistantSettings { enabled: true, history: true, backend: Backend::Anthropic, ..AssistantSettings::default() }, &[]);
        assert_eq!(s.groups[0].items[0].action, Action::Assistant(Edit::Enabled(false)));
        assert_eq!(s.groups[2].items[1].action, Action::Assistant(Edit::Backend(Backend::Anthropic)));
        assert_eq!(s.groups[3].items[0].action, Action::Assistant(Edit::History(false)));
        assert_eq!(s.groups[4].items[0].action, Action::Assistant(Edit::Speak(true)));
    }

    #[test]
    fn changes_save_to_the_prefs_and_read_back() {
        let mut prefs = Prefs::memory();
        apply(&mut prefs, &Edit::Enabled(true)).unwrap();
        apply(&mut prefs, &Edit::Backend(Backend::Anthropic)).unwrap();
        apply(&mut prefs, &Edit::History(true)).unwrap();
        apply(&mut prefs, &Edit::Speak(true)).unwrap();
        apply(&mut prefs, &Edit::App { id: "is.olibuijr.notes".into(), on: true }).unwrap();
        let s = AssistantSettings::load(&prefs);
        assert!(s.enabled && s.history && s.speak);
        assert_eq!(s.backend, Backend::Anthropic);
        assert!(s.allowed.contains("is.olibuijr.notes"));
        apply(&mut prefs, &Edit::App { id: "is.olibuijr.notes".into(), on: false }).unwrap();
        assert!(!AssistantSettings::load(&prefs).allowed.contains("is.olibuijr.notes"), "revoking removes the access");
    }
}
