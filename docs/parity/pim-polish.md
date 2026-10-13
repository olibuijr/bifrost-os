# PIM polish (Calendar, Contacts, Reminders): round 75

Slug `pim-polish`. Code and commit only; nothing was built, tested or run (the lead builds).

## State found in the apps

Most of the daily-driver list was already in the apps before this round:

- Calendar (`springboard/apps/calendar-desktop`): Day, Week, Month and Year views (`calendar/timeview.rs`), the event
  editor with invitees and their answers (`calendar/invitees.rs`), alerts through `notification-schedule` with a
  legacy fallback (`calendar/connect.rs`, `calendar/alerts.rs`), recurrence (`calendar/recur.rs`) and iCalendar
  import and export (`calendar/ics.rs`).
- Contacts (`springboard/apps/contacts-desktop`): groups saved in `groups.txt` (`src/group_file.rs`), vCard 3.0 and 4.0
  import and export (`src/exchange.rs`), possible duplicates (`src/dupes.rs`), pasteboard copy and search index
  (`src/services.rs`).
- Reminders (`springboard/apps/reminders-desktop`): lists, smart lists (Today, Scheduled, All, Flagged, Completed in
  `src/model.rs`), flags, subtasks (one level under a parent), due-date alerts and the Dock badge (`src/alerts.rs`,
  `src/system.rs`).

## Added this round

### Contacts: Smart Groups (new)

- `src/smart.rs`: four built-in rules over a contact's fields. With Email, With Phone, With Company, Missing Details
  (no phone or no email). Blank fields made of spaces do not count. Pure std, with unit tests.
- `src/contacts.rs`: `Filter::Smart(Smart)`, sidebar rows `LIST_SMART + offset` (110 to 113) under a "Smart Groups"
  header between Possible Duplicates and Groups, `sidebar_node` covers the new range, and `filter_ids` computes the
  members from the current contacts (nothing is stored, so the lists update with the contacts).
- Test: `a_smart_group_row_lists_the_contacts_that_match_its_rule` in `contacts.rs`.
- Version 1.8.0 in `manifest.toml`, `Cargo.toml`, `springboard/Cargo.lock` and a CHANGELOG entry.

## Not done this round

- Smart groups have no editor: the four rules are fixed. A user-defined rule would need a store format.
- Smart groups are not offered as a target of the Group picker or Add People (they are not stored groups).
- No reference measurement against the macOS 27 Contacts, Calendar or Reminders bundles under
  `~/Projects/macos-ref/extract/ref27` was done in this round; the rows above come from the existing code and tests.
- Calendar and Reminders need no change for the listed items beyond what is already present. Whether the views and the
  event editor match the reference layout is unmeasured and needs a Midget check.

## How to check on Midget

1. Open Contacts. The sidebar shows "Smart Groups" with four rows. Select With Email: only contacts with an email show.
2. Edit a contact to add an email. Its row appears under With Email without restarting.
3. Missing Details lists a contact with no phone, and drops it when a phone is added.
