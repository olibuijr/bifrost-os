# Parity wave 3, theme 07: scene and scene lifecycle (GameplayKit part)

Theme: `docs/parity-matrix/themes.json` entry `theme-07`, 30 gap rows across SpriteKit, SceneKit, GameplayKit, Spatial,
Screen Saver and Game Controller. This pass closes four GameplayKit rows, all of them pure data models in the UI crate.
The scene, rendering and physics rows (SpriteKit and SceneKit) and the Spatial and Screen Saver rows are not touched:
they need a scene renderer or a 3D pipeline that does not exist yet, and building a stub for them would not be honest.

## What was built

| Gap | Module | What it does |
|---|---|---|
| Entities and components (partial) | `springboard/ui/src/ecs.rs` | `Component` trait with a kind name and a default `update`. `Entity` owns at most one component per kind, replaces a same-kind component on add and returns the old one, and updates its components in the order they were added. `ComponentSystem` manages one kind: it holds entity ids in insertion order, skips ids that are missing from the caller's table, and reports how many components it updated. |
| Spatial partitioning (partial, quadtree only) | `springboard/ui/src/quadtree.rs` | `Quad` is an axis-aligned rectangle with both edges included. `Quadtree<T>` stores objects at points, splits a full leaf into four equal quadrants, stops splitting at a depth limit, refuses points outside its bounds, and answers region queries by visiting only the quadrants that intersect the region. Octree and R-tree are not built. Removal is not built either. |
| Procedural noise (partial) | `springboard/ui/src/noise.rs` | `NoiseSource` with constant, checkerboard, single-octave gradient (Perlin), and fractal sums of gradient noise in three folds (plain, billow, ridged), plus Voronoi (distance to the nearest feature point). Every source is deterministic from its seed and stays in -1..=1. `NoiseMap` samples a source on a grid with an origin and a sample distance. Noise combinators (add, multiply, displace, turbulence, terraces, transforms) are not built. |
| Decision trees and rule systems (partial) | `springboard/ui/src/decision.rs` | `DecisionNode` (question on an attribute, or action leaf) with `Test::Equals` or `Test::Where` branches, and `DecisionTree::find_action` that follows the answers from the root. Learned trees and randomised branches are not built. |
| Decision trees and rule systems (partial) | `springboard/ui/src/rules.rs` | `RuleSystem` with rules that have a predicate over the system and an `Action` (assert or retract a fact). Facts carry a membership grade clamped to 0..=1. `evaluate` fires the highest-salience matching rule, re-evaluates after each firing, and fires each rule at most once until `reset`. Fuzzy minimum and maximum over named facts are included. Data-driven predicates (NSPredicate-style strings) are not built: predicates are Rust closures. |

Each module is one `pub mod` line in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests` block with
behaviour tests whose expected values were worked out by hand from the rule each code path implements.

Decisions made here, not taken from the official pages:

- Fact grades are clamped to 0..=1, so repeated assertion saturates at 1. A retraction that lowers a grade to 0 or below
  removes the fact.
- In `rules.rs`, a rule fires at most once per `reset` cycle, and the highest salience wins with ties going to the
  earlier-added rule.
- Noise values are clamped to -1..=1. The Perlin source is scaled by sqrt(2) so its raw range fills that interval.
  Voronoi maps a distance cut at 1 to -1..=1. Other noise libraries use different scales, so values are not comparable.
- `Quadtree` edges are inclusive. A point on a shared edge goes to the first quadrant in the order lower-left,
  lower-right, upper-left, upper-right.

## Official API names it mirrors

From the GameplayKit landing page (`https://developer.apple.com/documentation/gameplaykit`), the Entities and components,
Spatial Partitioning, Procedural Noise, Decision Trees and Rule Systems sections, and the symbol pages:

- `GKEntity` (`init()`, `components`, `addComponent(_:)`, `update(deltaTime:)`, `component(ofType:)`,
  `removeComponent(ofType:)`); mirrored as `Entity::add`, `Entity::update`, `Entity::component`, `Entity::remove`.
  Apple's `component(ofType:)` takes a class; ours takes a kind string, which is a deliberate difference.
- `GKComponent`; mirrored as the `Component` trait (`kind`, `update`).
- `GKComponentSystem` (`init(componentClass:)`, `addComponent(foundIn:)`, `removeComponent(foundIn:)`, `update(deltaTime:)`);
  mirrored as `ComponentSystem::new`, `add_entity`, `remove_entity`, `update`.
- `GKQuadtree` (`init(boundingQuad:minimumCellSize:)`, `add(_:at:)`, `elements(in:)`, `remove(_:)`), with `GKQuad` as
  the bounds type. Mirrored as `Quadtree::new`, `insert`, `query`, and `Quad`. Minimum cell size is replaced by a depth
  limit.
- `GKNoiseSource`, `GKNoise`, `GKNoiseMap`, `GKBillowNoiseSource`, `GKPerlinNoiseSource`, `GKRidgedNoiseSource`,
  `GKVoronoiNoiseSource`, `GKCheckerboardNoiseSource`, `GKConstantNoiseSource`; mirrored as `NoiseSource`, `NoiseMap`
  (with `NoiseSource::Billow`, `Perlin`, `Ridged`, `Voronoi`, `Checkerboard`, `Constant`). `GKNoise` as a combinable
  object is not mirrored: sources are values, not a graph of noise objects.
- `GKDecisionTree` (`init(attribute:)`, `findAction(forAnswers:)`), `GKDecisionNode` (`createBranch(value:attribute:)`,
  `createBranch(predicate:attribute:)`); mirrored as `DecisionTree::new`, `find_action`, `DecisionNode::question`,
  `DecisionNode::action`, `DecisionNode::branch`, `Test::Equals`, `Test::Where`.
- `GKRuleSystem` (`init()`, `state`, `add(_:)`, `removeAllRules()`, `evaluate()`, `agenda`, `executed`, `reset()`,
  `assertFact(_:)`, `assertFact(_:grade:)`, `retractFact(_:)`, `retractFact(_:grade:)`, `grade(forFact:)`,
  `minimumGrade(forFacts:)`, `maximumGrade(forFacts:)`), `GKRule` (`init(predicate:assertingFact:grade:)`,
  `init(predicate:retractingFact:grade:)`, `salience`); mirrored as `RuleSystem`, `Rule`, `Action`. Apple's `state`
  dictionary is replaced by the typed fact set.

The docs were read with `python3 -I tools/parity/docc.py gameplaykit ...` (the DocC JSON). The symbol-level behaviour
of `addComponent(_:)` replacing a same-class component was not verified against its own page in this pass; the
replace-on-add rule in `ecs.rs` is our decision and is recorded as such.

## REA evidence

- `ev_d8720b4505aa082b4a313effff015a1e307780fe4cba664b9b678b6a862ab73e`: the rendered GameplayKit landing page
  (`https://developer.apple.com/documentation/gameplaykit`), title "GameplayKit | Apple Developer Documentation", with
  the Entities and Components, Spatial Partitioning, Procedural Noise, Decision Trees and Rule Systems headings.

## Matrix rows closed

Status is `partial` for each. Feature names are copied from `docs/parity-matrix/data/gameplaykit.json`.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| gameplaykit | Entities and components | partial | `springboard/ui/src/ecs.rs` |
| gameplaykit | Spatial partitioning | partial | `springboard/ui/src/quadtree.rs` |
| gameplaykit | Procedural noise | partial | `springboard/ui/src/noise.rs` |
| gameplaykit | Decision trees and rule systems | partial | `springboard/ui/src/decision.rs`, `springboard/ui/src/rules.rs` |

## Rows of the theme not closed

SpriteKit, SceneKit, Spatial, Screen Saver and Game Controller rows of `theme-07` are left as they are. GameplayKit
Obstacles, Constants, structures and enumerations, and the Strategists row are outside this pass.

## Verification

No build or test was run (the wave rules forbid it). Each module was checked by reading: borrows in `evaluate`,
`Node::insert` and the tree walks, the test expectations (worked out by hand for each case), and the owner name rule
through `python3 tools/check-names.py` (exit 0).
