# Date & Time and Language & Region (slug: date-language)

Reference: macOS 27 (26A434) settings panes Date & Time and Language & Region. Evidence used in this round: the
settings labels that the existing shell and the Golden Gate spec use (`Set Automatically`, `24-Hour Time`,
`Time Zone`, `Region`, `First Day of Week`) and the locale names the SDK already models. No REA session was run and
no evidence ids were recorded: the settings extension bundles under `~/Projects/macos-ref/extract/ref27/` were not
inspected in this round. Re-check the pane layout and the row labels against them before closing the rows in the matrix.

## Rows

### Date & Time (existing, verified in code)

- **Set Automatically** (network time on or off). API: `timedatectl set-ntp` through privd `time.set_ntp`.
  Paths: `springboard/src/shell/pages.rs` (PageId::DateTime), `settings.rs` (Toggle::AutoTime),
  `springboard/src/net/ntp.rs`, `springboard/src/bare/timezone.rs` (`set_ntp`, `timesyncd_enabled`).
  Done: the switch, the sync footer and the privd call. Midget check: the switch flips systemd-timesyncd.
- **24-Hour Time**. Paths: `Options.clock_24h`, `status::clock_in`, menu bar `desktop_bar.rs clock_text`.
  Done. Midget check: the menu bar clock follows the switch.
- **Time Zone**. API: `timedatectl set-timezone` through privd `time.set_timezone`. Paths:
  `shell/datetime_page.rs` (searchable zone picker), `bare/timezone.rs`. Done: search by city or region name.
  **Not done:** the map view. The picker is a list, not a map. Gap noted for the lead.

### Language & Region (new in this round)

- **Preferred languages**. API: `bifrost_ui::region::LANGUAGES`, `Region.languages`, `settings.ini` key `languages`.
  Paths: `springboard/ui/src/region.rs`, `shell/pages.rs` (PageId::Language), `shell/settings.rs` (Pick::Language,
  `change_region`). Done: the first language is the system's. Two languages are offered (English, Íslenska).
  Midget check: the page checkmark and the menu bar date change language.
- **Region** (date, number, currency and first-day formats). API: `bifrost_ui::region::REGIONS`, `Region.region`.
  Paths: `shell/pages.rs` (PageId::Region), `settings.rs` (Pick::Region). Done: twelve regions. The SDK
  `Locale::current()` reads the same file (`sdk/src/locale.rs`, `saved_region`), so apps format with the region.
- **First Day of Week**. API: `Locale::first_weekday()` (SDK, 0 = Sunday), `Region.first_weekday`. Paths:
  `shell/pages.rs` (Automatic, Sunday, Monday rows), `settings.rs` (Pick::FirstWeekday). Done in the SDK and the
  settings page. **Not done:** the Calendar app's month grid still starts on Sunday (`apps/calendar-desktop/src/calendar.rs`
  `grid_start`, the weekday header order). Adopt it there in a later round; it needs a build.
- **Currency**. API: `Locale::currency_code()` (now a String, with the override), `default_currency`. Done:
  the region's currency, shown on the page; the number formatter uses it.
- **Menu bar and lock screen date**. API: `bifrost_ui::region::short_date`, `status::date_text`. Done: "Tue 13 Oct"
  by default, "Tue Oct 13" for the United States, "þri. 13. okt." in Icelandic.
- **System locale**. API: privd `locale.set {lang, region}` through `localectl set-locale` (LANG and LC_TIME,
  LC_NUMERIC, LC_MONETARY, LC_MEASUREMENT, LC_PAPER). Paths: `springboard/src/privd/methods.rs` (`set_locale`,
  `valid_locale`), `privd_client.rs` (`set_locale`), `settings.rs` (`change_region`). Validation: the name must have
  glibc's shape and its source file in `/usr/share/i18n/locales`. Done in code. **Midget check needed:** `is_IS`
  and `en_IS` must be generated for the system call to succeed; a missing locale leaves the system as it was.

## Shared-file edits

- `springboard/ui/src/lib.rs`: one line, `pub mod region;`.
- `springboard/sdk/src/locale.rs`: `Locale` gained `first_weekday` and `currency` fields; `currency_code()` returns String.
- `springboard/src/bin/shell.rs`: one call publishes the saved region at start.

## Not verified

No build or test was run (brief rule). The new unit tests are in `ui/src/region.rs`, `sdk/src/locale.rs`
(`the_first_weekday_and_currency_follow_the_region_unless_set`), `src/privd/methods.rs` (locale tests),
`src/shell/pages.rs` (`language_and_region_mark_the_fresh_install_defaults`).
