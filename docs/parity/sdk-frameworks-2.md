# SDK parity round 2: Authentication Services, Core Motion, User Notifications UI

Slug `sdk-frameworks-2`. Picked from `pc parity coverage --sort coverage` as the lowest-coverage frameworks with
Partial rows that apps need. VisionKit and Speech (both 33-39%) were already reachable through the SDK
(`data_scanner`, `speech_recognition`, `speech_models`, `microphone`), so they were left as they are. The matrix rows
are not edited (lead closes them).

Scope: the pure models already exist in `bifrost_core` and `bifrost_ui` but had no app-facing path. This round
adds the app-facing path (SDK modules and re-exports) and host unit tests. Nothing is wired into the shell or
into an app yet, so each row still needs a Midget check.

Evidence: the rows and their notes come from `pc parity show` (REA evidence ids are in the row headers). No reference
files were copied into the repo.

## Authentication Services

| Row | API | BifrOSt path | Done | Still open |
|---|---|---|---|---|
| Web authentication sessions (Partial) | WebSession, PresentationContext, SessionError, State, scheme_of | `springboard/sdk/src/sign_in.rs` over `springboard/core/src/auth_session.rs` | App path to start, cancel, complete with a callback URL and read the outcome. Tests cover start, the callback scheme check, a missing and an invalid window, cancel. | No shell routing of the callback scheme to the requesting app; no browser sheet. |
| Authorization requests (Partial) | AuthorizationController, AuthorizationResult, RequestKind, Phase, ControllerError | `springboard/sdk/src/sign_in.rs` over `springboard/ui/src/authorization_request.rs` | Controller perform/complete/fail/cancel; result must answer a requested kind; password debug output is redacted. | No sign-in sheet; no provider. |
| One-time passcodes (Partial) | find_code, FoundCode | `springboard/sdk/src/sign_in.rs` over `springboard/ui/src/one_time_code.rs` | App can find a code next to a keyword in message text. | No message reader; no field binding. |
| AutoFill credential provider (Partial) | IdentityStore, CredentialIdentity, ServiceIdentifier, IdentityTypes, StoreError | `springboard/sdk/src/sign_in.rs` over `springboard/ui/src/credential_identity.rs` | Store and identity types are reachable from apps. | No system-wide AutoFill; the browser has no fill. |

## Core Motion

| Row | API | BifrOSt path | Done | Still open |
|---|---|---|---|---|
| Magnetometer (Partial) | magnetic_heading, true_heading, Calibration | `springboard/sdk/src/sensors.rs` over `springboard/core/src/magnetometer.rs` | SDK path to the heading math. Tests: true heading wraps; a field along gravity has no heading. | No magnetometer reader feeds it. |
| Altitude and ambient pressure (Partial) | absolute_altitude_m, RelativeAltimeter, AltitudeData | `springboard/sdk/src/sensors.rs` over `springboard/core/src/altitude.rs` | SDK path to the pressure conversion. Test: sea-level pressure gives 0 m. | No barometer reader. |
| Motion activity (Partial) | classify, Activity, Kind, Confidence | `springboard/sdk/src/sensors.rs` over `springboard/core/src/motion_activity.rs` | SDK path to the classifier. | No sensor feed. |
| Pedometer (Partial) | Pedometer, StepDetector | `springboard/sdk/src/sensors.rs` over `springboard/core/src/pedometer.rs` | SDK path. | No sensor feed. |
| Device motion (Partial) | MotionEstimator, DeviceMotion, Attitude, Vec3 | `springboard/sdk/src/sensors.rs` over `springboard/core/src/motion.rs` | SDK path to the attitude estimate and the vector type. | No accelerometer or gyroscope feed in this repo (see `inertial_sensors`, `motion_manager`, already re-exported in `sdk/src/lib.rs`). |

## User Notifications UI

| Row | API | BifrOSt path | Done | Still open |
|---|---|---|---|---|
| Media play and pause in notifications (Partial) | MediaControl, Command, Glyph | `springboard/sdk/src/usernotif.rs` (`media` module) over `springboard/ui/src/notifmedia.rs` | App path to the play and pause button model. Test: a hidden button takes no tap; a tap asks for the opposite of the playing state; the glyph follows `media_play`. | The shell does not draw the button on a card yet; no app sends the media notification. |

## Not changed

- Authentication Services: Sign in with Apple, credential import and export, account upgrades (n/a or Gap with no
  BifrOSt counterpart in this repo).
- Core Motion: raw accelerometer and gyroscope events (Gap), and the Apple-only sensor rows (n/a).
- User Notifications UI: the content app extension view (Gap) needs a shell-side view host; not started.

## Shared-file edits

- `springboard/sdk/src/lib.rs`: two `pub mod` lines (`sign_in`, `sensors`) after `data_scanner`.
