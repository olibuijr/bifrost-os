# WidgetKit parity: timelines and relevance (pure models)

Status: `partial`. The shell's widgets and the app-facing SDK are unchanged. This round adds two pure Rust models in
`bifrost-ui` (std only, no unsafe, no new crates) for the timeline and Smart Stack relevance rules. Nothing calls them yet.

## What was built

- `springboard/ui/src/timeline.rs`: `Timeline<T>`, `Entry<T>` and `ReloadPolicy` (`AtEnd`, `After(date)`, `Never`).
  - `Timeline::new` sorts entries by date, stable for equal dates.
  - `entry_at(now)` returns the latest entry whose date has passed. Before the first date it returns the first entry.
  - `next_reload(now)` returns the reload instant, never earlier than `now`. `Never` gives `None`.
- `springboard/ui/src/relevance.rs`: `Relevance` (score and duration), `Item` (date and optional relevance),
  `score_at` and `rank`.
  - An item is eligible from its date for `duration` seconds. The window is closed at the start and open at the end.
  - `rank` puts eligible items first, highest score first. Equal scores and ineligible items keep their given order.
- Module registration: one `pub mod` line each in `springboard/ui/src/lib.rs`.
- Tests: 5 in `timeline.rs`, 6 in `relevance.rs`. They cover sorting, entry selection at boundaries, reload
  policies, window edges, ties, and ineligible items.

Time is seconds since 1970-01-01 as `f64`. The models read no clock; the caller passes `now`.

## Official API names mirrored

- WidgetKit `Timeline` and `init(entries:policy:)`, `entries`, `policy`
- `TimelineEntry` (`date`) and `TimelineProvider` `Entry` (date only; the relevance attribute is in `Item`)
- `TimelineReloadPolicy` cases `atEnd`, `after(_:)`, `never`
- `TimelineEntryRelevance` (`init(score:duration:)`, `score`, `duration`) and `TimelineEntry.relevance`

## REA evidence

- `ev_efdb7db644c30fe8e0abeed6a5085c03048b862060b422f1e622cb7fba2f5222`: rendered WidgetKit landing page
  (`https://developer.apple.com/documentation/widgetkit`), captured with `rea-page.sh` on the local CDP browser.
  The page states that widgets use timelines and that WidgetKit requests updates in an energy-efficient way.
- The matrix row on the `docs/apple-parity-matrix` branch cited `ev_757db36dfe972d6a7e8fcaedce8a77ef37f344a4f7167139e0a500de852c58ab`.
  This round's capture replaces it in the JSON.

Official DocC reads (`python3 -I docc.py`): `widgetkit`, `widgetkit/timelineprovider`, `widgetkit/timelineentry`,
`widgetkit/timeline`, `widgetkit/timelinereloadpolicy` (with `atend`, `never`, `after(_:)` listed),
`widgetkit/timelineentryrelevance` (`score`, `duration`), `widgetkit/widgetfamily`.

## Gaps that remain

- No app-facing API: apps cannot provide a timeline, call reload, or register a widget kind. The models have no SDK
  wrapper and the shell does not use them.
- The shell Smart Stack is still a manual swipe list. Wiring `relevance::rank` into `springboard/src/shell/today.rs`
  is the next step.
- No large widget family, no widget deep links (`widgetURL`), no widget intents, no accessibility descriptions, no
  accented rendering mode, no Live Activities, no app-provided Control Center controls.
- Reload policy is a model only. There is no scheduler that fires a reload.

## Build and test

Not run in this stage by instruction. The stage that builds the crate must run `cargo test -p bifrost-ui timeline` and
`cargo test -p bifrost-ui relevance`.

## Team J: access, rendering modes (parity/team-j-notification-center)

Two rows move in this round. Both are pure host models; nothing is wired into the shell or apps yet, and nothing has
been built or run on Midget.

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/widget_access.rs` | Accessing location information in widgets; Making network requests in a widget extension | Per-widget grants keyed by bundle identifier: one location switch and a set of network hosts. A host normalizes from a host name or a URL (scheme, path, query and fragment dropped; ports and user information refused). A host grant covers the host and its subdomains and nothing else. Forgetting a widget drops its grants. |
| `springboard/ui/src/widget_render.rs` | WidgetRenderingMode (fullColor, accented, vibrant); widgetAccentable(_:) | Resolves the requested mode against a tinted look (accented and vibrant fall back to full colour otherwise), and tints one element: accentable elements take the accent colour in accented mode, every element becomes a grey of its luminance in vibrant mode. Alpha is kept. |

Matrix rows changed in `docs/parity-matrix/data/widgetkit.json`:

- Location and network access in widgets: gap to partial (`springboard/ui/src/widget_access.rs`).
- Rendering modes and accented look: note and path extended with `springboard/ui/src/widget_render.rs`; status stays partial.

Left open and why:

- Enforcement: the shell weather tile (`springboard/src/shell/weather/mod.rs`) still reads Open-Meteo directly. There is
  no widget process, so the grant has no caller until the shell asks it before a fetch.
- Widget container background: needs a drawing change in the shell tiles, not a model.
- Vibrant mode: the luminance rule is BifrOSt's own. It is not checked against the system material on Midget.

Official names were taken from the WidgetKit landing page and the rows in the matrix data; the access rules are
BifrOSt's own design, not Apple's behaviour.
