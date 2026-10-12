# Wave 3, theme-26: interactivity in widgets

Theme: "Interactivity in widgets (Button, Toggle, AppIntent in widget)", 13 gap rows across WidgetKit, App Intents,
SwiftUI (App structure, Data and storage), ExtensionKit, RelevanceKit and Now Playing.

This round builds two pure models in `springboard/ui/src` (bifrost-ui, no hardware, std only). Both are unit tested
on the host. They are not wired into the shell or the SDK yet, and nothing has been run on the VM or Midget.

## What was built

- `springboard/ui/src/widget_action.rs` (new). A widget holds controls and an optional link.
  - `Control` is a button (`Action::Button`) or a toggle (`Action::Toggle`), each naming the app intent it runs.
  - `press` runs a control. A toggle flips first and sends its new state as `Invocation.value`. A disabled control
    and a locked device both run nothing and leave a toggle unchanged, since Apple runs no action on a locked device.
  - `hit` finds the control under a point (the last one drawn wins). `tap` gives a `Tap`: a control that ran an
    intent, a consumed click, the widget's link to open, or nothing. A click on a control never reaches the link.
  - `valid_link` accepts a URL with a plain scheme and something after it, and refuses script and data schemes.
  - `describe_control` gives the label and the On or Off state of a toggle. `describe_widget` gives the explicit
    description, else the title, else None.
- `springboard/ui/src/skip_interval.rs` (new). The now-playing skip by interval.
  - `intervals` keeps the usable preferred intervals in order, dropping non-positive, non-finite and repeated values.
  - `default_interval` is the first usable one.
  - `target` moves a position forward or backward by an interval. It clamps at 0, and at the duration when the item
    has one. A live item has no upper bound.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each new module.

## Official Apple APIs mirrored

Read with `python3 -I tools/parity/docc.py`.

- WidgetKit, "Interactivity" section: `widgetURL(_:)` ("Sets the URL to open in the containing app when the user
  clicks the widget"), and the articles "Adding interactivity to widgets and Live Activities" and "Linking to specific
  app scenes from your widget or Live Activity". Source: https://developer.apple.com/documentation/widgetkit
- SwiftUI `Button` and `Link` ("A control for navigating to a URL"). Source: https://developer.apple.com/documentation/swiftui/button
  and https://developer.apple.com/documentation/swiftui/link
- The article "Adding accessible descriptions to widgets and Live Activities" is listed on the WidgetKit page.
  Its own page returned "not found" to REA (see evidence below).
- MediaPlayer `MPSkipIntervalCommand` ("the available skip intervals, in seconds, for a media item") and its
  `preferredIntervals`. Source: https://developer.apple.com/documentation/mediaplayer/mpskipintervalcommand

Apple's docs do not say which interval the system picks from `preferredIntervals`. `default_interval` is our own
rule, and that is recorded here rather than presented as Apple's behaviour.

## REA evidence

- WidgetKit landing page: `ev_16edf340e1226316306daee114bd6ac28ad56cedd937fb56d25e27db1c048066`
- "Adding interactivity to widgets and Live Activities": `ev_4f5bc609678930e23afaf2c5b4976329c8cd7f0c022b42194949daaa9ac7491a`
  (rendered text covers buttons, toggles, App Intents and the locked-device rule)
- `MPSkipIntervalCommand`: `ev_0bfbd28338b9816aace7664f24dcac346b2eb07eaa7aeb481f5428f1de06d7b3`
- "Adding accessible descriptions to widgets and Live Activities": `ev_ed6feaa413821c4fed960e4b64e5b380d4698084b6a2296fb3ee821179e2ae12`.
  This evidence is a "page you're looking for can't be found" render. It does not support the accessibility claim,
  which rests on the WidgetKit landing page's listing.

## Matrix rows closed

These are the status values to apply centrally. Each is `partial`, because the models are not wired into an app or
the shell.

| Framework slug | Feature (exact, from data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| widgetkit | Interactivity in widgets (Button, Toggle, AppIntent in widget) | partial | springboard/ui/src/widget_action.rs |
| widgetkit | Deep links from widgets (widgetURL, Link) | partial | springboard/ui/src/widget_action.rs |
| widgetkit | Accessibility descriptions for widgets | partial | springboard/ui/src/widget_action.rs |
| now-playing | Skip forward and backward by interval | partial | springboard/ui/src/skip_interval.rs |

## Not closed in this theme

- App Intents "Widgets, controls and Live Activities": needs the shell to host widget actions. Only the model exists.
- WidgetKit "Live Activities", "Configurable widgets", "Location and network access in widgets", ExtensionKit remote
  views, SwiftUI "App extensions", and the RelevanceKit "Time clues" row. None of these has a model here.
- RelevanceKit "Time clues" is near `springboard/ui/src/relevance.rs` (date windows), but Apple's clue API is not
  mirrored. The row stays a gap.
