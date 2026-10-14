//! Contacts in system search: each contact of the person's address book becomes one searchable item. The title is the
//! display name, and the phone numbers, email addresses, nickname, organization and job title are keywords, so a
//! search for any of them finds the person. A contact with no name and no identifier is not indexed. Pure: nothing is
//! sent here; the caller passes the items to [`CSSearchableIndex`](crate::quicksearch::CSSearchableIndex).

use bifrost_ui::contact_book::Contact;

use crate::quicksearch::{CSSearchableItem, CSSearchableItemAttributeSet};

/// The domain every contact item is indexed under, so the whole group can be deleted at once.
pub const DOMAIN: &str = "contacts";

/// The uniform type identifier of a contact item.
pub const CONTENT_TYPE: &str = "public.contact";

/// The name shown for a contact: prefix, given, middle, family and suffix in order, leaving out blanks. A contact with
/// no personal name shows its organization instead. Empty when the contact has neither.
pub fn display_name(contact: &Contact) -> String {
    let parts: Vec<&str> = [
        &contact.name_prefix,
        &contact.given_name,
        &contact.middle_name,
        &contact.family_name,
        &contact.name_suffix,
    ]
    .into_iter()
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .collect();
    if parts.is_empty() {
        contact.organization_name.trim().to_string()
    } else {
        parts.join(" ")
    }
}

/// The search item for one contact, or `None` when the contact has no name to show or no identifier to replace it by.
pub fn searchable_item(contact: &Contact) -> Option<CSSearchableItem> {
    let title = display_name(contact);
    if title.is_empty() || contact.identifier.is_empty() {
        return None;
    }
    let mut keywords: Vec<String> = Vec::new();
    keywords.extend(contact.phone_numbers.iter().map(|p| p.value.trim().to_string()));
    keywords.extend(contact.email_addresses.iter().map(|e| e.value.trim().to_string()));
    for text in [&contact.nickname, &contact.organization_name, &contact.job_title] {
        if !text.trim().is_empty() {
            keywords.push(text.trim().to_string());
        }
    }
    keywords.retain(|k| !k.is_empty());
    let refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
    let attributes = CSSearchableItemAttributeSet::new(&title)
        .content_type(CONTENT_TYPE)
        .keywords(&refs)
        .content_description(contact.organization_name.trim());
    Some(CSSearchableItem::new(&format!("{DOMAIN}.{}", contact.identifier), DOMAIN, attributes))
}

/// The search items for a list of contacts, in the order given. Contacts that cannot be indexed are left out.
pub fn searchable_items(contacts: &[Contact]) -> Vec<CSSearchableItem> {
    contacts.iter().filter_map(searchable_item).collect()
}
