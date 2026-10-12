# Wave 4: System Configuration

Two gap rows of the System Configuration framework now have a pure Rust model. Both are std-only and have no
hardware, network or system-file access. Neither is wired into the shell or an SDK client yet, and neither was built
or run in this wave (the wave rules forbid builds), so the tests are written but not executed.

## What was built

### Computer name and local host name (`springboard/core/src/host_name.rs`)

- `ComputerName`: the free-text name shown to people. Trimmed; 1 to 255 bytes; no control characters.
- `LocalHostName`: one DNS label (1 to 63 ASCII letters, digits or hyphens, no leading or trailing hyphen), stored
  in lower case.
- `LocalHostName::from_computer_name`: derives the label from the computer name (non-alphanumeric characters become
  hyphens, runs collapse, ends are trimmed, the result is cut to 63 characters, and the default `bifrost` is used
  when nothing is left).
- `HostNames`: keeps both names in step. `set_computer_name` re-derives the local name; `set_local_host_name` changes
  only the local name. A refused change leaves both names as they were.

Mirrors the official `SCPreferencesSetComputerName`, `SCPreferencesSetLocalHostName`,
`SCDynamicStoreCopyComputerName` and `SCDynamicStoreCopyLocalHostName` (the System Configuration collection,
https://developer.apple.com/documentation/systemconfiguration). The model covers the setters and the derived local
name, but not the dynamic-store copies, which need the system layer.

### Network services, sets and locations (`springboard/core/src/network_sets.rs`)

- `NetworkService`: id, name, interface, enabled flag.
- `NetworkSet`: id, name, ordered list of service ids.
- `NetworkLocations`: services, sets and the id of the current set. It provides `add_service`, `remove_service`,
  `set_enabled`, `add_set`, `remove_set` (the current set cannot be removed), `set_current`, `set_service_order`
  (every id must exist and appear once; a refused order leaves the old one), and `active_services` (the enabled
  services of the current set, in order).

Mirrors the official `SCNetworkServiceCreate`, `SCNetworkSetCreate`, `SCNetworkSetSetCurrent`,
`SCNetworkSetSetServiceOrder` and `SCNetworkServiceCopyAll` (SCNetworkConfiguration,
https://developer.apple.com/documentation/systemconfiguration/scnetworkconfiguration). The model is in memory only:
it does not persist, and it does not drive `netd`.

## REA evidence

- System Configuration landing page (`https://developer.apple.com/documentation/systemconfiguration`):
  `ev_f8c2edbe603c416312a2178bcbfe4460f398630ba7079487cfa3f7922adabe3e`. The rendered page lists SCNetworkConfiguration,
  SCNetworkReachability, SCPreferences and the other collections, and describes the reachability and configuration
  purpose.
- DocC checks (`python3 -I tools/parity/docc.py systemconfiguration`, `.../scnetworkconfiguration`,
  `.../scnetworkset`, `.../scpreferences`) confirm the function names above.

## Matrix rows closed

These rows go to `partial` (not `parity`: the models are not wired to an app, the shell or the network service).
The lead updates the matrix centrally.

| Framework slug | Feature (exact) | New status | BifrOSt path |
|---|---|---|---|
| system-configuration | Computer name and local host name | partial | springboard/core/src/host_name.rs |
| system-configuration | Network services, sets and locations | partial | springboard/core/src/network_sets.rs |

## Tests

- `host_name.rs`: six tests (computer name checks, derived local name examples such as `Olafs's  Desk!` ->
  `olafs-s-desk`, cut to 63 without a trailing hyphen, DNS label rules, set-computer-name moves the local name, and
  refused names leave both unchanged).
- `network_sets.rs`: seven tests (added services follow the set order, order replacement and disabled services,
  refused orders, switching sets, current-set removal, service removal from every order, duplicate and empty ids).

## Not done

- No SDK client exists for these models; apps cannot read or set the computer name or the services yet.
- `netd` does not use the service list, and nothing persists the locations.
- Reachability (`SCNetworkReachability`) is not in this wave.
