# Parity sdk-complete-28: models for popover tips, tooltips, preview transitions, spoken replies and the play queue

These pure models finish missing behaviour in five partial rows. Each one is testable without a window, a clock or a
speech engine. Tests: `springboard/ui/tests/parity_sdk-complete-28.rs` and `springboard/sdk/tests/parity_sdk-complete-28.rs`.

| Module | Mirrors | Wired | Needs the lab machine |
|---|---|---|---|
| `bifrost_ui::tip` (`Tip`, `TipStore`, `Rules`, `Invalidation`) | Popover tip content, display limits, required events, invalidation | Not yet. An app holds a `TipStore` and places the popover with `split::place_popover`. | Drawing the tip on screen and the action button. |
| `bifrost_ui::tooltip` (`Tooltips`, `Timing`) | Tooltip delay, hand-off between neighbours, grace period after a close | Not yet. The view layer feeds pointer enter and leave with the current time. | Hover wiring and drawing the tooltip. |
| `bifrost_ui::preview_transition` (`PanelFrame`, `lerp_rect`) | Preview panel open and close from the item's source frame, full-screen mode | Not yet. The shell asks for the rectangle at each animation frame. | Animating the panel and the Space toggle. |
| `bifrost_ui::voice_reply` (`spoken_reply`, `speak_reply`) | Spoken answers to resolved, ambiguous and unknown voice commands | Not yet. The voice path calls it after `voice_commands::resolve`. | Speech recognition and playback of the reply. |
| `bifrost_ui::media_queue` (`PlaybackQueue`) and `bifrost_app::media_intents` (`handle_request`) | Playback requests applied to an app's play queue; the dialog that answers | SDK re-exports for apps. | Handing the queue to the audio service and the voice path. |

Times are seconds passed in by the caller. Nothing in these modules reads a clock or draws a pixel.
