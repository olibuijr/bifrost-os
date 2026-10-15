//! Chess: a board with the full rules, a move list in standard notation, undo, and a computer opponent at four levels
//! or a second player at the same board. The board is drawn on the canvas; a tap on a square selects a piece, and a
//! tap on a highlighted square moves it. A pawn that reaches the last rank asks for the piece it becomes.

mod ai;
mod board;
mod game;
mod moves;

use ai::Level;
use board::{name as square_name, parse_square, sq, Colour, Kind, Piece, Position, Sq};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{Ui, Weight, CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MATERIAL_SELECTION, ROW_PRESSED, SEGMENT_ON, SYSTEM_YELLOW, WHITE, BLACK};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::widgets;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind};
use game::{describe, Game};
use moves::Move;

const MENU_NEW: u32 = 1;
const MENU_UNDO: u32 = 2;
const MENU_TWO_PLAYERS: u32 = 10;
/// Menu ids of the levels: `MENU_LEVEL_BASE` plus the index in `Level::ALL`.
const MENU_LEVEL_BASE: u32 = 11;

/// Space around the board and the panel, in points.
const PAD: f32 = 16.0;
const PANEL_W: f32 = 260.0;
const BUTTON_H: f32 = 26.0;
const GAP: f32 = 8.0;
const MOVE_ROW_H: f32 = 20.0;


/// What a tap does. The hit list is rebuilt on every frame, so a tap always meets the layout on screen.
#[derive(Clone, Copy, PartialEq)]
enum Act {
    Square(Sq),
    Promote(Kind),
    NewGame,
    Undo,
    /// `None` is two players at one board.
    Opponent(Option<Level>),
}

/// A tappable rectangle in points.
struct Hit {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    act: Act,
}

#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct ChessApp {
    game: Game,
    /// The computer's level; the computer plays Black. `None` is two players.
    opponent: Option<Level>,
    selected: Option<Sq>,
    /// A pawn move to the last rank, waiting for the piece it becomes: the from and to squares.
    pending: Option<(Sq, Sq)>,
    hits: Vec<Hit>,
    save_path: Option<std::path::PathBuf>,
}

/// Pointer position in points: input is in pixels, and the layout is in points.
fn to_points(ctx: &Ctx, x: f32, y: f32) -> (f32, f32) {
    let scale = ctx.ui.pt(1.0).max(0.001);
    (x / scale, y / scale)
}

/// The letter a piece is drawn with. Pawns have no letter in notation, so they are drawn as "P".
fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Pawn => "P",
        other => other.letter(),
    }
}

fn plate(canvas: &mut dyn Canvas, ui: &Ui, r: Rect, radius: f32, colour: u32) {
    canvas.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(radius), colour);
}

/// Draws `text` fitted to `w` points, centred in its box or left aligned at its left edge, vertically centred.
#[allow(clippy::too_many_arguments)]
fn label(canvas: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, colour: u32, r: Rect, centred: bool) {
    let img = ui.text(&ui.fit(text, pt, ui.pt(r.w)), pt, weight, colour);
    let left = if centred { ui.pt(r.x) + (ui.pt(r.w) - img.w as f32) / 2.0 } else { ui.pt(r.x) };
    let top = ui.pt(r.y) + (ui.pt(r.h) - img.h as f32) / 2.0;
    widgets::blit(canvas, &img, left as i32, top as i32);
}

/// A flat button with a centred label. Without an action it is drawn dimmed and does not respond to taps.
fn button(canvas: &mut dyn Canvas, ui: &Ui, hits: &mut Vec<Hit>, r: Rect, text: &str, act: Option<Act>, strong: bool) {
    plate(canvas, ui, r, 6.0, if strong { SEGMENT_ON.now() } else { FIELD_BG.now() });
    let colour = if act.is_some() { LABEL.now() } else { LABEL_3.now() };
    label(canvas, ui, text, MAC_SUBHEAD_PT, Weight::Regular, colour, r, true);
    if let Some(act) = act {
        hits.push(Hit { x: r.x, y: r.y, w: r.w, h: r.h, act });
    }
}

/// One piece on its square: the letter in its colour with a thin outline in the other colour, so it reads on both
/// square colours.
fn draw_piece(canvas: &mut dyn Canvas, ui: &Ui, piece: Piece, square: Rect) {
    let (ink, outline) = match piece.colour {
        Colour::White => (WHITE, BLACK),
        Colour::Black => (BLACK, WHITE),
    };
    let pt = square.w * 0.62;
    let text = glyph(piece.kind);
    let img = ui.text(text, pt, Weight::Bold, ink);
    let edge = ui.text(text, pt, Weight::Bold, outline);
    let left = ui.pt(square.x) + (ui.pt(square.w) - img.w as f32) / 2.0;
    let top = ui.pt(square.y) + (ui.pt(square.h) - img.h as f32) / 2.0;
    for (dx, dy) in [(-1.0f32, 0.0f32), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        widgets::blit(canvas, &edge, (left + dx) as i32, (top + dy) as i32);
    }
    widgets::blit(canvas, &img, left as i32, top as i32);
}

impl ChessApp {
    fn new() -> ChessApp {
        ChessApp { game: Game::new(), opponent: Some(Level::Medium), selected: None, pending: None, hits: Vec::new(), save_path: None }
    }

    /// True when it is the person's turn: always, in two-player mode; otherwise when it is not the computer's move.
    fn is_human_turn(&self) -> bool {
        self.opponent.is_none() || self.game.turn() == Colour::White
    }

    fn new_game(&mut self) {
        self.game = Game::new();
        self.selected = None;
        self.pending = None;
    }

    fn set_opponent(&mut self, level: Option<Level>) {
        self.opponent = level;
        self.selected = None;
        self.pending = None;
        self.computer_reply();
    }

    fn play(&mut self, mv: Move) {
        self.game.play(mv);
        self.selected = None;
        self.pending = None;
        self.computer_reply();
    }

    /// The computer answers when it is its move and the game goes on.
    fn computer_reply(&mut self) {
        let Some(level) = self.opponent else {
            return;
        };
        if self.game.turn() != Colour::Black || self.game.status().is_over() {
            return;
        }
        let seed = self.game.moves_played() as u64 + 1;
        if let Some(mv) = ai::choose(self.game.position(), level, seed) {
            self.game.play(mv);
        }
    }

    /// Takes back the last move, and with the computer also its answer, so the person is on the move again.
    fn undo(&mut self) {
        if !self.game.undo() {
            return;
        }
        if self.opponent.is_some() && self.game.turn() == Colour::Black {
            self.game.undo();
        }
        self.selected = None;
        self.pending = None;
    }

    fn tap_square(&mut self, s: Sq) {
        if self.game.status().is_over() || !self.is_human_turn() {
            return;
        }
        self.pending = None;
        if let Some(from) = self.selected {
            let options: Vec<Move> = self.game.legal_moves().into_iter().filter(|m| m.from == from && m.to == s).collect();
            if options.len() > 1 {
                // only a promotion has several moves between the same two squares
                self.pending = Some((from, s));
                self.selected = None;
                return;
            }
            if let Some(mv) = options.first().copied() {
                self.play(mv);
                return;
            }
        }
        let own = matches!(self.game.position().at(s), Some(p) if p.colour == self.game.turn());
        self.selected = if own { Some(s) } else { None };
    }

    fn promote(&mut self, kind: Kind) {
        if let Some((from, to)) = self.pending {
            self.play(Move::promoting(from, to, kind));
        }
    }

    fn perform(&mut self, act: Act) {
        match act {
            Act::Square(s) => self.tap_square(s),
            Act::Promote(kind) => self.promote(kind),
            Act::NewGame => self.new_game(),
            Act::Undo => self.undo(),
            Act::Opponent(level) => self.set_opponent(level),
        }
    }

    /// The action under a point, the topmost one drawn there.
    fn hit(&self, px: f32, py: f32) -> Option<Act> {
        self.hits.iter().rev().find(|h| px >= h.x && px < h.x + h.w && py >= h.y && py < h.y + h.h).map(|h| h.act)
    }

    fn draw_board(&mut self, canvas: &mut dyn Canvas, ui: &Ui, x0: f32, y0: f32, square: f32) {
        let light_square = CARD_BG.now();
        let dark_square = FIELD_BG.now();
        let last_move_tint = ROW_PRESSED.now();
        let selected_tint = MATERIAL_SELECTION.now();
        let pos: Position = *self.game.position();
        let last = self.game.last_move();
        let targets: Vec<Sq> = match self.selected {
            Some(from) => self.game.legal_moves().iter().filter(|m| m.from == from).map(|m| m.to).collect(),
            None => Vec::new(),
        };
        let king_in_check = if pos.in_check(pos.turn) { pos.king_square(pos.turn) } else { None };

        for rank in 0..8u8 {
            for file in 0..8u8 {
                let s = sq(file, rank);
                // rank 8 is at the top of the board
                let cell = Rect {
                    x: x0 + file as f32 * square,
                    y: y0 + (7 - rank) as f32 * square,
                    w: square,
                    h: square,
                };
                let light = (file + rank) % 2 == 1;
                canvas.fill_rect(
                    RectF::new(ui.pt(cell.x), ui.pt(cell.y), ui.pt(cell.w), ui.pt(cell.h)),
                    if light { light_square } else { dark_square },
                );
                if let Some(m) = last {
                    if m.from == s || m.to == s {
                        canvas.fill_rect(RectF::new(ui.pt(cell.x), ui.pt(cell.y), ui.pt(cell.w), ui.pt(cell.h)), last_move_tint);
                    }
                }
                if self.selected == Some(s) {
                    canvas.fill_rect(RectF::new(ui.pt(cell.x), ui.pt(cell.y), ui.pt(cell.w), ui.pt(cell.h)), selected_tint);
                }
                if king_in_check == Some(s) {
                    canvas.fill_rect(RectF::new(ui.pt(cell.x), ui.pt(cell.y), ui.pt(cell.w), ui.pt(cell.h)), SYSTEM_YELLOW);
                }
                if targets.contains(&s) {
                    let cx = ui.pt(cell.x + square / 2.0);
                    let cy = ui.pt(cell.y + square / 2.0);
                    let r = ui.pt(square * 0.16);
                    canvas.fill_rrect(RectF::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, LABEL_2.now());
                }
                if let Some(piece) = pos.at(s) {
                    draw_piece(canvas, ui, piece, cell);
                }
                self.hits.push(Hit { x: cell.x, y: cell.y, w: cell.w, h: cell.h, act: Act::Square(s) });
            }
        }
    }

    fn draw_panel(&mut self, canvas: &mut dyn Canvas, ui: &Ui, x: f32, top: f32, w: f32, height: f32) {
        let full = |y: f32, h: f32| Rect { x, y, w, h };
        let half_w = (w - GAP) / 2.0;

        let status = describe(self.game.status());
        label(canvas, ui, &status, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), full(top, 20.0), false);

        // the opponent: two players, or one of the levels; two buttons to a row
        let mut y = top + 30.0;
        let mut modes: Vec<Option<Level>> = vec![None];
        modes.extend(Level::ALL.iter().map(|level| Some(*level)));
        for (i, mode) in modes.iter().enumerate() {
            let col = (i % 2) as f32;
            let row = (i / 2) as f32;
            let r = Rect { x: x + col * (half_w + GAP), y: y + row * (BUTTON_H + GAP), w: half_w, h: BUTTON_H };
            let name = mode.map_or("Two Players", |level| level.name());
            let strong = self.opponent == *mode;
            button(canvas, ui, &mut self.hits, r, name, Some(Act::Opponent(*mode)), strong);
        }
        let rows = ((modes.len() + 1) / 2) as f32;
        y += rows * (BUTTON_H + GAP) + GAP;

        let undo = if self.game.moves_played() > 0 { Some(Act::Undo) } else { None };
        button(canvas, ui, &mut self.hits, Rect { x, y, w: half_w, h: BUTTON_H }, "New Game", Some(Act::NewGame), false);
        button(canvas, ui, &mut self.hits, Rect { x: x + half_w + GAP, y, w: half_w, h: BUTTON_H }, "Undo", undo, false);
        y += BUTTON_H + 2.0 * GAP;

        if self.pending.is_some() {
            label(canvas, ui, "Promote the pawn to", MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), full(y, 16.0), false);
            y += 20.0;
            let kinds = [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight];
            let quarter = (w - 3.0 * GAP) / 4.0;
            for (i, kind) in kinds.iter().enumerate() {
                let r = Rect { x: x + i as f32 * (quarter + GAP), y, w: quarter, h: BUTTON_H };
                button(canvas, ui, &mut self.hits, r, kind.name(), Some(Act::Promote(*kind)), false);
            }
            y += BUTTON_H + 2.0 * GAP;
        }

        label(canvas, ui, "Moves", MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), full(y, 16.0), false);
        y += 20.0;
        let notation = self.game.notation();
        if notation.is_empty() {
            label(canvas, ui, "No moves yet", MAC_BODY_PT, Weight::Regular, LABEL_3.now(), full(y, MOVE_ROW_H), false);
            return;
        }
        // one row per full move, the newest at the bottom
        let rows_total = (notation.len() + 1) / 2;
        let fit = ((height - PAD - y) / MOVE_ROW_H).floor().max(0.0) as usize;
        let first = rows_total.saturating_sub(fit);
        let col_w = (w - 30.0) / 2.0;
        for row in first..rows_total {
            let ry = y + (row - first) as f32 * MOVE_ROW_H;
            label(canvas, ui, &format!("{}.", row + 1), MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Rect { x, y: ry, w: 26.0, h: MOVE_ROW_H }, false);
            if let Some(white) = notation.get(2 * row) {
                label(canvas, ui, white, MAC_BODY_PT, Weight::Regular, LABEL.now(), Rect { x: x + 30.0, y: ry, w: col_w, h: MOVE_ROW_H }, false);
            }
            if let Some(black) = notation.get(2 * row + 1) {
                label(canvas, ui, black, MAC_BODY_PT, Weight::Regular, LABEL.now(), Rect { x: x + 30.0 + col_w, y: ry, w: col_w, h: MOVE_ROW_H }, false);
            }
        }
    }
}

impl App for ChessApp {
    fn id(&self) -> &str {
        "is.olibuijr.chess.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let path = ctx.data_dir.join("game.txt");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Some((level, game)) = restore_game(&contents) {
                self.opponent = level;
                self.game = game;
            }
        }
        self.save_path = Some(path);
        if self.persist() {
            ctx.enable_sudden_termination();
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Up, x, y, .. } => {
                let (px, py) = to_points(ctx, x, y);
                match self.hit(px, py) {
                    Some(act) => {
                        ctx.disable_sudden_termination();
                        self.perform(act);
                        if self.persist() { ctx.enable_sudden_termination(); }
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let mut items = vec![
            AppMenuItem::new(MENU_NEW, "New Game").key("cmd+n"),
            AppMenuItem::new(MENU_UNDO, "Undo Move").key("cmd+z").enabled(self.game.moves_played() > 0),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_TWO_PLAYERS, "Two Players").checked(self.opponent.is_none()),
        ];
        for (i, level) in Level::ALL.iter().enumerate() {
            items.push(AppMenuItem::new(MENU_LEVEL_BASE + i as u32, level.name()).checked(self.opponent == Some(*level)));
        }
        vec![AppMenu { title: "Game".into(), items }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        ctx.disable_sudden_termination();
        match id {
            MENU_NEW => self.new_game(),
            MENU_UNDO => self.undo(),
            MENU_TWO_PLAYERS => self.set_opponent(None),
            id if id >= MENU_LEVEL_BASE && id < MENU_LEVEL_BASE + Level::ALL.len() as u32 => {
                let level = Level::ALL[(id - MENU_LEVEL_BASE) as usize];
                self.set_opponent(Some(level));
            }
            _ => return false,
        }
        if self.persist() { ctx.enable_sudden_termination(); }
        true
    }

    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (cw, ch) = canvas.size();
        let ui = &ctx.ui;
        let scale = ui.pt(1.0).max(0.001);
        let width = cw as f32 / scale;
        let height = ch as f32 / scale;
        self.hits.clear();
        canvas.fill_rect(RectF::new(0.0, 0.0, cw as f32, ch as f32), CONTENT_BG.now());

        let board = (height - 2.0 * PAD).min(width - PANEL_W - 3.0 * PAD).max(160.0);
        let square = board / 8.0;
        self.draw_board(canvas, ui, PAD, PAD, square);
        let panel_x = PAD + board + PAD;
        let panel_w = (width - panel_x - PAD).max(120.0);
        self.draw_panel(canvas, ui, panel_x, PAD, panel_w, height);

        let mut d = Damage::new(cw as usize, ch as usize);
        d.all();
        d
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

impl ChessApp {
    fn persist(&self) -> bool {
        let Some(path) = &self.save_path else { return false };
        let Some(parent) = path.parent() else { return false };
        if std::fs::create_dir_all(parent).is_err() { return false; }
        let mut body = String::new();
        body.push_str(match self.opponent {
            None => "two\n",
            Some(Level::Beginner) => "beginner\n",
            Some(Level::Easy) => "easy\n",
            Some(Level::Medium) => "medium\n",
            Some(Level::Hard) => "hard\n",
        });
        for mv in self.game.moves_played_list() {
            body.push_str(&square_name(mv.from));
            body.push_str(&square_name(mv.to));
            if let Some(kind) = mv.promotion {
                body.push(match kind { Kind::Queen => 'q', Kind::Rook => 'r', Kind::Bishop => 'b', Kind::Knight => 'n', _ => return false });
            }
            body.push('\n');
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path)).is_ok()
    }
}

fn restore_game(contents: &str) -> Option<(Option<Level>, Game)> {
    let mut lines = contents.lines();
    let opponent = match lines.next()? {
        "two" => None,
        "beginner" => Some(Level::Beginner),
        "easy" => Some(Level::Easy),
        "medium" => Some(Level::Medium),
        "hard" => Some(Level::Hard),
        _ => return None,
    };
    let mut game = Game::new();
    for line in lines {
        let bytes = line.as_bytes();
        if bytes.len() != 4 && bytes.len() != 5 { return None; }
        let from = parse_square(line.get(..2)?)?;
        let to = parse_square(line.get(2..4)?)?;
        let mv = if bytes.len() == 5 {
            let kind = match bytes[4] { b'q' => Kind::Queen, b'r' => Kind::Rook, b'b' => Kind::Bishop, b'n' => Kind::Knight, _ => return None };
            Move::promoting(from, to, kind)
        } else { Move::new(from, to) };
        if !game.play(mv) { return None; }
    }
    Some((opponent, game))
}

fn main() {
    bifrost_app::run(ChessApp::new());
}
