# Wave 4: IOSurface gap rows (pure models)

Branch `parity/w4-iosurface`, on top of `parity/integration`. Three gap rows of the IOSurface framework
(`docs/parity-matrix/data/iosurface.json`) get a pure-Rust model each. None is wired into the shell or an
app yet, and nothing was built or run (wave rule: no builds in this round). All three rows move from
`gap` to `partial`.

## What was built

| Module | Official API mirrored | What it does |
|---|---|---|
| `springboard/ui/src/surface_use.rs` (`UseCount`, `UseError`) | `incrementUseCount()`, `decrementUseCount()`, `localUseCount`, `isInUse` | Counts holders of a buffer. Increment returns the new count. Decrement at zero returns `Underflow`, and increment at `u32::MAX` returns `Overflow`. The count never wraps. |
| `springboard/ui/src/surface_attach.rs` (`Attachments`, `Attachment`) | `setAttachment(_:forKey:)`, `attachment(forKey:)`, `removeAttachment(forKey:)`, `allAttachments()`, `removeAllAttachments()` | String-keyed dictionary of `Int`, `Text` or `Flag` values. Keys are kept sorted. `set` returns the replaced value. |
| `springboard/core/src/surface_registry.rs` (`SurfaceRegistry<T>`, `SurfaceId`) | `surfaceID`, lookup by ID (`IOSurfaceLookup` family) | Hands out IDs from 1 upward. A removed ID is never reused. `register` returns `None` when the ID space is used up. |

Scope limits, which are why the rows are `partial` and not `parity`:

- The registry lives in one process. Cross-process lookup needs a system service that does not exist yet.
- Attachments hold no colour space or ICC data. The colour-space row stays `gap`.
- No app-facing SDK type wraps these models yet. The `os` rows (fd passing, memfd) are unchanged.

## REA evidence (rendered official pages)

- `ev_62d0e2d6aec861420f1e147aa1e90b299b48173db94f1d68eea05aa48b61ca65`: IOSurface framework landing page
- `ev_08c99f235641dd1af9b14deefb7280fe592b35baebfc3192f44d8316c02a4385`: IOSurface class page (use-count, attachment and `surfaceID` members)
- `ev_792c9dc56ffa439fa391907ae826ff99ea5a4817438b69c12d017aa560dfa009`: IOSurfacePropertyKey page

DocC sources: `python3 -I tools/parity/docc.py iosurface` and `python3 -I tools/parity/docc.py iosurface/iosurface`.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| iosurface | Use counts and in-use state | partial | springboard/ui/src/surface_use.rs |
| iosurface | Surface attribute dictionary | partial | springboard/ui/src/surface_attach.rs |
| iosurface | Global buffer lookup by ID | partial | springboard/core/src/surface_registry.rs |

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked expected values (5 to 6 tests per module).
Not run: no builds were run in this round.
