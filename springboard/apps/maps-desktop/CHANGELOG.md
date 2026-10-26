# Changelog

## 0.7.6

- Automatic units follow the Settings region, so Iceland shows metric scales.

## 0.7.5

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.7.4

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.7.3

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.7.2

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.7.1 — 2026-10-24

- Zoom Out no longer shows empty squares around the old view for about two seconds: once the shown tiles are in, the
  tiles of the next level out load ahead, so a zoom out finds them ready (the scaled tiles of the level left still stand
  in for any not loaded yet).

## 0.7.0 — 2026-10-23

- Return moves to the first search result; zooming out keeps scaled tiles instead of black; Recents has a context menu.
- Clicks in extra windows land under the pointer on scaled screens.

## 0.6.7

- Maps refuses a redirect from an https tile, search or route server to plain http, and keeps cookies per host instead of one jar for every service. A server the person configured as http still works.

## 0.6.6

- Fix: the toolbar is drawn where its clicks land. It is laid out in the detail pane's own coordinates, so the SDK traffic-light inset now gets the pane's offset and leaves it beside the sidebar.

## 0.6.5

- Map tiles show: a finished tile fetch now wakes the app loop (the fetch threads added in 0.6.3 never did, so tiles that arrived stayed unseen until the next input).
- The toolbar and sidebar use symbols the kit draws (Directions, Drop Pin, Current Location, Map and Measure Distance showed a clipped label); Work has an icon.

## 0.6.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.6.3 — 2026-10-17

- The tile fetch, cache and loader moved into the shell (`springboard::maptiles`) so Photos can draw map tiles under its Places pins. Maps uses the same code through thin re-exports; the offline area helper stays here. Tiles are fetched on one thread each, paced as before, instead of the app dispatch queue.

## 0.6.2 — 2026-10-17

- Keys, the menu bar, the toolbar and the Go and File menus act on the focused window. A touch on a window (the main one or a New Window) gives it the focus; closing the focused window gives it back to the main window. The GPX file opens into the window that asked for it.

## 0.6.1 — 2026-10-17

- One shared store for every window: the saved places, the pins, the units setting and the offline areas are held once for the app (`state::Shared`) and saved from there. Two windows no longer overwrite each other's favorites and pins. The camera, the card and open sheets stay per window.
- The view and map type saved in settings.json are the last window's, so a restart reopens the view of the window that saved last.

## 0.6.0 — 2026-10-17

- File > New Window (Cmd+N) and the Dock menu's New Window open another Maps window. Each window has its own map state (view, pins, route, card); the window's own map and card take touches. Keys, the menu bar and the toolbar still act on the main window.
- A window closed by its traffic light stops following the location.
- Known limit: all windows read and write the same places.json and settings.json, and each saves its whole state, so the last window to save a change wins. A shared store is needed before this is safe for heavy use.

## 0.5.2 — 2026-10-17

- Add the location purpose string to the manifest (`[privacy] location`). Location requests without it are refused.

## 0.5.1 — 2026-10-14

- Clamp the location accuracy ring to the current map viewport instead of a fixed 2000 pt size.

## 0.5.0 - 2026-10-14

- Transit requests take a departure time: the planner model has leave-at and arrive-by times, checked as a date
  and a clock time before they reach the URL. The directions card has no time picker yet.
- Each transit leg names the stops in between when the planner lists them ("3 stops in between").
- Print draws the followed route line and the pins over the map picture, as the map shows them.
- The matrix rows for Current Location, the map data credit and the route alternatives cite the code that does it.

## 0.4.0 - 2026-10-14

- Hybrid map type (View > Hybrid, Cmd+3): the imagery with the place names drawn over it from a labels layer in its own
  cache folder. The map credit names the labels.
- Pinch to zoom: two fingers on the map zoom it, anchored at their midpoint. The mouse wheel is not wired yet (the
  shell does not send scroll events to apps).
- Share is a submenu of File: Copy Link, Copy Coordinates and the share sheet.
- Go > Add Place to Collection: one slot per collection (up to eight), so a place can be added from the keyboard
  without dragging it from the card.
- Print sends the map as it is on screen (the loaded tiles, without the route and pin overlays) as the first page,
  with the place or the directions text on the pages after it.
- The `map:` link scheme opens the app like `maps:` and `geo:`.

## 0.3.0 - 2026-10-13

- Current Location: a toolbar button and Go > Current Location (Cmd+L) ask the shell's location service for a position
  and centre the map on it. The first use asks for permission; a refusal, or no location set in Settings, shows why.
  The position is drawn as a dot with a ring as wide as the fix is uncertain, and Directions starts from it by default.
- Turn-by-turn guidance: Start Navigation on a route from the routing server follows the positions the location service
  reports. The card shows the next manoeuvre and its distance, the time and distance left, and the latest call ("In
  150 m, turn left onto ..."); the map follows the position. Leaving the route by more than 50 m (or the fix's
  uncertainty, if larger) asks for a new route, at most once every 15 seconds. Calls are shown, not spoken: the shell
  has no speech engine yet.
- Transit directions: a Transit mode on the directions card asks an OpenTripPlanner-compatible journey planner set as
  `transit_url` in settings.json and lists each journey leg by leg with the line, the stops and the times. With no
  planner set the card says so; there is no straight-line stand-in for a timetable.
- Show Traffic (View menu, Cmd+T): an optional traffic layer from the tile URL `traffic_url` in settings.json, drawn
  over the map and fetched again after five minutes. Without the setting the menu item says how to turn it on.
- Download Map keeps a list of saved offline areas in the same card; Delete removes an area and the tiles no other
  saved area needs.
- Still not done: 3D and street-level views, Dock New Window and Handoff.

## 0.2.0 - 2026-10-13

- Maps is now an app: a Golden Gate window with a sidebar (search, Home and Work, Favorites, Collections, Recents), the
  map under a unified toolbar, zoom buttons, a scale bar and the data credit.
- Pan by dragging, zoom with the buttons, double click and Cmd+plus and Cmd+minus; Map and Satellite types.
- Search with suggestions after a pause (one request a second), typed coordinates, saved places searched offline.
- Place cards with address, phone, website and opening hours; Drop Pin, Favorite, Set Home and Work, Collections.
- Directions by car, on foot and by bike with alternatives and steps, a labelled straight-line estimate when no routing
  server is set in settings.json.
- Copy Link, Copy Coordinates, Share, Print, Measure Distance, Download Map for a small area, GPX files, `maps:` and
  `geo:` links, kilometres or miles, and the `maps.search` and `maps.route` control methods.
- Not yet: transit, turn-by-turn guidance, current location (the shell has no location service), 3D, traffic, Dock New
  Window and Handoff.
