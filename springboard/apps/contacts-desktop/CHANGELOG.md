# Changelog

## 1.9.0

- New Card menu: Go to Next Card (cmd+]) and Go to Previous Card (cmd+[) move through the shown list; Make This My Card and Go to My Card mark and find your own card (saved beside the contacts); Merge Duplicates folds the open card's duplicates into it, keeping differing phone numbers, emails and addresses as extra values, and removes the other cards.

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
