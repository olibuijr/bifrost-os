//! Turn-based game AI as pure search over a game model. The app describes its game through the `GameModel` trait: who
//! is to move, the moves open to a player, how a move changes the state, and how good a state is for a player. Two
//! strategists then choose moves from that description.
//!
//! `Minimax` searches the moves exhaustively to a depth, with alpha-beta pruning. The player whose turn it is maximises
//! their own score and every other player's turn minimises it, so the search assumes the other players play against
//! the chosen player. It is deterministic. `MonteCarlo` estimates the value of each move from random playouts with
//! UCB1 selection, driven by a seeded generator, so a given seed always gives the same move.
//!
//! The model is never undone: the search clones the state and applies moves to the copy. Nothing here draws or plays
//! a sound; the caller owns the game and applies the move it gets back.

/// A turn-based game as seen by a strategist.
pub trait GameModel: Clone {
    /// Identifies a player. Compared for equality only.
    type Player: Copy + PartialEq;
    /// One change a player can make on their turn.
    type Move: Clone;

    /// Every player in the game.
    fn players(&self) -> Vec<Self::Player>;
    /// The player whose turn it is, or None when the game has no turn left.
    fn active_player(&self) -> Option<Self::Player>;
    /// The moves open to `player`. Empty unless `player` is the active player.
    fn moves_for(&self, player: Self::Player) -> Vec<Self::Move>;
    /// Changes the state to reflect `mv`.
    fn apply(&mut self, mv: &Self::Move);
    /// How desirable the state is for `player`. Higher is better.
    fn score(&self, player: Self::Player) -> f64;
    /// Whether the state is a win for `player`.
    fn is_win(&self, player: Self::Player) -> bool;
    /// Whether the state is a loss for `player`.
    fn is_loss(&self, player: Self::Player) -> bool;
}

/// A state is over when a player has won or lost, when no player is to move, or when the player to move has no moves.
fn is_over<M: GameModel>(state: &M) -> bool {
    if state.players().iter().any(|&p| state.is_win(p) || state.is_loss(p)) {
        return true;
    }
    match state.active_player() {
        None => true,
        Some(p) => state.moves_for(p).is_empty(),
    }
}

/// A small xorshift generator. A strategist keeps its own, so a seed always gives the same choices.
#[derive(Clone, Debug, PartialEq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator for `seed`. A zero seed is replaced by a fixed non-zero value, since xorshift cannot leave zero.
    pub fn new(seed: u64) -> Self {
        Rng { state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// A number in `0..n`. Returns 0 when `n` is 0 or 1.
    pub fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

const NEG_INF: f64 = f64::NEG_INFINITY;
const POS_INF: f64 = f64::INFINITY;

/// An exhaustive, deterministic search to a fixed depth in turns.
#[derive(Clone, Debug, PartialEq)]
pub struct Minimax {
    /// How many turns ahead to search, counting the move being chosen. Zero means no search, so no move is chosen.
    pub max_look_ahead_depth: usize,
}

impl Minimax {
    pub fn new(max_look_ahead_depth: usize) -> Self {
        Minimax { max_look_ahead_depth }
    }

    /// The best move for `player`, who must be the active player. Ties go to the move listed first. None when the
    /// player is not to move, the depth is zero, or no move is open.
    pub fn best_move<M: GameModel>(&self, state: &M, player: M::Player) -> Option<M::Move> {
        if self.max_look_ahead_depth == 0 || state.active_player() != Some(player) {
            return None;
        }
        let mut best: Option<M::Move> = None;
        let mut best_value = NEG_INF;
        let mut alpha = NEG_INF;
        for mv in state.moves_for(player) {
            let mut child = state.clone();
            child.apply(&mv);
            let value = self.search(&child, self.max_look_ahead_depth - 1, player, alpha, POS_INF);
            if best.is_none() || value > best_value {
                best_value = value;
                best = Some(mv);
            }
            if value > alpha {
                alpha = value;
            }
        }
        best
    }

    /// One of the `from_number_of_best` best moves for `player`, chosen with `rng`. The best moves are ranked by their
    /// exact minimax value; ties keep the order the moves were listed in. None under the same conditions as
    /// `best_move`, or when `from_number_of_best` is zero.
    pub fn random_move<M: GameModel>(
        &self,
        state: &M,
        player: M::Player,
        from_number_of_best: usize,
        rng: &mut Rng,
    ) -> Option<M::Move> {
        if self.max_look_ahead_depth == 0 || state.active_player() != Some(player) || from_number_of_best == 0 {
            return None;
        }
        let moves = state.moves_for(player);
        if moves.is_empty() {
            return None;
        }
        let mut ranked: Vec<(f64, usize)> = moves
            .iter()
            .enumerate()
            .map(|(i, mv)| {
                let mut child = state.clone();
                child.apply(mv);
                (self.search(&child, self.max_look_ahead_depth - 1, player, NEG_INF, POS_INF), i)
            })
            .collect();
        // sort_by is stable, so equal values keep the order the moves were listed in.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let k = from_number_of_best.min(ranked.len());
        let index = ranked[rng.below(k)].1;
        Some(moves[index].clone())
    }

    /// The value of `state` for `root` after searching `depth` more moves. Terminal states and the depth limit are
    /// scored by `score`.
    fn search<M: GameModel>(&self, state: &M, depth: usize, root: M::Player, mut alpha: f64, mut beta: f64) -> f64 {
        if depth == 0 || is_over(state) {
            return state.score(root);
        }
        let mover = match state.active_player() {
            Some(p) => p,
            None => return state.score(root),
        };
        let maximizing = mover == root;
        let mut best = if maximizing { NEG_INF } else { POS_INF };
        for mv in state.moves_for(mover) {
            let mut child = state.clone();
            child.apply(&mv);
            let value = self.search(&child, depth - 1, root, alpha, beta);
            if maximizing {
                if value > best {
                    best = value;
                }
                if best > alpha {
                    alpha = best;
                }
            } else {
                if value < best {
                    best = value;
                }
                if best < beta {
                    beta = best;
                }
            }
            if alpha >= beta {
                break;
            }
        }
        best
    }
}

/// A search node in the Monte Carlo tree. Wins are counted for the root player: 1 for a win, 0 for a loss, 0.5 for a
/// draw or an unfinished playout.
struct Node<M: GameModel> {
    state: M,
    mv: Option<M::Move>,
    parent: Option<usize>,
    children: Vec<usize>,
    untried: Vec<M::Move>,
    visits: u32,
    wins: f64,
}

fn make_node<M: GameModel>(state: M, mv: Option<M::Move>, parent: Option<usize>) -> Node<M> {
    let untried = if is_over(&state) {
        Vec::new()
    } else {
        match state.active_player() {
            Some(p) => state.moves_for(p),
            None => Vec::new(),
        }
    };
    Node { state, mv, parent, children: Vec::new(), untried, visits: 0, wins: 0.0 }
}

/// A Monte Carlo tree search with UCB1 selection and random playouts, driven by a seeded generator.
#[derive(Clone, Debug, PartialEq)]
pub struct MonteCarlo {
    /// How many iterations of selection, playout and update to run for each decision.
    pub budget: u32,
    /// The UCB1 exploration constant. Larger values try rarely visited moves more often.
    pub exploration: f64,
    /// The most moves one random playout may make before it is scored as unfinished.
    pub max_playout_moves: usize,
    rng: Rng,
}

impl MonteCarlo {
    /// A search with 1000 iterations, exploration sqrt(2), playouts capped at 200 moves, and the given seed.
    pub fn new(seed: u64) -> Self {
        MonteCarlo {
            budget: 1000,
            exploration: std::f64::consts::SQRT_2,
            max_playout_moves: 200,
            rng: Rng::new(seed),
        }
    }

    /// The move for `player`, who must be the active player, chosen as the child of the root that was visited most.
    /// Ties go to the move listed first. None when the budget is zero, the player is not to move, or no move is open.
    pub fn best_move<M: GameModel>(&mut self, state: &M, player: M::Player) -> Option<M::Move> {
        if self.budget == 0 || state.active_player() != Some(player) || is_over(state) {
            return None;
        }
        let mut nodes: Vec<Node<M>> = vec![make_node(state.clone(), None, None)];
        for _ in 0..self.budget {
            let mut idx = 0usize;
            loop {
                if !nodes[idx].untried.is_empty() {
                    let k = self.rng.below(nodes[idx].untried.len());
                    let mv = nodes[idx].untried.swap_remove(k);
                    let mut next = nodes[idx].state.clone();
                    next.apply(&mv);
                    nodes.push(make_node(next, Some(mv), Some(idx)));
                    let child = nodes.len() - 1;
                    nodes[idx].children.push(child);
                    idx = child;
                    break;
                }
                if nodes[idx].children.is_empty() {
                    break;
                }
                idx = self.select_child(&nodes, idx, player);
            }
            let reward = self.playout(&nodes[idx].state, player);
            let mut cur = Some(idx);
            while let Some(i) = cur {
                nodes[i].visits += 1;
                nodes[i].wins += reward;
                cur = nodes[i].parent;
            }
        }
        let mut best: Option<(u32, usize)> = None;
        for &child in &nodes[0].children {
            let visits = nodes[child].visits;
            if best.map_or(true, |(v, _)| visits > v) {
                best = Some((visits, child));
            }
        }
        best.and_then(|(_, child)| nodes[child].mv.clone())
    }

    /// The child of `idx` with the highest UCB1 value for the player to move at `idx`. Children are visited at least once
    /// before they are selected, because each is updated as soon as it is created.
    fn select_child<M: GameModel>(&self, nodes: &[Node<M>], idx: usize, root: M::Player) -> usize {
        let maximizing = nodes[idx].state.active_player() == Some(root);
        let parent_visits = f64::from(nodes[idx].visits.max(1));
        let mut best = nodes[idx].children[0];
        let mut best_value = NEG_INF;
        for &child in &nodes[idx].children {
            let c = &nodes[child];
            let visits = f64::from(c.visits.max(1));
            let q = c.wins / visits;
            let value = if maximizing { q } else { 1.0 - q };
            let ucb = value + self.exploration * (parent_visits.ln() / visits).sqrt();
            if ucb > best_value {
                best_value = ucb;
                best = child;
            }
        }
        best
    }

    /// Plays random moves from `state` until the game is over or the cap is reached, and scores the result for `root`.
    fn playout<M: GameModel>(&mut self, state: &M, root: M::Player) -> f64 {
        let mut s = state.clone();
        let mut steps = 0usize;
        while !is_over(&s) && steps < self.max_playout_moves {
            let mover = match s.active_player() {
                Some(p) => p,
                None => break,
            };
            let moves = s.moves_for(mover);
            if moves.is_empty() {
                break;
            }
            let k = self.rng.below(moves.len());
            s.apply(&moves[k]);
            steps += 1;
        }
        if s.is_win(root) {
            1.0
        } else if s.is_loss(root) {
            0.0
        } else {
            0.5
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Take-away: a pile of stones, each turn a player takes 1 or 2, and whoever takes the last stone wins. From a pile
    /// that is a multiple of 3 the player to move loses against perfect play.
    #[derive(Clone, Debug, PartialEq)]
    struct Take {
        stones: u32,
        active: usize,
        last: Option<usize>,
    }

    impl Take {
        fn new(stones: u32) -> Self {
            Take { stones, active: 0, last: None }
        }
    }

    impl GameModel for Take {
        type Player = usize;
        type Move = u32;

        fn players(&self) -> Vec<usize> {
            vec![0, 1]
        }

        fn active_player(&self) -> Option<usize> {
            if self.stones > 0 {
                Some(self.active)
            } else {
                None
            }
        }

        fn moves_for(&self, player: usize) -> Vec<u32> {
            if self.active_player() != Some(player) {
                return Vec::new();
            }
            (1..=self.stones.min(2)).collect()
        }

        fn apply(&mut self, mv: &u32) {
            self.stones -= mv;
            self.last = Some(self.active);
            self.active = 1 - self.active;
        }

        fn score(&self, player: usize) -> f64 {
            if self.is_win(player) {
                1.0
            } else if self.is_loss(player) {
                0.0
            } else {
                0.5
            }
        }

        fn is_win(&self, player: usize) -> bool {
            self.stones == 0 && self.last == Some(player)
        }

        fn is_loss(&self, player: usize) -> bool {
            self.stones == 0 && self.last.is_some() && self.last != Some(player)
        }
    }

    #[test]
    fn minimax_takes_the_move_that_leaves_the_opponent_a_lost_pile() {
        // From 4, taking 1 leaves 3, a losing pile for the opponent. Taking 2 leaves 2, which the opponent wins.
        let m = Minimax::new(10);
        assert_eq!(m.best_move(&Take::new(4), 0), Some(1));
    }

    #[test]
    fn minimax_takes_the_last_two_stones_when_that_wins_at_once() {
        let m = Minimax::new(10);
        assert_eq!(m.best_move(&Take::new(2), 0), Some(2));
    }

    #[test]
    fn minimax_breaks_ties_with_the_first_listed_move_when_every_move_loses() {
        // From 3 every move loses, so both are worth the same and the first listed (take 1) is returned.
        let m = Minimax::new(10);
        assert_eq!(m.best_move(&Take::new(3), 0), Some(1));
    }

    #[test]
    fn minimax_returns_none_for_a_player_not_to_move_and_for_zero_depth() {
        let m = Minimax::new(10);
        assert_eq!(m.best_move(&Take::new(4), 1), None);
        assert_eq!(Minimax::new(0).best_move(&Take::new(4), 0), None);
    }

    #[test]
    fn minimax_depth_limit_scores_the_frontier_with_score() {
        // Depth 1 from 2 stones: taking 2 ends the game in a win for the mover (value 1). Taking 1 reaches a position that
        // is not over and is not searched further, so it is scored 0.5.
        let m = Minimax::new(1);
        assert_eq!(m.best_move(&Take::new(2), 0), Some(2));
    }

    #[test]
    fn random_move_with_one_candidate_is_the_best_move() {
        let m = Minimax::new(10);
        let mut rng = Rng::new(42);
        assert_eq!(m.random_move(&Take::new(4), 0, 1, &mut rng), Some(1));
        assert_eq!(m.random_move(&Take::new(4), 0, 0, &mut rng), None);
    }

    #[test]
    fn random_move_with_several_candidates_picks_among_the_best_ones() {
        // From 4 the moves are worth 1 (take 1) and 0 (take 2). Asking for the 2 best allows both, so only those two
        // can come back, and the one returned is the same for the same seed.
        let m = Minimax::new(10);
        let mut rng = Rng::new(7);
        let first = m.random_move(&Take::new(4), 0, 2, &mut rng).unwrap();
        assert!(first == 1 || first == 2);
        let mut again = Rng::new(7);
        assert_eq!(m.random_move(&Take::new(4), 0, 2, &mut again), Some(first));
    }

    #[test]
    fn monte_carlo_finds_the_move_that_wins_at_once() {
        // From 2 stones, taking 2 is a sure win and taking 1 is a sure loss, so the visits settle on taking 2.
        let mut mc = MonteCarlo::new(7);
        mc.budget = 300;
        assert_eq!(mc.best_move(&Take::new(2), 0), Some(2));
    }

    #[test]
    fn monte_carlo_with_no_budget_or_no_turn_returns_none() {
        let mut mc = MonteCarlo::new(1);
        mc.budget = 0;
        assert_eq!(mc.best_move(&Take::new(2), 0), None);
        let mut mc = MonteCarlo::new(1);
        assert_eq!(mc.best_move(&Take::new(2), 1), None);
    }

    #[test]
    fn rng_stays_in_range_and_a_zero_seed_still_moves() {
        let mut rng = Rng::new(0);
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
        assert_ne!(rng.next_u64(), 0);
        for _ in 0..200 {
            assert!(rng.below(5) < 5);
        }
    }

}
