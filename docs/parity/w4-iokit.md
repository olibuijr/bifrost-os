# Wave 4: IOKit parity (power sources, serial ports)

Two `os`/`gap` rows of the IOKit matrix get an app-facing SDK API over capability the system layer already has.
Both are pure models with host unit tests. Nothing is wired into the shell or an app yet, and nothing has been built
or run on the VM or Midget.

## What was built

| Module | Role |
|---|---|
| `springboard/core/src/power_supply.rs` | Pure model. Reads the kernel power-supply class (`type`, `status`, `capacity`, `online`, energy and charge pairs, power and current) into one `PowerSource` per supply. Gives the percent (from capacity, or from the energy or charge pair when capacity is missing), the charging state, and whole-minute estimates to empty and to full. `sources_in` reads a class directory. |
| `springboard/sdk/src/power_sources.rs` | App API over the core model: `current()`, `current_in(dir)`, `battery()`, `first_battery(&[..])`, `time_remaining(&s)`. Re-exports `PowerSource`, `PowerState`, `SupplyKind`. |
| `springboard/core/src/serial_ports.rs` | Pure model. `classify` maps node names (`ttyS`, `ttyUSB`, `ttyACM`, `ttyAMA` with a numeric index) to a `SerialKind` and index. `ports_from` keeps only nodes linked to hardware and sorts by kind, then index. `scan_in` reads `/dev` and the tty class. |
| `springboard/sdk/src/serial_ports.rs` | App API over the core model: `ports()`, `ports_in(dev, tty_class)`, `port_named(&ports, name)`, `of_kind(&ports, kind)`. Re-exports `SerialKind`, `SerialPort`, `classify`. |

Registrations: one `pub mod` line each in `springboard/core/src/lib.rs` (`power_supply`, `serial_ports`) and in
`springboard/sdk/src/lib.rs` (`power_sources`, `serial_ports`).

Units follow the kernel ABI: energy in microwatt-hours, power in microwatts, charge in microampere-hours, current in
microamperes. Estimates are minutes computed as `amount * 60 / rate`, rounded down, with integer arithmetic only.

### Limits and decisions

- Power sources: the time estimates need a non-zero rate. Without one (for example a discharging battery with no
  `power_now` or `current_now`), both estimates are None. Percent is None for mains and USB supplies, which report no
  level.
- Serial ports: Linux has one node per port, so each `SerialPort` has one `path` (`/dev/ttyUSB0`). The official
  enumeration gives a dial-in and a call-out path per port; BifrOSt has no equivalent pair.
- A `ttyS` node is reported only when `/sys/class/tty/<name>/device` exists, so the empty onboard UART nodes that
  every Linux system creates are not listed as ports.
- Not built: opening a serial port, driving a modem (the official "Communicating with a Modem on a Serial Port"
  sample), and device add or remove events. The sandbox binds only `/dev/snd` and `/dev/dri`, so apps still cannot
  reach a serial node even with the listing.

## Official API names mirrored

- Power sources: the description keys of the official IOKit power-source API (current capacity, is charging, time to
  empty, time to full). The official IOKit documentation (`https://developer.apple.com/documentation/iokit`) does not
  list the power-source functions under its functions index, and the page path
  `https://developer.apple.com/documentation/iokit/iopowersources` returns 404 in the rendered page. The names are
  taken from the matrix reference (`IOPowerSources`), not from a verified official page.
- Serial ports: the official IOKit landing page lists the "Serial Ports" topic with the sample "Communicating with a
  Modem on a Serial Port" (`https://developer.apple.com/documentation/iokit/communicating_with_a_modem_on_a_serial_port`),
  which states the purpose (find and connect to a modem on a serial port). The matrix reference
  (`IOServiceMatching`, `IOBSDNameMatching`) is cited for enumeration; the sample body was not rendered.

## REA evidence

- `ev_c18bafc2538b7c3f924f6ce04b657ba43e392b51d71a7a3096bf4ae92a95dd7f`: rendered IOKit landing page. Headings:
  Serial Ports, Reference; the overview says the framework implements nonkernel access to IOKit objects.
- `ev_d503de99706afade350246320f79d04561ec11ea4649cf7d025827f9efacc634`: rendered
  `https://developer.apple.com/documentation/iokit/iopowersources`, which shows "The page you're looking for can't be
  found". It is evidence of the 404, not of an API page.

## Matrix rows closed

These are the rows for the lead to update centrally. Status is the new status proposed for each.

| Framework (slug) | Feature (as in `data/iokit.json`) | New status | BifrOSt path |
|---|---|---|---|
| iokit | Power sources and battery state | partial | `springboard/sdk/src/power_sources.rs` (over `springboard/core/src/power_supply.rs`) |
| iokit | Serial ports and modem | partial | `springboard/sdk/src/serial_ports.rs` (over `springboard/core/src/serial_ports.rs`); modem open not built |

## Tests

Each module has its own `#[cfg(test)] mod tests` with hand-worked values:

- `power_supply`: 30 Wh at 15 W is 120 minutes; 20 of 40 Wh at 10 W is 50 percent and 120 minutes to full; 3 Ah of 4 Ah
  at 1 A is 75 percent and 180 minutes; mains and USB supply kinds; capacity clamped to 100 and no estimate when full
  or without a rate; missing directory gives an empty list.
- `serial_ports`: kind and index classification, rejection of `ttyS`, `tty0`, `console` and similar names, unlinked
  nodes dropped, sort by kind then numeric index, missing `/dev` gives no ports.
- `power_sources` (SDK): first battery skips mains and keeps order; mains-only and empty lists give None; time
  remaining follows the state; missing class gives an empty list; `battery()` returns only battery kind.
- `serial_ports` (SDK): lookup by name, absent name, kind filter order, empty kind filter, missing directory.

No cargo or rustc run was made (per the task rules), so the tests have not been compiled or executed.
