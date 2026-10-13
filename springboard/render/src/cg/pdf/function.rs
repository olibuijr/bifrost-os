//! PDF functions: sampled (type 0), exponential (2), stitching (3) and
//! PostScript calculator (4), plus arrays of single-output functions.

use super::doc::Doc;
use super::obj::Obj;

#[derive(Clone, Debug)]
enum Op {
    Num(f32),
    Name(String),
    If(Vec<Op>),
    IfElse(Vec<Op>, Vec<Op>),
}

#[derive(Clone, Debug)]
pub enum Function {
    /// the identity (used when a tint transform is missing)
    Identity,
    Sampled { domain: Vec<f32>, range: Vec<f32>, size: Vec<usize>, encode: Vec<f32>, decode: Vec<f32>, samples: Vec<f32>, n_out: usize },
    Exponential { domain: Vec<f32>, range: Vec<f32>, c0: Vec<f32>, c1: Vec<f32>, n: f32 },
    Stitching { domain: Vec<f32>, range: Vec<f32>, funcs: Vec<Function>, bounds: Vec<f32>, encode: Vec<f32> },
    Calculator { domain: Vec<f32>, range: Vec<f32>, code: Vec<Op> },
    /// one function per output component
    Multi(Vec<Function>),
}

fn nums(doc: &Doc, d: &super::obj::Dict, key: &str) -> Vec<f32> {
    match doc.field(d, key) {
        Obj::Array(a) => a.iter().filter_map(|o| doc.get(o).num()).map(|v| v as f32).collect(),
        _ => vec![],
    }
}

fn parse_calc(tokens: &mut std::iter::Peekable<std::vec::IntoIter<String>>) -> Vec<Op> {
    let mut out = vec![];
    while let Some(t) = tokens.next() {
        match t.as_str() {
            "}" => break,
            "{" => {
                let body = parse_calc(tokens);
                // `{a} if` or `{a} {b} ifelse`
                match tokens.peek().map(String::as_str) {
                    Some("if") => {
                        tokens.next();
                        out.push(Op::If(body));
                    }
                    Some("{") => {
                        tokens.next();
                        let other = parse_calc(tokens);
                        if tokens.peek().map(String::as_str) == Some("ifelse") {
                            tokens.next();
                        }
                        out.push(Op::IfElse(body, other));
                    }
                    _ => {}
                }
            }
            other => match other.parse::<f32>() {
                Ok(v) => out.push(Op::Num(v)),
                Err(_) => out.push(Op::Name(other.to_string())),
            },
        }
    }
    out
}

fn tokenize_calc(data: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(data);
    let mut out = vec![];
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '{' | '}' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn run_calc(code: &[Op], st: &mut Vec<f32>, depth: u8) {
    if depth > 16 {
        return;
    }
    let pop = |st: &mut Vec<f32>| st.pop().unwrap_or(0.0);
    for op in code {
        match op {
            Op::Num(v) => st.push(*v),
            Op::If(body) => {
                if pop(st) != 0.0 {
                    run_calc(body, st, depth + 1);
                }
            }
            Op::IfElse(a, b) => {
                if pop(st) != 0.0 {
                    run_calc(a, st, depth + 1);
                } else {
                    run_calc(b, st, depth + 1);
                }
            }
            Op::Name(n) => {
                let b = |v: bool| if v { 1.0 } else { 0.0 };
                match n.as_str() {
                    "abs" => {
                        let a = pop(st);
                        st.push(a.abs());
                    }
                    "add" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(x + y);
                    }
                    "sub" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(x - y);
                    }
                    "mul" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(x * y);
                    }
                    "div" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(if y != 0.0 { x / y } else { 0.0 });
                    }
                    "idiv" => {
                        let (y, x) = (pop(st) as i32, pop(st) as i32);
                        st.push(if y != 0 { (x / y) as f32 } else { 0.0 });
                    }
                    "mod" => {
                        let (y, x) = (pop(st) as i32, pop(st) as i32);
                        st.push(if y != 0 { (x % y) as f32 } else { 0.0 });
                    }
                    "neg" => {
                        let a = pop(st);
                        st.push(-a);
                    }
                    "atan" => {
                        let (den, num) = (pop(st), pop(st));
                        let mut d = num.atan2(den).to_degrees();
                        if d < 0.0 {
                            d += 360.0;
                        }
                        st.push(d);
                    }
                    "ceiling" => {
                        let a = pop(st);
                        st.push(a.ceil());
                    }
                    "floor" => {
                        let a = pop(st);
                        st.push(a.floor());
                    }
                    "round" => {
                        let a = pop(st);
                        st.push((a + 0.5).floor());
                    }
                    "truncate" | "cvi" => {
                        let a = pop(st);
                        st.push(a.trunc());
                    }
                    "cvr" => {}
                    "cos" => {
                        let a = pop(st);
                        st.push(a.to_radians().cos());
                    }
                    "sin" => {
                        let a = pop(st);
                        st.push(a.to_radians().sin());
                    }
                    "exp" => {
                        let (e, base) = (pop(st), pop(st));
                        let v = base.powf(e);
                        st.push(if v.is_finite() { v } else { 0.0 });
                    }
                    "ln" => {
                        let a = pop(st);
                        st.push(if a > 0.0 { a.ln() } else { 0.0 });
                    }
                    "log" => {
                        let a = pop(st);
                        st.push(if a > 0.0 { a.log10() } else { 0.0 });
                    }
                    "sqrt" => {
                        let a = pop(st);
                        st.push(a.max(0.0).sqrt());
                    }
                    "and" => {
                        let (y, x) = (pop(st) as i32, pop(st) as i32);
                        st.push((x & y) as f32);
                    }
                    "or" => {
                        let (y, x) = (pop(st) as i32, pop(st) as i32);
                        st.push((x | y) as f32);
                    }
                    "xor" => {
                        let (y, x) = (pop(st) as i32, pop(st) as i32);
                        st.push((x ^ y) as f32);
                    }
                    "not" => {
                        let a = pop(st);
                        // booleans (0/1) negate logically, integers bitwise
                        st.push(if a == 0.0 { 1.0 } else if a == 1.0 { 0.0 } else { !(a as i32) as f32 });
                    }
                    "bitshift" => {
                        let (s, x) = (pop(st) as i32, pop(st) as i32);
                        let v = if s >= 0 { x.checked_shl(s as u32).unwrap_or(0) } else { x >> ((-s).min(31)) };
                        st.push(v as f32);
                    }
                    "eq" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x == y));
                    }
                    "ne" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x != y));
                    }
                    "ge" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x >= y));
                    }
                    "gt" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x > y));
                    }
                    "le" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x <= y));
                    }
                    "lt" => {
                        let (y, x) = (pop(st), pop(st));
                        st.push(b(x < y));
                    }
                    "true" => st.push(1.0),
                    "false" => st.push(0.0),
                    "pop" => {
                        pop(st);
                    }
                    "dup" => {
                        let a = st.last().copied().unwrap_or(0.0);
                        st.push(a);
                    }
                    "exch" => {
                        let n = st.len();
                        if n >= 2 {
                            st.swap(n - 1, n - 2);
                        }
                    }
                    "copy" => {
                        let n = pop(st).max(0.0) as usize;
                        if n <= st.len() && n < 100 {
                            let start = st.len() - n;
                            let part: Vec<f32> = st[start..].to_vec();
                            st.extend(part);
                        }
                    }
                    "index" => {
                        let n = pop(st).max(0.0) as usize;
                        let v = st.len().checked_sub(n + 1).and_then(|i| st.get(i).copied()).unwrap_or(0.0);
                        st.push(v);
                    }
                    "roll" => {
                        let (j, n) = (pop(st) as i32, pop(st).max(0.0) as usize);
                        if n > 0 && n <= st.len() {
                            let start = st.len() - n;
                            let k = j.rem_euclid(n as i32) as usize;
                            st[start..].rotate_right(k);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Function {
    /// Reads a function object (a dictionary, a stream, `/Identity`, or an array of functions).
    pub fn parse(doc: &Doc, o: &Obj) -> Option<Function> {
        Self::parse_depth(doc, o, 0)
    }

    fn parse_depth(doc: &Doc, o: &Obj, depth: u8) -> Option<Function> {
        if depth > 8 {
            return None;
        }
        let obj = doc.get(o);
        if let Obj::Array(a) = &obj {
            let fs: Option<Vec<Function>> = a.iter().map(|f| Self::parse_depth(doc, f, depth + 1)).collect();
            return fs.map(Function::Multi);
        }
        if obj.name() == Some("Identity") {
            return Some(Function::Identity);
        }
        let d = obj.dict()?;
        let domain = nums(doc, d, "Domain");
        let range = nums(doc, d, "Range");
        match doc.field(d, "FunctionType").int()? {
            0 => {
                let Obj::Stream(s) = &obj else { return None };
                let size: Vec<usize> = nums(doc, d, "Size").iter().map(|v| (*v as usize).max(1)).collect();
                let n_in = domain.len() / 2;
                let n_out = range.len() / 2;
                if n_in == 0 || n_out == 0 || size.len() != n_in {
                    return None;
                }
                let bps = doc.field(d, "BitsPerSample").int().unwrap_or(8) as usize;
                let mut encode = nums(doc, d, "Encode");
                if encode.len() < 2 * n_in {
                    encode = size.iter().flat_map(|&n| [0.0, (n - 1) as f32]).collect();
                }
                let mut decode = nums(doc, d, "Decode");
                if decode.len() < 2 * n_out {
                    decode = range.clone();
                }
                let data = doc.decode(s).0;
                let total: usize = size.iter().product::<usize>().checked_mul(n_out)?;
                if total > 16_000_000 {
                    return None;
                }
                let max = if bps >= 32 { u32::MAX as f64 } else { ((1u64 << bps) - 1) as f64 };
                let mut samples = Vec::with_capacity(total);
                let mut bit = 0usize;
                for _ in 0..total {
                    let mut v = 0u64;
                    for _ in 0..bps {
                        let byte = data.get(bit / 8).copied().unwrap_or(0);
                        v = v << 1 | ((byte >> (7 - bit % 8)) & 1) as u64;
                        bit += 1;
                    }
                    samples.push((v as f64 / max) as f32);
                }
                Some(Function::Sampled { domain, range, size, encode, decode, samples, n_out })
            }
            2 => {
                let mut c0 = nums(doc, d, "C0");
                let mut c1 = nums(doc, d, "C1");
                if c0.is_empty() {
                    c0 = vec![0.0];
                }
                if c1.is_empty() {
                    c1 = vec![1.0];
                }
                Some(Function::Exponential { domain, range, c0, c1, n: doc.field(d, "N").num().unwrap_or(1.0) as f32 })
            }
            3 => {
                let Obj::Array(fs) = doc.field(d, "Functions") else { return None };
                let funcs: Option<Vec<Function>> = fs.iter().map(|f| Self::parse_depth(doc, f, depth + 1)).collect();
                Some(Function::Stitching { domain, range, funcs: funcs?, bounds: nums(doc, d, "Bounds"), encode: nums(doc, d, "Encode") })
            }
            4 => {
                let Obj::Stream(s) = &obj else { return None };
                let toks = tokenize_calc(&doc.decode(s).0);
                let mut it = toks.into_iter().peekable();
                if it.peek().map(String::as_str) == Some("{") {
                    it.next();
                }
                Some(Function::Calculator { domain, range, code: parse_calc(&mut it) })
            }
            _ => None,
        }
    }

    /// The number of outputs when it is known without evaluating.
    pub fn n_out(&self) -> Option<usize> {
        match self {
            Function::Identity => None,
            Function::Sampled { n_out, .. } => Some(*n_out),
            Function::Exponential { c0, .. } => Some(c0.len()),
            Function::Stitching { funcs, .. } => funcs.first().and_then(Function::n_out),
            Function::Calculator { range, .. } => Some(range.len() / 2),
            Function::Multi(f) => Some(f.len()),
        }
    }

    pub fn eval(&self, input: &[f32]) -> Vec<f32> {
        let clip = |v: &mut Vec<f32>, range: &[f32]| {
            for (i, x) in v.iter_mut().enumerate() {
                if let (Some(&lo), Some(&hi)) = (range.get(2 * i), range.get(2 * i + 1)) {
                    *x = x.clamp(lo.min(hi), hi.max(lo));
                }
            }
        };
        let dom = |x: f32, i: usize, domain: &[f32]| match (domain.get(2 * i), domain.get(2 * i + 1)) {
            (Some(&lo), Some(&hi)) => x.clamp(lo.min(hi), hi.max(lo)),
            _ => x,
        };
        match self {
            Function::Identity => input.to_vec(),
            Function::Multi(fs) => fs.iter().map(|f| f.eval(input).first().copied().unwrap_or(0.0)).collect(),
            Function::Exponential { domain, range, c0, c1, n } => {
                let x = dom(input.first().copied().unwrap_or(0.0), 0, domain);
                let p = if *n == 1.0 { x } else { x.powf(*n) };
                let mut out: Vec<f32> = (0..c0.len().max(c1.len())).map(|i| c0.get(i).copied().unwrap_or(0.0) + p * (c1.get(i).copied().unwrap_or(1.0) - c0.get(i).copied().unwrap_or(0.0))).collect();
                clip(&mut out, range);
                out
            }
            Function::Stitching { domain, range, funcs, bounds, encode } => {
                let x = dom(input.first().copied().unwrap_or(0.0), 0, domain);
                let (d0, d1) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
                let k = funcs.len();
                if k == 0 {
                    return vec![];
                }
                let mut i = 0;
                while i < bounds.len() && i + 1 < k && x >= bounds[i] {
                    i += 1;
                }
                let lo = if i == 0 { d0 } else { bounds[i - 1] };
                let hi = if i < bounds.len() { bounds[i] } else { d1 };
                let (e0, e1) = (encode.get(2 * i).copied().unwrap_or(0.0), encode.get(2 * i + 1).copied().unwrap_or(1.0));
                let t = if hi == lo { e0 } else { e0 + (x - lo) * (e1 - e0) / (hi - lo) };
                let mut out = funcs[i].eval(&[t]);
                clip(&mut out, range);
                out
            }
            Function::Calculator { domain, range, code } => {
                let mut st: Vec<f32> = input.iter().enumerate().map(|(i, &x)| dom(x, i, domain)).collect();
                run_calc(code, &mut st, 0);
                let n = range.len() / 2;
                let mut out: Vec<f32> = if st.len() >= n { st[st.len() - n..].to_vec() } else { st };
                clip(&mut out, range);
                out
            }
            Function::Sampled { domain, range, size, encode, decode, samples, n_out } => {
                let m = size.len();
                // encoded position per input, split into floor index and fraction
                let mut idx = Vec::with_capacity(m);
                let mut frac = Vec::with_capacity(m);
                for i in 0..m {
                    let x = dom(input.get(i).copied().unwrap_or(0.0), i, domain);
                    let (d0, d1) = (domain[2 * i], domain[2 * i + 1]);
                    let (e0, e1) = (encode[2 * i], encode[2 * i + 1]);
                    let e = if d1 == d0 { e0 } else { e0 + (x - d0) * (e1 - e0) / (d1 - d0) };
                    let e = e.clamp(0.0, (size[i] - 1) as f32);
                    let f = e.floor();
                    idx.push(f as usize);
                    frac.push(e - f);
                }
                let mut out = vec![0.0f32; *n_out];
                for corner in 0..(1usize << m) {
                    let mut weight = 1.0f32;
                    let mut offset = 0usize;
                    let mut stride = 1usize;
                    for i in 0..m {
                        let hi = corner >> i & 1 == 1;
                        let pos = if hi { (idx[i] + 1).min(size[i] - 1) } else { idx[i] };
                        weight *= if hi { frac[i] } else { 1.0 - frac[i] };
                        offset += pos * stride;
                        stride *= size[i];
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (j, o) in out.iter_mut().enumerate() {
                        *o += weight * samples.get(offset * n_out + j).copied().unwrap_or(0.0);
                    }
                }
                for (j, o) in out.iter_mut().enumerate() {
                    let (d0, d1) = (decode[2 * j], decode[2 * j + 1]);
                    *o = d0 + *o * (d1 - d0);
                }
                clip(&mut out, range);
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;
    use crate::cg::pdf::obj::{Dict, Stream};
    use std::rc::Rc;

    fn doc() -> Doc {
        Doc::open(tiny_pdf("")).unwrap()
    }

    fn arr(v: &[f64]) -> Obj {
        Obj::Array(v.iter().map(|x| Obj::Real(*x)).collect())
    }

    fn stream(entries: Vec<(&str, Obj)>, data: &[u8]) -> Obj {
        let dict: Dict = entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Obj::Stream(Rc::new(Stream { dict, data: data.to_vec() }))
    }

    #[test]
    fn exponential_interpolates() {
        let d: Dict = [("FunctionType", Obj::Int(2)), ("Domain", arr(&[0.0, 1.0])), ("C0", arr(&[0.0, 1.0])), ("C1", arr(&[1.0, 0.0])), ("N", Obj::Int(1))]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let f = Function::parse(&doc(), &Obj::Dict(d)).unwrap();
        assert_eq!(f.eval(&[0.25]), vec![0.25, 0.75]);
        assert_eq!(f.eval(&[7.0]), vec![1.0, 0.0], "input is clamped to the domain");
    }

    #[test]
    fn calculator_runs_stack_operators() {
        let s = stream(vec![("FunctionType", Obj::Int(4)), ("Domain", arr(&[0.0, 1.0])), ("Range", arr(&[0.0, 1.0, 0.0, 1.0]))], b"{ dup 0.5 gt { 1 exch sub } if dup }");
        let f = Function::parse(&doc(), &s).unwrap();
        assert_eq!(f.eval(&[0.25]), vec![0.25, 0.25]);
        assert_eq!(f.eval(&[0.75]), vec![0.25, 0.25]);
        let s = stream(vec![("FunctionType", Obj::Int(4)), ("Domain", arr(&[0.0, 1.0])), ("Range", arr(&[0.0, 1.0]))], b"{ 0.5 lt { 0 } { 1 } ifelse }");
        let f = Function::parse(&doc(), &s).unwrap();
        assert_eq!((f.eval(&[0.2])[0], f.eval(&[0.9])[0]), (0.0, 1.0));
    }

    #[test]
    fn sampled_interpolates_between_samples() {
        let s = stream(
            vec![
                ("FunctionType", Obj::Int(0)),
                ("Domain", arr(&[0.0, 1.0])),
                ("Range", arr(&[0.0, 1.0])),
                ("Size", Obj::Array(vec![Obj::Int(3)])),
                ("BitsPerSample", Obj::Int(8)),
            ],
            &[0, 255, 0],
        );
        let f = Function::parse(&doc(), &s).unwrap();
        assert!((f.eval(&[0.25])[0] - 0.5).abs() < 1e-3);
        assert!((f.eval(&[0.5])[0] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn stitching_picks_the_subdomain() {
        let e = |c0: f64, c1: f64| {
            let d: Dict = [("FunctionType", Obj::Int(2)), ("Domain", arr(&[0.0, 1.0])), ("C0", arr(&[c0])), ("C1", arr(&[c1])), ("N", Obj::Int(1))].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            Obj::Dict(d)
        };
        let d: Dict = [
            ("FunctionType", Obj::Int(3)),
            ("Domain", arr(&[0.0, 1.0])),
            ("Functions", Obj::Array(vec![e(0.0, 1.0), e(10.0, 20.0)])),
            ("Bounds", arr(&[0.5])),
            ("Encode", arr(&[0.0, 1.0, 0.0, 1.0])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let f = Function::parse(&doc(), &Obj::Dict(d)).unwrap();
        assert!((f.eval(&[0.25])[0] - 0.5).abs() < 1e-5);
        assert!((f.eval(&[0.75])[0] - 15.0).abs() < 1e-4);
    }
}
