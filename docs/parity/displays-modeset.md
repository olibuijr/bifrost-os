# Displays: live mode change, arrangement, mirroring, confirm and revert (round 75)

Slug `displays-modeset`. Builds on `displays-sound-live.md` (its "Rows not closed" list names this work) and
`cg-services.md`. The owner rule: wire what exists into working behaviour, and say where a model is not yet applied to
hardware.

## Rows

| Row (matrix entry) | API | BifrOSt path | Done | Still open / Midget check |
| --- | --- | --- | --- | --- |
| Display services (Quartz Display Services), mode change (`set_mode`) | `CGDisplaySetDisplayMode`-style mode change through the display list | `springboard/src/shell/services/display_services.rs` (`set_mode`, `MODE_REQUEST`, `MODE_RESULT`); `springboard/src/bare/wayland.rs` (`output_modes`, zwlr output management client); `springboard/src/bare/kms.rs` (`wayland_output_modes`); `springboard/src/bin/shell.rs` (drained each loop); `springboard/sdk/src/displays.rs` (`set_mode` answers `ModeChange`) | Shell to window server request path. `set_mode` answers at once and never `pending`: `{applied: true}` for the current mode, else `{applied: false, requested: true}`; the outcome arrives as the `mode_changed` event. Refused when no output management is present (bare shell). | Build and run on Midget: a mode from `modes` applies, the panel shows it, `mode_changed` arrives with `applied: true`. |
| Graphics accelerators and displays / refresh mode switching (avkit "Display management (HDR, refresh mode switching)") | Connector mode switch on the scanout | `springboard/wm/src/drm.rs` (`Drm::set_mode`, hook installed in `run`, every connector mode announced by `open_gpu` with `Output::add_mode`); `springboard/wm/src/session_protocols.rs` (`apply_outputs` marks the scene dirty after a change) | Pick the connector mode matching size and refresh exactly, then smithay `DrmCompositor::use_mode`: the atomic or legacy commit is chosen by smithay for the driver. The swapchain is resized by smithay. A Vulkan scanout refuses the change (its swapchain is not resized yet). | HDR is not covered. Midget: an atomic driver and the legacy path both show the new mode; no black screen on a refused or failed set. |
| Scale | `Output` scale with the mode | Shell: `springboard/src/shell/services/display_services.rs` (the scale is the shell layout's `s`, from the output size); wm `set_scale` stays ignored (`session_protocols.rs`) | Not a separate request. The Interface Scale page keeps its own meaning; a mode change re-lays out the shell from the new pixel size. | Scale through the window server is not applied. |
| Arrangement of displays (Settings: Arrange) | Global origin of each output | `springboard/wm/src/output_layout.rs` (`extend_row`, `check`, `overlaps`) | Pure model, host-tested: a row from the origin, overlap and duplicate checks. | Not applied to hardware: `drm.rs` drives one connector (one CRTC and one `Output`). A second output needs its own CRTC and `Output` first. |
| Mirroring (Settings: Mirror Displays) | Mirror placement | `springboard/wm/src/output_layout.rs` (`mirror`) | Pure model: every output at the origin, unknown source refused. | Not applied to hardware, same reason as arrangement. `set_mirror` stays refused in the service. |
| Confirm and revert after 15 s | Keep or revert a display change | `springboard/src/shell/services/display_services.rs` (`CONFIRM_SECS`, `Confirm`, tick reverts, `keep_mode`, `revert_mode`; events `mode_changed`, `mode_reverting`, `mode_reverted`) | An applied change is kept only with `keep_mode`; otherwise the shell asks the window server for the previous mode after 15 s. A second change is refused while one is pending. | The Settings sheet (Keep / Revert with a countdown) is not built: no mode picker exists in the Displays pane yet (it still reads the mode as automatic). Midget check once a picker exists. |

## Settings Displays

`springboard/src/shell/pages.rs` (`desktop_display`) has no mode picker: resolution and refresh are read-only and
selected automatically, as the page footer says. The service is ready for a picker; adding one is UI work outside this
slice.

## Evidence

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists the Displays settings extension
  (`System/Library/ExtensionKit/Extensions/DisplaysSettingsIntentsExtension.appex`). No REA measurement was run for this
  round (the extract for the Displays pane is not decoded yet), so the behaviour above follows the existing shell and wm
  code and the matrix notes, not the reference binaries.
- Smithay 0.7.0: `DrmCompositor::use_mode` (`src/backend/drm/compositor/mod.rs`) and `DrmSurface::use_mode` dispatch to
  the atomic or legacy surface.

## Not built or checked

- Nothing was compiled or run (round rule: code and commit only). Every new signature was checked by reading the
  call sites and the crates in the cargo registry; the lead builds.
- Midget checks listed in the table above.
- `docs/parity/displays-sound-live.md` and `docs/parity/cg-services.md` still say that `set_mode` answers `pending`;
  they describe the state before this round.
