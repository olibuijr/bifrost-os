# G2 `wm-drm-vulkan`: the Vulkan render server on the DRM backend (partial)

Status: a first slice. The DRM backend opens the Vulkan render server on the scanout GPU when the switch is on, and
logs the result. Scanout still goes through Smithay's GLES `DrmCompositor`. The frame loop is not wired, so the
desktop does not yet composite with Vulkan on the DRM backend.

## What is implemented

- `springboard/wm/src/drm_vulkan.rs` (new, declared in `lib.rs`):
  - `render_node_name(entries)`: pure. Picks the `renderD*` entry (lowest number) from a card's `device/drm` listing.
    Only `renderD` followed by digits counts.
  - `render_node_for_card(card)`: reads `/sys/class/drm/<card>/device/drm` and places the render node in the card's
    `/dev/dri` directory.
  - `probe(card)`: `render_server::server::Server::new(Some(node), Bgra8Unorm)`. Returns the error text on failure.
- `springboard/wm/src/drm.rs`, `run()`: after `open_gpu`, when `render_server::selected()` is true, runs `probe` on
  the primary GPU and logs either `vulkan render server ready ...; scanout stays GLES` or
  `vulkan render server unavailable, GLES: <error>`. Both outcomes keep the GLES path. The probe result is dropped.

## Mapping to the contract

- Device on the scanout GPU's render node: done (`probe`).
- GLES stays the default and the fallback: done (the probe never changes the scanout path).
- Switch: `BIFROST_WM_RENDERER=vulkan` or `/etc/bifrost/renderer` says `vulkan` (`render_server::selected`).

## What is left (todo_left)

1. Keep the `Server` in `Drm` instead of dropping it, and add an `enum Renderer { Gles, Vulkan }` path in `render`.
2. Allocate the scanout ring with GBM for the primary plane's formats and modifiers (`OutputTargets::from_gbm`, ring
   of `RING` = 3, with `SAMPLED` usage for live glass). Today `DrmCompositor` allocates its own buffers.
3. Per frame: build the scene (`wm-scene`'s builder; `render_server::scene_build` does not exist in this tree yet),
   `OutputTargets::acquire`, `Server::frame` with the damage of the buffer age, and `Submission::sync_file`.
4. Atomic KMS commit with the sync_file as `IN_FENCE_FD`. Smithay 0.7's `DrmSurface` commit path does not take an
   in-fence in the API used here, so this needs a raw atomic commit through the DRM fd (or the Smithay commit
   with the property set added by hand). Check Smithay's `drm::surface` source before choosing.
5. Page-flip events (`DrmEvent::VBlank`) feed `FrameClock::vblank` and `OutputTargets::flipped`, and the frame
   callbacks come from `FrameClock`.
6. Failure paths: `Server::frame` error or commit `EBUSY` calls `OutputTargets::abandon`. A VT switch resets the ring.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

Only the probe is testable today:

1. `bifrost pc midget update --restart`, then `python3 tools/midget.py recovery` must say `phase=accepted`.
2. Set the switch: `ssh midget 'echo vulkan | sudo tee /etc/bifrost/renderer'` (or the env var
   `BIFROST_WM_RENDERER=vulkan` in the compositor's unit).
3. Restart the session and read the log: `journalctl -b -t bifrost-wm | grep vulkan`. Expect
   `vulkan render server ready on /dev/dri/renderD128; scanout stays GLES` (or the error text when it is missing).
4. The desktop must still show and respond normally, because scanout did not change.
5. Unit tests of the pure part: `cargo test -p bifrost-wm drm_vulkan` (run by the lead; not run in this slice).

## Build

`CARGO_TARGET_DIR=.../springboard/target-agents cargo check --offline -j 2 -p bifrost-wm --tests` passes.
`python3 tools/check-names.py` passes.
