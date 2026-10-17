//! The purchase sheet for a paid app. GET on a paid app opens it: the app, its price, then Buy or Cancel. When a passcode is
//! set, Buy asks for it on the keypad first. The sheet only decides; the store records a confirmed purchase in the
//! entitlement ledger (`AppStore::finish_purchase`). Nothing is charged to a payment account from here.

use crate::shell::passcode::{self, retry_text, Keypad, Pad, Verdict};
use crate::shell::ui::Ui;
use crate::shell::Event;
use bifrost_ui::purchase::{Price, Purchase};
use std::path::Path;

/// What a touch on the sheet decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetOut {
    /// The customer confirmed (and gave the passcode when one is set): the purchase may be recorded.
    Confirmed,
    Cancelled,
}

/// A rectangle in pane pixels: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// One open purchase sheet.
pub struct Sheet {
    pub id: String,
    pub name: String,
    pub purchase: Purchase,
    /// A line under the price, e.g. a wrong-passcode message.
    pub message: String,
    passcode: passcode::Store,
    keypad: Option<Keypad>,
    /// The installed account's content limits switch purchases off.
    restricted: bool,
}

impl Sheet {
    /// Opens the sheet for `id` at `price`, showing the confirmation. The passcode is read from `root/passcode.ini`.
    pub fn open(root: &Path, id: &str, name: &str, price: Price) -> Self {
        let mut purchase = Purchase::new(id, price);
        purchase.begin();
        Sheet {
            id: id.to_string(),
            name: name.to_string(),
            purchase,
            message: String::new(),
            passcode: passcode::Store::load(root.join("passcode.ini")),
            keypad: None,
            restricted: crate::bare::account_policy::feature_switched_off(bifrost_ui::account_policy::Feature::Purchases),
        }
    }

    /// The keypad, once Buy has asked for the passcode.
    pub fn keypad(&self) -> Option<&Keypad> {
        self.keypad.as_ref()
    }

    /// Whether the keypad still needs frames (the wrong-code shake).
    pub fn animating(&self, t: f64) -> bool {
        self.keypad.as_ref().is_some_and(|k| k.animating(t))
    }

    /// Buy was tapped. With a passcode set the keypad asks for it; without one the purchase is confirmed at once. When
    /// the installed account's content limits switch purchases off, nothing is confirmed.
    fn buy(&mut self) -> Option<SheetOut> {
        if self.restricted {
            self.message = "Purchases are switched off for this account.".to_string();
            return None;
        }
        if !self.passcode.enabled() {
            return Some(SheetOut::Confirmed);
        }
        let mut pad = Keypad::new("Enter Passcode", self.passcode.len(), false);
        pad.message = format!("Buy {} for {}", self.name, self.purchase.price.display());
        self.keypad = Some(pad);
        None
    }

    /// Checks the typed passcode. A wrong one clears the entry and shows why; the passcode store counts the failure.
    fn check(&mut self, t: f64, now: i64) -> Option<SheetOut> {
        let code = self.keypad.as_ref().map(|k| k.digits().to_string()).unwrap_or_default();
        let verdict = self.passcode.verify(&code, now);
        let pad = self.keypad.as_mut()?;
        match verdict {
            Verdict::Ok => Some(SheetOut::Confirmed),
            Verdict::Wrong { locked, .. } => {
                pad.message = if locked > 0 { retry_text(locked) } else { "Wrong passcode".to_string() };
                pad.reject(t);
                None
            }
            Verdict::Locked(secs) => {
                pad.message = retry_text(secs);
                pad.reject(t);
                None
            }
        }
    }

    /// A touch in the sheet's coordinates (pane pixels). On the card only a lifted touch counts (Buy or Cancel); on the
    /// keypad the digits feed the passcode check. `t` is the shell clock in ms and `now` the Unix time in seconds.
    pub fn touch(&mut self, ui: &Ui, w: i32, h: i32, ev: Event, t: f64, now: i64) -> Option<SheetOut> {
        if self.keypad.is_none() {
            let Event::Up(x, y) = ev else { return None };
            let (buy, cancel) = sheet_buttons(ui, w, h);
            if inside(buy, x, y) {
                return self.buy();
            }
            if inside(cancel, x, y) {
                return Some(SheetOut::Cancelled);
            }
            return None;
        }
        let key = self.keypad.as_mut()?.touch(ui, w, h, ev)?;
        let need = self.passcode.len();
        match key {
            Pad::Cancel => Some(SheetOut::Cancelled),
            Pad::Delete => {
                if let Some(pad) = self.keypad.as_mut() {
                    pad.pop();
                }
                None
            }
            Pad::Digit(d) => {
                let full = {
                    let pad = self.keypad.as_mut()?;
                    pad.push(d, need) && pad.digits().len() == need
                };
                if full {
                    self.check(t, now)
                } else {
                    None
                }
            }
            Pad::Done => None,
        }
    }
}

fn inside((x0, y0, w, h): Rect, x: f32, y: f32) -> bool {
    x >= x0 && x < x0 + w && y >= y0 && y < y0 + h
}

/// The card's rectangle, centred in a `w` x `h` pane.
pub fn sheet_card(ui: &Ui, w: i32, h: i32) -> Rect {
    let cw = (w as f32 - ui.pt(40.0)).min(ui.pt(340.0));
    let ch = ui.pt(200.0);
    ((w as f32 - cw) / 2.0, (h as f32 - ch) / 2.0, cw, ch)
}

/// The Buy and Cancel buttons along the card's bottom edge: (buy, cancel).
pub fn sheet_buttons(ui: &Ui, w: i32, h: i32) -> (Rect, Rect) {
    let (cx, cy, cw, ch) = sheet_card(ui, w, h);
    let pad = ui.pt(20.0);
    let gap = ui.pt(12.0);
    let bw = (cw - 2.0 * pad - gap) / 2.0;
    let by = cy + ch - ui.pt(64.0);
    let bh = ui.pt(40.0);
    ((cx + pad, by, bw, bh), (cx + pad + bw + gap, by, bw, bh))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price() -> Price {
        Price::new(990, 0, "ISK")
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("store-sheet-{tag}-{}", std::process::id()))
    }

    #[test]
    fn buy_without_a_passcode_confirms_at_once() {
        let dir = scratch("none");
        let mut s = Sheet::open(&dir, "is.o.chess", "Chess", price());
        assert_eq!(s.purchase.step(), &bifrost_ui::purchase::Step::ConfirmationShown);
        assert_eq!(s.buy(), Some(SheetOut::Confirmed));
        assert!(s.keypad().is_none());
    }

    #[test]
    fn content_limits_stop_a_purchase_before_any_confirmation() {
        let dir = scratch("limited");
        let mut s = Sheet::open(&dir, "is.o.chess", "Chess", price());
        s.restricted = true;
        assert_eq!(s.buy(), None);
        assert!(s.keypad().is_none());
        assert_eq!(s.message, "Purchases are switched off for this account.");
    }

    #[test]
    fn with_a_passcode_buy_asks_for_it_and_the_right_one_confirms() {
        let dir = scratch("right");
        std::fs::create_dir_all(&dir).unwrap();
        passcode::Store::load(dir.join("passcode.ini")).set("1234").unwrap();
        let mut s = Sheet::open(&dir, "is.o.chess", "Chess", price());
        assert_eq!(s.buy(), None, "the keypad comes up first");
        let pad = s.keypad.as_mut().unwrap();
        assert_eq!(pad.message, "Buy Chess for 990 ISK");
        for d in [1, 2, 3, 4] {
            pad.push(d, 4);
        }
        assert_eq!(s.check(0.0, 0), Some(SheetOut::Confirmed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_passcode_clears_the_entry_and_does_not_confirm() {
        let dir = scratch("wrong");
        std::fs::create_dir_all(&dir).unwrap();
        passcode::Store::load(dir.join("passcode.ini")).set("1234").unwrap();
        let mut s = Sheet::open(&dir, "is.o.chess", "Chess", price());
        s.buy();
        for d in [9, 9, 9, 9] {
            s.keypad.as_mut().unwrap().push(d, 4);
        }
        assert_eq!(s.check(0.0, 0), None);
        let pad = s.keypad.as_ref().unwrap();
        assert_eq!(pad.message, "Wrong passcode", "one failure starts no lockout");
        assert_eq!(pad.digits(), "", "the entry is cleared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_card_buttons_sit_inside_the_card() {
        let ui = crate::shell::tests::ui();
        let (cx, cy, cw, ch) = sheet_card(&ui, 800, 600);
        let (buy, cancel) = sheet_buttons(&ui, 800, 600);
        for (x, y, w, h) in [buy, cancel] {
            assert!(x >= cx && x + w <= cx + cw && y >= cy && y + h <= cy + ch);
        }
        assert!(buy.0 + buy.2 < cancel.0, "Buy and Cancel do not overlap");
    }
}
