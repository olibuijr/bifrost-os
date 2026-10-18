//! GameplayKit for apps: the game-logic models, re-exported from `bifrost_ui` so an app takes one import. Entities
//! and components (`ecs`), state machines, grid pathfinding (`grid_graph`), agents and steering (`agents`, `obstacle`),
//! procedural noise (`noise`), decision trees and rule systems (`decision`, `rules`, `rule_system`), turn-based
//! strategists (`strategist`), spatial partitioning (`quadtree`) and the GameplayKit value types (`gameplay_values`).
//!
//! The models are deterministic and touch no clock or GPU. Randomness comes from `crate::random`, which an app seeds
//! from `testing::seed()`. See docs/parity/gameplaykit-random.md.

pub use bifrost_ui::{
    agents, decision, ecs, gameplay_values, grid_graph, noise, obstacle, quadtree, rule_system, rules, state_machine,
    strategist,
};

#[cfg(test)]
mod tests {
    use super::{grid_graph, noise, quadtree};

    #[test]
    fn the_facade_reaches_pathfinding_partitioning_and_noise() {
        // Pathfinding: a straight route across an open grid exists and includes both ends.
        let grid = grid_graph::GridGraph::new(grid_graph::Cell { x: 0, y: 0 }, 3, 3, false);
        let route = grid.find_path(grid_graph::Cell { x: 0, y: 0 }, grid_graph::Cell { x: 2, y: 0 });
        let route = route.expect("an open grid has a route");
        assert_eq!(route.first(), Some(&grid_graph::Cell { x: 0, y: 0 }));
        assert_eq!(route.last(), Some(&grid_graph::Cell { x: 2, y: 0 }));

        // Spatial partitioning: a point inserted through the facade comes back from a query that covers it.
        let mut tree = quadtree::QuadTree::new(quadtree::Area::new(0.0, 0.0, 10.0, 10.0), 4, 6);
        assert!(tree.insert(quadtree::Point { x: 1.0, y: 1.0 }, 7u32));
        assert_eq!(tree.query(quadtree::Area::new(0.0, 0.0, 2.0, 2.0)), vec![7]);
        assert!(tree.query(quadtree::Area::new(5.0, 5.0, 9.0, 9.0)).is_empty());

        // Noise: a seeded value basis is deterministic and finite at the same point.
        let field = noise::Noise::new(noise::Basis::Value { seed: 9 });
        let a = field.sample(1.5, 2.5);
        assert!(a.is_finite());
        assert_eq!(a, field.sample(1.5, 2.5));
    }
}
