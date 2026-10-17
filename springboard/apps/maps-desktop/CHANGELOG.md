# Changelog

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
