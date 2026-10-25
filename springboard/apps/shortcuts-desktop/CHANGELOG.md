# Changelog

## 1.14.4

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 1.14.3

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 1.14.2

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 1.14.1

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 1.14.0 — 2026-10-23

- A click selects, a double-click edits and the play button runs a shortcut; a card context menu; Start Timer starts a Clock timer; errors wrap.

## 1.13.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.13.0 — 2026-10-17

- Automations can start on more events: when an app closes, at login, when a Focus turns on or off, and at a time of day (every day or on weekdays). The Automation row steps through them.
- Ask Before Running is on for each automation by default; its row under Automation switches it off to run at once.
- A Run Shell Script step records a one-time confirmation per shortcut, kept in the list file. The app still refuses shell steps until the shell runs them.
- The shell does not deliver app, login, Focus or time events to the app yet, so automations do not run by themselves on the device.

## 1.12.0 — 2026-10-17

- Add a Run Script step: runs a line of the BifrOSt script language with the previous output as the first item of argv. Notifications from the script are shown; dialogs, other apps and shell commands are refused with a reason.

## 1.11.3 — 2026-10-14

- Use the shared extra-small spacing token for the shortcut step connector gap.

## 1.11.2 — 2026-10-14

- Move Rename and Find into the Shortcuts app menu instead of duplicating the shell Edit menu.

## 1.11.1 — 2026-10-14

- Use the shared grouped-card radius token for the result panel.

## 1.11.0 — 2026-10-14

- Automation row in the editor's Details group, under Menu Key. Each tap steps through Off, each app opening (When Notes Opens and so on) and the charge falling below 10, 20, 30 or 50 percent. The choice is saved in the list file.
- Window menu: All Shortcuts (Command-1); Previous Shortcut and Next Shortcut (Command-[ and Command-]) while a shortcut is open in the editor.
- Not done: the app does not feed app-open or battery events to the automations, so an automation does not run by itself yet; user-made folders, a custom colour, Choose from Menu, the Output step, Ask for Input and Get Contents of URL running in the app, live preview while dragging a step, drag-to-import of files, the share sheet, and a shell-wide hotkey.

## 1.10.0 — 2026-10-14

- Personal automations in the shortcut model: a shortcut can run when an app opens or when the battery charge falls below a level (5 to 95 percent). The automation is saved after the menu key in the list file; older versions read the file and ignore it. A battery change starts an automation only when the charge crosses below its level, so the first reading starts nothing. Duplicating a shortcut and importing one drop its automation.
- Gallery: four more starters under the existing headings (Sign Off, Date Check, Eye Rest, Listen Mode), fourteen in all.
- Help menu: Shortcuts Help opens the result panel with one row per task (make, run, menu keys, sharing, the gallery).
- Not done: the Automation row in the editor, the battery reading and app-open events that would start an automation while the app runs, a Window menu, user-made folders, a custom colour, Choose from Menu, the Output step, Ask for Input and Get Contents of URL running in the app, live preview while dragging a step, drag-to-import of files, the share sheet, and a shell-wide hotkey.

## 1.9.0 — 2026-10-14

- Editor: each plain step has a grip at its right edge. Drag the grip up or down and release to move the step to that row. Openers, Otherwise and the ends keep their place so blocks keep their shape. The step moves when released; a line marks where it lands.
- Tile menu: a long press (about half a second) on a tile opens Run, Rename, Duplicate and Delete under it. A tap anywhere else closes the menu.
- Menu Key row in the editor: each shortcut takes one of the keys 1 to 9 (Control and Command with the digit), shown in the Shortcuts menu beside its name. Choosing a key another shortcut has moves it. Copies and imports take no key. The first four starter shortcuts keep keys 1 to 4.
- Repeat with Each: runs its steps once for each line of the previous output, with the line as {Item}; at most 20 lines. Stop ends the shortcut at that step; the steps after it are listed as not run.
- Insert Variable rows: while a text step is typed into, a row for each variable (Output, then the names Set Variable steps set) inserts {Name} into the text.
- Search also filters the Add Action list (built-in actions and intents) and the gallery (name, line or heading).
- Gallery: starters are grouped under Focus, Everyday and Tools; three more starters (Wind Down, Lunch Break, Battery Check).
- Glyph picker has 21 symbols (eight added at the end, so saved glyph numbers keep their meaning); the colour row gains brown.
- The list file keeps the menu key as a field after the glyph and folder; older versions ignore it.
- Not done: user-made folders, the automations (app open and battery triggers), a shell-wide hotkey, a custom colour, Choose from Menu, the Output step, Ask for Input and Get Contents of URL running in the app, drag-to-import of files, the share sheet. The Automation row and the Menu Key row that the 1.8.0 entry describes were not in that release's code: the menu key is added here, the automations are still missing.

## 1.8.0 — 2026-10-13

- New action kinds in the Add Action list, in groups: Text, Ask for Input, Set Variable and Show Result (Text and Variables); Get Contents of URL and Save File (Web and Files); If, Otherwise, Repeat, Choose from Menu, Menu Item and End (Scripting). The text, variable and scripting steps run inside the app. Show Result opens the result panel, Ask for Input and Choose from Menu ask their question in a panel before the run, and Save File writes into the app's Saved folder. Get Contents of URL is stored and listed, but the app has no network access yet, so a run says it is not available. System steps still cannot run on this device.
- Every step passes its result on as {Previous}; Set Variable names it so later steps can use {Name}. While you type into a text row, Insert Variable adds the next variable's token.
- Blocks: adding an If, Repeat or menu adds its End; removing the opener removes the End; rows inside a block are indented.
- Each action row has move up and move down arrows.
- The editor has a Details group: Name (type to rename), Folder, Menu Key, Daily Reminder and Automation. Automation picks When App Opens or When Battery Is Low with a stepper; the battery automation is checked every minute and runs the shortcut once each time the charge falls below the level. The app-open automation is stored only: nothing tells the app when another app opens.
- A Glyph picker beside the colours; the tile shows the symbol. Duplicate Shortcut and Export Shortcut rows next to Delete.
- The sidebar gains Gallery (six starter shortcuts, tap to add a copy) and a Folders section with counts; selecting a folder filters the grid. The toolbar has a search field that filters the tiles.
- Menus: File (New Shortcut, Duplicate Shortcut, Import Shortcut, Export Shortcut), Shortcut (Run Shortcut, Delete Shortcut, and one item per shortcut with its Cmd+1 to Cmd+9 key) and View (Find, All Shortcuts, Gallery). Import and Export use the open and save panels and a .shortcut file. Edit > Paste works while typing.
- The shortcut file keeps older lines readable: folder, glyph, menu key and automation are extra fields at the end of the line. The SDK glyph set gains chevron.up.
- Needs system 0.4.84, as before.

## 1.8.0 — 2026-10-13

- Steps can carry text and pass it on. New kinds in the Add Action list: Text, Ask for Input, Get Contents of URL, Save File, Show Result and Set Variable. Each step's output is the next step's input; `{Output}` in a step's text stands for the previous output and `{Name}` for a variable set with Set Variable. Tapping a step with text opens a field to type it.
- If and Repeat add blocks. Add Action inserts the whole block (If with Otherwise and End If, Repeat with End Repeat); steps inside are indented, and removing the opener removes the block. If tests the previous output (has a value, is empty, contains, is); Repeat runs 1 to 20 times. A run is limited to 200 steps.
- Run works inside the app. Running a shortcut walks its steps and shows each one with what happened in the result panel. Text, variables, blocks, Copy Text, Show Notification and Show Result run in the app. A step that needs the shell (Wi-Fi, Low Power, Do Not Disturb, brightness, apps, screenshots, timers) stops the shortcut with a reason, and the steps after it are listed as not run.
- Editor: a Name row renames the shortcut, Duplicate Shortcut adds a copy after it (without the daily reminder), Export Shortcut writes it to a file, and the up and down buttons on each step reorder the steps. A Folder row files the shortcut under Favorites, Home or Work, and a Glyph row picks the glyph shown on its tile.
- Sidebar: Gallery, with seven starter shortcuts to add, and one row per folder with the number of shortcuts in it. Import from File in the Gallery reads an exported file.
- The grid has a search field in the toolbar. It matches names, steps and folders.
- Menus: File (New Shortcut, Duplicate, Import, Export, Delete), Edit (Rename, Find) and Shortcuts (Run, Gallery and one item per shortcut; the first nine have Control-Command-digit keys that work while the app is in front).
- The list file keeps the glyph and the folder as two more fields on a shortcut's line; older files read as before, and older versions of the app ignore the new fields.
- The `shortcuts` command runs the new steps it can reach: Text, Set Variable, If and Repeat, a shell script whose output goes to the next step, Save File in Documents and Show Result.
- Adds the documents permission, for Import and Export.
- Needs system 0.4.84, as before.

## 1.7.0 — 2026-10-13

- The action library gains four kinds, in the Add Action list after Start Timer: Show Notification (a message from a list), Copy Text (a text from a list, put on the clipboard), Open Folder (a folder in the home directory: Desktop, Documents, Downloads, Pictures or Music) and Run Shell Script (a command from a list). Their values are picked from their lists, so a saved value always has a known meaning.
- The shortcut model moved to the shell crate (`springboard::shortcuts`), so the app, its control socket and the new `shortcuts` command read and write one file. The app's own copy is gone.
- Plans show each new step's line the same way as the others, from one shared line function.
- Needs system 0.4.84, as before. The app still cannot run its steps itself; see the `shortcuts` command for the steps the command line can run.

## 1.6.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.6.1 — 2026-10-12

- Removed the unused procedural icon and the unused state dump; the app uses its icon.png.

## 1.6.0 — 2026-10-12

- "Plan Shortcut" shows its result in a panel at the bottom of the window instead of a one-line message: the shortcut's name, how many steps it has, and each step on its own numbered row. Long plans show the first eight rows and then "and N more".
- The panel has a "Run Shortcut" button that runs the same shortcut next, and a Done button. The panel closes when you tap Done, or after eight seconds.
- Run and plan results that have no rows still show the one-line message, as before.
- Nothing else changes: the grid, the editor, the scripting and control socket behave as before. The plan result from the control socket gains a snippet and a follow-on, and its value and dialog are the same.

## 1.5.0 — 2026-10-12

- The Add Action list has an Intents section. It lists the actions other apps can ask Shortcuts to do, each with its name and a one-line description.
- Tapping an intent in that list runs it on the shortcut you are editing. Its result shows at the bottom of the window: the steps for "Plan Shortcut", or the reason for "Run Shortcut" (this device cannot run shortcut actions yet).
- Nothing else changes; the grid, the editor and the scripting and control socket behave as before.

## 1.4.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.4.0 — 2026-10-12

- The shortcut picker results are built from one typed list: each shortcut has its position as its id, its name, its colour and its step count, so searches from other apps read the same values as the window.
- The shortcut in front in the window (an open editor or picker) is the one used when "Run Shortcut" or "Plan Shortcut" is asked without a shortcut. A grid with no editor open still asks for one.
- "Plan Shortcut" works out its steps on a separate worker thread, so the window never waits for a plan.
- Shortcut parameters are optional in the intent declaration; a request with no shortcut and nothing in front says that a shortcut is required.
- Needs system 0.4.84, as before.

## 1.3.0 — 2026-10-12

- Shortcuts can be used from other apps and from automation. A shortcut is found by its id or its name; a name that two shortcuts share asks for the id, and a name that matches none says so.
- "Plan Shortcut" lists a shortcut's steps in order without running them. "Run Shortcut" finds the shortcut and then says that this device cannot run shortcut actions yet, so it never reports a run that did not happen.
- A scripting view of the list: a shortcut's name and colour can be read and changed, and its action count is read-only. Changes are checked, saved, and an open window shows them at once.
- A shortcut can have a daily reminder: the "Daily Reminder" row in its editor steps through a few times (07:00 to 22:30) and then Off. At that time the normal notification appears, and it says that this device cannot run shortcut actions yet.
- "Open at Login" in All Shortcuts asks the desktop to start Shortcuts at login. The row shows Needs Approval until the user allows it in Settings > General > Login Items.
- Needs system 0.4.84, as before.

## 1.2.2

- Compliance: grid tile sizes and the Remove badge's glyph come from named tokens; the badge's glyph bar is a capsule sized from the spacing token.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate layout: a Library sidebar (All Shortcuts with its count) beside the content in a full-size window, and a unified toolbar with the title, New Shortcut, Edit/Done, Run and Back in place of the iPad navigation bar and large title.
- Light and Dark follow the system appearance; colours come from the named system tokens.
- Toolbar buttons are published as controls so they stay clickable in the window's drag band. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 10 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.
- Fixes the coloured fringes on the action-count badges of the shortcut cards.

## 1.0.0 — 2026-10-08

- Shortcuts for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Show when a shortcut cannot run instead of indicating successful execution.
