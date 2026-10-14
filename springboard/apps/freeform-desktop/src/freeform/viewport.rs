//! The board's view: how far it is magnified (`zoom`) and how far it is shifted (`pan`, in canvas pixels). Strokes
//! and items keep their own coordinates, the board's; only what is painted and what the pointer means go through the
//! view. A board point (x, y) appears on the picture at `(x * zoom + pan.0, y * zoom + pan.1)`.
use super::*;

/// The least and most the board is magnified.
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 4.0;
/// One Zoom In or Zoom Out multiplies or divides the zoom by this.
const ZOOM_STEP: f32 = 1.25;
/// One arrow key moves the view this far (points).
const PAN_STEP_PT: f32 = 48.0;

/// The point on the picture where board point `p` appears.
pub fn to_screen(zoom: f32, pan: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    (p.0 * zoom + pan.0, p.1 * zoom + pan.1)
}

/// The board point that appears at picture point `p`.
pub fn to_board(zoom: f32, pan: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    ((p.0 - pan.0) / zoom, (p.1 - pan.1) / zoom)
}

/// The shift that keeps the board point under picture point `anchor` where it is when the zoom changes to `new_zoom`.
pub fn pan_after_zoom(zoom: f32, pan: (f32, f32), new_zoom: f32, anchor: (f32, f32)) -> (f32, f32) {
    let board = to_board(zoom, pan, anchor);
    (anchor.0 - board.0 * new_zoom, anchor.1 - board.1 * new_zoom)
}

pub fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    }
}

impl Freeform {
    /// The view is the identity: nothing is magnified or shifted.
    pub(super) fn view_is_plain(&self) -> bool {
        self.zoom == 1.0 && self.pan == (0.0, 0.0)
    }

    /// The board point under picture point `p`.
    pub(super) fn board_pt(&self, p: (f32, f32)) -> (f32, f32) {
        to_board(self.zoom, self.pan, p)
    }

    /// Where board point `p` appears on the picture.
    pub(super) fn view_pt(&self, p: (f32, f32)) -> (f32, f32) {
        to_screen(self.zoom, self.pan, p)
    }

    /// Where a board box appears on the picture.
    pub(super) fn view_rect(&self, r: Rect) -> Rect {
        let (x, y) = self.view_pt((r.x, r.y));
        Rect { x, y, w: r.w * self.zoom, h: r.h * self.zoom }
    }

    /// `drawing` as it appears: every point moved by the view. Ink widths are not changed here; they are painted at
    /// the board's scale times the zoom.
    pub(super) fn view_drawing(&self, drawing: &Drawing) -> Drawing {
        let mut shown = drawing.clone();
        if !self.view_is_plain() {
            shown.transform(Affine::scale(self.zoom, self.zoom).then(Affine::translate(self.pan.0, self.pan.1)));
        }
        shown
    }

    /// The strokes of the board as they appear.
    pub(super) fn view_strokes(&self) -> Drawing {
        self.view_drawing(&self.model)
    }

    /// Paints the items as they appear into `px`, a picture of the canvas size. `editing` is the item and cell that
    /// shows the caret, at the caret position (none for an export).
    pub(super) fn paint_items(&self, px: &mut [u32], ui: &Ui, editing: Option<(u32, usize)>) {
        if self.view_is_plain() {
            items::paint(px, self.cw, self.ch, ui, &self.items, &self.pics.scaled, editing, self.caret, 1.0);
        } else {
            let shown = self.items.viewed(self.zoom, self.pan);
            items::paint(px, self.cw, self.ch, ui, &shown, &self.pics.scaled, editing, self.caret, self.zoom);
        }
    }

    /// The zoom as a whole number of per cent.
    pub fn zoom_percent(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    pub fn can_zoom_in(&self) -> bool {
        self.zoom < MAX_ZOOM - 1e-3
    }

    pub fn can_zoom_out(&self) -> bool {
        self.zoom > MIN_ZOOM + 1e-3
    }

    /// The view is not the plain one (Actual Size has something to do).
    pub fn view_changed_from_plain(&self) -> bool {
        !self.view_is_plain()
    }

    /// View > Zoom In: the board is magnified, the middle of the window staying where it is.
    pub fn zoom_in(&mut self, ui: &Ui) {
        self.set_zoom(ui, self.zoom * ZOOM_STEP);
    }

    /// View > Zoom Out.
    pub fn zoom_out(&mut self, ui: &Ui) {
        self.set_zoom(ui, self.zoom / ZOOM_STEP);
    }

    /// View > Actual Size: the zoom and the shift go back to the plain view.
    pub fn zoom_actual(&mut self, ui: &Ui) {
        if self.view_is_plain() {
            return;
        }
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.repaint(ui, None);
        self.say("Zoom 100%".to_string());
    }

    fn set_zoom(&mut self, ui: &Ui, wanted: f32) {
        let zoom = clamp_zoom(wanted);
        if (zoom - self.zoom).abs() < 1e-4 {
            return;
        }
        let middle = (self.cw as f32 / 2.0, self.ch as f32 / 2.0);
        self.pan = pan_after_zoom(self.zoom, self.pan, zoom, middle);
        self.zoom = zoom;
        self.repaint(ui, None);
        let percent = self.zoom_percent();
        self.say(format!("Zoom {percent}%"));
    }

    /// Moves the view by (dx, dy) picture pixels (positive moves the board right and down).
    pub fn pan_by(&mut self, ui: &Ui, dx: f32, dy: f32) {
        self.pan = (self.pan.0 + dx, self.pan.1 + dy);
        self.repaint(ui, None);
    }

    /// View > Hand Tool: a drag on the board moves the view.
    pub fn choose_hand(&mut self, ui: &Ui) {
        self.finish_edit(ui);
        self.tool = Tool::Hand;
        self.selection = None;
        self.pick = None;
        self.dirty = true;
    }

    /// The hand tool is in hand.
    pub fn hand_tool(&self) -> bool {
        self.tool == Tool::Hand
    }

    /// Arrow keys move the view when nothing is being typed.
    pub(super) fn arrow_pans(&mut self, ui: &Ui, code: u32) -> bool {
        let step = ui.pt(PAN_STEP_PT);
        let (dx, dy) = match code {
            37 => (step, 0.0),
            39 => (-step, 0.0),
            38 => (0.0, step),
            40 => (0.0, -step),
            _ => return false,
        };
        self.pan_by(ui, dx, dy);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ui() -> Ui {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn board(ui: &Ui) -> Freeform {
        let mut app = Freeform::new(ui, 800, 600);
        app.ensure_canvas(ui);
        app
    }

    fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> Stroke {
        Stroke { ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 }, points: vec![StrokePoint::at(x0, y0), StrokePoint::at(x1, y1)] }
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn the_view_maps_board_points_to_the_picture_and_back() {
        let (zoom, pan) = (2.0, (10.0, -20.0));
        assert_eq!(to_screen(zoom, pan, (5.0, 5.0)), (20.0, -10.0));
        assert_eq!(to_board(zoom, pan, (20.0, -10.0)), (5.0, 5.0));
        let p = (123.0, 45.5);
        assert!(near(to_board(zoom, pan, to_screen(zoom, pan, p)), p));
    }

    #[test]
    fn zooming_keeps_the_anchor_point_still() {
        let (zoom, pan) = (1.0, (0.0, 0.0));
        let anchor = (400.0, 300.0);
        let new_pan = pan_after_zoom(zoom, pan, 2.0, anchor);
        assert!(near(to_screen(2.0, new_pan, to_board(zoom, pan, anchor)), anchor));
        let again = pan_after_zoom(2.0, new_pan, 0.5, (100.0, 50.0));
        assert!(near(to_screen(0.5, again, to_board(2.0, new_pan, (100.0, 50.0))), (100.0, 50.0)));
    }

    #[test]
    fn the_zoom_is_kept_between_the_limits() {
        assert_eq!(clamp_zoom(100.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(0.0001), MIN_ZOOM);
        assert_eq!(clamp_zoom(f32::NAN), 1.0);
        assert_eq!(clamp_zoom(1.5), 1.5);
    }

    #[test]
    fn zoom_in_and_out_stop_at_the_limits_and_actual_size_goes_back() {
        let ui = test_ui();
        let mut app = board(&ui);
        assert!(app.can_zoom_in() && app.can_zoom_out() && !app.view_changed_from_plain());
        for _ in 0..20 {
            app.zoom_in(&ui);
        }
        assert_eq!(app.zoom, MAX_ZOOM);
        assert!(!app.can_zoom_in());
        for _ in 0..40 {
            app.zoom_out(&ui);
        }
        assert_eq!(app.zoom, MIN_ZOOM);
        assert!(!app.can_zoom_out());
        assert_eq!(app.zoom_percent(), 25);
        app.zoom_actual(&ui);
        assert_eq!((app.zoom, app.pan), (1.0, (0.0, 0.0)));
        assert!(!app.view_changed_from_plain());
        assert_eq!(app.zoom_percent(), 100);
    }

    #[test]
    fn the_middle_of_the_window_stays_put_while_zooming() {
        let ui = test_ui();
        let mut app = board(&ui);
        let middle = (app.cw as f32 / 2.0, app.ch as f32 / 2.0);
        let board_point = app.board_pt(middle);
        app.zoom_in(&ui);
        app.zoom_in(&ui);
        assert!(near(app.view_pt(board_point), middle));
        assert_eq!(app.zoom_percent(), 156);
    }

    #[test]
    fn strokes_are_painted_through_the_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        let (cx, cy) = (app.cw as f32 / 2.0, app.ch as f32 / 2.0);
        app.model = Drawing::from_strokes(vec![line(cx - 20.0, cy, cx + 20.0, cy)]);
        app.repaint(&ui, None);
        let at = |app: &Freeform, x: f32, y: f32| app.canvas[y as usize * app.cw + x as usize] & 255;
        assert!(at(&app, cx + 10.0, cy) < 100, "on the stroke at 100%");
        assert!(at(&app, cx + 30.0, cy) > 200, "off its end at 100%");
        app.zoom_in(&ui);
        app.zoom_in(&ui);
        app.zoom_in(&ui);
        assert!(at(&app, cx + 30.0, cy) < 100, "the stroke reaches 30 px right of the middle at 195%");
        assert!(at(&app, cx + 10.0, cy) < 100, "and still covers the middle");
        assert!(at(&app, cx, cy + 4.0 * app.zoom) > 200, "and is thicker, but not by more than the zoom");
    }

    #[test]
    fn items_are_painted_through_the_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        let (cx, cy) = (app.cw as f32 / 2.0, app.ch as f32 / 2.0);
        app.items.add(Item { color: items::NOTE_COLORS[3], ..Item::new(ItemKind::Sticky, cx - 50.0, cy - 50.0, 100.0, 100.0) });
        app.repaint(&ui, None);
        let at = |app: &Freeform, x: f32, y: f32| app.canvas[y as usize * app.cw + x as usize];
        assert_eq!(at(&app, cx, cy + 45.0), items::NOTE_COLORS[3]);
        assert_eq!(at(&app, cx, cy + 90.0), PAPER, "100 px below the middle is paper at 100%");
        app.zoom = 2.0;
        app.pan = (cx - cx * 2.0, cy - cy * 2.0);
        app.repaint(&ui, None);
        assert_eq!(at(&app, cx, cy + 90.0), items::NOTE_COLORS[3], "the note reaches 100 px below the middle at 200%");
    }

    #[test]
    fn a_pen_stroke_at_another_zoom_is_stored_in_board_coordinates() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.zoom = 2.0;
        app.pan = (10.0, 20.0);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Pen;
        app.touch(&ui, Event::Down(110.0, top + 120.0), 0.0);
        app.touch(&ui, Event::Move(210.0, top + 120.0), 0.0);
        app.touch(&ui, Event::Up(210.0, top + 120.0), 0.0);
        let pts = &app.model.strokes()[0].points;
        let (first, last) = (&pts[0], &pts[pts.len() - 1]);
        assert!(near((first.x, first.y), (50.0, 50.0)), "({}, {})", first.x, first.y);
        assert!(near((last.x, last.y), (100.0, 50.0)), "({}, {})", last.x, last.y);
        // painted where the pointer was
        assert!(app.canvas[120 * app.cw + 160] & 255 < 100);
    }

    #[test]
    fn a_press_at_another_zoom_hits_the_item_under_it() {
        let ui = test_ui();
        let mut app = board(&ui);
        let id = app.items.add(Item::new(ItemKind::Sticky, 100.0, 100.0, 100.0, 100.0));
        app.zoom = 2.0;
        app.pan = (0.0, 0.0);
        app.tool = Tool::Lasso;
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(250.0, top + 250.0), 0.0);
        app.touch(&ui, Event::Up(250.0, top + 250.0), 0.0);
        assert_eq!(app.pick, Some(Pick::Item(id)), "screen (250, 250) is board (125, 125)");
        app.touch(&ui, Event::Down(150.0, top + 150.0), 5000.0);
        app.touch(&ui, Event::Up(150.0, top + 150.0), 5000.0);
        assert!(app.pick.is_none(), "screen (150, 150) is board (75, 75), outside the note");
    }

    #[test]
    fn an_item_dragged_while_zoomed_moves_by_the_board_distance() {
        let ui = test_ui();
        let mut app = board(&ui);
        let id = app.items.add(Item::new(ItemKind::Sticky, 100.0, 100.0, 100.0, 100.0));
        app.zoom = 2.0;
        app.tool = Tool::Lasso;
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(250.0, top + 250.0), 0.0);
        app.touch(&ui, Event::Move(290.0, top + 270.0), 0.0);
        app.touch(&ui, Event::Up(290.0, top + 270.0), 0.0);
        let it = app.items.get(id).unwrap();
        assert!(near((it.x, it.y), (120.0, 110.0)), "40 and 20 screen pixels are 20 and 10 board points at 200%");
    }

    #[test]
    fn the_hand_tool_drags_the_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.choose_hand(&ui);
        assert!(app.hand_tool());
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(100.0, top + 100.0), 0.0);
        assert_eq!(app.pointer_for(&ui, 100.0, top + 100.0), Kind::ClosedHand);
        app.touch(&ui, Event::Move(130.0, top + 90.0), 0.0);
        app.touch(&ui, Event::Move(150.0, top + 95.0), 0.0);
        app.touch(&ui, Event::Up(150.0, top + 95.0), 0.0);
        assert_eq!(app.pan, (50.0, -5.0));
        assert!(app.model.is_empty() && app.undo.is_empty(), "moving the view draws and changes nothing");
        assert_eq!(app.pointer_for(&ui, 100.0, top + 100.0), Kind::OpenHand);
    }

    #[test]
    fn the_arrow_keys_move_the_view_but_not_while_typing() {
        let ui = test_ui();
        let mut app = board(&ui);
        assert!(app.key(&ui, &KeyInput::Code(39)));
        assert_eq!(app.pan, (-PAN_STEP_PT, 0.0), "right moves the view right: the board goes left");
        assert!(app.key(&ui, &KeyInput::Code(40)));
        assert!(app.key(&ui, &KeyInput::Code(37)));
        assert!(app.key(&ui, &KeyInput::Code(38)));
        assert_eq!(app.pan, (0.0, 0.0));
        app.insert_note(&ui);
        assert!(!app.key(&ui, &KeyInput::Code(39)), "typing owns the keys");
        assert_eq!(app.pan, (0.0, 0.0));
    }

    #[test]
    fn undo_after_a_zoom_paints_the_board_again_at_the_next_draw() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = Drawing::from_strokes(vec![line(10.0, 10.0, 50.0, 10.0)]);
        app.push_undo();
        app.zoom_in(&ui);
        app.undo_step();
        assert!(app.repaint_due, "the saved picture was made at another zoom");
        app.repaint(&ui, None);
        assert!(!app.repaint_due);
    }

    #[test]
    fn a_new_board_starts_at_the_plain_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.zoom_in(&ui);
        app.pan_by(&ui, 30.0, 30.0);
        app.new_board(&ui);
        assert_eq!((app.zoom, app.pan), (1.0, (0.0, 0.0)));
    }

    #[test]
    fn a_new_note_appears_in_the_middle_of_what_is_in_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        let middle = (app.cw as f32 / 2.0, app.ch as f32 / 2.0);
        app.zoom = 2.0;
        app.pan = (middle.0 - 50.0 * 2.0, middle.1 - 70.0 * 2.0);
        app.insert_note(&ui);
        let it = app.items.list()[0].clone();
        assert!(near((it.x + it.w / 2.0, it.y + it.h / 2.0), (50.0, 70.0)), "board (50, 70) is at the middle of the window");
    }

    #[test]
    fn the_export_follows_the_view() {
        let ui = test_ui();
        let mut app = board(&ui);
        let (cx, cy) = (app.cw as f32 / 2.0, app.ch as f32 / 2.0);
        app.model = Drawing::from_strokes(vec![line(cx - 20.0, cy, cx + 20.0, cy)]);
        app.zoom_in(&ui);
        app.zoom_in(&ui);
        app.zoom_in(&ui);
        let page = app.pdf_pages(&ui).remove(0);
        let text = String::from_utf8(page.content).unwrap();
        // 20 points either side at 195.3% reach about 39 points either side of the middle
        let from = format!("{:.2}", cx - 20.0 * app.zoom);
        assert!(text.contains(&from), "{from} in {text}");
        assert!(text.contains(&format!("{:.2} w", 4.0 * app.zoom)), "the line is as thick as it looks: {text}");
        assert_eq!(&app.export_png(&ui)[1..4], b"PNG");
    }
}
