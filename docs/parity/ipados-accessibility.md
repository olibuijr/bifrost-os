# Tablet accessibility: custom rotors and Guided Access

Status: partial, built on `parity/team-p-accessibility`, not yet verified on the tablet. The matrix rows are in
`docs/parity-matrix/ipados/data/uikit-accessibility.json` ("Rotors and navigation", "Guided Access") and
`docs/parity-matrix/ipados/data/uikit-app-scene-lifecycle.json` ("Guided Access", `UIGuidedAccessRestrictionDelegate`).

## Custom rotors

A custom rotor is a named list of elements the screen reader steps through, such as "Unread" in a mail list. An app
declares its rotors with `Ctx::set_custom_rotors`. Each item is a `Node::id` the app reported in `controls`, listed in the
order the user should reach them.

```rust
use bifrost_app::accessibility_rotor::CustomRotor;

ctx.set_custom_rotors(vec![CustomRotor::new("Unread", unread_ids.clone())]);
```

What the code does:

- `springboard/ui/src/ax_rotor.rs` holds the rotor model. `clean` trims names, drops empty or repeated rotors and repeated
  items, and applies the caps (8 rotors, 256 items, 40 bytes of name). `RotorCycle` orders the rotors the user cycles
  through: "All", the standard rotors (`ax::STANDARD_ROTORS`), then the app's custom rotors in declared order. Indices wrap.
  `step_custom` moves through a custom rotor with `AxCustomRotor::step`.
- `springboard/proto/src/axrotor.rs` carries the list as `ToShell::AxRotors` (tag `0x213`). Oversized lists are cut on
  encode and refused on decode.
- `springboard/src/shell/apphost.rs` keeps the cleaned list per app. `springboard/src/shell/accessibility/app_rotors.rs`
  builds the app's `RotorCycle` from its host.

Not yet done: the screen reader's rotor keys still cycle only the standard rotors. Switching them to `RotorCycle` (and
listing app rotors in `ax list`) is the next step.

## Guided Access

Guided Access is single-app mode. An app asks for a session with its own restrictions, and the user decides whether each
restriction is allowed.

```rust
use bifrost_app::guided_access::{GuidedAccessChange, GuidedRestriction};

// in App::configure or on a user action
ctx.request_guided_access(&[
    GuidedRestriction::new("touch", "Touch").with_detail("Lock the touch panel while the lesson runs"),
    GuidedRestriction::new("sleep", "Sleep").set_allowed(false),
]);

// in App::guided_access
fn guided_access(&mut self, ctx: &Ctx, change: GuidedAccessChange) -> bool {
    match change {
        GuidedAccessChange::Session(active) => { /* the session started or was refused (false) */ }
        GuidedAccessChange::Restriction { id, allowed } => { /* the user changed one restriction */ }
    }
    true
}

ctx.end_guided_access();
```

Setting: Settings > Accessibility > Guided Access (`a11y_guided_access` in `settings.ini`), off by default. The shell refuses
a request while the setting is off and answers `Session(false)`.

What the code does:

- `springboard/sdk/src/guided_access.rs` is the app side: the request and end calls on `Ctx`, and the `GuidedAccessChange`
  the `App::guided_access` hook receives.
- `springboard/proto/src/guided.rs` carries `ToShell::GuidedAccessRequest` (tag `0x170`), `ToApp::GuidedAccessState`
  (`0x171`) and `ToApp::GuidedRestriction` (`0x172`). A restriction identifier is at most 64 bytes, a title or detail at most
  160, and a session declares at most 16 restrictions.
- `springboard/src/shell/accessibility/guided.rs` holds the session model: one app at a time, refused when the setting is
  off or another app holds the session, the app's own restart replaces its restrictions, a restriction's state is recorded,
  and an unknown restriction permits everything. The shell answers each request on the tick and ends a session when its
  app exits.
- `springboard/src/shell/pages.rs` and `springboard/src/shell/settings.rs` add the Settings switch;
  `springboard/src/shell/accessibility/mod.rs` stores it.

Not yet done: the session options sheet where the user changes restrictions (the shell's `guided_set_restriction` is the
entry point for it), and enforcement that keeps Home, the Dock and other windows out of a session. Until the desk reads
`Guided::permitted`, a running session records its state but does not limit the desk.

## Desktop behaviour

The request, the restrictions and the session model do not depend on the form factor. On the desktop the same setting and
the same app calls apply; the restriction identifiers are app defined, so pointer and keyboard restrictions are declared by
the app in the same way as touch ones.

## Tests

Unit tests cover the rotor model (cleaning, caps, wrapping, the order of All, standard and custom rotors), the wire round
trips and caps for the three new tags, and the session model (setting off, one app at a time, restart by the holder,
restriction states reported once). They have not been run on the tablet.
