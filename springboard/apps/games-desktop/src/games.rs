//! The Games app: 2048 (swipe), Minesweeper (tap to dig, flag mode) and Klondike Solitaire.
//! Best scores live in `games.ini` (a `Prefs` file) in the app's data dir.


use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE2_PT, MAC_TITLE3_PT};
use crate::celebration::Celebration;
use crate::phase::{Flow, Phase};
use crate::solitaire;
use crate::tictactoe::{Mark, TicTacToe};
use crate::tile_motion::{scaled, TileMotion};
use bifrost_core::random::{RandomSource, Xorshift64};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::kit::ui::{
    CARD_BG, CONTENT_BG, CARD_RADIUS_PT, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, SEPARATOR_C, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE,
    SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW, WHITE,
};
use bifrost_app::kit::{Event, Surface, TextCache, Ui, Weight};
use springboard::shell::prefs::Prefs;
use springboard::shell::Effect;
use std::path::PathBuf;

/// Sidebar width in points (Golden Gate source list).
const SIDEBAR_W: f32 = 200.0;
/// Rows start under the traffic lights, as in Files.
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
const ROW_GAME: u32 = 100;
/// Content rows, in points from the top of the content area: status, message, then the board.
const STATUS_TOP_PT: f32 = SPACE_XXL;
const MSG_TOP_PT: f32 = 56.0;
const ROW_PT: f32 = 30.0;
const BOARD_TOP_PT: f32 = MSG_TOP_PT + ROW_PT + SPACE_M;
const BOARD_BOTTOM_PT: f32 = SPACE_XXXL;
const BOARD_SIDE_PAD_PT: f32 = SPACE_XXL * 2.0;
const BOARD_MIN_PT: f32 = SPACE_HUGE * 3.0;
const BAR_TITLE: u32 = 1;
pub const BAR_NEW: u32 = 2;
pub const BAR_FLAG: u32 = 3;
pub const BAR_UNDO: u32 = 4;
pub const BAR_DRAW: u32 = 5;
pub const BAR_AUTO: u32 = 6;
const RED: u32 = SYSTEM_RED;
const GREEN: u32 = SYSTEM_GREEN;
const N: usize = 4;
const MW: usize = 9;
const MH: usize = 9;
const MINES: usize = 10;
/// Frame interval of the win celebration, in milliseconds (about 60 frames a second).
const CELEBRATION_FRAME_MS: f64 = 16.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Game {
    G2048,
    Mines,
    Solitaire,
    TicTacToe,
}

/// The games in sidebar order: a row's index is the game's discriminant.
const GAME_ORDER: [Game; 4] = [Game::G2048, Game::Mines, Game::Solitaire, Game::TicTacToe];

#[derive(Clone, Copy, Default)]
struct Cell {
    mine: bool,
    open: bool,
    flag: bool,
    near: u8,
}

pub struct Games {
    w: i32,
    h: i32,
    game: Game,
    prefs: Prefs,
    rng: Xorshift64,
    // 2048
    grid: [[u32; N]; N],
    score: u32,
    best_2048: u32,
    p2048: Flow,
    // minesweeper
    cells: Vec<Cell>,
    placed: bool,
    flag_mode: bool,
    pm: Flow,
    started: Option<f64>,
    elapsed: u32,
    best_mines: u32,
    sol: solitaire::Solitaire,
    // tic-tac-toe against the computer (see `tictactoe`)
    ttt: TicTacToe,
    // touch
    down: (f32, f32),
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool, u32),
    /// Low Power Mode or a serious thermal state: the clock redraws less often (see `clock_step`).
    reduced: bool,
    /// The main display's visible size in points, from the shell's `screens` service.
    display: Option<(f32, f32)>,
    /// A line from the system (a controller plugged in or out) shown in the message row until the next touch.
    notice: Option<String>,
    /// The win celebration in progress, if any (see `celebration`).
    burst: Option<Celebration>,
    /// Whether each game was won at the last check (2048, Minesweeper, Solitaire, Tic-Tac-Toe), so a win celebrates once.
    won_seen: [bool; 4],
    /// Window pixels at the centre of the board, from the last draw: where a celebration starts.
    board_centre: (f32, f32),
    /// The 2048 tiles moving now (a merge swells, a new tile grows). See `tile_motion`.
    motion: TileMotion,
    /// The app clock (ms) at the last touch, tick or draw; the time a slide starts its motion at.
    now: f64,
}

/// A non-zero xorshift seed taken from the wall clock, so every launch deals differently.
fn clock_seed() -> u64 {
    // `testing::seed()` is fixed under the testing kit (BIFROST_TEST_PIN=1), so snapshots deal the same board
    let nanos = bifrost_app::testing::seed();
    (nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03) | 1
}

impl Games {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        let seed = clock_seed();
        let mut g = Self {
            w,
            h,
            game: Game::G2048,
            prefs: Prefs::memory(),
            rng: Xorshift64::new(seed),
            grid: [[0; N]; N],
            score: 0,
            best_2048: 0,
            p2048: Flow::new(),
            cells: vec![Cell::default(); MW * MH],
            placed: false,
            flag_mode: false,
            pm: Flow::new(),
            started: None,
            elapsed: 0,
            best_mines: 0,
            sol: solitaire::Solitaire::new((seed >> 32) as u32),
            ttt: TicTacToe::new(),
            down: (0.0, 0.0),
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold, LABEL.now()),
            reduced: false,
            display: None,
            notice: None,
            burst: None,
            won_seen: [false; 4],
            board_centre: (0.0, 0.0),
            motion: TileMotion::default(),
            now: 0.0,
        };
        g.new_2048();
        g
    }

    pub fn with_storage(mut self, dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        self.prefs = Prefs::load(dir.join("games.ini"));
        let get = |k: &str| self.prefs.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        self.best_2048 = get("best_2048");
        self.best_mines = get("best_mines_secs");
        self
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    /// Lower the motion of the game (Low Power Mode, serious heat) or restore it.
    pub fn set_reduced_effects(&mut self, on: bool) {
        if self.reduced != on {
            self.reduced = on;
            self.dirty = true;
        }
    }

    /// The main display's visible size in points, or None when the shell has not reported a display.
    pub fn set_display(&mut self, display: Option<(f32, f32)>) {
        self.display = display;
    }

    /// Shows `text` in the message row until the next touch; `None` restores the game's own hint.
    pub fn set_notice(&mut self, text: Option<String>) {
        self.notice = text;
        self.dirty = true;
    }

    /// Seconds between two redraws of the Minesweeper clock: every second, or every 5 s with reduced effects.
    fn clock_step(&self) -> u32 {
        if self.reduced {
            5
        } else {
            1
        }
    }

    fn rand(&mut self) -> u32 {
        (self.rng.next_u64() >> 32) as u32
    }

    // ---- 2048 ----

    fn new_2048(&mut self) {
        self.grid = [[0; N]; N];
        self.score = 0;
        self.p2048.go(Phase::Playing);
        self.spawn();
        self.spawn();
    }

    /// Puts a new 2 (or, one time in ten, a 4) on a random empty cell, and returns that cell.
    fn spawn(&mut self) -> Option<(usize, usize)> {
        let empty: Vec<(usize, usize)> = (0..N).flat_map(|r| (0..N).map(move |c| (r, c))).filter(|&(r, c)| self.grid[r][c] == 0).collect();
        if empty.is_empty() {
            return None;
        }
        let (r, c) = empty[self.rand() as usize % empty.len()];
        self.grid[r][c] = if self.rand() % 10 == 0 { 4 } else { 2 };
        Some((r, c))
    }

    /// Slides one line toward index 0, merging equal pairs once; returns the points gained. `shift` uses
    /// `slide_merges`, which also reports the merged positions; this form is what the slide tests check.
    #[cfg(test)]
    fn slide(line: [u32; N]) -> ([u32; N], u32) {
        let (out, pts, _) = Self::slide_merges(line);
        (out, pts)
    }

    /// `slide`, and which output positions hold a merged tile (a pair that became one tile).
    fn slide_merges(line: [u32; N]) -> ([u32; N], u32, [bool; N]) {
        let tiles: Vec<u32> = line.iter().copied().filter(|&v| v != 0).collect();
        let mut out = [0; N];
        let mut merged = [false; N];
        let (mut i, mut k, mut pts) = (0, 0, 0);
        while i < tiles.len() {
            if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] {
                out[k] = tiles[i] * 2;
                merged[k] = true;
                pts += out[k];
                i += 2;
            } else {
                out[k] = tiles[i];
                i += 1;
            }
            k += 1;
        }
        (out, pts, merged)
    }

    /// The grid cell of output position `j` in line `i` for a slide `dir` (the mapping `shift` uses).
    fn cell_of(dir: u8, i: usize, j: usize) -> (usize, usize) {
        match dir {
            0 => (i, j),
            1 => (i, N - 1 - j),
            2 => (j, i),
            _ => (N - 1 - j, i),
        }
    }

    /// dir: 0 left, 1 right, 2 up, 3 down.
    fn shift(&mut self, dir: u8) {
        if self.p2048.phase() == Phase::Lost {
            return;
        }
        let mut moved = false;
        let mut merged_cells = Vec::new();
        for i in 0..N {
            let mut line = [0; N];
            for j in 0..N {
                line[j] = match dir {
                    0 => self.grid[i][j],
                    1 => self.grid[i][N - 1 - j],
                    2 => self.grid[j][i],
                    _ => self.grid[N - 1 - j][i],
                };
            }
            let (out, pts, merged) = Self::slide_merges(line);
            for j in 0..N {
                if merged[j] {
                    merged_cells.push(Self::cell_of(dir, i, j));
                }
            }
            self.score += pts;
            for j in 0..N {
                let cell = match dir {
                    0 => &mut self.grid[i][j],
                    1 => &mut self.grid[i][N - 1 - j],
                    2 => &mut self.grid[j][i],
                    _ => &mut self.grid[N - 1 - j][i],
                };
                if *cell != out[j] {
                    moved = true;
                    *cell = out[j];
                }
            }
        }
        if !moved {
            return;
        }
        let fresh = self.spawn();
        self.plan_motion(&merged_cells, fresh);
        if self.score > self.best_2048 {
            self.best_2048 = self.score;
            let _ = self.prefs.set("best_2048", &self.best_2048.to_string());
        }
        if self.p2048.phase() == Phase::Playing && self.grid.iter().flatten().any(|&v| v >= 2048) {
            self.p2048.go(Phase::Won);
        }
        if !self.can_move() {
            self.p2048.go(Phase::Lost);
        }
    }

    /// Plans the motion of one slide: each merged tile swells and the new tile grows in. Tiles that only slide are not
    /// animated (see `tile_motion`). With reduced effects nothing moves.
    fn plan_motion(&mut self, merged: &[(usize, usize)], fresh: Option<(usize, usize)>) {
        self.motion.clear();
        if self.reduced {
            return;
        }
        for &cell in merged {
            self.motion.swell(cell, self.now);
        }
        if let Some(cell) = fresh {
            self.motion.grow(cell, self.now);
        }
    }

    fn can_move(&self) -> bool {
        for r in 0..N {
            for c in 0..N {
                let v = self.grid[r][c];
                if v == 0 || (c + 1 < N && self.grid[r][c + 1] == v) || (r + 1 < N && self.grid[r + 1][c] == v) {
                    return true;
                }
            }
        }
        false
    }

    // ---- Minesweeper ----

    fn new_mines(&mut self) {
        self.cells = vec![Cell::default(); MW * MH];
        self.placed = false;
        self.pm.go(Phase::Playing);
        self.started = None;
        self.elapsed = 0;
    }

    fn neighbours(i: usize) -> Vec<usize> {
        let (x, y) = ((i % MW) as i32, (i / MW) as i32);
        let mut v = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (x + dx, y + dy);
                if (dx, dy) != (0, 0) && nx >= 0 && ny >= 0 && nx < MW as i32 && ny < MH as i32 {
                    v.push(ny as usize * MW + nx as usize);
                }
            }
        }
        v
    }

    /// Lays the mines after the first dig so that cell and its ring are safe.
    fn place_mines(&mut self, safe: usize) {
        let mut ring = Self::neighbours(safe);
        ring.push(safe);
        let mut left = MINES;
        while left > 0 {
            let i = self.rand() as usize % (MW * MH);
            if !self.cells[i].mine && !ring.contains(&i) {
                self.cells[i].mine = true;
                left -= 1;
            }
        }
        for i in 0..MW * MH {
            self.cells[i].near = Self::neighbours(i).iter().filter(|&&n| self.cells[n].mine).count() as u8;
        }
        self.placed = true;
    }

    fn dig(&mut self, i: usize, t: f64) {
        if self.pm.phase() != Phase::Playing || self.cells[i].flag || self.cells[i].open {
            return;
        }
        if !self.placed {
            self.place_mines(i);
            self.started = Some(t);
        }
        if self.cells[i].mine {
            self.cells[i].open = true;
            self.pm.go(Phase::Lost);
            for c in self.cells.iter_mut().filter(|c| c.mine) {
                c.open = true;
            }
            return;
        }
        let mut stack = vec![i];
        while let Some(c) = stack.pop() {
            if self.cells[c].open || self.cells[c].flag {
                continue;
            }
            self.cells[c].open = true;
            if self.cells[c].near == 0 {
                stack.extend(Self::neighbours(c));
            }
        }
        if self.cells.iter().all(|c| c.mine || c.open) {
            self.pm.go(Phase::Won);
            if let Some(t0) = self.started {
                self.elapsed = ((t - t0) / 1000.0) as u32;
            }
            if self.best_mines == 0 || self.elapsed < self.best_mines {
                self.best_mines = self.elapsed.max(1);
                let _ = self.prefs.set("best_mines_secs", &self.best_mines.to_string());
            }
        }
    }

    // ---- layout (the kit works in points, the board in pixels) ----

    /// The content area in pixels: right of the sidebar, under the unified toolbar.
    fn content(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y) = (ui.pt(SIDEBAR_W), ui.pt(UNIFIED_BAR_PT));
        (x, y, (self.w as f32 - x).max(1.0), (self.h as f32 - y).max(1.0))
    }

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.pt(1.0) }
    }

    /// The source list: one row per game, the open one selected.
    fn sidebar(&self) -> SourceList {
        let rows = [("2048", "square.grid.2x2"), ("Minesweeper", "rectangle.split.3x1"), ("Solitaire", "doc"), ("Tic-Tac-Toe", "xmark")];
        let items = rows.iter().enumerate().map(|(k, (l, sym))| SidebarItem::row(ROW_GAME + k as u32, l).with_symbol(sym)).collect();
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(ROW_GAME + self.game as u32));
        list
    }

    fn title(&self) -> &'static str {
        match self.game {
            Game::G2048 => "2048",
            Game::Mines => "Minesweeper",
            Game::Solitaire => "Solitaire",
            Game::TicTacToe => "Tic-Tac-Toe",
        }
    }

    /// The unified toolbar: the game's title, then its commands (Chess, Solitaire: Game menu commands).
    fn bar(&self, ui: &Ui) -> Toolbar {
        let tw = ui.measure(self.title(), MAC_TITLE3_PT, Weight::Bold) / ui.pt(1.0) + SPACE_XS;
        let mut items = vec![ToolbarItem::custom(BAR_TITLE, self.title(), tw), ToolbarItem::flexible_space()];
        match self.game {
            Game::G2048 | Game::TicTacToe => {}
            Game::Mines => items.push(ToolbarItem::button(BAR_FLAG, "Flag").selected(self.flag_mode)),
            Game::Solitaire => {
                items.push(ToolbarItem::button(BAR_UNDO, "Undo").in_group(1).enabled(self.sol.can_undo()));
                items.push(ToolbarItem::button(BAR_DRAW, if self.sol.draw3 { "Draw 3" } else { "Draw 1" }).in_group(1));
                items.push(ToolbarItem::button(BAR_AUTO, "Auto").in_group(1).enabled(self.sol.can_auto()));
            }
        }
        items.push(ToolbarItem::icon(BAR_NEW, "New Game", "plus"));
        Toolbar::new(items)
    }

    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = SIDEBAR_W + SPACE_M;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.pt(1.0) - x - SPACE_L).max(0.0), h }
    }

    /// The same layout `widgets::toolbar` draws (titled buttons sized to their label).
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        self.bar(ui).layout_with(self.bar_bounds(ui), width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Every toolbar button as a control, so the shell lets clicks through the window-move band.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        self.bar_layout(ui)
            .visible
            .iter()
            .filter_map(|(id, r)| bar.items.iter().find(|i| i.id == *id && *id != BAR_TITLE).map(|i| (i, *r)))
            .map(|(i, r)| Node { id: i.id as u64, role: Role::Button, label: i.label.clone(), value: String::new(), bounds: r, enabled: i.enabled })
            .collect()
    }

    /// Runs toolbar command `id` (a click or an accessibility activation).
    pub fn command(&mut self, id: u32) {
        let seed = self.rand();
        match (id, self.game) {
            (BAR_NEW, Game::G2048) => self.new_2048(),
            (BAR_NEW, Game::Mines) => self.new_mines(),
            (BAR_NEW, Game::Solitaire) => self.sol.deal(seed),
            (BAR_NEW, Game::TicTacToe) => self.ttt = TicTacToe::new(),
            (BAR_FLAG, Game::Mines) => self.flag_mode = !self.flag_mode,
            (BAR_UNDO, Game::Solitaire) => self.sol.undo(),
            (BAR_DRAW, Game::Solitaire) => {
                let d3 = !self.sol.draw3;
                self.sol.set_draw3(d3, seed);
            }
            (BAR_AUTO, Game::Solitaire) => self.sol.auto_complete(),
            _ => return,
        }
        self.dirty = true;
    }

    /// A slide from a controller's direction pad or left stick (`dir` as `shift`: 0 left, 1 right, 2 up, 3 down).
    /// 2048 slides its tiles; Tic-Tac-Toe moves its cursor. The other games have no cursor for a controller yet.
    pub fn gamepad_slide(&mut self, dir: u8) {
        match self.game {
            Game::G2048 => self.shift(dir),
            Game::TicTacToe => self.ttt.move_cursor(dir),
            Game::Mines | Game::Solitaire => return,
        }
        self.dirty = true;
    }

    /// The controller's South button. In Tic-Tac-Toe it marks the square under the cursor while the game is on. In the
    /// other games it starts a new game, but only once the current one is over, so a game in progress is never dropped
    /// by a stray press. A won 2048 board can still be played on, so only a lost one counts as over.
    pub fn gamepad_new_game(&mut self) {
        if self.game == Game::TicTacToe && !self.ttt.over() {
            let cell = self.ttt.cursor();
            self.ttt_play(cell);
            self.dirty = true;
            return;
        }
        let over = match self.game {
            Game::G2048 => self.p2048.phase() == Phase::Lost,
            Game::Mines => self.pm.phase() != Phase::Playing,
            Game::Solitaire => self.sol.won(),
            Game::TicTacToe => self.ttt.over(),
        };
        if over {
            self.command(BAR_NEW);
        }
    }

    /// The player's X on `cell`, then the computer's O at once when the game is still on.
    fn ttt_play(&mut self, cell: usize) {
        if self.ttt.play(cell) {
            self.ttt.computer_move();
        }
    }

    /// The square under window point (x, y) on the Tic-Tac-Toe board, if the point is on the board.
    fn ttt_cell_at(&self, ui: &Ui, x: f32, y: f32) -> Option<usize> {
        let (bx, by, side) = self.board(ui);
        if x < bx || y < by || x >= bx + side || y >= by + side {
            return None;
        }
        let cs = side / 3.0;
        let c = (((x - bx) / cs) as usize).min(2);
        let r = (((y - by) / cs) as usize).min(2);
        Some(r * 3 + c)
    }

    /// The board sits under the status and message rows, centred in what is left of the content area.
    fn board(&self, ui: &Ui) -> (f32, f32, f32) {
        let (cx, cy, cw, ch) = self.content(ui);
        let side = (cw - ui.pt(BOARD_SIDE_PAD_PT)).min(ch - ui.pt(BOARD_TOP_PT + BOARD_BOTTOM_PT)).max(ui.pt(BOARD_MIN_PT));
        (cx + (cw - side) / 2.0, cy + ui.pt(BOARD_TOP_PT), side)
    }

    fn mine_at(&self, ui: &Ui, x: f32, y: f32) -> Option<usize> {
        let (bx, by, side) = self.board(ui);
        if x < bx || y < by || x >= bx + side || y >= by + side {
            return None;
        }
        let cs = side / MW as f32;
        Some(((y - by) / cs) as usize * MW + ((x - bx) / cs) as usize)
    }

    fn place_sol(&mut self, ui: &Ui) {
        let (cx, cy, ..) = self.content(ui);
        self.sol.origin = (cx, cy);
        self.sol.height = self.h as f32;
    }

    /// Whether the game on screen is won.
    fn is_won(&self) -> bool {
        match self.game {
            Game::G2048 => self.p2048.phase() == Phase::Won,
            Game::Mines => self.pm.phase() == Phase::Won,
            Game::Solitaire => self.sol.won(),
            Game::TicTacToe => self.ttt.phase() == Phase::Won,
        }
    }

    /// Starts the celebration when the game on screen has just been won. A game that stays won does not celebrate
    /// again; a new game clears its flag, so its own win celebrates.
    fn note_win(&mut self, t: f64) {
        let won = self.is_won();
        let idx = self.game as usize;
        if won && !self.won_seen[idx] {
            // The seed comes from the clock, not the game's random source, so the deals do not change.
            self.burst = Some(Celebration::start(t.to_bits(), self.board_centre, t));
        }
        self.won_seen[idx] = won;
    }

    // ---- input ----

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Vec<Effect> {
        self.now = t;
        self.place_sol(ui);
        let (cx, cy, cw, _) = self.content(ui);
        let cwi = cw as i32;
        let pt = ui.pt(1.0);
        let in_content = |x: f32, y: f32| x >= cx && y >= cy;
        match ev {
            Event::Down(x, y) => {
                self.down = (x, y);
                self.notice = None;
                if self.game == Game::Solitaire && in_content(x, y) {
                    let seed = self.rand();
                    self.sol.touch(ui, ev, cwi, self.h, seed);
                }
            }
            Event::Move(..) if self.game == Game::Solitaire => {
                let seed = self.rand();
                if self.sol.touch(ui, ev, cwi, self.h, seed) {
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                let (d0, d1) = self.down;
                if !in_content(d0, d1) {
                    if self.game == Game::Solitaire {
                        self.sol.cancel_drag();
                    }
                    if d0 < cx {
                        let (list, b) = (self.sidebar(), self.sidebar_bounds(ui));
                        let hit = list.hit(b, SIDEBAR_SCROLL, d0 / pt, d1 / pt);
                        if hit.is_some() && hit == list.hit(b, SIDEBAR_SCROLL, x / pt, y / pt) {
                            self.game = GAME_ORDER[(hit.unwrap() - ROW_GAME) as usize % GAME_ORDER.len()];
                        }
                    } else {
                        let l = self.bar_layout(ui);
                        let hit = l.hit(d0 / pt, d1 / pt);
                        let enabled = self.controls(ui).iter().any(|n| Some(n.id as u32) == hit && n.enabled);
                        if enabled && hit == l.hit(x / pt, y / pt) {
                            self.command(hit.unwrap());
                        }
                    }
                } else if self.game == Game::Solitaire {
                    let seed = self.rand();
                    self.sol.touch(ui, ev, cwi, self.h, seed);
                } else if self.game == Game::G2048 {
                    let (dx, dy) = (x - d0, y - d1);
                    if dx.abs().max(dy.abs()) > ui.pt(30.0) {
                        self.shift(if dx.abs() > dy.abs() { if dx < 0.0 { 0 } else { 1 } } else if dy < 0.0 { 2 } else { 3 });
                    }
                } else if self.game == Game::TicTacToe {
                    // a square counts only when the press and the release are on the same square
                    if let (Some(a), Some(b)) = (self.ttt_cell_at(ui, d0, d1), self.ttt_cell_at(ui, x, y)) {
                        if a == b {
                            self.ttt.set_cursor(a);
                            self.ttt_play(a);
                        }
                    }
                } else if let Some(i) = self.mine_at(ui, x, y) {
                    if (x - d0).abs().max((y - d1).abs()) <= ui.pt(SPACE_XL) {
                        if self.flag_mode {
                            if !self.cells[i].open && self.pm.phase() == Phase::Playing {
                                self.cells[i].flag = !self.cells[i].flag;
                            }
                        } else {
                            self.dig(i, t);
                        }
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
        self.note_win(t);
        vec![]
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        self.now = t;
        self.note_win(t);
        if self.motion.is_live(t) {
            self.dirty = true;
        }
        let mut live = false;
        if let Some(b) = self.burst.as_mut() {
            b.advance(t);
            live = !b.is_done();
        }
        if live {
            self.dirty = true;
        } else {
            self.burst = None;
        }
        if let (Some(t0), Game::Mines, Phase::Playing) = (self.started, self.game, self.pm.phase()) {
            // the shown time is whole seconds, rounded down to the redraw step; the best time is taken from the exact
            // moment of the winning dig (see `dig`), so a slower redraw never changes a record
            let step = self.clock_step();
            let e = ((t - t0) / 1000.0) as u32;
            let shown = e - e % step;
            if shown != self.elapsed {
                self.elapsed = shown;
                self.dirty = true;
            }
        }
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        // a celebration or a moving tile asks for a frame every 16 ms while it lives
        let frame = self.burst.as_ref().map(|_| t + CELEBRATION_FRAME_MS);
        let frame = frame.or_else(|| self.motion.is_live(t).then(|| t + CELEBRATION_FRAME_MS));
        let clock = self.started.and_then(|t0| {
            (self.game == Game::Mines && self.pm.phase() == Phase::Playing)
                .then(|| t0 + (self.elapsed as f64 + self.clock_step() as f64) * 1000.0)
        });
        match (frame, clock) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn describe(&self) -> String {
        let display = match self.display {
            Some((w, h)) => format!("{w}x{h}"),
            None => "none".to_string(),
        };
        format!(
            "games_game={:?}\ngames_score={}\ngames_best_2048={}\ngames_mines_phase={:?}\ngames_best_mines={}\ngames_effects={}\ngames_display_pt={}\ngames_burst={}\ngames_tiles_moving={}\ngames_ttt_marks={}\ngames_ttt_phase={:?}\n{}",
            self.game,
            self.score,
            self.best_2048,
            self.pm.phase(),
            self.best_mines,
            if self.reduced { "reduced" } else { "full" },
            display,
            self.burst.as_ref().map_or(0, |b| b.live()),
            self.motion.live(self.now),
            self.ttt.marks(),
            self.ttt.phase(),
            self.sol.describe()
        )
    }

    // ---- drawing ----

    fn text_c(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, w: Weight, color: u32, r: (f32, f32, f32, f32)) {
        let img = self.cache.get(ui, text, pt, w, color);
        s.blit(img, (r.0 + (r.2 - img.w as f32) / 2.0) as i32, (r.1 + (r.3 - img.h as f32) / 2.0) as i32, 255, 255);
    }

    /// The win celebration's dots, drawn over the board.
    fn draw_burst(&self, s: &mut Surface, ui: &Ui) {
        if let Some(b) = &self.burst {
            b.draw(s, ui);
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.now = t;
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold, LABEL.now());
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        if self.cache.len() > 128 {
            self.cache.clear();
        }
        self.place_sol(ui);
        let (bx, by, side) = self.board(ui);
        self.board_centre = (bx + side / 2.0, by + side / 2.0);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        // Golden Gate full-size window: the sidebar runs to the top under the traffic lights (the shell draws them)
        let sb = self.sidebar_bounds(ui);
        widgets::source_list(s, ui, sb, &self.sidebar(), SIDEBAR_SCROLL, None);
        s.fill_rect(ui.pt(SIDEBAR_W) as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        let pt = ui.pt(1.0);
        widgets::toolbar_band(s, ui, Rect { x: SIDEBAR_W, y: 0.0, w: self.w as f32 / pt - SIDEBAR_W, h: UNIFIED_BAR_PT });
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), None);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let im = ui.text(&ui.fit(self.title(), MAC_TITLE3_PT, ui.pt(r.w)), MAC_TITLE3_PT, Weight::Bold, LABEL.now());
            s.blit(&im, ui.pt(r.x) as i32, ui.pt(r.y + r.h / 2.0) as i32 - im.h as i32 / 2, 255, 255);
        }
        let (cx, cy, cw, _) = self.content(ui);
        if self.game == Game::Solitaire {
            let status = format!("Moves {}", self.sol.moves());
            let r = (cx, cy + ui.pt(SPACE_M), cw, ui.pt(SPACE_XL));
            let (msg, col) = if self.sol.won() { (format!("You won!  {status}"), SYSTEM_GREEN) } else { (status, LABEL_2.now()) };
            self.text_c(s, ui, &msg, MAC_SUBHEAD_PT, Weight::Regular, col, r);
            let h = self.h;
            self.sol.draw(s, ui, &mut self.cache, cw as i32, h);
            self.draw_burst(s, ui);
            return;
        }
        let status = match self.game {
            Game::G2048 => format!("Score {}   Best {}", self.score, self.best_2048),
            Game::Solitaire => String::new(),
            Game::TicTacToe => "You are X   Computer is O".to_string(),
            Game::Mines => {
                let flags = self.cells.iter().filter(|c| c.flag).count();
                let best = if self.best_mines == 0 { "-".to_string() } else { format!("{}s", self.best_mines) };
                format!("Mines {}   Time {}s   Best {}", MINES as i32 - flags as i32, self.elapsed, best)
            }
        };
        let r = (cx, cy + ui.pt(STATUS_TOP_PT), cw, ui.pt(ROW_PT));
        self.text_c(s, ui, &status, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), r);
        let result = match (self.game, self.p2048.phase(), self.pm.phase()) {
            (Game::G2048, Phase::Won, _) => Some("You made 2048!"),
            (Game::G2048, Phase::Lost, _) => Some("No moves left"),
            (Game::TicTacToe, _, _) => self.ttt.result(),
            (Game::Mines, _, Phase::Won) => Some("Cleared!"),
            (Game::Mines, _, Phase::Lost) => Some("Boom"),
            _ => None,
        };
        let hint = match self.game {
            Game::G2048 => "Swipe or drag to slide tiles",
            Game::TicTacToe => "Click a square to play X",
            _ => if self.flag_mode { "Flag mode: click to mark" } else { "Click to dig" },
        };
        // a result always wins the row; a system notice (a controller) replaces only the hint
        let msg: String = match (result, &self.notice) {
            (Some(m), _) => m.to_string(),
            (None, Some(n)) => ui.fit(n, MAC_BODY_PT, cw),
            (None, None) => hint.to_string(),
        };
        let r = (cx, cy + ui.pt(MSG_TOP_PT), cw, ui.pt(ROW_PT));
        let color = if msg == "Boom" || msg == "No moves left" { SYSTEM_RED } else if msg == "Cleared!" || msg.starts_with("You") { SYSTEM_GREEN } else { LABEL.now() };
        self.text_c(s, ui, &msg, MAC_BODY_PT, Weight::Semibold, color, r);
        match self.game {
            Game::G2048 => self.draw_2048(s, ui),
            Game::Mines => self.draw_mines(s, ui),
            Game::TicTacToe => self.draw_ttt(s, ui),
            Game::Solitaire => {}
        }
        self.draw_burst(s, ui);
    }

    /// Tile plate and number colour: low tiles on the field grey, higher ones step through the system colours.
    fn tile_color(v: u32) -> (u32, u32) {
        const RAMP: [u32; 9] = [SYSTEM_YELLOW, SYSTEM_ORANGE, SYSTEM_RED, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_BLUE, SYSTEM_TEAL, SYSTEM_GREEN, SYSTEM_GREEN];
        match v {
            0 => (CARD_BG.now(), LABEL.now()),
            2 | 4 => (FIELD_BG.now(), LABEL.now()),
            _ => (RAMP[(v.trailing_zeros() as usize - 3).min(RAMP.len() - 1)], WHITE),
        }
    }

    fn draw_2048(&mut self, s: &mut Surface, ui: &Ui) {
        let (bx, by, side) = self.board(ui);
        s.fill_rrect(bx, by, side, side, ui.pt(CARD_RADIUS_PT), SEPARATOR_C.now());
        let gap = ui.pt(SPACE_S);
        let cs = (side - gap * (N as f32 + 1.0)) / N as f32;
        for r in 0..N {
            for c in 0..N {
                let v = self.grid[r][c];
                let cell = (bx + gap + c as f32 * (cs + gap), by + gap + r as f32 * (cs + gap), cs, cs);
                let rect = scaled(cell, self.motion.scale((r, c), self.now));
                let (bg, fg) = Self::tile_color(v);
                s.fill_rrect(rect.0, rect.1, rect.2, rect.3, ui.pt(SPACE_S), bg);
                if v != 0 {
                    let pt = if v < 100 { MAC_LARGE_TITLE_PT } else if v < 1000 { MAC_TITLE1_PT } else { MAC_TITLE2_PT };
                    self.text_c(s, ui, &v.to_string(), pt, Weight::Bold, fg, rect);
                }
            }
        }
    }

    /// The 3 by 3 board: X in the tint, O in the label colour, the cursor square on the field grey.
    fn draw_ttt(&mut self, s: &mut Surface, ui: &Ui) {
        let (bx, by, side) = self.board(ui);
        s.fill_rrect(bx, by, side, side, ui.pt(CARD_RADIUS_PT), SEPARATOR_C.now());
        let gap = ui.pt(SPACE_S);
        let cs = (side - gap * 4.0) / 3.0;
        for i in 0..9 {
            let (r, c) = (i / 3, i % 3);
            let rect = (bx + gap + c as f32 * (cs + gap), by + gap + r as f32 * (cs + gap), cs, cs);
            let bg = if i == self.ttt.cursor() { FIELD_BG.now() } else { CARD_BG.now() };
            s.fill_rrect(rect.0, rect.1, rect.2, rect.3, ui.pt(SPACE_S), bg);
            match self.ttt.cell(i) {
                Some(Mark::X) => self.text_c(s, ui, "X", MAC_TITLE1_PT, Weight::Bold, SYSTEM_BLUE, rect),
                Some(Mark::O) => self.text_c(s, ui, "O", MAC_TITLE1_PT, Weight::Bold, LABEL.now(), rect),
                None => {}
            }
        }
    }

    fn draw_mines(&mut self, s: &mut Surface, ui: &Ui) {
        let (bx, by, side) = self.board(ui);
        s.fill_rrect(bx, by, side, side, ui.pt(SPACE_S), SEPARATOR_C.now());
        let cs = side / MW as f32;
        let pad = ui.pt(1.5);
        for i in 0..MW * MH {
            let c = self.cells[i];
            let rect = (bx + (i % MW) as f32 * cs + pad, by + (i / MW) as f32 * cs + pad, cs - pad * 2.0, cs - pad * 2.0);
            s.fill_rrect(rect.0, rect.1, rect.2, rect.3, ui.pt(SPACE_XS), if c.open { if c.mine { RED } else { DETAIL_BG.now() } } else { FIELD_BG.now() });
            if c.open && c.mine {
                self.text_c(s, ui, "*", MAC_TITLE1_PT, Weight::Bold, WHITE, rect);
            } else if c.open && c.near > 0 {
                let col = [LABEL.now(), SYSTEM_BLUE, GREEN, RED, SYSTEM_PURPLE, SYSTEM_ORANGE, SYSTEM_TEAL, LABEL.now(), LABEL_2.now()][c.near as usize];
                self.text_c(s, ui, &c.near.to_string(), MAC_TITLE3_PT, Weight::Bold, col, rect);
            } else if c.flag {
                self.text_c(s, ui, "F", MAC_TITLE3_PT, Weight::Bold, SYSTEM_ORANGE, rect);
            }
        }
    }
}

/// The kit with the shell's font and symbols, for tests of the app and of its glue (`system.rs`).
#[cfg(test)]
pub(crate) fn test_ui() -> Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Centre of toolbar button `id` in window pixels.
    fn bar_centre(g: &Games, ui: &Ui, id: u32) -> (f32, f32) {
        let r = g.controls(ui).into_iter().find(|n| n.id == id as u64).unwrap().bounds;
        (ui.pt(r.x + r.w / 2.0), ui.pt(r.y + r.h / 2.0))
    }

    #[test]
    fn swipe_ending_on_new_game_button_does_not_fire_it() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.score = 77;
        let (bx, by) = bar_centre(&g, &ui, BAR_NEW);
        g.touch(&ui, Event::Down(bx, by + 300.0), 0.0);
        g.touch(&ui, Event::Up(bx, by), 0.0);
        assert!(g.score >= 77, "new game must not reset the score");
    }

    #[test]
    fn tap_on_new_game_button_fires_it() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.score = 77;
        let (bx, by) = bar_centre(&g, &ui, BAR_NEW);
        g.touch(&ui, Event::Down(bx, by), 0.0);
        g.touch(&ui, Event::Up(bx, by), 0.0);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn every_control_is_in_the_unified_bar_right_of_the_sidebar() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        for game in [Game::G2048, Game::Mines, Game::Solitaire] {
            g.game = game;
            assert!(g.controls(&ui).iter().all(|n| n.bounds.y < UNIFIED_BAR_PT && n.bounds.x >= SIDEBAR_W));
        }
    }

    #[test]
    fn solitaire_drag_onto_undo_leaves_no_ghost_drag() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.game = Game::Solitaire;
        let (ux, uy) = bar_centre(&g, &ui, BAR_NEW);
        g.touch(&ui, Event::Down(ux, uy), 0.0);
        g.sol.start_test_drag();
        g.touch(&ui, Event::Up(ux, uy), 0.0);
        assert!(!g.sol.is_dragging());
    }

    #[test]
    fn launches_use_clock_seeds() {
        let a = clock_seed();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_ne!(a, clock_seed());
    }

    #[test]
    fn winning_dig_records_fresh_elapsed_time() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.place_mines(0);
        g.started = Some(0.0);
        let target = (0..MW * MH).rfind(|&i| !g.cells[i].mine).unwrap();
        for i in 0..MW * MH {
            g.cells[i].open = !g.cells[i].mine && i != target;
            g.cells[i].flag = false;
        }
        g.dig(target, 42_000.0);
        assert_eq!(g.pm.phase(), Phase::Won);
        assert_eq!(g.best_mines, 42);
    }

    #[test]
    fn the_games_rng_is_the_same_xorshift_sequence_as_before() {
        // The app's words came from an inline xorshift64 before the core random source; the sequence must not change,
        // or the blessed snapshots and the seeded deals would move.
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        g.rng = Xorshift64::new(x);
        for _ in 0..16 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            assert_eq!(g.rand(), (x >> 32) as u32);
        }
    }

    #[test]
    fn a_lost_board_does_not_move_until_restarted() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.p2048.go(Phase::Lost);
        g.shift(0);
        assert_eq!(g.p2048.phase(), Phase::Lost);
        assert!(g.p2048.go(Phase::Playing));
        assert_eq!(g.p2048.phase(), Phase::Playing);
    }
}

#[cfg(test)]
mod slide_tests {
    use super::*;

    #[test]
    fn a_line_slides_to_the_edge_and_closes_gaps() {
        assert_eq!(Games::slide([0, 2, 0, 8]), ([2, 8, 0, 0], 0));
        assert_eq!(Games::slide([2, 0, 0, 4]), ([2, 4, 0, 0], 0));
    }

    #[test]
    fn equal_neighbours_merge_once_and_score_the_result() {
        assert_eq!(Games::slide([2, 2, 2, 2]), ([4, 4, 0, 0], 8));
        assert_eq!(Games::slide([0, 2, 0, 2]), ([4, 0, 0, 0], 4));
    }

    #[test]
    fn a_new_tile_does_not_merge_with_the_one_it_just_made() {
        // 2+2 makes 4; that 4 must not then join the 4 after it
        assert_eq!(Games::slide([2, 2, 4, 0]), ([4, 4, 0, 0], 4));
        assert_eq!(Games::slide([4, 2, 2, 0]), ([4, 4, 0, 0], 4));
    }

    #[test]
    fn an_empty_or_full_unmatched_line_is_unchanged() {
        assert_eq!(Games::slide([0, 0, 0, 0]), ([0, 0, 0, 0], 0));
        assert_eq!(Games::slide([2, 4, 8, 16]), ([2, 4, 8, 16], 0));
    }

    #[test]
    fn a_controller_slides_only_the_2048_board() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.grid = [[0; N]; N];
        g.grid[0][0] = 2;
        g.game = Game::Mines;
        g.gamepad_slide(1);
        assert_eq!(g.grid[0][0], 2, "a slide on the minesweeper board must not touch the 2048 grid");
        g.game = Game::G2048;
        g.gamepad_slide(1);
        // the tile slides to the right edge; a spawn can only fill an empty cell, so the 2 stays there
        assert_eq!(g.grid[0][N - 1], 2);
    }

    #[test]
    fn a_controller_starts_a_new_game_only_once_the_game_is_over() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.score = 77;
        g.gamepad_new_game();
        assert_eq!(g.score, 77, "a game in progress is not dropped by a stray press");
        g.p2048.go(Phase::Lost);
        g.gamepad_new_game();
        assert_eq!(g.score, 0);
        assert_eq!(g.p2048.phase(), Phase::Playing);
    }

    #[test]
    fn reduced_effects_redraw_the_clock_every_five_seconds_and_the_record_stays_exact() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.game = Game::Mines;
        g.new_mines();
        g.started = Some(0.0);
        g.set_reduced_effects(true);
        g.tick(&ui, 2000.0);
        assert_eq!(g.elapsed, 0, "two seconds are not shown with a 5 s step");
        assert_eq!(g.next_wake(2000.0), Some(5000.0));
        g.tick(&ui, 5000.0);
        assert_eq!(g.elapsed, 5);
        g.set_reduced_effects(false);
        g.tick(&ui, 6000.0);
        assert_eq!(g.elapsed, 6);
        assert_eq!(g.next_wake(6000.0), Some(7000.0));
    }

    #[test]
    fn the_describe_dump_reports_the_effects_and_the_display() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        assert!(g.describe().contains("games_effects=full\ngames_display_pt=none\n"));
        g.set_reduced_effects(true);
        g.set_display(Some((1440.0, 816.0)));
        assert!(g.describe().contains("games_effects=reduced\ngames_display_pt=1440x816\n"));
    }

    #[test]
    fn a_win_celebrates_once_and_a_new_game_rearms_it() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.p2048.go(Phase::Won);
        g.note_win(1000.0);
        assert!(g.burst.is_some());
        g.burst = None;
        // still won: no second celebration
        g.note_win(2000.0);
        assert!(g.burst.is_none());
        // a new game clears the flag, so it does not celebrate while playing
        g.new_2048();
        g.note_win(3000.0);
        assert!(g.burst.is_none());
        // and its own win celebrates again
        g.p2048.go(Phase::Won);
        g.note_win(4000.0);
        assert!(g.burst.is_some());
    }

    #[test]
    fn a_celebration_asks_for_frames_until_its_dots_die_out() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        assert_eq!(g.next_wake(1000.0), None);
        assert!(g.describe().contains("games_burst=0\n"));
        g.burst = Some(Celebration::start(7, (600.0, 400.0), 1000.0));
        assert_eq!(g.next_wake(1000.0), Some(1016.0));
        // no dots exist until the first step
        assert!(g.describe().contains("games_burst=0\n"));
        // a tick every 100 ms, for 3 s: the burst has died out by about 2 s
        for k in 1..=30 {
            g.tick(&ui, 1000.0 + k as f64 * 100.0);
        }
        assert!(g.burst.is_none());
        assert_eq!(g.next_wake(4000.0), None);
    }
}

#[cfg(test)]
mod tile_tests {
    use super::*;

    #[test]
    fn a_merge_swells_the_merged_tile_and_the_new_tile_grows() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.grid = [[2, 2, 0, 0], [0; N], [0; N], [0; N]];
        g.now = 1.0;
        g.shift(0);
        assert_eq!(g.grid[0][0], 4);
        // the merged tile starts at full size and peaks 0.08 s later
        assert!((g.motion.scale((0, 0), 1.0) - 1.0).abs() < 1e-3);
        assert!((g.motion.scale((0, 0), 1.08) - 1.12).abs() < 1e-3);
        // the new tile is the one cell, other than the merged tile, that now holds a tile
        let fresh = (0..N)
            .flat_map(|r| (0..N).map(move |c| (r, c)))
            .find(|&(r, c)| (r, c) != (0, 0) && g.grid[r][c] != 0)
            .expect("a new tile");
        assert!((g.motion.scale(fresh, 1.0) - 0.5).abs() < 1e-3);
        assert!((g.motion.scale(fresh, 1.2) - 1.0).abs() < 1e-3);
        assert!(g.motion.is_live(1.05));
        assert!(!g.motion.is_live(1.5));
    }

    #[test]
    fn reduced_effects_plan_no_tile_motion() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.reduced = true;
        g.grid = [[2, 2, 0, 0], [0; N], [0; N], [0; N]];
        g.now = 1.0;
        g.shift(0);
        assert_eq!(g.grid[0][0], 4);
        assert_eq!(g.motion.live(1.01), 0);
    }

    #[test]
    fn a_controller_moves_the_tic_tac_toe_cursor_and_marks_the_square_under_it() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.game = Game::TicTacToe;
        g.gamepad_slide(0);
        assert_eq!(g.ttt.cursor(), 3, "left of the centre square");
        assert_eq!(g.ttt.marks(), 0, "a move of the cursor marks nothing");
        g.gamepad_slide(1);
        assert_eq!(g.ttt.cursor(), 4);
        g.gamepad_new_game();
        assert_eq!(g.ttt.cell(4), Some(Mark::X), "South marks the square under the cursor");
        // after a centre opening the computer's first best square is the top left corner
        assert_eq!(g.ttt.cell(0), Some(Mark::O));
        assert_eq!(g.ttt.marks(), 2);
        assert!(g.describe().contains("games_ttt_marks=2\ngames_ttt_phase=Playing\n"));
    }

    #[test]
    fn a_tic_tac_toe_board_is_not_restarted_by_a_stray_press() {
        let ui = test_ui();
        let mut g = Games::new(&ui, 1200, 780);
        g.game = Game::TicTacToe;
        g.gamepad_new_game();
        assert_eq!(g.ttt.marks(), 2);
        g.gamepad_new_game();
        assert_eq!(g.ttt.marks(), 2, "the square under the cursor is taken, and the game goes on");
        assert_eq!(g.ttt.cell(4), Some(Mark::X));
    }
}

