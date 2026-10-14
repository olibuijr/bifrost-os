# Parity sdk-os-07: system service surfaces for apps

Fifteen matrix rows were `os`: the system or a daemon handled the capability, and apps had no SDK surface for it. This
note lists what the SDK now offers for each one, what is wired, and what needs the lab machine.

## What is new

Module `bifrost_app::system` (`springboard/sdk/src/system/`). Each module is pure: it validates, decodes or describes
what a system service applies, and it opens no socket, device or file that its arguments do not name.

| Module | Mirrors | Wired to | Not yet |
|---|---|---|---|
| `update_integrity` | download checks for updates | the same size and SHA-256 check as `shell::update` | the manifest is not signed |
| `ipv4_config` | IPv4 method, address, prefix and router | netlink and DHCP in the shell | no app can send a configuration |
| `dma_buf` | linear dma-buf plane descriptors | the window protocol's buffer parameters | the file descriptor stays with the system |
| `sprite_shader` | shader source, attributes and uniforms | none: the presenter runs no application shader | drawing |
| `effect_nodes` | crop, transform and effect nodes | none: the shell draws its own effects | an offscreen filtered buffer |
| `execution_policy` | launch refusal domain and codes | `bare::app_policy::Policy::from_permissions` | sandbox setup failures are unnamed |
| `app_sandbox` | the capabilities a policy grants | `bare::app_policy::Policy` | apps cannot change the sandbox |
| `authorization_lock` | the lock state of a privileged gate | privd `login.authenticate` replies (`wrong_password`, `locked_out`, `no_password`) | the lock view is drawn by the shell |

## Existing modules now cited by rows

- `keychain_protection`: the six data protection classes and their readability rules. The store does not enforce them yet.
- `local_auth`: the authentication context, its policies and errors. The shell draws the prompt. There is no biometry.
- `network_path`: the connectivity path, interface kinds and the change monitor that reports only real changes.
- `reachability`: the default-route check and the TCP probe.
- `device_registry`: device listing by class, property matching and arrival or removal changes.
- `display_link` and `pixel_buffer`: refresh timing and lockable pixel buffers.

## Tests

`springboard/sdk/tests/parity_sdk-os-07.rs` covers every module above. The tests are headless: no network, no device,
no display and no clock. The launch-policy and sandbox tests run on Linux only, because the shell's app policy is
Linux-only.

## Needs the lab machine

- Confirming that the shell's services apply what the models validate (netlink changes, the update check, privd replies).
- The keychain protection classes need a store that enforces them before an app can rely on them.
- The GPU presenter and shader paths need a device before any app-facing drawing can use them.
