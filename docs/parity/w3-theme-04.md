# Wave 3, theme 04: hands-free profile (audio gateway and device)

## What was built

`springboard/ui/src/hfp.rs` (`bifrost_ui::hfp`, pure std, no I/O). It is a model of the hands-free
profile's AT-command dialogue, in both roles:

- `Command` and `parse_command`: reads the device-to-gateway lines (`AT+BRSF`, `AT+CIND=?`, `AT+CIND?`,
  `AT+CMER`, `ATA`, `AT+CHUP`, `ATD<number>;`, `AT+VGS`, `AT+VGM`). Keywords are case-insensitive. Anything else
  is kept as `Unknown`.
- `Reply` and `parse_reply`: reads `OK`, `ERROR`, `RING`, `+BRSF`, `+CIND` values, `+CIEV` and other lines.
- `Gateway`: answers each command with its reply lines and an optional `Event` for the shell (answer, hang up,
  dial, speaker volume, microphone gain). It also reports indicator changes as `+CIEV` lines when reporting is on.
- `HandsFree`: the device side of the service-level connection. It runs the steps Features, IndicatorList,
  IndicatorStatus, Reporting and Connected, and returns the next command line for each. An `ERROR` reply
  moves it to `Failed`, and `connect()` can start it again.
- `Indicators` and `Indicator`: the seven indicators in list order, with their ranges, the `+CIND` list and
  status replies, and range checks.
- `ag_feature` and `hf_feature`: feature bit masks for the two roles.
- `Framer`: splits a byte stream into lines at CR or LF and keeps an unfinished tail.

The shell's radio, RFCOMM transport and SCO audio are not part of this module. Nothing here touches hardware.

## Official API names mirrored

Apple's `IOBluetoothHandsFree` page (macOS 10.7 and later; DocC via `tools/parity/docc.py iobluetooth/iobluetoothhandsfree`):

- `init(device:delegate:)`, `connect()`, `disconnect()`, `connectSCO()`, `disconnectSCO()`, `isConnected`,
  `isSCOConnected()`, `indicator(_:)`, `setIndicator(_:value:)`, `supportedFeatures`, `deviceSupportedFeatures`,
  `deviceCallHoldModes`, `inputVolume`, `outputVolume`, `isInputMuted`, `isOutputMuted`, `delegate`.

Mapped to this module: `HandsFree::connect` and `HandsFree::step` (connect and state), `Indicators::get` and
`set` (`indicator` and `setIndicator`), `HandsFree::ag_features` and `Gateway::hf_features` (feature masks),
`Gateway::speaker` and `mic` (volumes). The SCO link, call-hold modes, SMS and the Objective-C delegate are not
modelled. The page's `init(device:delegate:)` has no counterpart because the device is the transport here.

## REA evidence

- `ev_3991f46c0bab24f85765b051480429e8106fcbba3ab94405a3c11d5142be2d43`: rendered page
  `https://developer.apple.com/documentation/iobluetooth/iobluetoothhandsfree`.

## Matrix rows closed

| Framework slug | Feature (exactly as in data/iobluetooth.json) | New status | BifrOSt path |
|---|---|---|---|
| iobluetooth | Hands-free profile (audio gateway and device) | partial | springboard/ui/src/hfp.rs |

Partial, not parity: the module models the dialogue and the indicators only. Without an RFCOMM transport and a
SCO audio path, no app can run a call. Indicator list ranges are checked against the reply, but the `+CIND` list
reply from a peer is accepted without being parsed (see gaps). `AT+CHLD` (three-way call handling) is not
modelled.

## Gaps left

- The `+CIND: (...)` list reply from the gateway is ignored, not parsed. The device uses its own range table.
- Call-hold (`AT+CHLD`), voice recognition (`AT+BVRA`), caller ID (`AT+CLIP`) and in-band ring are not modelled.
- Memory dial (`ATD>`) and the SMS services page are not modelled.
- The `+VGS`/`+VGM` replies to the peer's unsolicited volume messages are not modelled.
- The module is not reachable from apps yet: there is no `springboard/sdk/src` API for it.

## Tests

`cargo` was not run in this round (the brief forbids builds). The `#[cfg(test)]` module in `hfp.rs` has twelve
tests, worked out by hand from the code rules: command parsing with case folding, the CMER fourth field, dial
with a semicolon, volume range, the indicator list and status strings, indicator range and change reporting,
gateway handshake and CIEV output (feature mask 4 + 64 = 68), the device's full connection sequence, rejection and
reconnect, the malformed value count, and the framer's CR and LF splitting.
