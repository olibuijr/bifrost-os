# Wave 3: GameplayKit gap rows (pure Rust)

Branch `parity/w3-gameplaykit`, on top of `parity/integration` (waves 1 and 2).

## What was built

Three pure models in `springboard/ui/src` (bifrost-ui). Each is std-only, has no unsafe code, and has a
`#[cfg(test)]` module with hand-worked expected values. None is wired into an app or the shell yet.

| Module | Mirrors (official API names) | What it does |
|---|---|---|
| `springboard/ui/src/noise.rs` | GKNoise, GKNoiseSource, GKCoherentNoiseSource, GKPerlinNoiseSource (value basis only), GKBillowNoiseSource, GKRidgedNoiseSource, GKCheckerboardNoiseSource, GKConstantNoiseSource | Seeded value-noise lattice with smooth, billow and ridged octave shapes, plus checkerboard and constant bases. Octave amplitudes are normalised. |
| `springboard/ui/src/quadtree.rs` | GKQuadtree, GKQuadtreeNode | Point quadtree with half-open rectangles, split at a capacity, depth limit, insert, remove, rectangle query and clear. |
| `springboard/ui/src/rule_system.rs` | GKRuleSystem, GKRule (predicate and action form; GKNSPredicateRule not mirrored) | Rules with salience over named numeric facts. Sweeps repeat until nothing fires. Each rule fires at most once per evaluation. |

Not built in this wave: Perlin gradient noise and Voronoi (GKVoronoiNoiseSource) noise, octrees and R-trees,
decision trees, and strategists.

Module names are new. No `noise`, `quadtree` or `rule_system` module existed before this wave.
`content_rules.rs` is an existing file and was not touched.

## Official documentation

- GameplayKit overview: https://developer.apple.com/documentation/gameplaykit
  (DocC: `python3 -I tools/parity/docc.py gameplaykit --depth 1`)
- REA evidence of the rendered page (landing page with all topic sections):
  `ev_4a79163fbf365442f04943ba2d3c3ddf874f21b4d2d280cf079fea137d3fa837`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/gameplaykit`)

## Matrix rows closed

Feature text is copied exactly from `docs/parity-matrix/data/gameplaykit.json`. The new status is `partial`
because each row still lacks the parts that were not built.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| gameplaykit | Procedural noise | partial | springboard/ui/src/noise.rs |
| gameplaykit | Spatial partitioning | partial | springboard/ui/src/quadtree.rs |
| gameplaykit | Decision trees and rule systems | partial | springboard/ui/src/rule_system.rs |

## Tests (expected values worked by hand; not run, per the wave brief)

- noise: a constant basis keeps its value; ridged of 0.25 is 0.75 and billow of 0.25 is -0.5; checkerboard
  parity at four points including a negative cell; value noise equals the lattice value at integer points and
  the mean of two lattice values halfway along an edge; fractal output stays in [-1, 1].
- quadtree: insert count and rejected maximum edge; four quadrant queries; half-open edge membership at x = 4;
  remove and its misses; a depth-0 tree keeps all points; touching rectangles do not intersect.
- rule_system: a three-rule chain (health 3 becomes 13, alarm 1, state 2); nothing fires at health 7; a rule
  fires once; salience order with ties in insertion order; a later sweep fires a rule that an earlier sweep
  enabled; missing-fact reads and accumulated adds.

No builds or test runs were done in this wave.
