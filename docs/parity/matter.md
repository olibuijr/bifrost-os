# Matter parity

Framework: Matter (official reference: https://developer.apple.com/documentation/matter, macOS 13.0+).
Status after this round: **partial** (was gap). Matrix data: `docs/parity-matrix/data/matter.json`.

## What was built

`springboard/core/src/matter.rs` (std only, no crates, no unsafe, no hardware or network). It adds the
two pure models that Matter onboarding and discovery need. Nothing else is wired up yet.

1. **Setup payloads** (feature "Onboarding a Matter device", now partial).
   - `parse_qr` decodes the QR setup code: the "MT:" prefix, Base38 (`base38_decode`), and the 88-bit
     payload (version, vendor ID, product ID, commissioning flow, discovery capabilities, 12-bit
     discriminator, 27-bit passcode, padding). It rejects a wrong prefix, version, flow 3, non-zero
     padding, a wrong length and invalid passcodes.
   - `parse_manual_code` decodes the 11 digit manual pairing code and the 21 digit form that adds the
     vendor and product IDs. Spaces and hyphens are ignored. The Verhoeff check digit is verified
     (`verhoeff_check`).
   - `passcode_is_valid` applies the setup passcode rules (no zero, no repeated digit, not 12345678 or
     87654321, at most 99999998).
   - Vector check: `MT:-24J0AFN00KA0648G00` decodes to vendor 0xFFF1, product 0x8001, discriminator 3840,
     passcode 20202021. The manual code `34970112332` decodes to short discriminator 15 and passcode
     20202021. Both were checked independently with a Python reference before the Rust was written.

2. **Commissionable and operational discovery** (feature "Local network discovery", now partial).
   - `service_kind` recognises `_matterc._udp` (commissionable) and `_matter._tcp` (operational), with or
     without a trailing dot or `.local`.
   - `commissionable_from_txt` parses the 16 hex digit instance name and the TXT keys D, VP, CM, DN and PH.
     Unknown keys and value-less attributes are skipped.
   - `operational_from_instance` parses `<compressed fabric>-<node>` in hex.

## Official API names mirrored

Onboarding: `MTROnboardingPayloadParser`, `MTRQRCodeSetupPayloadParser`, `MTROptionalQRCodeInfo`.
Discovery: the controller-side commissionable and operational service lookup that the framework performs
internally. The Rust names are plain (`parse_qr`, `parse_manual_code`, `commissionable_from_txt`), because
the framework's Objective-C class names are not allowed in source (owner rule).

## REA evidence

- `ev_1c491308595380b42b0236b16cdb1db42b103ac4c9db808a74e8bb7912c6298d`: rendered official page
  (developer.apple.com/documentation/matter), captured with `tools/parity/rea-page.sh`. It states that
  commissioning provides credentials and that accessories expose clusters such as On/Off and Level Control.
- `ev_4fba8af2b79b1f68820857e35d218726fe5948025c83cf73ddc4ee1a9725412c`: the evidence id already in
  `matter.json` (`rea_evidence`), kept unchanged.
- `docc.py matter/onboarding-a-matter-device` gives the official abstract only ("Prepare your app to
  discover and control a Matter device"). The onboarding article body is not in the DocC index.

## Remaining gaps

- No controller: no fabric, operational certificates, commissioning session, or storage. (`MTRDeviceController`,
  `MTRFabricInfo`.)
- No Thread: no border router, no network dataset, no OpenThread.
- No attribute or command client and no subscriptions (`MTRDevice`, `MTRReadParams`, `MTRSubscribeParams`).
- No cluster model. Typed clusters (about 200 in the framework) are not started.
- No mDNS or DNS-SD transport. The parsers have no responder or querier, and apps cannot browse services.
  The only LAN beacon is `springboard/src/net/share.rs`.
- No camera or BLE source for setup codes. The Bluetooth module does classic pairing only.
- No SDK surface or Home-style app. Whether to add one is a product decision.

## Tests

`#[cfg(test)] mod tests` in `springboard/core/src/matter.rs` covers: the QR vector, a packed round trip
through Base38, QR rejections (prefix, version, padding, flow, passcode), Base38 character and group errors,
passcode validity, the Verhoeff example, the 11 digit manual vector, manual-code rejections, the 21 digit
form, commissionable TXT parsing and rejection, and service names with operational instances. These were not
run in this round (no builds in this stage).
