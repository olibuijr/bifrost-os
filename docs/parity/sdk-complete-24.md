# Parity sdk-complete-24: notification, authentication, locale, dates and formatting

This round completes the partial rows of the notification, local authentication, locale, date and video model groups.
All new behaviour is headless and deterministic; the tests live in `springboard/<crate>/tests/parity_sdk-complete-24.rs`.

## What was built

- **Notification observers** (`springboard/sdk/src/notification.rs`): `ObserverToken::suspend` and `resume` pause
  delivery to one observer without removing it. `distributed_user_info` sorts a distributed post's user info, keeps at
  most 32 entries and cuts each value to 1024 characters. The sender's own observers receive the same payload.
- **Token table** (`springboard/core/src/notify_center.rs`): the system event names are constants, and
  `volume_change` maps a change in the mounted volume count to a mount or an unmount name.
- **Shell system names** (`springboard/src/shell/services/notify_bus.rs`): the time zone file, the host name file and the
  mount table are read on the existing one-second poll. Changes post `system.timezone.change`,
  `system.hostname.change`, `system.volume.mount` and `system.volume.unmount`.
- **Authentication errors** (`springboard/sdk/src/local_auth.rs`): `LAError::code` and `LAError::from_code` use the
  numeric codes of the authentication error domain, and `ERROR_DOMAIN` names the BifrOSt domain.
- **Locale** (`springboard/sdk/src/locale.rs`): `region_code`, `measurement_system` and `calendar_identifier`, and
  `currency_symbol` for the common currency codes.
- **Dates** (`springboard/sdk/src/date_interval.rs`): a half-open span of absolute time with containment, duration and
  intersection. A span never runs backwards.
- **ISO 8601** (`springboard/sdk/src/formatters/iso8601.rs`): writes the internet date-time form with a `Z` or a
  `+hh:mm` offset, and reads it back, refusing dates that do not exist.

## Not built, and why

- Observer queue argument: the runtime delivers observers on the main thread only, so there is no second queue to bind to.
- Clock set, low disk space, disk cache and log database names: the shell has no source for them yet. They need a
  wall-clock jump monitor, a free-space probe and the cache and log stores.
- System-wide notification names: cross-process posting outside BifrOSt apps has no bus here.
- Password sheet drawing: the sheet is a shell drawing change and needs a check on the lab machine.
- Spell-out in languages other than English, plural rules, person-name and interval formatters: each needs grammar data
  that the locale tables do not carry yet.
- Week-of-year ranges and weekday ordinal components in the calendar: they need week rules that the calendar does not model.
- A caller for the video track model: no library holds video items yet.

Everything here is host-tested only. The lab machine confirms the file-based system names, the shell notifications and
the password prompt.
