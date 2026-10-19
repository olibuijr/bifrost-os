//! A CPU runtime for compiled compute graphs: [`run`] walks an [`Executable`]'s schedule and computes every node, so a
//! graph built with `graph::Graph` produces values (the runtime half of the graph engine; `graph` records and checks
//! the graph, this module executes it).
//!
//! Every operator of `graph` runs: element-wise binary operators with broadcasting, unary operators, relu, reshape,
//! cast, transpose, batched matrix product and concatenation, on every element type. Values are computed in `f64`
//! and written back in the node's element type, so Float32 arithmetic is correctly rounded for one operator at a
//! time, Float16 and BFloat16 round to nearest even, and integer results saturate at the type's range (they do not
//! wrap). An integer divide by zero is an error ([`RunError::DivideByZero`]); a float divide by zero follows IEEE.
//!
//! Limits: one thread, no fused kernels and no GPU path (placement is decided in `compute`); the reduced-precision
//! option is recorded but this runtime always computes at full precision, which the option allows.

use std::collections::HashMap;

use crate::graph::{Binary, Executable, Graph, GraphError, Kind, TensorId, Unary};
use crate::tensor::{DataType, TensorData, TensorType};

/// Why a run failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// The feeds do not match the executable's signature, or the schedule names a node the graph lacks.
    Graph(GraphError),
    /// A constant node has no payload (the graph was not built with `Graph::constant`).
    MissingConstant(TensorId),
    /// An integer division by zero at this node.
    DivideByZero(TensorId),
    /// A node's computed length differs from its type (a bug in the graph or in this runtime).
    Inconsistent(TensorId),
}

impl From<GraphError> for RunError {
    fn from(e: GraphError) -> Self {
        RunError::Graph(e)
    }
}

/// One node's values: the type and the elements in row-major order.
#[derive(Clone, Debug)]
struct Val {
    ty: TensorType,
    v: Vec<f64>,
}

/// Runs `exe` (compiled from `graph`) on `feeds` and returns the requested outputs in the order they were requested.
pub fn run(graph: &Graph, exe: &Executable, feeds: &[(TensorId, TensorData)]) -> Result<Vec<(TensorId, TensorData)>, RunError> {
    let signature: Vec<(TensorId, TensorType)> = feeds.iter().map(|(id, d)| (*id, d.ty().clone())).collect();
    exe.check_feeds(&signature)?;
    let mut vals: HashMap<TensorId, Val> = HashMap::new();
    for &id in exe.order() {
        let node = graph.node(id).ok_or(GraphError::Unknown(id))?;
        let get = |vals: &HashMap<TensorId, Val>, x: &TensorId| -> Result<Val, RunError> { vals.get(x).cloned().ok_or(RunError::Graph(GraphError::Unknown(*x))) };
        let out = match &node.kind {
            Kind::Placeholder => {
                let data = feeds.iter().find(|(f, _)| *f == id).map(|(_, d)| d).ok_or(GraphError::MissingFeed(id))?;
                Val { ty: data.ty().clone(), v: decode(data) }
            }
            Kind::Constant => {
                let data = graph.constant_data(id).ok_or(RunError::MissingConstant(id))?;
                Val { ty: data.ty().clone(), v: decode(data) }
            }
            Kind::Binary(op, a, b) => binary(id, *op, &get(&vals, a)?, &get(&vals, b)?, &node.ty)?,
            Kind::Unary(op, x) => {
                let x = get(&vals, x)?;
                Val { v: x.v.iter().map(|&e| unary(*op, e)).collect(), ty: node.ty.clone() }
            }
            Kind::Relu(x) => {
                let x = get(&vals, x)?;
                Val { v: x.v.iter().map(|&e| if e > 0.0 { e } else { 0.0 }).collect(), ty: node.ty.clone() }
            }
            Kind::Reshape(x, _) => Val { v: get(&vals, x)?.v, ty: node.ty.clone() },
            Kind::Cast(x, _) => Val { v: get(&vals, x)?.v, ty: node.ty.clone() },
            Kind::Transpose(x, perm) => transpose(&get(&vals, x)?, perm, &node.ty),
            Kind::Matmul(a, b) => matmul(&get(&vals, a)?, &get(&vals, b)?, &node.ty),
            Kind::Concat(parts, axis) => {
                let parts = parts.iter().map(|p| get(&vals, p)).collect::<Result<Vec<_>, _>>()?;
                concat(&parts, *axis, &node.ty)
            }
        };
        // a cast or a reshape changes the type or the shape the next node reads, so the value is stored as the node's
        let out = settle(id, out, &node.ty)?;
        vals.insert(id, out);
    }
    exe.outputs()
        .iter()
        .map(|(id, ty)| -> Result<(TensorId, TensorData), RunError> {
            let v = vals.get(id).ok_or(GraphError::Unknown(*id))?;
            Ok((*id, encode(ty, &v.v)))
        })
        .collect()
}

/// The value stored for a node: the node's type, with every element rounded or saturated to its element type so that
/// the next node reads exactly what a stored tensor would hold.
fn settle(id: TensorId, val: Val, ty: &TensorType) -> Result<Val, RunError> {
    if val.v.len() != ty.shape.elements().unwrap_or(usize::MAX) {
        return Err(RunError::Inconsistent(id));
    }
    let v = val.v.iter().map(|&e| quantize(ty.data_type, e)).collect();
    Ok(Val { ty: ty.clone(), v })
}

/// `e` as the value an element of type `dt` holds.
fn quantize(dt: DataType, e: f64) -> f64 {
    match dt {
        DataType::Float32 => f64::from(e as f32),
        DataType::Float16 => f64::from(f16_to_f32(f32_to_f16(e as f32))),
        DataType::BFloat16 => f64::from(bf16_to_f32(f32_to_bf16(e as f32))),
        DataType::Int8 => f64::from(e as i8),
        DataType::UInt8 => f64::from(e as u8),
        DataType::Int16 => f64::from(e as i16),
        DataType::Int32 => f64::from(e as i32),
        DataType::Bool => f64::from(u8::from(e != 0.0)),
    }
}

fn decode(data: &TensorData) -> Vec<f64> {
    let b = data.bytes();
    match data.ty().data_type {
        DataType::Float32 => b.chunks_exact(4).map(|c| f64::from(f32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect(),
        DataType::Float16 => b.chunks_exact(2).map(|c| f64::from(f16_to_f32(u16::from_le_bytes([c[0], c[1]])))).collect(),
        DataType::BFloat16 => b.chunks_exact(2).map(|c| f64::from(bf16_to_f32(u16::from_le_bytes([c[0], c[1]])))).collect(),
        DataType::Int8 => b.iter().map(|&x| f64::from(x as i8)).collect(),
        DataType::UInt8 => b.iter().map(|&x| f64::from(x)).collect(),
        DataType::Int16 => b.chunks_exact(2).map(|c| f64::from(i16::from_le_bytes([c[0], c[1]]))).collect(),
        DataType::Int32 => b.chunks_exact(4).map(|c| f64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect(),
        DataType::Bool => b.iter().map(|&x| f64::from(u8::from(x != 0))).collect(),
    }
}

fn encode(ty: &TensorType, v: &[f64]) -> TensorData {
    let bytes: Vec<u8> = match ty.data_type {
        DataType::Float32 => v.iter().flat_map(|&e| (e as f32).to_le_bytes()).collect(),
        DataType::Float16 => v.iter().flat_map(|&e| f32_to_f16(e as f32).to_le_bytes()).collect(),
        DataType::BFloat16 => v.iter().flat_map(|&e| f32_to_bf16(e as f32).to_le_bytes()).collect(),
        DataType::Int8 => v.iter().map(|&e| (e as i8) as u8).collect(),
        DataType::UInt8 => v.iter().map(|&e| e as u8).collect(),
        DataType::Int16 => v.iter().flat_map(|&e| (e as i16).to_le_bytes()).collect(),
        DataType::Int32 => v.iter().flat_map(|&e| (e as i32).to_le_bytes()).collect(),
        DataType::Bool => v.iter().map(|&e| u8::from(e != 0.0)).collect(),
    };
    // the length matches the type by construction (`settle` checked the element count)
    TensorData::new(ty.clone(), bytes).expect("encoded length matches the type")
}

/// For every element of an output of `out_dims`, the linear index of the input element it reads (stride 0 along
/// broadcast dimensions).
fn broadcast_map(out_dims: &[usize], in_dims: &[usize]) -> Vec<usize> {
    let n: usize = out_dims.iter().product();
    let lead = out_dims.len() - in_dims.len();
    let mut in_strides = vec![0usize; out_dims.len()];
    let mut stride = 1usize;
    for i in (0..in_dims.len()).rev() {
        in_strides[lead + i] = if in_dims[i] == 1 { 0 } else { stride };
        stride *= in_dims[i];
    }
    let mut map = Vec::with_capacity(n);
    let mut idx = vec![0usize; out_dims.len()];
    for _ in 0..n {
        map.push(idx.iter().zip(&in_strides).map(|(i, s)| i * s).sum::<usize>());
        for d in (0..out_dims.len()).rev() {
            idx[d] += 1;
            if idx[d] < out_dims[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    map
}

fn binary(id: TensorId, op: Binary, a: &Val, b: &Val, ty: &TensorType) -> Result<Val, RunError> {
    let dims = ty.shape.dims();
    let (ma, mb) = (broadcast_map(dims, a.ty.shape.dims()), broadcast_map(dims, b.ty.shape.dims()));
    let integer = !ty.data_type.is_float();
    let mut v = Vec::with_capacity(ma.len());
    for (&ia, &ib) in ma.iter().zip(&mb) {
        let (x, y) = (a.v[ia], b.v[ib]);
        v.push(match op {
            Binary::Add => x + y,
            Binary::Subtract => x - y,
            Binary::Multiply => x * y,
            Binary::Divide if integer => {
                if y == 0.0 {
                    return Err(RunError::DivideByZero(id));
                }
                (x / y).trunc()
            }
            Binary::Divide => x / y,
            Binary::Maximum => x.max(y),
            Binary::Minimum => x.min(y),
        });
    }
    Ok(Val { ty: ty.clone(), v })
}

fn unary(op: Unary, e: f64) -> f64 {
    match op {
        Unary::Negate => -e,
        Unary::Absolute => e.abs(),
        Unary::Square => e * e,
        Unary::Sqrt => e.sqrt(),
        Unary::Exponential => e.exp(),
    }
}

fn row_major_strides(dims: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; dims.len()];
    for i in (0..dims.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * dims[i + 1];
    }
    s
}

fn transpose(x: &Val, perm: &[usize], ty: &TensorType) -> Val {
    let in_strides = row_major_strides(x.ty.shape.dims());
    let out_dims = ty.shape.dims();
    let strides: Vec<usize> = perm.iter().map(|&p| in_strides[p]).collect();
    let n: usize = out_dims.iter().product();
    let mut v = Vec::with_capacity(n);
    let mut idx = vec![0usize; out_dims.len()];
    for _ in 0..n {
        v.push(x.v[idx.iter().zip(&strides).map(|(i, s)| i * s).sum::<usize>()]);
        for d in (0..out_dims.len()).rev() {
            idx[d] += 1;
            if idx[d] < out_dims[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    Val { ty: ty.clone(), v }
}

fn matmul(a: &Val, b: &Val, ty: &TensorType) -> Val {
    let (da, db, dout) = (a.ty.shape.dims(), b.ty.shape.dims(), ty.shape.dims());
    let (m, k) = (da[da.len() - 2], da[da.len() - 1]);
    let n = db[db.len() - 1];
    let batch_out = &dout[..dout.len() - 2];
    let batch_count: usize = batch_out.iter().product();
    let ba = broadcast_map(batch_out, &da[..da.len() - 2]);
    let bb = broadcast_map(batch_out, &db[..db.len() - 2]);
    let mut v = Vec::with_capacity(batch_count * m * n);
    for batch in 0..batch_count {
        let (oa, ob) = (ba[batch] * m * k, bb[batch] * k * n);
        for i in 0..m {
            for j in 0..n {
                let mut sum = 0.0;
                for t in 0..k {
                    sum += a.v[oa + i * k + t] * b.v[ob + t * n + j];
                }
                v.push(sum);
            }
        }
    }
    Val { ty: ty.clone(), v }
}

fn concat(parts: &[Val], axis: usize, ty: &TensorType) -> Val {
    let dims = ty.shape.dims();
    let outer: usize = dims[..axis].iter().product();
    let inner: usize = dims[axis + 1..].iter().product();
    let mut v = Vec::with_capacity(dims.iter().product());
    for o in 0..outer {
        for p in parts {
            let chunk = p.ty.shape.dims()[axis] * inner;
            v.extend_from_slice(&p.v[o * chunk..(o + 1) * chunk]);
        }
    }
    Val { ty: ty.clone(), v }
}

/// IEEE half precision to single.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let frac = u32::from(h & 0x3ff);
    match exp {
        0 => {
            let magnitude = frac as f32 * (1.0 / 16_777_216.0); // frac * 2^-24
            if sign == 0 { magnitude } else { -magnitude }
        }
        31 => f32::from_bits(sign | 0x7f80_0000 | (frac << 13)),
        _ => f32::from_bits(sign | ((exp + 112) << 23) | (frac << 13)),
    }
}

/// Single precision to IEEE half, rounding to nearest even; overflow becomes infinity.
pub fn f32_to_f16(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x007f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let halfway = 1u32 << (shift - 1);
        let mut r = half;
        if rem > halfway || (rem == halfway && half & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) {
        r += 1;
    }
    sign | r as u16
}

/// Brain floating point to single.
pub fn bf16_to_f32(b: u16) -> f32 {
    f32::from_bits(u32::from(b) << 16)
}

/// Single precision to brain floating point, rounding to nearest even.
pub fn f32_to_bf16(f: f32) -> u16 {
    if f.is_nan() {
        return ((f.to_bits() >> 16) as u16) | 0x0040;
    }
    let x = f.to_bits();
    let round = ((x >> 16) & 1) + 0x7fff;
    ((x.wrapping_add(round)) >> 16) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32s(dims: &[usize], v: &[f32]) -> TensorData {
        TensorData::from_f32(dims, v).unwrap()
    }

    fn out_f32(r: &[(TensorId, TensorData)], i: usize) -> Vec<f32> {
        r[i].1.f32_values().unwrap()
    }

    fn ints(dt: DataType, dims: &[usize], v: &[i32]) -> TensorData {
        let bytes = match dt {
            DataType::Int32 => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            DataType::Int8 => v.iter().map(|&x| x as i8 as u8).collect(),
            other => panic!("{other:?}"),
        };
        TensorData::new(TensorType::new(dt, dims), bytes).unwrap()
    }

    fn i32s(d: &TensorData) -> Vec<i32> {
        d.bytes().chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn a_dense_layer_runs_matmul_bias_and_relu() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[2, 3]);
        let w = g.constant(f32s(&[3, 2], &[1.0, -1.0, 0.5, 2.0, -2.0, 1.0]));
        let bias = g.constant(f32s(&[2], &[0.5, -10.0]));
        let y = g.matmul(x, w).unwrap();
        let y = g.binary(Binary::Add, y, bias).unwrap();
        let y = g.relu(y).unwrap();
        let exe = g.compile(&[y]).unwrap();
        let out = run(&g, &exe, &[(x, f32s(&[2, 3], &[1.0, 2.0, 3.0, 0.0, 1.0, 0.0]))]).unwrap();
        // row 0: [1*1+2*0.5+3*-2, 1*-1+2*2+3*1] = [-4, 6]; plus bias [-3.5, -4] -> relu [0, 0]
        // row 1: [0.5, 2] + bias [1.0, -8] -> relu [1.0, 0]
        assert_eq!(out_f32(&out, 0), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(out[0].0, y);
    }

    #[test]
    fn binary_operators_broadcast_and_keep_the_order_of_operands() {
        let mut g = Graph::new();
        let a = g.constant(f32s(&[2, 1], &[10.0, 20.0]));
        let b = g.constant(f32s(&[3], &[1.0, 2.0, 4.0]));
        let sub = g.binary(Binary::Subtract, a, b).unwrap();
        let div = g.binary(Binary::Divide, a, b).unwrap();
        let max = g.binary(Binary::Maximum, a, b).unwrap();
        let min = g.binary(Binary::Minimum, a, b).unwrap();
        let mul = g.binary(Binary::Multiply, a, b).unwrap();
        let exe = g.compile(&[sub, div, max, min, mul]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        assert_eq!(out_f32(&out, 0), [9.0, 8.0, 6.0, 19.0, 18.0, 16.0]);
        assert_eq!(out_f32(&out, 1), [10.0, 5.0, 2.5, 20.0, 10.0, 5.0]);
        assert_eq!(out_f32(&out, 2), [10.0, 10.0, 10.0, 20.0, 20.0, 20.0]);
        assert_eq!(out_f32(&out, 3), [1.0, 2.0, 4.0, 1.0, 2.0, 4.0]);
        assert_eq!(out_f32(&out, 4), [10.0, 20.0, 40.0, 20.0, 40.0, 80.0]);
        assert_eq!(out[0].1.ty().shape.dims(), [2, 3]);
    }

    #[test]
    fn unary_operators_and_a_float_divide_by_zero_follow_ieee() {
        let mut g = Graph::new();
        let x = g.constant(f32s(&[4], &[-4.0, 0.0, 9.0, 1.0]));
        let neg = g.unary(Unary::Negate, x).unwrap();
        let abs = g.unary(Unary::Absolute, x).unwrap();
        let sq = g.unary(Unary::Square, x).unwrap();
        let root = g.unary(Unary::Sqrt, abs).unwrap();
        let exp = g.unary(Unary::Exponential, x).unwrap();
        let zero = g.constant(f32s(&[], &[0.0]));
        let div = g.binary(Binary::Divide, x, zero).unwrap();
        let exe = g.compile(&[neg, abs, sq, root, exp, div]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        assert_eq!(out_f32(&out, 0), [4.0, -0.0, -9.0, -1.0]);
        assert_eq!(out_f32(&out, 1), [4.0, 0.0, 9.0, 1.0]);
        assert_eq!(out_f32(&out, 2), [16.0, 0.0, 81.0, 1.0]);
        assert_eq!(out_f32(&out, 3), [2.0, 0.0, 3.0, 1.0]);
        assert!((out_f32(&out, 4)[3] - std::f32::consts::E).abs() < 1e-6);
        let d = out_f32(&out, 5);
        assert!(d[0] == f32::NEG_INFINITY && d[1].is_nan() && d[2] == f32::INFINITY);
    }

    #[test]
    fn transpose_reshape_and_concat_move_elements_to_the_right_places() {
        let mut g = Graph::new();
        let x = g.constant(f32s(&[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
        let t = g.transpose(x, &[1, 0]).unwrap();
        let r = g.reshape(t, &[6]).unwrap();
        let y = g.constant(f32s(&[2, 2], &[7.0, 8.0, 9.0, 10.0]));
        let c0 = g.concat(&[x, y], 1).unwrap(); // [2, 5]
        let z = g.constant(f32s(&[1, 3], &[0.0, 0.0, 0.0]));
        let c1 = g.concat(&[x, z], 0).unwrap(); // [3, 3]
        let exe = g.compile(&[t, r, c0, c1]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        assert_eq!(out_f32(&out, 0), [1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert_eq!(out[0].1.ty().shape.dims(), [3, 2]);
        assert_eq!(out_f32(&out, 1), [1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert_eq!(out_f32(&out, 2), [1.0, 2.0, 3.0, 7.0, 8.0, 4.0, 5.0, 6.0, 9.0, 10.0]);
        assert_eq!(out_f32(&out, 3), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_three_dimensional_transpose_and_a_batched_matmul_broadcast_the_batch() {
        let mut g = Graph::new();
        let a = g.constant(f32s(&[2, 2, 2], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]));
        let t = g.transpose(a, &[2, 0, 1]).unwrap();
        let eye = g.constant(f32s(&[2, 2], &[0.0, 1.0, 1.0, 0.0])); // swaps columns; its batch broadcasts
        let p = g.matmul(a, eye).unwrap();
        let exe = g.compile(&[t, p]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        // t[k][i][j] = a[i][j][k]
        assert_eq!(out_f32(&out, 0), [1.0, 3.0, 5.0, 7.0, 2.0, 4.0, 6.0, 8.0]);
        assert_eq!(out_f32(&out, 1), [2.0, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]);
    }

    #[test]
    fn casts_truncate_saturate_and_round_to_the_target_type() {
        let mut g = Graph::new();
        let x = g.constant(f32s(&[5], &[1.9, -1.9, 300.0, -300.0, 0.5]));
        let i32c = g.cast(x, DataType::Int32).unwrap();
        let i8c = g.cast(x, DataType::Int8).unwrap();
        let b = g.cast(x, DataType::Bool).unwrap();
        let h = g.cast(x, DataType::Float16).unwrap();
        let back = g.cast(h, DataType::Float32).unwrap();
        let exe = g.compile(&[i32c, i8c, b, back]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        assert_eq!(i32s(&out[0].1), [1, -1, 300, -300, 0]);
        assert_eq!(out[1].1.bytes(), [1u8, 255, 127, 128, 0], "Int8 saturates: -300 is -128 (0x80)");
        assert_eq!(out[2].1.bytes(), [1u8, 1, 1, 1, 1]);
        assert_eq!(out_f32(&out, 3), [1.900390625, -1.900390625, 300.0, -300.0, 0.5]);
    }

    #[test]
    fn integer_arithmetic_truncates_divides_and_refuses_zero() {
        let mut g = Graph::new();
        let a = g.constant(ints(DataType::Int32, &[3], &[7, -7, 100]));
        let b = g.constant(ints(DataType::Int32, &[3], &[2, 2, 7]));
        let q = g.binary(Binary::Divide, a, b).unwrap();
        let s = g.binary(Binary::Multiply, a, b).unwrap();
        let exe = g.compile(&[q, s]).unwrap();
        let out = run(&g, &exe, &[]).unwrap();
        assert_eq!(i32s(&out[0].1), [3, -3, 14]);
        assert_eq!(i32s(&out[1].1), [14, -14, 700]);
        let z = g.constant(ints(DataType::Int32, &[3], &[1, 0, 1]));
        let bad = g.binary(Binary::Divide, a, z).unwrap();
        let exe = g.compile(&[bad]).unwrap();
        assert_eq!(run(&g, &exe, &[]), Err(RunError::DivideByZero(bad)));
        // Int8 products saturate instead of wrapping
        let p = g.constant(ints(DataType::Int8, &[1], &[100]));
        let sat = g.binary(Binary::Multiply, p, p).unwrap();
        let exe = g.compile(&[sat]).unwrap();
        assert_eq!(run(&g, &exe, &[]).unwrap()[0].1.bytes(), [127u8]);
    }

    #[test]
    fn feeds_are_checked_against_the_signature_before_anything_runs() {
        let mut g = Graph::new();
        let x = g.placeholder(DataType::Float32, &[2]);
        let y = g.unary(Unary::Negate, x).unwrap();
        let exe = g.compile(&[y]).unwrap();
        assert_eq!(run(&g, &exe, &[]), Err(RunError::Graph(GraphError::MissingFeed(x))));
        assert_eq!(run(&g, &exe, &[(x, f32s(&[3], &[1.0, 2.0, 3.0]))]), Err(RunError::Graph(GraphError::FeedType(x))));
        assert_eq!(run(&g, &exe, &[(x, f32s(&[2], &[1.0, 2.0])), (y, f32s(&[2], &[1.0, 2.0]))]), Err(RunError::Graph(GraphError::NotAFeed(y))));
        let ok = run(&g, &exe, &[(x, f32s(&[2], &[1.0, -2.0]))]).unwrap();
        assert_eq!(out_f32(&ok, 0), [-1.0, 2.0]);
    }

    #[test]
    fn only_the_nodes_an_output_needs_are_computed() {
        let mut g = Graph::new();
        let a = g.constant(f32s(&[1], &[2.0]));
        let unused_zero = g.constant(ints(DataType::Int32, &[1], &[0]));
        let _bad = g.binary(Binary::Divide, unused_zero, unused_zero).unwrap();
        let out = g.unary(Unary::Square, a).unwrap();
        let exe = g.compile(&[out]).unwrap();
        assert_eq!(out_f32(&run(&g, &exe, &[]).unwrap(), 0), [4.0]);
    }

    #[test]
    fn half_and_brain_float_conversions_round_to_nearest_even() {
        for v in [0.0f32, 1.0, -2.5, 65504.0, 6.1035156e-5, 5.9604645e-8] {
            assert_eq!(f16_to_f32(f32_to_f16(v)), v, "{v} is exactly representable");
        }
        assert_eq!(f32_to_f16(65520.0), 0x7c00, "past the largest half rounds to infinity");
        assert_eq!(f32_to_f16(1.0 + 1.0 / 2048.0), 0x3c00, "a tie rounds to even");
        assert_eq!(f32_to_f16(1.0 + 3.0 / 2048.0), 0x3c02);
        assert_eq!(f32_to_f16(1e-9), 0, "below half the smallest subnormal rounds to zero");
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
        assert_eq!(f32_to_f16(f32::NEG_INFINITY), 0xfc00);
        assert_eq!(bf16_to_f32(f32_to_bf16(1.0)), 1.0);
        assert_eq!(f32_to_bf16(1.0 + 1.0 / 256.0), 0x3f80, "a tie rounds to even");
        assert_eq!(f32_to_bf16(1.0 + 3.0 / 256.0), 0x3f82);
        assert!(bf16_to_f32(f32_to_bf16(f32::NAN)).is_nan());
    }

    #[test]
    fn half_precision_graphs_round_each_result_to_half() {
        let mut g = Graph::new();
        let a = g.placeholder(DataType::Float16, &[1]);
        let h = TensorData::new(TensorType::new(DataType::Float16, &[1]), f32_to_f16(1.0).to_le_bytes().to_vec()).unwrap();
        let big = g.constant(TensorData::new(TensorType::new(DataType::Float16, &[1]), f32_to_f16(2048.0).to_le_bytes().to_vec()).unwrap());
        let sum = g.binary(Binary::Add, big, a).unwrap();
        let exe = g.compile(&[sum]).unwrap();
        let out = run(&g, &exe, &[(a, h)]).unwrap();
        // 2048 + 1 = 2049 is not a half (the spacing there is 2); the tie rounds to even, which is 2048
        assert_eq!(f16_to_f32(u16::from_le_bytes([out[0].1.bytes()[0], out[0].1.bytes()[1]])), 2048.0);
    }
}
