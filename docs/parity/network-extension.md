# Network Extension parity

Official documentation: https://developer.apple.com/documentation/networkextension (macOS 10.10).

## Hotspot helper (team A5)

`springboard/sdk/src/hotspot_helper.rs` models the helper side of `NEHotspotHelper`. An app calls `Helper::register` with a
name and the command kinds it handles (`Evaluate`, `Authenticate`, `Maintain`, `Logoff`). The shell calls `deliver` with a
command for a network, and the app takes commands in order with `take_command`. `respond` builds the answer: a confidence
(`None`, `Low`, `High`) is accepted only for a successful evaluation, and every other answer must carry `Confidence::None`.

Not done: the system does not send commands yet, and no captive login page is driven through the helper. The accessory
join model (`springboard/sdk/src/accessory_hotspot.rs`) and the hotspot configuration model
(`springboard/sdk/src/hotspot_configuration.rs`) cover the other two rows of the hotspot family.

Built on parity/team-a-05; not yet verified on Midget.

## Remaining gaps

1. The system never sends helper commands, so the model has no radio or network transport behind it.
2. The App extension host (`NEAppExtensionConfiguration`) is still absent: no network provider can be shipped by an app.
