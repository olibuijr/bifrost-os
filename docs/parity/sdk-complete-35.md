# Parity sdk-complete-35: scene nodes, transport policy, result codes, call cards, picker requests

Round w8 finished nine partial rows in the SpriteKit, security, call, GameplayKit and External Accessory areas. Each
model is pure: no clock, no network, no device. Tests are in `springboard/ui/tests/parity_sdk-complete-35.rs` and
`springboard/sdk/tests/parity_sdk-complete-35.rs`.

## What each model mirrors

- `bifrost_ui::sk_node` (`NodeTree`): a tree of positioned nodes with rotation, scale, alpha, z order and hidden
  subtrees, shape, label and image content, and an optional camera node. `draw_list` returns the visible items in
  paint order with view frames. Each node owns an action runner (`sk_action::Runner`).
- `bifrost_ui::sk_action::Keyframes`: each `Key` carries its own timing (hold or linear) and an easing `Curve` for the
  segment it starts.
- `bifrost_app::transport_policy` (`TransportPolicy`): plain http is allowed only for excepted hosts, with optional
  subdomain coverage; `check_redirect` refuses an https-to-http hop unless the host is excepted.
- `bifrost_app::security_status`: the result codes the keychain and trust services report, each with a meaning and a
  category. The keychain's own error codes are checked against this table in the tests.
- `bifrost_ui::call_ui::CardStack`: ringing cards, oldest on screen; `prune` drops cards whose calls the call
  controller no longer rings.
- `bifrost_app::bluetooth_picker::PickerRequest`: an app's name filter and device kinds; `Outcome::address` gives the
  app the chosen address only.
- `bifrost_ui::gameplay_values`: the value types and option sets are now covered by headless tests.

## What is wired

- The URL client refuses an https request redirected to plain http (`url::resolve`).
- The call card model and the picker model are not yet used by the shell or by apps.

## What needs the lab machine

- Drawing a node tree into a window surface, and checking the result on Midget.
- Showing call cards on the shell surface, and checking them on Midget.
- Sending a picker request across the shell boundary, and the pairing sheet on the device.
- Reading an exception list from the app manifest.
- `kSecMatchLimitAll` needs a shell list operation.
