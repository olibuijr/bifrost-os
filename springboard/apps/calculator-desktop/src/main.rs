//! Calculator as an installed BifrOSt app: the former in-shell Calculator,
//! unchanged, hosted through the SDK.

mod calc;
mod convert;
mod engine;
mod eval;
mod format;
mod history;
mod icon;
mod paste;
mod sci;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, Command, Commands, Ctx, Input, TouchKind, KeyInput};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;
use engine::{Key, Operator};
use std::path::Path;

struct CalculatorApp {
    calc: Option<calc::Calculator>,
}

impl App for CalculatorApp {
    fn id(&self) -> &str {
        "is.olibuijr.calculator.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.calc {
            Some(c) => c.set_size(w, h),
            None => self.calc = Some(calc::Calculator::new(&ctx.ui, w, h).with_storage(ctx.data_dir.clone())),
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        // Edit > Copy and Edit > Paste from the shell's Edit menu (the items `commands` advertises)
        if let Input::Command(command) = &ev {
            let Some(calc) = &mut self.calc else { return false };
            return match command {
                Command::Copy => {
                    calc.copy(&ctx.ui);
                    false
                }
                Command::Paste => calc.paste(&ctx.ui),
                _ => false,
            };
        }
        if let Input::Key(input) = &ev {
            let Some(calc) = &mut self.calc else { return false };
            let keys = match input {
                KeyInput::Text(text) => text.chars().filter_map(|ch| match ch {
                    '0'..='9' => Some(Key::Digit(ch as u8 - b'0')),
                    '.' | ',' => Some(Key::Decimal),
                    '+' => Some(Key::Operator(Operator::Add)),
                    '-' => Some(Key::Operator(Operator::Subtract)),
                    '*' => Some(Key::Operator(Operator::Multiply)),
                    '/' => Some(Key::Operator(Operator::Divide)),
                    '=' => Some(Key::Equals),
                    '%' => Some(Key::Percent),
                    _ => None,
                }).collect::<Vec<_>>(),
                KeyInput::Code(8) => vec![Key::Backspace],
                KeyInput::Code(13) => vec![Key::Equals],
                KeyInput::Code(27) => vec![Key::Clear],
                _ => return false,
            };
            let changed = !keys.is_empty();
            for key in keys { calc.hardware_key(&ctx.ui, key); }
            return changed;
        }
        let (Some(c), Input::Touch { kind, x, y, t, .. }) = (&mut self.calc, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        c.touch(&ctx.ui, e, t);
        true
    }

    /// Copy and Paste are enabled in the shell's Edit menu only when they have something to act on.
    fn commands(&self, ctx: &Ctx) -> Commands {
        let Some(c) = &self.calc else { return Commands::default() };
        Commands::default().with(Command::Copy, c.can_copy(&ctx.ui)).with(Command::Paste, c.can_paste())
    }

    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("evaluate", "Evaluate", "Evaluates an expression and returns the result.").param(Param::new("expression", "Expression", ParamType::String))]
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if id != "evaluate" {
            return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
        }
        let expression = intents::str_param(params, "expression")?;
        // the angle unit the user set in Calculator (Rad or Deg), as the keypad uses it
        let angle = history::Settings::load(&data_dir.join(history::SETTINGS_FILE)).angle;
        let value = eval::evaluate(expression, angle).map_err(|e| {
            intents::failed(match e {
                eval::EvalError::Syntax => "not a valid expression".to_string(),
                eval::EvalError::Domain => "the result is outside the function's domain".to_string(),
                eval::EvalError::Overflow => "the result is too large".to_string(),
            })
        })?;
        Ok(IntentResult::value(Json::Num(value)).with_dialog(&format!("{expression} = {}", format::format_number(value))))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.calc.as_mut().is_some_and(|c| c.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.calc.as_ref().and_then(|c| c.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.calc.as_ref().map(|c| c.controls(&ctx.ui)).unwrap_or_default()
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(c) = &mut self.calc {
            c.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

#[cfg(test)]
pub fn test_ui() -> bifrost_app::Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = bifrost_app::Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

fn main() {
    bifrost_app::run(CalculatorApp { calc: None });
}
