//! The progress card for nearby transfers: one row per running or just-finished
//! transfer, pinned under the menu bar. The rows come from `net::share::Tracker`.

use crate::gfx::Surface;
use crate::net::share::{human_size, Status, Transfer};
use crate::shell::ui::{Ui, Weight};

const W_PT: f32 = 300.0;
const ROW_PT: f32 = 56.0;
const GAP_PT: f32 = 8.0;
const PAD_PT: f32 = 12.0;
const BAR_PT: f32 = 4.0;
const RADIUS_PT: f32 = 6.0;
/// Rows drawn at once; the rest wait their turn.
pub const MAX_ROWS: usize = 3;

/// The headline: "Sending a.bin", "Received a.bin", ...
pub fn title(t: &Transfer) -> String {
    match (&t.status, t.outgoing) {
        (Status::Running, true) => format!("Sending {}", t.name),
        (Status::Running, false) => format!("Receiving {}", t.name),
        (Status::Done, true) => format!("Sent {}", t.name),
        (Status::Done, false) => format!("Received {}", t.name),
        (Status::Failed(_), _) => format!("Could not share {}", t.name),
    }
}

/// The second line: the other device, then the progress or the reason it stopped.
pub fn detail(t: &Transfer) -> String {
    let who = if t.outgoing { format!("to {}", t.peer) } else { format!("from {}", t.peer) };
    match &t.status {
        Status::Running if t.total > 0 => format!("{who} · {} of {}", human_size(t.done), human_size(t.total)),
        Status::Running | Status::Done => who,
        Status::Failed(e) => format!("{who} · {e}"),
    }
}

/// How much of the bar is filled, from 0 to 1.
pub fn fraction(t: &Transfer) -> f32 {
    match &t.status {
        Status::Done => 1.0,
        Status::Failed(_) => 0.0,
        Status::Running if t.total == 0 => 0.0,
        Status::Running => t.done as f32 / t.total as f32,
    }
}

/// The card rows' rectangles (x, y, w, h) in px: top centre, just below the menu bar.
pub fn rows(ui: &Ui, w: i32, count: usize) -> Vec<(f32, f32, f32, f32)> {
    let (pw, rh, gap) = (ui.pt(W_PT), ui.pt(ROW_PT), ui.pt(GAP_PT));
    let x = (w as f32 - pw) / 2.0;
    let top = ui.pt(crate::scene::STATUS_PT) + gap;
    (0..count.min(MAX_ROWS)).map(|i| (x, top + i as f32 * (rh + gap), pw, rh)).collect()
}

/// Paints the rows for `transfers` (newest last, as the tracker keeps them).
pub fn draw(transfers: &[Transfer], s: &mut Surface<'_>, ui: &Ui, w: i32) {
    for (t, (x, y, pw, rh)) in transfers.iter().zip(rows(ui, w, transfers.len())) {
        s.fill_rrect(x, y, pw, rh, ui.pt(RADIUS_PT), 0xFF2C2C2E);
        let (pad, inner) = (ui.pt(PAD_PT), pw - ui.pt(PAD_PT) * 2.0);
        let head = ui.text(&ui.fit(&title(t), 13.0, inner), 13.0, Weight::Semibold, 0xFFFFFFFF);
        s.blit(&head, (x + pad) as i32, (y + ui.pt(8.0)) as i32, 255, 255);
        let line = ui.text(&ui.fit(&detail(t), 11.0, inner), 11.0, Weight::Regular, 0xFF98989D);
        s.blit(&line, (x + pad) as i32, (y + ui.pt(26.0)) as i32, 255, 255);
        let (bar_y, bar_h) = (y + rh - ui.pt(BAR_PT) - ui.pt(8.0), ui.pt(BAR_PT));
        s.fill_rrect(x + pad, bar_y, inner, bar_h, bar_h / 2.0, 0xFF48484A);
        let color = if matches!(t.status, Status::Failed(_)) { 0xFFFF453A } else { 0xFF0A84FF };
        let filled = inner * fraction(t);
        if filled > 0.0 {
            s.fill_rrect(x + pad, bar_y, filled, bar_h, bar_h / 2.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn transfer(outgoing: bool, status: Status, done: u64, total: u64) -> Transfer {
        Transfer { id: 1, outgoing, peer: "Titan".into(), name: "a.bin".into(), done, total, status, finished: Some(Instant::now()) }
    }

    #[test]
    fn titles_follow_direction_and_outcome() {
        assert_eq!(title(&transfer(true, Status::Running, 0, 1)), "Sending a.bin");
        assert_eq!(title(&transfer(false, Status::Running, 0, 1)), "Receiving a.bin");
        assert_eq!(title(&transfer(true, Status::Done, 1, 1)), "Sent a.bin");
        assert_eq!(title(&transfer(false, Status::Done, 1, 1)), "Received a.bin");
        assert_eq!(title(&transfer(true, Status::Failed("declined".into()), 0, 1)), "Could not share a.bin");
    }

    #[test]
    fn details_show_the_peer_and_progress() {
        assert_eq!(detail(&transfer(true, Status::Running, 512, 1024)), "to Titan · 512 B of 1.0 KB");
        assert_eq!(detail(&transfer(false, Status::Running, 0, 0)), "from Titan");
        assert_eq!(detail(&transfer(true, Status::Failed("declined".into()), 0, 1)), "to Titan · declined");
    }

    #[test]
    fn bar_fills_with_progress_and_completes() {
        assert_eq!(fraction(&transfer(true, Status::Running, 1, 4)), 0.25);
        assert_eq!(fraction(&transfer(true, Status::Running, 0, 0)), 0.0);
        assert_eq!(fraction(&transfer(true, Status::Done, 4, 4)), 1.0);
        assert_eq!(fraction(&transfer(true, Status::Failed("x".into()), 2, 4)), 0.0);
    }

    #[test]
    fn rows_stack_under_the_menu_bar_and_are_capped() {
        let ui = crate::shell::tests::ui();
        let r = rows(&ui, 1200, 5);
        assert_eq!(r.len(), MAX_ROWS);
        assert!(r[0].1 >= ui.pt(crate::scene::STATUS_PT));
        assert!(r[1].1 > r[0].1 + r[0].3 - 1.0, "rows do not overlap");
        assert!((r[0].0 + r[0].2 / 2.0 - 600.0).abs() < 1.0, "centred");
        assert!(rows(&ui, 1200, 0).is_empty());
    }
}
