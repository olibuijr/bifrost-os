//! The app menus (Edit, View, Stack) and their actions, the keyboard map and the history context menu.

use super::*;
use crate::format::MAX_PLACES;
use crate::programmer::WORD_SIZES;
use bifrost_app::proto::{AppMenu, AppMenuItem, ContextItem};

const MENU_COPY_EXPRESSION: u32 = 1;
const MENU_COPY_RESULT: u32 = 2;
const MENU_BASIC: u32 = 10;
const MENU_SCIENTIFIC: u32 = 11;
const MENU_PROGRAMMER: u32 = 12;
const MENU_RPN: u32 = 13;
const MENU_CONVERT: u32 = 14;
const MENU_HISTORY: u32 = 15;
const MENU_THOUSANDS: u32 = 16;
const MENU_SCI_NOTATION: u32 = 17;
/// View > Decimal Comma: shows "1,5" instead of "1.5" (and "." to group thousands). It was an unlabelled "1.5" toolbar
/// chip before (E2E W4-7).
pub(crate) const MENU_DECIMAL_COMMA: u32 = 18;
const MENU_PLACES_AUTO: u32 = 20;
/// Decimal Places 0 to 15 are `MENU_PLACES + places`.
const MENU_PLACES: u32 = 100;
/// Word Size items are `MENU_WORD + index in WORD_SIZES`.
const MENU_WORD: u32 = 200;
const MENU_SWAP: u32 = 50;
const MENU_DROP: u32 = 51;
const MENU_ROLL_UP: u32 = 52;
const MENU_ROLL_DOWN: u32 = 53;

/// The history row context menu.
const CTX_COPY_RESULT: u32 = 1;

/// Window > Always on Top: the item's id (outside the ids of the calculator's own menus). The shell app keeps the
/// checked state (`CalculatorApp.floating`) and sends the window level.
pub const MENU_ALWAYS_ON_TOP: u32 = 300;

/// The Window menu: Always on Top, checked while the window floats.
pub fn window_menu(floating: bool) -> AppMenu {
    AppMenu { title: "Window".into(), items: vec![AppMenuItem::new(MENU_ALWAYS_ON_TOP, "Always on Top").checked(floating)] }
}
const CTX_COPY_EXPRESSION: u32 = 2;
const CTX_DELETE: u32 = 3;

impl Calculator {
    /// The menus the app adds to the menu bar: Edit (copy the expression or the last result), View (the keypad,
    /// RPN, Convert, the history and how numbers are shown) and Stack (the RPN stack operations). Called after
    /// every event, so the checked and enabled states follow the app.
    pub fn menus(&self, ui: &Ui) -> Vec<AppMenu> {
        let sci = self.sci_active(ui);
        let prog = self.programmer.is_some() && self.convert.is_none();
        let convert = self.convert.is_some();
        let basic = !sci && !prog && !convert;
        let places = self.settings.places;
        let word = self.programmer.as_ref().map(Programmer::bits);
        let mut view = vec![
            AppMenuItem::new(MENU_BASIC, "Basic").key("cmd+1").checked(basic),
            AppMenuItem::new(MENU_SCIENTIFIC, "Scientific").key("cmd+2").checked(sci),
            AppMenuItem::new(MENU_PROGRAMMER, "Programmer").key("cmd+3").checked(prog),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_RPN, "RPN Mode").key("cmd+r").checked(self.settings.rpn),
            AppMenuItem::new(MENU_CONVERT, "Unit Conversion").key("cmd+u").checked(convert),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_HISTORY, if self.history_open { "Hide History" } else { "Show History" }).key("cmd+y"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_THOUSANDS, "Thousands Separator").checked(self.settings.thousands),
            AppMenuItem::new(MENU_DECIMAL_COMMA, "Decimal Comma (1,5)").checked(self.settings.locale.decimal() == ','),
            AppMenuItem::submenu("Decimal Places"),
            AppMenuItem::new(MENU_PLACES_AUTO, "Automatic").checked(places.is_none()).depth(1),
        ];
        for p in 0..=MAX_PLACES {
            view.push(AppMenuItem::new(MENU_PLACES + u32::from(p), &p.to_string()).checked(places == Some(p)).depth(1));
        }
        view.push(AppMenuItem::new(MENU_SCI_NOTATION, "Scientific Notation").checked(self.settings.sci_notation));
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::submenu("Word Size"));
        for (index, bits) in WORD_SIZES.iter().enumerate() {
            view.push(
                AppMenuItem::new(MENU_WORD + index as u32, &format!("{bits} bit"))
                    .enabled(prog)
                    .checked(word == Some(*bits))
                    .depth(1),
            );
        }
        let rpn = self.rpn_active();
        vec![
            AppMenu {
                title: "Calculator".into(),
                items: vec![
                    AppMenuItem::new(MENU_COPY_EXPRESSION, "Copy Expression").key("cmd+shift+c").enabled(self.expression_for_copy(ui).is_some()),
                    AppMenuItem::new(MENU_COPY_RESULT, "Copy Last Result").key("cmd+alt+c").enabled(!self.store.is_empty()),
                ],
            },
            AppMenu { title: "View".into(), items: view },
            AppMenu {
                title: "Stack".into(),
                items: vec![
                    AppMenuItem::new(MENU_SWAP, "Swap").key("cmd+e").enabled(rpn),
                    AppMenuItem::new(MENU_DROP, "Drop").key("cmd+delete").enabled(rpn),
                    AppMenuItem::new(MENU_ROLL_UP, "Roll Up").key("cmd+up").enabled(rpn),
                    AppMenuItem::new(MENU_ROLL_DOWN, "Roll Down").key("cmd+down").enabled(rpn),
                ],
            },
        ]
    }

    /// The user chose menu item `id`; true when the app needs a new frame.
    pub fn menu_action(&mut self, ui: &Ui, id: u32) -> bool {
        match id {
            MENU_COPY_EXPRESSION => return self.copy_expression(ui),
            MENU_COPY_RESULT => return self.copy_last_result(),
            MENU_BASIC => self.set_mode(Mode::Basic),
            MENU_SCIENTIFIC => self.set_mode(Mode::Scientific),
            MENU_PROGRAMMER => self.set_mode(Mode::Programmer),
            MENU_RPN => {
                self.convert = None;
                self.programmer = None;
                self.settings.rpn = !self.settings.rpn;
                if self.settings.rpn {
                    self.settings.scientific = false;
                }
            }
            MENU_CONVERT => {
                self.convert = if self.convert.is_some() { None } else { Some(Converter::with_rates(self.rates.clone())) };
            }
            MENU_HISTORY => {
                self.history_open = !self.history_open;
                self.hist_offset = 0;
            }
            MENU_THOUSANDS => self.settings.thousands = !self.settings.thousands,
            MENU_DECIMAL_COMMA => self.settings.locale = self.settings.locale.toggled(),
            MENU_SCI_NOTATION => self.settings.sci_notation = !self.settings.sci_notation,
            MENU_PLACES_AUTO => self.settings.places = None,
            _ if (MENU_PLACES..MENU_PLACES + u32::from(MAX_PLACES) + 1).contains(&id) => {
                self.settings.places = Some((id - MENU_PLACES) as u8);
            }
            _ if (MENU_WORD..MENU_WORD + WORD_SIZES.len() as u32).contains(&id) => {
                self.programmer_press(PKey::SetBits(WORD_SIZES[(id - MENU_WORD) as usize]));
            }
            MENU_SWAP if self.rpn_active() => self.rpn_press(RKey::Swap),
            MENU_DROP if self.rpn_active() => self.rpn_press(RKey::Drop),
            MENU_ROLL_UP if self.rpn_active() => self.rpn_press(RKey::RollUp),
            MENU_ROLL_DOWN if self.rpn_active() => self.rpn_press(RKey::RollDown),
            _ => return false,
        }
        self.save_settings();
        self.dirty = true;
        true
    }

    /// The expression Copy Expression puts on the pasteboard: the one being worked on (scientific expression, a
    /// finished Programmer calculation), else the latest calculation of the history.
    fn expression_for_copy(&self, ui: &Ui) -> Option<String> {
        let live = if self.sci_active(ui) {
            self.sci.expr.clone()
        } else if let Some(p) = self.programmer.as_ref().filter(|_| self.programmer_active(ui)) {
            p.expression.clone()
        } else {
            String::new()
        };
        let text = if live.trim().is_empty() { self.store.get(0)?.expression.clone() } else { live };
        let text = pretty(text.trim_end_matches(|c: char| c == '=' || c == ' ')).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn set_copy(&mut self, number: String) {
        let shown = localize_with(&number, self.settings.locale, self.settings.thousands);
        self.copy_request = Some(CopyRequest { number, shown });
    }

    /// Edit > Copy Expression: the expression goes to the pasteboard like a Copy.
    pub fn copy_expression(&mut self, ui: &Ui) -> bool {
        let Some(text) = self.expression_for_copy(ui) else { return false };
        self.set_copy(text);
        true
    }

    /// Edit > Copy Last Result: the newest history result as plain digits.
    pub fn copy_last_result(&mut self) -> bool {
        let Some(result) = self.store.get(0).map(|e| e.result.clone()) else { return false };
        self.set_copy(result);
        true
    }

    /// A typed character: digits, the arithmetic keys and, depending on the keypad, parentheses, `^`, `!`, `e`, `p`
    /// (pi) and `E` (exponent) on the scientific one, and hex digits and the bitwise operators in Programmer.
    /// True when the character did something.
    pub fn typed(&mut self, ui: &Ui, ch: char) -> bool {
        if self.programmer_active(ui) {
            let key = match ch {
                'a'..='f' => Some(PKey::Digit(ch as u8 - b'a' + 10)),
                'A'..='F' => Some(PKey::Digit(ch as u8 - b'A' + 10)),
                '&' => Some(PKey::Op(BinOp::And)),
                '|' => Some(PKey::Op(BinOp::Or)),
                '^' => Some(PKey::Op(BinOp::Xor)),
                '~' => Some(PKey::Not),
                '<' => Some(PKey::Op(BinOp::Shl)),
                '>' => Some(PKey::Op(BinOp::Shr)),
                _ => None,
            };
            if let Some(key) = key {
                self.programmer_press(key);
                self.dirty = true;
                return true;
            }
        }
        if self.sci_active(ui) {
            let key = match ch {
                '(' => Some(SciKey::Open),
                ')' => Some(SciKey::Close),
                '^' => Some(SciKey::Postfix("^")),
                '!' => Some(SciKey::Postfix("!")),
                'e' => Some(SciKey::Text("e")),
                'p' => Some(SciKey::Text("π")),
                'E' => Some(SciKey::Exp),
                // Option-minus types an en dash: the sign key
                '\u{2013}' => Some(SciKey::Sign),
                _ => None,
            };
            if let Some(key) = key {
                self.sci_press(key);
                self.dirty = true;
                return true;
            }
        }
        let key = match ch {
            '0'..='9' => Key::Digit(ch as u8 - b'0'),
            '.' | ',' => Key::Decimal,
            '+' => Key::Operator(Operator::Add),
            '-' | '\u{2212}' => Key::Operator(Operator::Subtract),
            '*' | '\u{d7}' => Key::Operator(Operator::Multiply),
            '/' | '\u{f7}' => Key::Operator(Operator::Divide),
            '=' => Key::Equals,
            '%' => Key::Percent,
            // Space enters the number on the RPN stack, like Return
            ' ' if self.rpn_active() => Key::Equals,
            _ => return false,
        };
        self.hardware_key(ui, key);
        true
    }

    /// A secondary click at (`x`, `y`): the context menu of the history row under it, if the history is open.
    pub fn history_context(&mut self, ui: &Ui, x: f32, y: f32) -> Option<Vec<ContextItem>> {
        if !self.history_open {
            return None;
        }
        let Some(Target::Result(row)) = self.hit(ui, x, y) else { return None };
        self.ctx_row = Some(row);
        let item = |id: u32, label: &str| ContextItem { id, label: label.to_string(), enabled: true };
        Some(vec![item(CTX_COPY_RESULT, "Copy Result"), item(CTX_COPY_EXPRESSION, "Copy Expression"), item(CTX_DELETE, "Delete")])
    }

    /// The keypad pull-down of a compact window (Golden Gate Calculator's mode menu): Basic, Scientific and Programmer,
    /// then Convert and RPN Mode, the chosen ones checked. The ids are the View menu's, so a choice does what the View
    /// menu item does.
    pub(super) fn mode_menu_items(&self) -> Vec<ContextItem> {
        let convert = self.convert.is_some();
        let prog = self.programmer.is_some() && !convert;
        let sci = self.settings.scientific && !self.settings.rpn && !prog && !convert;
        let item = |id: u32, label: &str, on: bool| ContextItem::new(id, &format!("{} {label}", if on { "\u{2713}" } else { "\u{2003}" }));
        vec![
            item(MENU_BASIC, "Basic", !sci && !prog && !convert),
            item(MENU_SCIENTIFIC, "Scientific", sci),
            item(MENU_PROGRAMMER, "Programmer", prog),
            ContextItem::separator(),
            item(MENU_CONVERT, "Convert", convert),
            item(MENU_RPN, "RPN Mode", self.settings.rpn),
        ]
    }

    /// The user chose an item of the history row menu or of the keypad pull-down; true when the app needs a new frame.
    pub fn context_action(&mut self, ui: &Ui, id: u32) -> bool {
        if matches!(id, MENU_BASIC | MENU_SCIENTIFIC | MENU_PROGRAMMER | MENU_CONVERT | MENU_RPN) {
            self.ctx_row = None;
            return self.menu_action(ui, id);
        }
        let Some(row) = self.ctx_row.take() else { return false };
        let Some(entry) = self.store.get(row).cloned() else { return false };
        match id {
            CTX_COPY_RESULT => self.set_copy(entry.result),
            CTX_COPY_EXPRESSION => self.set_copy(pretty(entry.expression.trim_end_matches(|c: char| c == '=' || c == ' '))),
            CTX_DELETE => self.delete_entry(ui, row),
            _ => return false,
        }
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landscape() -> (Ui, Calculator) {
        let ui = crate::test_ui();
        let mut app = Calculator::new(&ui, 1920, 1200);
        app.settings.scientific = true;
        (ui, app)
    }

    fn portrait() -> (Ui, Calculator) {
        let ui = crate::test_ui();
        let app = Calculator::new(&ui, 1200, 1920);
        (ui, app)
    }

    fn type_str(app: &mut Calculator, ui: &Ui, text: &str) {
        for ch in text.chars() {
            app.typed(ui, ch);
        }
    }

    fn item<'a>(menus: &'a [AppMenu], title: &str, item: &str) -> &'a AppMenuItem {
        menus.iter().find(|m| m.title == title).unwrap().items.iter().find(|i| i.title == item).unwrap()
    }

    #[test]
    fn the_menus_are_valid_for_the_shell() {
        let (ui, app) = landscape();
        let menus = app.menus(&ui);
        assert_eq!(menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["Calculator", "View", "Stack"]);
        for menu in &menus {
            assert!(menu.items.len() <= bifrost_app::proto::MAX_APP_MENU_ITEMS, "{}", menu.title);
            for it in &menu.items {
                assert!(it.title.chars().count() <= bifrost_app::proto::MAX_APP_MENU_TITLE);
                assert!(it.depth <= bifrost_app::proto::MAX_APP_MENU_DEPTH);
                if !it.key.is_empty() {
                    assert!(bifrost_app::controls::keys::KeyCombo::parse(&it.key).is_some(), "{}", it.key);
                }
            }
        }
        let mut ids: Vec<u32> = menus.iter().flat_map(|m| m.items.iter()).filter(|i| i.id != 0).map(|i| i.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "every item has its own id");
        assert_eq!(item(&menus, "View", "Basic").key, "cmd+1");
        assert_eq!(item(&menus, "View", "Scientific").key, "cmd+2");
        assert_eq!(item(&menus, "View", "Programmer").key, "cmd+3");
        assert_eq!(item(&menus, "View", "RPN Mode").key, "cmd+r");
    }

    #[test]
    fn the_window_menu_lists_always_on_top_checked_while_it_floats() {
        let (ui, app) = landscape();
        let mut menus = app.menus(&ui);
        let ids: Vec<u32> = menus.iter().flat_map(|m| m.items.iter()).map(|i| i.id).collect();
        assert!(!ids.contains(&MENU_ALWAYS_ON_TOP), "the Window item id is not one of the calculator's own");
        menus.push(window_menu(false));
        assert!(!item(&menus, "Window", "Always on Top").checked);
        assert_eq!(item(&menus, "Window", "Always on Top").id, MENU_ALWAYS_ON_TOP);
        let mut floating = app.menus(&ui);
        floating.push(window_menu(true));
        assert!(item(&floating, "Window", "Always on Top").checked);
        assert_eq!(floating.last().map(|m| m.title.as_str()), Some("Window"), "the Window menu comes after the calculator's");
    }

    #[test]
    fn the_view_menu_switches_the_keypad_and_shows_the_mode() {
        let (ui, mut app) = portrait();
        assert!(item(&app.menus(&ui), "View", "Basic").checked);
        assert!(app.menu_action(&ui, MENU_SCIENTIFIC));
        assert!(app.sci_active(&ui));
        assert!(item(&app.menus(&ui), "View", "Scientific").checked);
        assert!(!item(&app.menus(&ui), "View", "Basic").checked);
        assert!(app.menu_action(&ui, MENU_PROGRAMMER));
        assert!(app.programmer_active(&ui) && !app.sci_active(&ui));
        assert!(item(&app.menus(&ui), "View", "Programmer").checked);
        assert!(item(&app.menus(&ui), "View", "8 bit").enabled);
        assert!(app.menu_action(&ui, MENU_BASIC));
        assert!(!app.programmer_active(&ui) && !app.sci_active(&ui), "Basic leaves both wide keypads");
        assert!(!item(&app.menus(&ui), "View", "8 bit").enabled, "word size needs Programmer");
        assert!(app.menu_action(&ui, MENU_CONVERT));
        assert!(app.convert.is_some() && item(&app.menus(&ui), "View", "Unit Conversion").checked);
        assert!(app.menu_action(&ui, MENU_CONVERT));
        assert!(app.convert.is_none());
        assert!(app.menu_action(&ui, MENU_HISTORY));
        assert!(app.history_open && item(&app.menus(&ui), "View", "Hide History").enabled);
        assert!(!app.menu_action(&ui, 9999), "an unknown id does nothing");
    }

    #[test]
    fn the_view_settings_change_how_results_show_and_persist() {
        let dir = std::env::temp_dir().join(format!("sb-calc-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ui = crate::test_ui();
        {
            let mut app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
            app.paste_text(&ui, "1234567.25");
            app.hardware_key(&ui, Key::Operator(Operator::Add));
            app.hardware_key(&ui, Key::Digit(1));
            app.hardware_key(&ui, Key::Equals);
            assert_eq!(app.display_shown(&ui), "1,234,568.25");
            app.menu_action(&ui, MENU_THOUSANDS);
            assert_eq!(app.display_shown(&ui), "1234568.25");
            app.menu_action(&ui, MENU_PLACES + 3);
            assert_eq!(app.display_shown(&ui), "1234568.250");
            assert!(item(&app.menus(&ui), "View", "3").checked);
            app.menu_action(&ui, MENU_SCI_NOTATION);
            assert_eq!(app.display_shown(&ui), "1.235e6");
            app.menu_action(&ui, MENU_PLACES_AUTO);
            assert_eq!(app.display_shown(&ui), "1.23456825e6");
            app.menu_action(&ui, MENU_SCI_NOTATION);
            app.menu_action(&ui, MENU_PLACES);
            assert_eq!(app.display_shown(&ui), "1234568");
            // typing is never rewritten
            app.hardware_key(&ui, Key::Clear);
            app.hardware_key(&ui, Key::Digit(5));
            assert_eq!(app.display_shown(&ui), "5");
            app.menu_action(&ui, MENU_RPN);
        }
        let app = Calculator::new(&ui, 1200, 1920).with_storage(dir.clone());
        assert!(!app.settings.thousands);
        assert_eq!(app.settings.places, Some(0));
        assert!(!app.settings.sci_notation);
        assert!(app.settings.rpn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_expression_and_copy_last_result() {
        let (ui, mut app) = landscape();
        assert!(!app.menu_action(&ui, MENU_COPY_RESULT), "nothing to copy yet");
        assert!(!item(&app.menus(&ui), "Calculator", "Copy Last Result").enabled);
        type_str(&mut app, &ui, "2+3*4=");
        assert_eq!(app.display_text(&ui), "14");
        assert!(item(&app.menus(&ui), "Calculator", "Copy Expression").enabled);
        assert!(app.menu_action(&ui, MENU_COPY_EXPRESSION));
        let copy = app.take_copy().unwrap();
        assert_eq!(copy.number, "2+3×4");
        assert!(app.menu_action(&ui, MENU_COPY_RESULT));
        assert_eq!(app.take_copy().unwrap().number, "14");
        let (ui, mut basic) = portrait();
        for key in [Key::Digit(6), Key::Operator(Operator::Multiply), Key::Digit(7), Key::Equals] {
            basic.hardware_key(&ui, key);
        }
        assert!(basic.copy_expression(&ui));
        assert_eq!(basic.take_copy().unwrap().number, "6 × 7");
        assert!(basic.copy_last_result());
        assert_eq!(basic.take_copy().unwrap().number, "42");
    }

    #[test]
    fn the_keyboard_reaches_the_scientific_keys() {
        let (ui, mut app) = landscape();
        type_str(&mut app, &ui, "(2+3)^2=");
        assert_eq!(app.display_text(&ui), "25");
        app.hardware_key(&ui, Key::Clear);
        type_str(&mut app, &ui, "5!=");
        assert_eq!(app.display_text(&ui), "120");
        app.hardware_key(&ui, Key::Clear);
        type_str(&mut app, &ui, "2p=");
        assert_eq!(app.display_text(&ui), "6.28318531");
        app.hardware_key(&ui, Key::Clear);
        type_str(&mut app, &ui, "e");
        assert_eq!(app.sci.expr, "e");
        app.hardware_key(&ui, Key::Clear);
        type_str(&mut app, &ui, "15E2=");
        assert_eq!(app.display_text(&ui), "1500");
        app.hardware_key(&ui, Key::Clear);
        type_str(&mut app, &ui, "5\u{2013}");
        assert_eq!(app.sci.expr, "-(5)", "Option-minus changes the sign");
        // not scientific: the extra characters are ignored
        let (ui, mut basic) = portrait();
        assert!(!basic.typed(&ui, '('));
        assert!(!basic.typed(&ui, 'e'));
        assert!(basic.typed(&ui, '7'));
        assert_eq!(basic.display(), "7");
    }

    #[test]
    fn programmer_mode_takes_hex_digits_and_bitwise_keys_from_the_keyboard() {
        let (ui, mut app) = landscape();
        app.menu_action(&ui, MENU_PROGRAMMER);
        assert!(app.programmer_active(&ui));
        app.programmer_press(PKey::SetBase(Base::Hex));
        type_str(&mut app, &ui, "f0&3c=");
        assert_eq!(app.display_text(&ui), "30");
        assert_eq!(app.store.len(), 1, "a finished Programmer calculation goes to the history");
        assert_eq!(app.store.get(0).unwrap().result, "48");
        type_str(&mut app, &ui, "~");
        assert_eq!(app.display_text(&ui), "FFFFFFFFFFFFFFCF");
        assert_eq!(app.copy_text(&ui).as_deref(), Some("FFFFFFFFFFFFFFCF"));
        assert!(!app.typed(&ui, 'g'));
        assert!(!app.typed(&ui, '('));
    }

    #[test]
    fn rpn_mode_runs_on_the_basic_keypad() {
        let (ui, mut app) = portrait();
        app.menu_action(&ui, MENU_RPN);
        assert!(app.rpn_active());
        assert!(item(&app.menus(&ui), "View", "RPN Mode").checked);
        assert!(item(&app.menus(&ui), "Stack", "Swap").enabled);
        // 3 enter 4 +
        type_str(&mut app, &ui, "3 4+");
        assert_eq!(app.display_text(&ui), "7");
        assert_eq!(app.store.len(), 1);
        assert_eq!(app.store.get(0).unwrap().expression, "3 + 4");
        // enter key stays Return
        app.hardware_key(&ui, Key::Equals);
        assert_eq!(app.rpn.stack(), &[7.0, 7.0]);
        assert_eq!(app.expression_text(&ui), "7");
        app.menu_action(&ui, MENU_ROLL_DOWN);
        app.menu_action(&ui, MENU_SWAP);
        app.menu_action(&ui, MENU_DROP);
        assert_eq!(app.rpn.stack(), &[7.0]);
        assert_eq!(app.copy_text(&ui).as_deref(), Some("7"));
        assert!(app.paste_text(&ui, "12.5"));
        assert_eq!(app.display_text(&ui), "12.5");
        app.menu_action(&ui, MENU_RPN);
        assert!(!app.rpn_active());
        assert!(!item(&app.menus(&ui), "Stack", "Swap").enabled);
    }

    #[test]
    fn the_history_context_menu_copies_and_deletes_rows() {
        let (ui, mut app) = portrait();
        for n in 0..3 {
            app.store.push_at(Entry { expression: format!("{n}+1"), result: (n + 1).to_string(), value: (n + 1) as f64 }, 0);
        }
        assert!(app.history_context(&ui, 100.0, 1000.0).is_none(), "only with the history open");
        app.history_open = true;
        let (x, y) = {
            let (x, y, w, h) = app.result_rect(&ui, 1);
            (x + w / 2.0, y + h / 2.0)
        };
        let items = app.history_context(&ui, x, y).unwrap();
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Copy Result", "Copy Expression", "Delete"]);
        assert!(app.context_action(&ui, CTX_COPY_RESULT));
        assert_eq!(app.take_copy().unwrap().number, "2", "row 1 is the middle entry");
        assert!(!app.context_action(&ui, CTX_DELETE), "the menu target is used once");
        app.history_context(&ui, x, y).unwrap();
        assert!(app.context_action(&ui, CTX_COPY_EXPRESSION));
        assert_eq!(app.take_copy().unwrap().number, "1+1");
        app.history_context(&ui, x, y).unwrap();
        assert!(app.context_action(&ui, CTX_DELETE));
        assert_eq!(app.store.len(), 2);
        assert_eq!(app.store.get(1).unwrap().result, "1");
        assert!(app.history_context(&ui, 700.0, 700.0).is_none(), "outside the sidebar there is no row");
    }
}
