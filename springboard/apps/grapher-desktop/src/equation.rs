//! What an equation line means, and how it becomes points on the plane.
//!
//! Accepted lines:
//! - `y = f(x)`, or a bare `f(x)`: a graph of a function of `x`;
//! - `x = X(t), y = Y(t)`: a parametric curve;
//! - `r = R(t)`: a polar curve, where `t` is the angle in radians (`theta` is accepted too).
//!
//! A parametric or polar line may end with `; t in [a, b]` to choose the range of `t` (the default is [0, 2 pi]).
//! The bounds are constant expressions.

use crate::expr::{parse, Expr};
use crate::view::View;
use std::f64::consts::TAU;

/// The number of steps a parametric or polar curve is sampled with.
pub const PARAM_STEPS: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// `y = f(x)`.
    Function(Expr),
    /// `x = X(t), y = Y(t)` for `t` from `from` to `to`.
    Parametric { x: Expr, y: Expr, from: f64, to: f64 },
    /// `r = R(t)`, the points `(R cos t, R sin t)` for `t` from `from` to `to`.
    Polar { r: Expr, from: f64, to: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Equation {
    pub shape: Shape,
}

/// Reads one equation line. The error text is for the user.
pub fn parse_equation(line: &str) -> Result<Equation, String> {
    let (body, range) = match line.split_once(';') {
        Some((body, range)) => (body.trim(), Some(parse_range(range)?)),
        None => (line.trim(), None),
    };
    if body.is_empty() {
        return Err("the equation is empty".to_string());
    }
    let parts: Vec<&str> = body.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [single] => single_part(single, range),
        [first, second] => {
            let (lx, rx) = split_side(first)?;
            let (ly, ry) = split_side(second)?;
            if lx != "x" || ly != "y" {
                return Err("a parametric pair is written x = X(t), y = Y(t)".to_string());
            }
            let (from, to) = range.unwrap_or((0.0, TAU));
            Ok(Equation { shape: Shape::Parametric { x: compile(rx)?, y: compile(ry)?, from, to } })
        }
        _ => Err("use a comma only in a parametric pair x = X(t), y = Y(t)".to_string()),
    }
}

fn single_part(body: &str, range: Option<(f64, f64)>) -> Result<Equation, String> {
    match body.split_once('=') {
        None => {
            reject_range(range)?;
            Ok(Equation { shape: Shape::Function(compile(body)?) })
        }
        Some((lhs, rhs)) => match lhs.trim() {
            "y" => {
                reject_range(range)?;
                Ok(Equation { shape: Shape::Function(compile(rhs)?) })
            }
            "r" => {
                let (from, to) = range.unwrap_or((0.0, TAU));
                Ok(Equation { shape: Shape::Polar { r: compile(rhs)?, from, to } })
            }
            _ => Err("the left side must be y (for y = f(x)) or r (for a polar curve)".to_string()),
        },
    }
}

fn reject_range(range: Option<(f64, f64)>) -> Result<(), String> {
    match range {
        Some(_) => Err("a range applies only to parametric and polar curves".to_string()),
        None => Ok(()),
    }
}

fn split_side(part: &str) -> Result<(&str, &str), String> {
    let (lhs, rhs) = part.split_once('=').ok_or_else(|| format!("'{part}' has no '='"))?;
    Ok((lhs.trim(), rhs.trim()))
}

fn compile(src: &str) -> Result<Expr, String> {
    parse(src.trim()).map_err(|e| e.to_string())
}

/// `t in [a, b]`, with constant bounds.
fn parse_range(text: &str) -> Result<(f64, f64), String> {
    const HINT: &str = "the range must look like t in [0, 6.28]";
    let rest = text.trim().strip_prefix("t in").ok_or(HINT)?.trim();
    let inner = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')).ok_or(HINT)?;
    let (a, b) = inner.split_once(',').ok_or(HINT)?;
    let from = constant(a)?;
    let to = constant(b)?;
    Ok((from, to))
}

fn constant(src: &str) -> Result<f64, String> {
    let value = compile(src)?.eval(0.0, 0.0);
    if value.is_finite() {
        Ok(value)
    } else {
        Err("the range bounds must be numbers".to_string())
    }
}

impl Equation {
    /// The curve as polylines in world units, for a plot of `w` by `h` device pixels shown through `view`.
    /// A function is sampled once per device pixel column; a gap (a point outside the domain, or a jump across
    /// the plot) starts a new polyline.
    pub fn polylines(&self, view: &View, w: f64, h: f64) -> Vec<Vec<(f64, f64)>> {
        match &self.shape {
            Shape::Function(f) => function_lines(f, view, w, h),
            Shape::Parametric { x, y, from, to } => {
                sample_lines(|t| (x.eval(0.0, t), y.eval(0.0, t)), *from, *to)
            }
            Shape::Polar { r, from, to } => sample_lines(
                |theta| {
                    let radius = r.eval(0.0, theta);
                    (radius * theta.cos(), radius * theta.sin())
                },
                *from,
                *to,
            ),
        }
    }
}

fn function_lines(f: &Expr, view: &View, w: f64, h: f64) -> Vec<Vec<(f64, f64)>> {
    let mut lines = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut previous_py: Option<f64> = None;
    let columns = w.ceil().max(1.0) as usize;
    for column in 0..=columns {
        let (x, _) = view.to_world(column as f64, 0.0, w, h);
        let y = f.eval(x, 0.0);
        if !y.is_finite() {
            close_line(&mut lines, &mut current);
            previous_py = None;
            continue;
        }
        let (_, py) = view.to_px(x, y, w, h);
        if let Some(previous) = previous_py {
            if (py - previous).abs() > 2.0 * h {
                close_line(&mut lines, &mut current);
            }
        }
        current.push((x, y));
        previous_py = Some(py);
    }
    close_line(&mut lines, &mut current);
    lines
}

fn sample_lines(point: impl Fn(f64) -> (f64, f64), from: f64, to: f64) -> Vec<Vec<(f64, f64)>> {
    let mut lines = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    for step in 0..=PARAM_STEPS {
        let t = from + (to - from) * step as f64 / PARAM_STEPS as f64;
        let (x, y) = point(t);
        if x.is_finite() && y.is_finite() {
            current.push((x, y));
        } else {
            close_line(&mut lines, &mut current);
        }
    }
    close_line(&mut lines, &mut current);
    lines
}

fn close_line(lines: &mut Vec<Vec<(f64, f64)>>, current: &mut Vec<(f64, f64)>) {
    if !current.is_empty() {
        lines.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(line: &str) -> Shape {
        parse_equation(line).unwrap_or_else(|e| panic!("{line}: {e}")).shape
    }

    #[test]
    fn function_forms() {
        assert!(matches!(kind("y = x^2"), Shape::Function(_)));
        assert!(matches!(kind("sin(x)"), Shape::Function(_)));
        assert!(matches!(kind("  y = 2x  "), Shape::Function(_)));
        assert!(matches!(kind("y=2x"), Shape::Function(_)));
    }

    #[test]
    fn polar_defaults_to_a_full_turn() {
        match kind("r = 1 + cos(t)") {
            Shape::Polar { from, to, .. } => {
                assert_eq!(from, 0.0);
                assert!((to - TAU).abs() < 1e-12);
            }
            other => panic!("expected a polar curve, got {other:?}"),
        }
    }

    #[test]
    fn parametric_with_a_range() {
        match kind("x = cos(t), y = sin(t); t in [0, 3]") {
            Shape::Parametric { from, to, .. } => {
                assert_eq!(from, 0.0);
                assert_eq!(to, 3.0);
            }
            other => panic!("expected a parametric curve, got {other:?}"),
        }
    }

    #[test]
    fn rejected_lines() {
        assert!(parse_equation("").is_err());
        assert!(parse_equation("x = 2").is_err());
        assert!(parse_equation("x = 1, z = 2").is_err());
        assert!(parse_equation("y = x; t in [0, 1]").is_err());
        assert!(parse_equation("x = cos(t), y = sin(t); t in 0, 1").is_err());
        assert!(parse_equation("x = cos(t), y = sin(t); t in [0, 1/0]").is_err());
        assert!(parse_equation("max(x, 2)").is_err());
        assert!(parse_equation("y = (").is_err());
    }

    #[test]
    fn function_points_lie_on_the_curve() {
        let view = View { cx: 0.0, cy: 0.0, ppu: 10.0 };
        let eq = parse_equation("y = x").unwrap();
        let lines = eq.polylines(&view, 100.0, 100.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 101);
        let (x0, y0) = lines[0][0];
        let (x1, y1) = lines[0][100];
        assert!((x0 + 5.0).abs() < 1e-9 && (y0 + 5.0).abs() < 1e-9);
        assert!((x1 - 5.0).abs() < 1e-9 && (y1 - 5.0).abs() < 1e-9);
        assert!(lines[0].iter().all(|(x, y)| (x - y).abs() < 1e-9));
    }

    #[test]
    fn a_pole_splits_the_curve() {
        let view = View { cx: 0.0, cy: 0.0, ppu: 10.0 };
        let lines = parse_equation("y = 1/x").unwrap().polylines(&view, 100.0, 100.0);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!(line.iter().all(|(x, _)| *x != 0.0));
        }
    }

    #[test]
    fn a_steep_jump_on_screen_splits_the_curve() {
        // tan(x) has poles at +-pi/2, which lie inside a 10 unit window at 10 pixels per unit
        let view = View { cx: 0.0, cy: 0.0, ppu: 10.0 };
        let lines = parse_equation("y = tan(x)").unwrap().polylines(&view, 100.0, 100.0);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn polar_circle_has_constant_radius() {
        let view = View::default();
        let lines = parse_equation("r = 1").unwrap().polylines(&view, 100.0, 100.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), PARAM_STEPS + 1);
        assert!(lines[0].iter().all(|(x, y)| ((x * x + y * y).sqrt() - 1.0).abs() < 1e-9));
        let (x0, y0) = lines[0][0];
        assert!((x0 - 1.0).abs() < 1e-9 && y0.abs() < 1e-9);
    }

    #[test]
    fn parametric_unit_circle() {
        let view = View::default();
        let lines = parse_equation("x = cos(t), y = sin(t)").unwrap().polylines(&view, 100.0, 100.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].iter().all(|(x, y)| ((x * x + y * y).sqrt() - 1.0).abs() < 1e-9));
    }
}
