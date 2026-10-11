//! Auto Layout's engine (`NativeLayoutConstraint`, `NativeLayoutConstraint.Priority`): an incremental Cassowary solver
//! (Badros, Borning, Stuckey 2001; the simplex tableau layout follows Kiwi) with no dependencies.
//!
//! Variables are numbers the solver picks; a constraint is `expression  (==|<=|>=)  0` with a [`Strength`]. Required
//! constraints must hold or `add` fails with [`SolverError::Unsatisfiable`]; weaker ones are minimised as a weighted sum
//! of violations, so a higher priority always wins a conflict against a lower one. Edit variables
//! ([`Solver::add_edit`], [`Solver::suggest`]) re-solve in place for a window resize.
//!
//! ```ignore
//! let mut s = Solver::new();
//! let (x, w) = (s.var(), s.var());
//! s.add(Expr::var(x) - Expr::constant(10.0), Rel::Eq, Strength::REQUIRED)?; // x == 10
//! s.add(Expr::var(w) - Expr::constant(50.0), Rel::Le, Strength::STRONG)?;   // w <= 50
//! assert_eq!(s.value(x), 10.0);
//! ```
use std::collections::BTreeMap;
use std::ops::{Add, Mul, Sub};

/// A solver variable (layout attributes map to these). Created by [`Solver::var`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Var(u32);

/// Constraint strength. Larger wins; [`Strength::REQUIRED`] is absolute.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Strength(pub f64);

impl Strength {
    pub const REQUIRED: Strength = Strength(1_001_001_000.0);
    pub const STRONG: Strength = Strength(1_000_000.0);
    pub const MEDIUM: Strength = Strength(1_000.0);
    pub const WEAK: Strength = Strength(1.0);
    /// `NativeLayoutPriority`: 1000 and above is required; 1 to 999 scale linearly and always rank below required.
    /// (`defaultHigh` 750, `defaultLow` 250, `fittingSizeLevel` 50.)
    pub fn from_priority(p: u16) -> Strength {
        if p >= 1000 { Strength::REQUIRED } else { Strength(p.max(1) as f64 * 1000.0) }
    }
    pub fn is_required(self) -> bool {
        self.0 >= Self::REQUIRED.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    Le,
    Eq,
    Ge,
}

/// A linear expression: `sum(coeff * var) + constant`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expr {
    pub terms: Vec<(Var, f64)>,
    pub constant: f64,
}

impl Expr {
    pub fn constant(c: f64) -> Expr {
        Expr { terms: Vec::new(), constant: c }
    }
    pub fn var(v: Var) -> Expr {
        Expr { terms: vec![(v, 1.0)], constant: 0.0 }
    }
    pub fn term(v: Var, k: f64) -> Expr {
        Expr { terms: vec![(v, k)], constant: 0.0 }
    }
    pub fn plus(&self, o: &Expr) -> Expr {
        let mut r = self.clone();
        r.terms.extend(o.terms.iter().copied());
        r.constant += o.constant;
        r
    }
    pub fn minus(&self, o: &Expr) -> Expr {
        self.plus(&o.scale(-1.0))
    }
    pub fn scale(&self, k: f64) -> Expr {
        Expr { terms: self.terms.iter().map(|&(v, c)| (v, c * k)).collect(), constant: self.constant * k }
    }
}
impl From<Var> for Expr {
    fn from(v: Var) -> Expr {
        Expr::var(v)
    }
}
impl From<f64> for Expr {
    fn from(c: f64) -> Expr {
        Expr::constant(c)
    }
}
impl Add for Expr {
    type Output = Expr;
    fn add(self, o: Expr) -> Expr {
        self.plus(&o)
    }
}
impl Sub for Expr {
    type Output = Expr;
    fn sub(self, o: Expr) -> Expr {
        self.minus(&o)
    }
}
impl Mul<f64> for Expr {
    type Output = Expr;
    fn mul(self, k: f64) -> Expr {
        self.scale(k)
    }
}

/// Handle of an added constraint, for [`Solver::remove`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstraintId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverError {
    /// A required constraint conflicts with the required constraints already present. The solver is left as it was
    /// before the call; the failed constraint is not added.
    Unsatisfiable(ConstraintId),
    UnknownConstraint,
    DuplicateEdit,
    UnknownEdit,
    /// An edit variable cannot be required.
    BadStrength,
    /// The tableau became unbounded or lost a pivot; indicates a bug or non-finite input.
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Invalid,
    External,
    Slack,
    Error,
    Dummy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sym(u32, Kind);
const INVALID: Sym = Sym(0, Kind::Invalid);

fn near_zero(v: f64) -> bool {
    v.abs() < 1e-8
}

#[derive(Clone, Debug, Default)]
struct Row {
    constant: f64,
    cells: BTreeMap<Sym, f64>,
}

impl Row {
    fn insert_sym(&mut self, s: Sym, k: f64) {
        let e = self.cells.entry(s).or_insert(0.0);
        *e += k;
        if near_zero(*e) {
            self.cells.remove(&s);
        }
    }
    fn insert_row(&mut self, o: &Row, k: f64) {
        self.constant += o.constant * k;
        for (&s, &c) in &o.cells {
            self.insert_sym(s, c * k);
        }
    }
    fn reverse_sign(&mut self) {
        self.constant = -self.constant;
        self.cells.values_mut().for_each(|c| *c = -*c);
    }
    /// Rewrites `0 = row` so that `s` is the subject: `s = row'`.
    fn solve_for(&mut self, s: Sym) {
        let k = -1.0 / self.cells.remove(&s).unwrap_or(1.0);
        self.constant *= k;
        self.cells.values_mut().for_each(|c| *c *= k);
    }
    /// Row of `lhs = row` re-solved for `rhs`.
    fn solve_pair(&mut self, lhs: Sym, rhs: Sym) {
        self.insert_sym(lhs, -1.0);
        self.solve_for(rhs);
    }
    fn coeff(&self, s: Sym) -> f64 {
        self.cells.get(&s).copied().unwrap_or(0.0)
    }
    fn substitute(&mut self, s: Sym, r: &Row) {
        if let Some(c) = self.cells.remove(&s) {
            self.insert_row(r, c);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tag {
    marker: Sym,
    other: Sym,
}
struct Stored {
    tag: Tag,
    strength: f64,
}
struct EditInfo {
    tag: Tag,
    constraint: ConstraintId,
    constant: f64,
}

/// The incremental solver. Adding, removing and suggesting values update the current solution without starting over.
#[derive(Default)]
pub struct Solver {
    next_var: u32,
    next_sym: u32,
    next_cons: u32,
    cons: BTreeMap<ConstraintId, Stored>,
    rows: BTreeMap<Sym, Row>,
    vars: BTreeMap<Var, Sym>,
    edits: BTreeMap<Var, EditInfo>,
    infeasible: Vec<Sym>,
    objective: Row,
    artificial: Option<Row>,
}

impl Solver {
    pub fn new() -> Solver {
        Solver::default()
    }
    pub fn var(&mut self) -> Var {
        self.next_var += 1;
        Var(self.next_var)
    }
    fn sym(&mut self, k: Kind) -> Sym {
        self.next_sym += 1;
        Sym(self.next_sym, k)
    }
    /// Current value of `v` (0 for a variable no constraint has constrained).
    pub fn value(&self, v: Var) -> f64 {
        self.vars.get(&v).and_then(|s| self.rows.get(s)).map_or(0.0, |r| r.constant)
    }
    pub fn constraint_count(&self) -> usize {
        self.cons.len()
    }

    /// Adds `expr  rel  0`. A required constraint that cannot hold together with the other required ones is refused.
    pub fn add(&mut self, expr: Expr, rel: Rel, strength: Strength) -> Result<ConstraintId, SolverError> {
        let id = ConstraintId(self.next_cons);
        self.next_cons += 1;
        let s = strength.0.min(Strength::REQUIRED.0);
        let (mut row, tag) = self.create_row(&expr, rel, s);
        let mut subject = Self::choose_subject(&row, tag);
        if subject == INVALID && row.cells.keys().all(|k| k.1 == Kind::Dummy) {
            if !near_zero(row.constant) {
                return Err(SolverError::Unsatisfiable(id));
            }
            subject = tag.marker;
        }
        if subject == INVALID {
            let backup = (self.rows.clone(), self.objective.clone(), self.infeasible.clone());
            if !self.add_with_artificial(row)? {
                (self.rows, self.objective, self.infeasible) = backup;
                return Err(SolverError::Unsatisfiable(id));
            }
        } else {
            row.solve_for(subject);
            self.substitute(subject, &row);
            self.rows.insert(subject, row);
        }
        self.cons.insert(id, Stored { tag, strength: s });
        self.optimize(false)?;
        Ok(id)
    }

    pub fn remove(&mut self, id: ConstraintId) -> Result<(), SolverError> {
        let st = self.cons.remove(&id).ok_or(SolverError::UnknownConstraint)?;
        let tag = st.tag;
        if tag.marker.1 == Kind::Error {
            self.remove_marker_effects(tag.marker, st.strength);
        }
        if tag.other.1 == Kind::Error {
            self.remove_marker_effects(tag.other, st.strength);
        }
        if self.rows.remove(&tag.marker).is_none() {
            let leaving = self.marker_leaving_row(tag.marker).ok_or(SolverError::Internal)?;
            let mut row = self.rows.remove(&leaving).ok_or(SolverError::Internal)?;
            row.solve_pair(leaving, tag.marker);
            self.substitute(tag.marker, &row);
        }
        self.optimize(false)
    }

    /// Makes `v` adjustable by [`Solver::suggest`] at `strength` (less than required), e.g. the window width.
    pub fn add_edit(&mut self, v: Var, strength: Strength) -> Result<(), SolverError> {
        if self.edits.contains_key(&v) {
            return Err(SolverError::DuplicateEdit);
        }
        if strength.is_required() {
            return Err(SolverError::BadStrength);
        }
        let id = self.add(Expr::var(v), Rel::Eq, strength)?;
        let tag = self.cons[&id].tag;
        self.edits.insert(v, EditInfo { tag, constraint: id, constant: 0.0 });
        Ok(())
    }
    pub fn remove_edit(&mut self, v: Var) -> Result<(), SolverError> {
        let info = self.edits.remove(&v).ok_or(SolverError::UnknownEdit)?;
        self.remove(info.constraint)
    }
    /// Moves the edit variable towards `value` and re-solves incrementally (dual simplex).
    pub fn suggest(&mut self, v: Var, value: f64) -> Result<(), SolverError> {
        let info = self.edits.get_mut(&v).ok_or(SolverError::UnknownEdit)?;
        let delta = value - info.constant;
        info.constant = value;
        let tag = info.tag;
        if let Some(r) = self.rows.get_mut(&tag.marker) {
            r.constant -= delta;
            if r.constant < 0.0 {
                self.infeasible.push(tag.marker);
            }
        } else if let Some(r) = self.rows.get_mut(&tag.other) {
            r.constant += delta;
            if r.constant < 0.0 {
                self.infeasible.push(tag.other);
            }
        } else {
            for (&s, r) in self.rows.iter_mut() {
                let c = r.coeff(tag.marker);
                if c != 0.0 {
                    r.constant += delta * c;
                    if r.constant < 0.0 && s.1 != Kind::External {
                        self.infeasible.push(s);
                    }
                }
            }
        }
        self.dual_optimize()
    }

    fn var_sym(&mut self, v: Var) -> Sym {
        if let Some(&s) = self.vars.get(&v) {
            return s;
        }
        let s = self.sym(Kind::External);
        self.vars.insert(v, s);
        s
    }

    fn create_row(&mut self, e: &Expr, rel: Rel, strength: f64) -> (Row, Tag) {
        let mut row = Row { constant: e.constant, cells: BTreeMap::new() };
        for &(v, k) in &e.terms {
            if near_zero(k) {
                continue;
            }
            let s = self.var_sym(v);
            match self.rows.get(&s) {
                Some(r) => row.insert_row(r, k),
                None => row.insert_sym(s, k),
            }
        }
        let required = strength >= Strength::REQUIRED.0;
        let mut tag = Tag { marker: INVALID, other: INVALID };
        match rel {
            Rel::Le | Rel::Ge => {
                let k = if rel == Rel::Le { 1.0 } else { -1.0 };
                let slack = self.sym(Kind::Slack);
                tag.marker = slack;
                row.insert_sym(slack, k);
                if !required {
                    let err = self.sym(Kind::Error);
                    tag.other = err;
                    row.insert_sym(err, -k);
                    self.objective.insert_sym(err, strength);
                }
            }
            Rel::Eq => {
                if !required {
                    let (p, m) = (self.sym(Kind::Error), self.sym(Kind::Error));
                    tag = Tag { marker: p, other: m };
                    row.insert_sym(p, -1.0);
                    row.insert_sym(m, 1.0);
                    self.objective.insert_sym(p, strength);
                    self.objective.insert_sym(m, strength);
                } else {
                    let d = self.sym(Kind::Dummy);
                    tag.marker = d;
                    row.insert_sym(d, 1.0);
                }
            }
        }
        if row.constant < 0.0 {
            row.reverse_sign();
        }
        (row, tag)
    }

    fn choose_subject(row: &Row, tag: Tag) -> Sym {
        if let Some((&s, _)) = row.cells.iter().find(|(s, _)| s.1 == Kind::External) {
            return s;
        }
        for m in [tag.marker, tag.other] {
            if matches!(m.1, Kind::Slack | Kind::Error) && row.coeff(m) < 0.0 {
                return m;
            }
        }
        INVALID
    }

    fn add_with_artificial(&mut self, row: Row) -> Result<bool, SolverError> {
        let art = self.sym(Kind::Slack);
        self.rows.insert(art, row.clone());
        self.artificial = Some(row);
        let run = self.optimize(true);
        let ok = near_zero(self.artificial.as_ref().map_or(1.0, |r| r.constant));
        self.artificial = None;
        run?;
        if let Some(mut r) = self.rows.remove(&art) {
            if r.cells.is_empty() {
                return Ok(ok);
            }
            let Some((&entering, _)) = r.cells.iter().find(|(s, _)| matches!(s.1, Kind::Slack | Kind::Error)) else {
                return Ok(false);
            };
            r.solve_pair(art, entering);
            self.substitute(entering, &r);
            self.rows.insert(entering, r);
        }
        for r in self.rows.values_mut() {
            r.cells.remove(&art);
        }
        self.objective.cells.remove(&art);
        Ok(ok)
    }

    fn substitute(&mut self, s: Sym, r: &Row) {
        for (&k, row) in self.rows.iter_mut() {
            row.substitute(s, r);
            if k.1 != Kind::External && row.constant < 0.0 {
                self.infeasible.push(k);
            }
        }
        self.objective.substitute(s, r);
        if let Some(a) = self.artificial.as_mut() {
            a.substitute(s, r);
        }
    }

    /// Primal simplex on the objective (or the artificial objective).
    fn optimize(&mut self, artificial: bool) -> Result<(), SolverError> {
        for _ in 0..100_000 {
            let obj = if artificial { self.artificial.as_ref().ok_or(SolverError::Internal)? } else { &self.objective };
            let Some((&entering, _)) = obj.cells.iter().find(|(s, &c)| s.1 != Kind::Dummy && c < 0.0) else {
                return Ok(());
            };
            let mut best = f64::MAX;
            let mut leaving = None;
            for (&s, r) in &self.rows {
                if s.1 == Kind::External {
                    continue;
                }
                let c = r.coeff(entering);
                if c < 0.0 {
                    let ratio = -r.constant / c;
                    if ratio < best {
                        best = ratio;
                        leaving = Some(s);
                    }
                }
            }
            let leaving = leaving.ok_or(SolverError::Internal)?;
            let mut row = self.rows.remove(&leaving).ok_or(SolverError::Internal)?;
            row.solve_pair(leaving, entering);
            self.substitute(entering, &row);
            self.rows.insert(entering, row);
        }
        Err(SolverError::Internal)
    }

    fn dual_optimize(&mut self) -> Result<(), SolverError> {
        while let Some(leaving) = self.infeasible.pop() {
            if !self.rows.get(&leaving).is_some_and(|r| r.constant < 0.0) {
                continue;
            }
            let mut ratio = f64::MAX;
            let mut entering = INVALID;
            for (&s, &c) in &self.rows[&leaving].cells {
                if c > 0.0 && s.1 != Kind::Dummy {
                    let r = self.objective.coeff(s) / c;
                    if r < ratio {
                        ratio = r;
                        entering = s;
                    }
                }
            }
            if entering == INVALID {
                return Err(SolverError::Internal);
            }
            let mut row = self.rows.remove(&leaving).ok_or(SolverError::Internal)?;
            row.solve_pair(leaving, entering);
            self.substitute(entering, &row);
            self.rows.insert(entering, row);
        }
        Ok(())
    }

    fn remove_marker_effects(&mut self, marker: Sym, strength: f64) {
        match self.rows.get(&marker).cloned() {
            Some(r) => self.objective.insert_row(&r, -strength),
            None => self.objective.insert_sym(marker, -strength),
        }
    }

    fn marker_leaving_row(&self, marker: Sym) -> Option<Sym> {
        let (mut r1, mut r2) = (f64::MAX, f64::MAX);
        let (mut first, mut second, mut third) = (None, None, None);
        for (&s, r) in &self.rows {
            let c = r.coeff(marker);
            if c == 0.0 {
                continue;
            }
            if s.1 == Kind::External {
                third = Some(s);
            } else if c < 0.0 {
                let q = -r.constant / c;
                if q < r1 {
                    r1 = q;
                    first = Some(s);
                }
            } else {
                let q = r.constant / c;
                if q < r2 {
                    r2 = q;
                    second = Some(s);
                }
            }
        }
        first.or(second).or(third)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(s: &mut Solver, e: Expr, st: Strength) -> ConstraintId {
        s.add(e, Rel::Eq, st).unwrap()
    }
    fn v(x: Var) -> Expr {
        Expr::var(x)
    }
    fn c(k: f64) -> Expr {
        Expr::constant(k)
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_required_equality_pins_the_variable() {
        let mut s = Solver::new();
        let x = s.var();
        eq(&mut s, v(x) - c(10.0), Strength::REQUIRED);
        assert!(close(s.value(x), 10.0));
    }

    #[test]
    fn a_centered_view_sits_in_the_middle_of_its_parent() {
        let mut s = Solver::new();
        let (pl, pr, l, r) = (s.var(), s.var(), s.var(), s.var());
        eq(&mut s, v(pl), Strength::REQUIRED);
        eq(&mut s, v(pr) - c(400.0), Strength::REQUIRED);
        eq(&mut s, (v(l) + v(r)) * 0.5 - (v(pl) + v(pr)) * 0.5, Strength::REQUIRED);
        eq(&mut s, v(r) - v(l) - c(100.0), Strength::REQUIRED);
        assert!(close(s.value(l), 150.0) && close(s.value(r), 250.0));
    }

    #[test]
    fn equal_widths_share_the_space_between_margins() {
        let mut s = Solver::new();
        let (w1, w2) = (s.var(), s.var());
        eq(&mut s, v(w1) - v(w2), Strength::REQUIRED);
        eq(&mut s, v(w1) + v(w2) + c(20.0) - c(320.0), Strength::REQUIRED);
        assert!(close(s.value(w1), 150.0) && close(s.value(w2), 150.0));
    }

    #[test]
    fn min_and_max_clamp_a_weak_preference() {
        for (want, expect) in [(200.0, 80.0), (10.0, 50.0), (60.0, 60.0)] {
            let mut s = Solver::new();
            let w = s.var();
            s.add(v(w) - c(50.0), Rel::Ge, Strength::REQUIRED).unwrap();
            s.add(v(w) - c(80.0), Rel::Le, Strength::REQUIRED).unwrap();
            eq(&mut s, v(w) - c(want), Strength::WEAK);
            assert!(close(s.value(w), expect), "want {want}: {}", s.value(w));
        }
    }

    #[test]
    fn the_higher_priority_wins_a_conflict() {
        let mut s = Solver::new();
        let x = s.var();
        eq(&mut s, v(x) - c(20.0), Strength::from_priority(250));
        eq(&mut s, v(x) - c(10.0), Strength::from_priority(750));
        assert!(close(s.value(x), 10.0));
        let mut s = Solver::new();
        let x = s.var();
        eq(&mut s, v(x) - c(20.0), Strength::WEAK);
        eq(&mut s, v(x) - c(10.0), Strength::MEDIUM);
        assert!(close(s.value(x), 10.0));
    }

    #[test]
    fn priority_999_is_still_below_required() {
        assert!(Strength::from_priority(999) < Strength::REQUIRED);
        assert!(Strength::from_priority(1000).is_required());
        assert!(Strength::from_priority(1) < Strength::from_priority(2));
    }

    #[test]
    fn conflicting_required_constraints_are_reported_and_not_added() {
        let mut s = Solver::new();
        let x = s.var();
        eq(&mut s, v(x) - c(1.0), Strength::REQUIRED);
        let err = s.add(v(x) - c(2.0), Rel::Eq, Strength::REQUIRED);
        assert!(matches!(err, Err(SolverError::Unsatisfiable(_))));
        assert_eq!(s.constraint_count(), 1);
        assert!(close(s.value(x), 1.0));
        let err = s.add(v(x) - c(5.0), Rel::Ge, Strength::REQUIRED);
        assert!(matches!(err, Err(SolverError::Unsatisfiable(_))));
        assert_eq!(s.constraint_count(), 1);
        assert!(close(s.value(x), 1.0));
        s.add(v(x) - c(5.0), Rel::Le, Strength::REQUIRED).unwrap();
    }

    #[test]
    fn removing_a_constraint_lets_the_weaker_one_through() {
        let mut s = Solver::new();
        let x = s.var();
        let strong = eq(&mut s, v(x) - c(10.0), Strength::STRONG);
        eq(&mut s, v(x) - c(20.0), Strength::WEAK);
        assert!(close(s.value(x), 10.0));
        s.remove(strong).unwrap();
        assert!(close(s.value(x), 20.0));
        assert_eq!(s.remove(strong), Err(SolverError::UnknownConstraint));
    }

    #[test]
    fn an_edit_variable_resizes_the_layout_incrementally() {
        let mut s = Solver::new();
        let (win, view) = (s.var(), s.var());
        eq(&mut s, v(view) + c(40.0) - v(win), Strength::REQUIRED);
        s.add(v(view) - c(100.0), Rel::Ge, Strength::REQUIRED).unwrap();
        s.add_edit(win, Strength::STRONG).unwrap();
        s.suggest(win, 800.0).unwrap();
        assert!(close(s.value(view), 760.0));
        s.suggest(win, 500.0).unwrap();
        assert!(close(s.value(view), 460.0));
        // Below the minimum the required floor wins over the strong suggestion.
        s.suggest(win, 50.0).unwrap();
        assert!(close(s.value(view), 100.0) && close(s.value(win), 140.0));
        s.suggest(win, 300.0).unwrap();
        assert!(close(s.value(view), 260.0));
    }

    #[test]
    fn edit_variable_errors() {
        let mut s = Solver::new();
        let x = s.var();
        assert_eq!(s.suggest(x, 1.0), Err(SolverError::UnknownEdit));
        assert_eq!(s.add_edit(x, Strength::REQUIRED), Err(SolverError::BadStrength));
        s.add_edit(x, Strength::MEDIUM).unwrap();
        assert_eq!(s.add_edit(x, Strength::MEDIUM), Err(SolverError::DuplicateEdit));
        s.suggest(x, 7.0).unwrap();
        assert!(close(s.value(x), 7.0));
        s.remove_edit(x).unwrap();
    }

    #[test]
    fn a_chain_of_views_with_spacing_fills_the_parent() {
        let mut s = Solver::new();
        let xs: Vec<Var> = (0..4).map(|_| s.var()).collect(); // left edges
        let ws: Vec<Var> = (0..4).map(|_| s.var()).collect();
        eq(&mut s, v(xs[0]) - c(8.0), Strength::REQUIRED);
        for i in 0..3 {
            eq(&mut s, v(xs[i + 1]) - v(xs[i]) - v(ws[i]) - c(8.0), Strength::REQUIRED);
            eq(&mut s, v(ws[i + 1]) - v(ws[i]), Strength::REQUIRED);
        }
        eq(&mut s, v(xs[3]) + v(ws[3]) + c(8.0) - c(440.0), Strength::REQUIRED);
        // 8 + 4w + 3*8 + 8 = 440
        assert!(close(s.value(ws[0]), 100.0), "{}", s.value(ws[0]));
        assert!(close(s.value(xs[3]), 332.0));
    }

    #[test]
    fn inequality_with_a_strong_preference_and_a_weak_one() {
        let mut s = Solver::new();
        let (a, b) = (s.var(), s.var());
        s.add(v(a) - v(b), Rel::Le, Strength::REQUIRED).unwrap(); // a <= b
        eq(&mut s, v(a) - c(30.0), Strength::STRONG);
        eq(&mut s, v(b) - c(10.0), Strength::WEAK);
        assert!(close(s.value(a), 30.0) && close(s.value(b), 30.0));
    }

    #[test]
    fn many_constraints_stay_consistent_after_removals() {
        let mut s = Solver::new();
        let xs: Vec<Var> = (0..20).map(|_| s.var()).collect();
        let mut ids = Vec::new();
        for w in xs.windows(2) {
            ids.push(eq(&mut s, v(w[1]) - v(w[0]) - c(5.0), Strength::REQUIRED));
        }
        eq(&mut s, v(xs[0]), Strength::REQUIRED);
        assert!(close(s.value(xs[19]), 95.0));
        s.remove(ids[9]).unwrap();
        eq(&mut s, v(xs[19]) - c(200.0), Strength::STRONG);
        assert!(close(s.value(xs[19]), 200.0) && close(s.value(xs[9]), 45.0));
    }
}
