# Milestone: Unified Logging and Console (2026-10-10)

Goal: one system log that every app, service and the shell writes to with a subsystem and category, a central
registry of those subsystems, and a Console system app (installed by default) to read, stream and search it.

## What Apple does (the reference)

- **Unified Logging** (`os` framework, developer.apple.com/documentation/os/logging): `Logger(subsystem:category:)`
  (Swift) / `os_log_create` (C). The subsystem is a reverse-DNS id (`com.example.notes`), the category a component
  (`sync`, `ui`). Levels: `debug`, `info`, `notice` (default), `error`, `fault`. Debug is not stored, info is kept in
  memory only, notice and above are persisted. Dynamic values are `<private>` unless marked `.public`.
- **logd** owns the store; apps never write files. Per-subsystem levels are set with `log config --subsystem X
  --mode level:debug` or a configuration profile, so the registry is the subsystem plus its settings.
- **OSLogStore** (`OSLogStore(scope: .currentProcessIdentifier)` / `.local()` with an entitlement) reads entries back
  as `OSLogEntryLog` (date, process, pid, subsystem, category, level, composedMessage).
- **OSSignposter** marks intervals and events for Instruments.
- **`log` CLI**: `log show --predicate 'subsystem == "x"' --last 1h`, `log stream --level debug`, `log config`.
- **Console.app** (Console User Guide): sidebar with devices and Log Reports (Crash Reports, Spin Reports, Log Reports,
  Diagnostic Reports, system.log); a Start/Pause streaming button; a table (Type, Time, Process, Message, with
  Subsystem and Category columns optional); search tokens (`process:`, `subsystem:`, `category:`, `type:`), saved
  searches; Action menu "Include Info Messages" / "Include Debug Messages"; a detail pane for the selected entry.

## BifrOSt design

| Apple | BifrOSt |
|---|---|
| logd store | journald (already running). No new daemon. |
| `Logger(subsystem:category:)` | `bifrost_app::log::Logger::new("is.olibuijr.notes", "sync")` with `debug info notice error fault`; written to the native journald socket (`/run/systemd/journal/socket`, no dependency) with fields `BIFROST_SUBSYSTEM`, `BIFROST_CATEGORY`, `PRIORITY` (debug 7, info 6, notice 5, error 3, fault 2), `SYSLOG_IDENTIFIER` = app id. Falls back to stderr. Shell and services use the same crate. |
| privacy `<private>` | `log::private(v)` wraps a value; it is stored redacted unless the subsystem's mode says `private: on`. |
| subsystem registry + `log config` | Apps declare subsystems in `manifest.toml` (`[[log.subsystem]] id, categories, default level`). Registry: `/var/lib/bifrost/log/registry.toml` (built at install from manifests); overrides in `/etc/bifrost/log.toml`. `Logger` reads the effective level once and on SIGHUP-free reload (mtime check), so disabled debug costs one compare. |
| `log` CLI | `bifrost-log show|stream|config|subsystems` (journalctl underneath, predicate subset: subsystem, category, process, level, message contains, last). |
| `OSLogStore` | `bifrost_app::log::LogStore::current_process()` for any app; `LogStore::local()` only for `kind = "system-app"` with `entitlements = ["log-store"]` (enforced by the shell, which brokers the read). Entries are `LogEntry { date, process, pid, subsystem, category, level, message }`. |
| `OSSignposter` | Phase 3: `Signposter::begin_interval/end_interval` written as journal entries with `BIFROST_SIGNPOST` id and type. |
| Console.app | `springboard/apps/console-desktop`, `kind = "system-app"` (preinstalled, not removable), Golden Gate kit: source list (Live: this Mac; Reports: Crash Reports from systemd-coredump, Shell Panics, Log Files /var/log), unified toolbar with Start/Pause, Clear, search field with tokens, Info/Debug toggles; table Type badge, Time, Process, Subsystem, Category, Message; detail pane. Light and Dark, semantic colours only. |
| Crash Reports | coredumpctl list/info, shown as reports with a backtrace; the shell's panic log alongside. |
| agent access | the MCP `logs`/`crash` tools read the same fields (filter by subsystem). |

## Phases

1. **Logging API and registry.** SDK `log` module, journald writer, levels, privacy, registry from manifests, overrides,
   `bifrost-log` CLI, `LogStore` read API with the entitlement. Adopt in the shell (a `springboard` subsystem with
   `ota`, `store`, `apphost`, `presenter` categories) and in Notes. Docs in `docs/bifrost-ui.md`.
2. **Console app.** System app on the kit; streaming, filters, search tokens, detail pane, crash reports. Preinstalled.
3. **Signposts and reports.** Signposter, spin reports from the shell's frame watchdog, export/share an entry set.

Acceptance: an app logs `notes sync error` and it shows in Console within one second, filtered by `subsystem:`; debug
messages appear only after `bifrost-log config --subsystem is.olibuijr.notes --level debug`; a crashed app appears
under Crash Reports with a backtrace; Console is on a fresh install without the App Store.
