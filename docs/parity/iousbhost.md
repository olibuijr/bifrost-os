# IOUSBHost

Framework: IOUSBHost, https://developer.apple.com/documentation/iousbhost (macOS 10.15).
Branch: parity/team-k-accessory-access (built on arm). Not yet verified on Midget.

## What was built

- `springboard/core/src/usb_claim.rs` (`bifrost_core::usb_claim`): the claim plan for one interface alternate setting.
  `plan_claim` takes the parsed device and configuration descriptors and returns:
  - the control requests in order: `SET_CONFIGURATION` (skipped when the device already reports that configuration)
    and `SET_INTERFACE`, built with the existing `set_configuration` and `set_interface` helpers;
  - the pipes the claimed interface opens, from the existing `usb_transfer::Pipe` geometry.
  `ClaimPlan::pipe` and `ClaimPlan::pipes_of` look them up. Errors: no configuration, no such interface, a duplicate
  endpoint address. Seven host unit tests.
- `springboard/sdk/src/usb_device.rs` (`bifrost_app::usb_device`): re-exports the plan and the descriptor, setup
  packet and pipe types an app reads.

## Rows changed

- Device driver access (claim, configure, descriptors, device requests): `partial`. The claim is now planned for apps.
  Nothing sends the requests.

## Not built, and why

- Sending requests and opening pipes: the app sandbox binds no `/dev/bus/usb` (`springboard/src/bare/app_policy.rs`),
  and no transport exists. A transport needs an owner decision on whether a desktop gives apps raw USB at all. The
  matrix triage note says likely n/a; the framework's main uses (cameras, audio, printers, input) are covered at the
  system layer.
- Interface, pipe and stream I/O (`usb_pipe.rs`): unchanged; the model is there, the transport is not.
- Scanners: no scanner class exists in the framework's documentation, so nothing was added.
- Host controller and state machines: n/a, kernel-owned on Linux.

## Tests

Host unit tests written in `springboard/core/src/usb_claim.rs`, not run in this round:
`unconfigured_device_gets_set_configuration_then_set_interface`,
`already_configured_device_skips_set_configuration`, `a_different_active_configuration_is_selected_again`,
`pipes_follow_the_claimed_alternate_setting`, `pipes_of_filters_by_transfer_type`,
`missing_interface_or_alternate_is_refused`, `zero_configuration_value_or_no_configurations_is_refused`,
`duplicate_endpoint_addresses_are_refused`, `claim_errors_have_messages`.

## REA evidence

`ev_a8792b7a789634fc2e35b8dfc894673ef8d9fe7860fc71b9c0e72e944432958c` (landing page, from the matrix row).
