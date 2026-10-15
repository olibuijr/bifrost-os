//! Contacts' links to the desktop's shared services.
//!
//! - The pasteboard: "Copy Contact" puts one item on the general board with two representations, the plain text
//!   (name, company, phone, email, address, one per line) and the vCard 3.0 card. Text fields take the text; a
//!   receiving app that reads the vCard type gets the card.
//! - The search index: every contact is indexed under its name, with its company, phone numbers and emails as
//!   keywords, so the desktop search finds the person by any of them. The index is only sent to when a contact
//!   changes, and the first sync clears what an earlier run left behind.
//!
//! The values are built here and sent by `main.rs` on real events (a copy, a change to the list); nothing is sent
//! per frame.

use crate::exchange::{self, Version};
use crate::extras::{Extra, Extras};
use bifrost_app::pasteboard_types::{Board, Item, STRING};
use bifrost_app::quicksearch::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet};
use bifrost_app::Ctx;
use springboard::shell::contactstore::Contact;
use std::collections::{BTreeMap, BTreeSet};

/// The pasteboard type of a vCard representation.
pub const VCARD: &str = "public.vcard";
/// The search domain every contact is indexed under.
pub const DOMAIN: &str = "is.olibuijr.contacts.desktop";

/// The name a contact is known by in messages and in the index title.
pub fn label(c: &Contact) -> String {
    let name = c.display_name().trim();
    if c.name.trim().is_empty() && c.company.trim().is_empty() && c.phone.trim().is_empty() && c.email.trim().is_empty() {
        "contact".to_string()
    } else {
        name.to_string()
    }
}

/// The plain text of a contact: its non-empty fields, one per line, then its extra values.
pub fn plain_text(c: &Contact, extra: &[Extra]) -> String {
    let mut lines = vec![c.name.trim(), c.company.trim(), c.phone.trim(), c.email.trim(), c.address.trim()];
    lines.extend(extra.iter().map(|e| e.value.trim()));
    lines.retain(|line| !line.is_empty());
    lines.join("\n")
}

/// The pasteboard item of a contact: the plain text first, so a text field takes it, then the vCard.
pub fn contact_item(c: &Contact, extra: &[Extra]) -> Item {
    let mut values = Extras::default();
    for value in extra {
        values.add(c.id, value.clone());
    }
    Item::default()
        .with(STRING, plain_text(c, extra).as_bytes())
        .with(VCARD, exchange::export(std::slice::from_ref(c), &values, Version::V3).as_bytes())
}

/// Writes a contact, with its extra values, to the general board. The reply (write result) arrives in `App::service_reply`.
pub fn copy_contact(ctx: &Ctx, c: &Contact, extra: &[Extra]) -> u32 {
    Board::general().write(ctx, &[contact_item(c, extra)])
}

/// The status line after a copy: the reply decides it, so a missing pasteboard service is not reported as success.
pub fn copy_reply_text(label: &str, ok: bool) -> String {
    if ok {
        format!("Copied {label} as vCard and text")
    } else {
        format!("Could not copy {label}")
    }
}

/// The search index id of the contact `id`.
pub fn item_id(id: u64) -> String {
    format!("contact.{id}")
}

/// The searchable item of a contact: its label as the title, its phone numbers and emails as keywords.
pub fn search_item(c: &Contact) -> CSSearchableItem {
    let mut keywords = vec![c.name.trim(), c.company.trim(), c.phone.trim(), c.email.trim()];
    keywords.retain(|k| !k.is_empty());
    let mut description = vec![c.phone.trim(), c.email.trim()];
    description.retain(|d| !d.is_empty());
    let attrs = CSSearchableItemAttributeSet::new(&label(c)).keywords(&keywords).content_description(&description.join(", "));
    CSSearchableItem::new(&item_id(c.id), DOMAIN, attrs)
}

/// What a contact looked like when it was last sent to the index: the fields the item is built from.
fn fingerprint(c: &Contact) -> String {
    [c.name.as_str(), c.company.as_str(), c.phone.as_str(), c.email.as_str()].join("\u{1f}")
}

/// The index changes needed: the positions (in the contact list) to index, and the ids to delete.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub index: Vec<usize>,
    pub remove: Vec<u64>,
}

/// Compares the contacts with what was sent. A contact is indexed when it is new or its fields changed; a sent one
/// that is no longer in the list is removed.
pub fn plan(sent: &BTreeMap<u64, String>, contacts: &[Contact]) -> Plan {
    let index = contacts
        .iter()
        .enumerate()
        .filter(|(_, c)| sent.get(&c.id) != Some(&fingerprint(c)))
        .map(|(i, _)| i)
        .collect();
    let live: BTreeSet<u64> = contacts.iter().map(|c| c.id).collect();
    let remove = sent.keys().copied().filter(|id| !live.contains(id)).collect();
    Plan { index, remove }
}

/// The search index as this run has left it.
#[derive(Default)]
pub struct SearchIndex {
    /// The fingerprint of each contact sent to the index, by contact id.
    sent: BTreeMap<u64, String>,
    /// Whether the first sync has run (it clears items left by an earlier run).
    started: bool,
}

impl SearchIndex {
    /// Brings the index in line with `contacts`. The first sync clears the index and indexes every contact; later
    /// syncs send only what changed. Returns how many items were sent or deleted (0 sends nothing).
    pub fn sync(&mut self, ctx: &Ctx, contacts: &[Contact]) -> usize {
        let index = CSSearchableIndex::default_index();
        if !self.started {
            self.started = true;
            self.sent.clear();
            index.delete_all_searchable_items(ctx);
        }
        let plan = plan(&self.sent, contacts);
        let changes = plan.index.len() + plan.remove.len();
        if changes == 0 {
            return 0;
        }
        if !plan.index.is_empty() {
            let items: Vec<CSSearchableItem> = plan.index.iter().map(|&i| search_item(&contacts[i])).collect();
            // A refused item (an identifier over the wire cap) is not sent; contact ids are short, so none is.
            index.index_checked(ctx, &items);
            for &i in &plan.index {
                self.sent.insert(contacts[i].id, fingerprint(&contacts[i]));
            }
        }
        if !plan.remove.is_empty() {
            let ids: Vec<String> = plan.remove.iter().map(|&id| item_id(id)).collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            index.delete_searchable_items(ctx, &refs);
            for id in &plan.remove {
                self.sent.remove(id);
            }
        }
        changes
    }
}

/// Headless model tests: the pasteboard text, the search item and the index plan are built from values in memory.
#[cfg(test)]
mod headless_model_tests {
    use super::*;

    fn contact(id: u64, name: &str, company: &str, phone: &str, email: &str) -> Contact {
        Contact {
            id,
            name: name.into(),
            phone: phone.into(),
            email: email.into(),
            notes: String::new(),
            favorite: false,
            company: company.into(),
            address: String::new(),
            labels: [0; 3],
        }
    }

    #[test]
    fn the_plain_text_lists_the_non_empty_fields_one_per_line() {
        let c = contact(1, "  Ada Lovelace ", "Analytical", "555 1234", "");
        assert_eq!(plain_text(&c, &[]), "Ada Lovelace\nAnalytical\n555 1234");
    }

    #[test]
    fn an_unnamed_contact_is_labelled_contact() {
        assert_eq!(label(&contact(2, "  ", "", "", "")), "contact");
    }

    #[test]
    fn a_contact_is_indexed_under_its_item_id_with_its_fields_as_keywords() {
        let item = search_item(&contact(7, "Ada Lovelace", "Analytical", "555 1234", "ada@example.test"));
        assert_eq!(item.unique_identifier, item_id(7));
        assert_eq!(item.unique_identifier, "contact.7");
        assert_eq!(item.domain_identifier, DOMAIN);
        assert_eq!(item.attribute_set.keywords, ["Ada Lovelace", "Analytical", "555 1234", "ada@example.test"]);
        assert_eq!(item.attribute_set.content_description, "555 1234, ada@example.test");
    }

    #[test]
    fn the_index_plan_sends_new_and_changed_contacts_and_removes_the_gone_ones() {
        let mut sent = BTreeMap::new();
        assert_eq!(plan(&sent, &[contact(1, "Ada", "", "1", ""), contact(2, "Bo", "", "2", "")]), Plan { index: vec![0, 1], remove: vec![] });
        sent.insert(1, fingerprint(&contact(1, "Ada", "", "1", "")));
        sent.insert(9, fingerprint(&contact(9, "Gone", "", "", "")));
        // contact 1 changed its phone, contact 2 is new, contact 9 is no longer in the list
        assert_eq!(plan(&sent, &[contact(1, "Ada", "", "3", ""), contact(2, "Bo", "", "2", "")]), Plan { index: vec![0, 1], remove: vec![9] });
        sent.insert(1, fingerprint(&contact(1, "Ada", "", "3", "")));
        sent.insert(2, fingerprint(&contact(2, "Bo", "", "2", "")));
        sent.remove(&9);
        assert_eq!(plan(&sent, &[contact(1, "Ada", "", "3", ""), contact(2, "Bo", "", "2", "")]), Plan { index: vec![], remove: vec![] }, "nothing changed, nothing sent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::Font;
    use bifrost_app::proto::{Configure, ToShell};
    use bifrost_app::Ui;

    fn test_ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn anna() -> Contact {
        Contact {
            id: 1,
            name: "Anna Ásgeirsdóttir".into(),
            phone: "+354 555 1234".into(),
            email: "anna@example.is".into(),
            address: "Laugavegur 1, Reykjavík".into(),
            ..Default::default()
        }
    }

    fn bo() -> Contact {
        Contact { id: 2, name: "Bo Berg".into(), email: "bo@example.is".into(), ..Default::default() }
    }

    #[test]
    fn plain_text_lists_the_filled_fields_one_per_line() {
        assert_eq!(plain_text(&anna(), &[]), "Anna Ásgeirsdóttir\n+354 555 1234\nanna@example.is\nLaugavegur 1, Reykjavík");
        assert_eq!(plain_text(&bo(), &[]), "Bo Berg\nbo@example.is");
    }

    #[test]
    fn the_plain_text_and_the_vcard_carry_the_extra_values() {
        use crate::extras::ValueKind;
        let extra = [Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0199".into() }];
        assert_eq!(plain_text(&bo(), &extra), "Bo Berg\nbo@example.is\n555 0199");
    }

    #[test]
    fn a_contact_item_has_the_text_first_and_then_the_vcard() {
        let item = contact_item(&anna(), &[]);
        let utis: Vec<&str> = item.representations.iter().map(|r| r.uti.as_str()).collect();
        assert_eq!(utis, [STRING, VCARD]);
        let card = String::from_utf8(item.data(VCARD).unwrap().to_vec()).unwrap();
        assert!(card.starts_with("BEGIN:VCARD"), "{card}");
        assert!(card.contains("END:VCARD"), "{card}");
    }

    #[test]
    fn copy_contact_writes_the_item_to_the_general_board() {
        let ctx = test_ctx();
        let request = copy_contact(&ctx, &anna(), &[]);
        let sent = ctx.take_sent();
        let [ToShell::ServiceCall { request: sent_request, service, method, body }] = sent.as_slice() else {
            panic!("expected one service call, got {sent:?}")
        };
        assert_eq!(*sent_request, request);
        assert_eq!((service.as_str(), method.as_str()), ("pasteboard", "write"));
        assert!(body.contains("\"general\""), "{body}");
        assert!(body.contains(STRING) && body.contains(VCARD), "{body}");
    }

    #[test]
    fn the_copy_status_reports_a_refused_copy() {
        assert_eq!(copy_reply_text("Anna", true), "Copied Anna as vCard and text");
        assert_eq!(copy_reply_text("Anna", false), "Could not copy Anna");
    }

    #[test]
    fn a_search_item_is_titled_by_the_name_and_keyed_by_phones_and_emails() {
        let item = search_item(&anna());
        assert_eq!(item.unique_identifier, "contact.1");
        assert_eq!(item.domain_identifier, DOMAIN);
        assert_eq!(item.attribute_set.title, "Anna Ásgeirsdóttir");
        assert!(item.attribute_set.keywords.contains(&"+354 555 1234".to_string()));
        assert!(item.attribute_set.keywords.contains(&"anna@example.is".to_string()));
    }

    #[test]
    fn the_plan_indexes_new_and_changed_contacts_and_removes_gone_ones() {
        let mut sent = BTreeMap::new();
        sent.insert(1, fingerprint(&anna()));
        sent.insert(2, fingerprint(&bo()));
        let mut changed = bo();
        changed.phone = "+354 777 0000".into();
        let new = Contact { id: 3, name: "Cia".into(), ..Default::default() };
        let contacts = [anna(), changed, new];
        assert_eq!(plan(&sent, &contacts), Plan { index: vec![1, 2], remove: vec![] });
        assert_eq!(plan(&sent, &[anna()]), Plan { index: vec![], remove: vec![2] });
    }

    #[test]
    fn the_first_sync_indexes_everything_then_only_changes_are_sent() {
        let ctx = test_ctx();
        let mut index = SearchIndex::default();
        assert_eq!(index.sync(&ctx, &[anna(), bo()]), 2);
        let sent = ctx.take_sent();
        assert!(matches!(sent[0], ToShell::QuickSearchDelete { .. }), "clears the old items first: {sent:?}");
        let ToShell::QuickSearchIndex(items) = &sent[1] else { panic!("{sent:?}") };
        assert_eq!(items.iter().map(|i| i.unique_id.as_str()).collect::<Vec<_>>(), ["contact.1", "contact.2"]);

        assert_eq!(index.sync(&ctx, &[anna(), bo()]), 0, "nothing changed, nothing sent");
        assert!(ctx.take_sent().is_empty());

        assert_eq!(index.sync(&ctx, &[anna()]), 1);
        let sent = ctx.take_sent();
        let [ToShell::QuickSearchDelete { values, .. }] = sent.as_slice() else { panic!("{sent:?}") };
        assert_eq!(values.clone(), vec!["contact.2".to_string()]);
    }
}
