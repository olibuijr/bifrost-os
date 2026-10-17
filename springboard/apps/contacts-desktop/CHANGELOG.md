# Changelog

## 1.16.0 — 2026-10-17

- Model for the merge review (`dupes::review`, `dupes::merge_with`): it lists the fields where duplicates disagree, and folds the cards with the value picked for each field (a phone or email takes its label along). The review sheet is not drawn yet, so Card > Merge Duplicates still keeps the first card's values.

## 1.15.0 — 2026-10-17

- Cmd+W closes the card window it is typed in. Keys and menu items follow the key window: the shell tells the app which window is key (`WindowFocus`) and sends it the input (`WindowInput`), and the app's `window_input` closes the card (`ctx.close_window`).

## 1.14.0 — 2026-10-17

- A tap on an email in a card opens a new message in Mail, prefilled with the address (`bifrost_app::mail_compose`); a tap on a phone number opens a `tel:` link through the shell. Other values still copy on a tap, and the Copy buttons still copy.

## 1.13.0 — 2026-10-17

- Card windows take touches: the card scrolls on its own scroller, a tap on a value copies it, and Edit shows the contact in the main window and opens its editor (`touch_card`, `edit_from_card`, `window_event`). Touches do not change the main window's screen.

## 1.12.0 — 2026-10-17

- Card windows draw: Card > Open in Separate Window and a double-click on a list row open the contact's card in its own window (`Ctx::open_window`, `CardWindows`). The window draws the card at its size (`Contacts::draw_card`, `draw_window`) and its close unbinds the card (`window_closed`). Touches in a card window are not routed yet, so the card is read-only.

## 1.11.1 — 2026-10-17

- Card windows now bind to the SDK window ids (`card_windows.rs`, `CardWindows::bind`), so the app keeps one window registry. Nothing is opened from the UI yet.

## 1.11.0 — 2026-10-17

- Card windows model (`card_windows.rs`): a contact can be opened in its own window through the SDK window registry, and a card that is already open is focused. The window is not drawn yet, because the shell does not host a second window of an app; the Card > Open in New Window menu item and the double-click are not wired.

## 1.10.1 — 2026-10-14

- Removed unused keychain access; the company avatar and background printing now use shared appearance-aware colours and dispatch.

## 1.10.0

- Mark as a Company: the list sorts and sections a company by its company name, as the card and the row already show it.
- Recently Changed keeps each edited contact between runs. The contacts are saved with the display settings, and a deleted
  contact leaves the list.

## 1.9.0

- New Card menu: Go to Next Card (cmd+]) and Go to Previous Card (cmd+[) move through the shown list; Make This My Card and Go to My Card mark and find your own card (saved beside the contacts); Merge Duplicates folds the open card's duplicates into it, keeping differing phone numbers, emails and addresses as extra values, and removes the other cards.

## 1.9.0 — 2026-10-13

- Card fields: Card > Add Field adds a nickname, job title, department, URL, birthday, pronouns, phonetic name, related
  name or profile to a card. The fields show on the card (a press copies the value), are found by the search, and are
  read from and written to vCards (`NICKNAME`, `TITLE`, the second part of `ORG`, `URL`, `BDAY`, `RELATED` and so on).
  They are saved in `details.txt` beside the contacts file.
- Mark as a Company (Card menu, or the switch in the edit form): the card shows its company as the name, with a building
  in place of the initials.
- My Card: Card > Make This My Card and Go to My Card. The card carries a "Me" mark in the list and a caption on the card.
- Go to Next Card and Go to Previous Card (Cmd+] and Cmd+[) move along the list and scroll it to the card.
- Look for Duplicates opens the Possible Duplicates list. On that list, Merge Duplicates folds the open card's twins into
  the first card (missing fields are filled in, different phones, emails and addresses become extra values, notes are
  joined, card fields are kept, groups and My Card follow) and Merge All does every group at once.
- Import reads comma, semicolon and tab separated tables (`.csv`, `.tsv`, `.txt`) and LDIF files (`.ldif`, `.ldf`) from the
  Import folder as well as vCards. The first row of a table names the columns; the status line says how many columns were
  mapped and which were ignored.
- Contacts archive: File > Export Contacts Archive writes every data file of the Contacts folder as one
  `contacts.bcarchive`. File > Restore from Archive replaces the address book with the newest archive in the Import folder
  after a second confirmation, and puts the old data back if the archive does not read.
- Print and Export as PDF (File menu): the open card or the contact list as it is shown, and address labels. Export as PDF
  saves a PDF in the Export folder.
- Last Import in the sidebar lists the contacts the most recent file import added; it is kept between runs.
- Settings: Prefer Nicknames, Include Notes in vCards (exports can leave the notes out) and the card template, the fields a
  new card's form shows. The Settings screen scrolls.
- Smart lists you make: File > New Smart List opens an editor with a name, up to six rules (a field, a condition such as
  contains, is, begins with or is set, and a value) and a choice between all rules and any rule. The lists appear under
  Smart Groups in the sidebar; Edit List changes or deletes one. They are saved in `smartlists.txt`.
- Card > Select Contacts: a press on a row marks it instead of opening it. New Group From Selection makes a group of the
  marked contacts, and Merge Selected Cards folds them into the first.
- File > Refresh Accounts starts the CardDAV sync at once.
- Intents for the shell and the control socket: `view-card`, `create-contact` and `search-contacts`.

## 1.8.1

- Code cleanup: the unused `needs_frame` and `show_contact` helpers are removed. No behaviour change.

## 1.8.0 — 2026-10-13

- Internet Accounts: the CardDAV address books switched on for Contacts are read in the background and merged into the
  contacts (a duplicate card is skipped). The password comes from the passwords group; the sync runs at start and every
  15 minutes. The file import uses the same merge.

## 1.8.0

- Smart Groups in the sidebar: With Email, With Phone, With Company and Missing Details. Each lists the contacts whose fields match, and updates as contacts change.

## 1.7.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.7.0

- A contact can hold more than one phone number, email and address. The first of each is still the contact's main value; the others are kept beside it and show on the card under their labels (Mobile, Home, Work, Other).
- Searching the list also finds a contact by any of its extra phone numbers, emails and addresses.
- Importing vCards keeps the extra phone numbers and emails of each card (up to six extra values per contact). Export writes them back, and Copy Contact puts them in the text and the vCard.
- Your existing contacts open as before. The extra values are saved in their own file (values.txt) beside the contacts file, and deleting a contact removes its extra values.
- Still to come: adding or editing the extra values in the contact editor, and merging duplicate contacts.

## 1.6.0

- The File menu now has Import vCards, Export vCard 3.0 and Export vCard 4.0, so the vCard files can be reached from the
  menu bar as well as the toolbar.
- The View menu has Hide Sidebar (or Show Sidebar) and two name orders: First Name First (the default) and Last Name
  First. Last name first shows a name as "Lovelace, Ada" in the list and on the card. The list order is still set in
  Settings. The choice is saved with the sort order.

## 1.5.0

- A contact's card lists its phone, email and address under the labels you chose (Mobile, Home, Work and so on). Only the
  values the contact has are shown, in the same order as before. Copying a value works as before.
- The list and the contact card sit on the window's content colour, so the background matches other apps in Light and
  Dark.
- Still to come: choosing whether names show first name or last name first. The sort order (Settings) is the only name
  order for now.

## 1.4.0

- Groups can be renamed and deleted. With a group shown in the sidebar, Rename Group types the new name in the search
  box's place (Return saves, Escape cancels). Delete Group removes the group only; its contacts stay in the list.
- Add People opens a picker that lists every contact. A press adds that person to the group shown; people already in
  the group cannot be chosen again. Done ends the picker.
- Settings (gear button) has a Sort Order row: System Default, Family Name, Given Name or Keep Order. The choice is
  saved in `display.txt` beside the contacts file. Keep Order lists contacts in the order they were added.
- Groups are saved with their numbers (`groups.txt` format 2), so a deleted group leaves no gap in the file. Files
  written by earlier versions still open.

## 1.3.0

- Copy Contact on the contact card puts the person on the shared pasteboard as a vCard and as plain text (name, company,
  phone, email, address, one per line). Pasting into a text field gives the text; apps that take vCards get the card.
  The status line says whether the copy worked.
- Contacts are indexed for desktop search by name, company, phone numbers and emails, so Search finds a person by any of
  them. Only changed contacts are sent again, and the index is cleared and rebuilt when the app starts.
- Import and export of contacts as vCard files. Import reads every `.vcf` file in the Contacts `Import` folder, in
  vCard 3.0 or 4.0; a card that matches a contact already in the list is skipped. Export writes `contacts-3.0.vcf` or
  `contacts-4.0.vcf` to the `Export` folder. Groups (vCard 4.0 `KIND:group` cards) are not imported.
- The postal address is kept on import and export.
- Contact groups: New Group makes a group and shows it in a Groups section of the sidebar. With a group shown, Add to
  Group or Remove from Group on an open contact changes its membership. Groups are saved in `groups.txt` beside the
  contacts file.
- Recently Changed lists the contacts added or edited since the app started. Possible Duplicates lists contacts with the
  same email, the same phone number (last seven digits), or the same name with no conflicting phone or email.
- The list sorts by family name, then given name, with a section per family-name initial. Accents sort with their base
  letter, and the Icelandic letters keep their places (ð after d, þ after z).

## 1.2.3

- The lists sidebar (All Contacts, Favorites) opens by default in windows at least 600 pt wide, so the root screen
  publishes the sidebar rows the compliance gate checks. The toolbar toggle still hides it.
- Icon fallback uses the named system colours instead of literal RGB values (compliance COL-01).

## 1.2.2

- Golden Gate compliance: every visible control has an accessibility label (sidebar lists, contact rows, the card and
  form buttons, form fields, copy rows); pressing one through accessibility runs the same action as a tap.
- Avatars use a neutral tone that keeps white initials legible in Light and Dark.
- Sudden termination: the app opts in to `ProcessInfo` sudden termination except while the editor is open, since an
  open form holds unsaved edits.
- Compliance test (`tests/compliance.rs`): the root renders at 1200x780 and scale 1.5625 in Light and Dark with the
  chrome and labelled controls present.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate layout: a full-size window with the contact list and its search field beside the contact card, under a
  unified toolbar (sidebar toggle, New Contact, Edit or Cancel and Done). No iPad navigation bar, back chevron, large
  title, A-Z index strip or on-screen keyboard; typing goes to the focused field.
- A Lists sidebar (All Contacts, Favorites) drawn with the kit source list, shown with the sidebar button.
- Light and Dark: every colour is a system token that follows the appearance; text uses the macOS type scale.
- Address values show on the card and copy like phone numbers and email addresses.
- Edit menu (Cut, Copy, Paste, Select All, Undo) works in the search field and the form fields. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 20 padding and gap values moved to the nearest standard step (at most 2 pt).
- 3 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.1 — 2026-10-08

- The contact count footer is 13 pt, matching the iPadOS footer style.

## 1.0.0 — 2026-10-08

- Contacts for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
