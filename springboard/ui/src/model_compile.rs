//! Ahead-of-time compilation of a model description into a plan artifact for one target.
//!
//! A model is a chain of operators that starts from an input width. [`compile`] checks every shape along the chain
//! and serializes the plan as a canonical little-endian byte string, so the same description always yields the same
//! bytes. [`prepare`] runs the compile only when a [`SpecializationCache`] holds no artifact for the model and target,
//! which lets an app pay the compile once and reuse it. No kernels are generated here and nothing is written to disk.

use crate::model_specialization::{Policy, SpecializationCache};

/// Magic bytes at the start of every plan artifact.
pub const MAGIC: &[u8; 4] = b"BFMP";
/// Layout version of the plan artifact.
pub const VERSION: u8 = 1;

/// The device class a plan is compiled for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Cpu,
    Gpu,
}

impl Target {
    /// The name the target is cached under, and the options text of its artifact.
    pub fn name(self) -> &'static str {
        match self {
            Target::Cpu => "cpu",
            Target::Gpu => "gpu",
        }
    }

    fn tag(self) -> u8 {
        match self {
            Target::Cpu => 0,
            Target::Gpu => 1,
        }
    }
}

/// One operator of a model chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A fully connected layer. Its input width is the width the chain carries into it.
    Dense { outputs: u32 },
    /// An element-wise rectifier. The width is unchanged.
    Relu,
    /// A normalised exponential over the width. The width is unchanged.
    Softmax,
}

/// A model description: a name, the width of its input and the operators applied in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    pub name: String,
    pub input_width: u32,
    pub ops: Vec<Op>,
}

/// Why a model description cannot be compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// The model has no name, so it cannot be cached.
    EmptyName,
    /// A width is zero: the input width, or the outputs of a dense operator.
    ZeroWidth,
    /// The model has no operators.
    NoOperators,
}

/// The width the chain carries out of the last operator, after checking the description.
pub fn output_width(graph: &Graph) -> Result<u32, CompileError> {
    if graph.name.is_empty() {
        return Err(CompileError::EmptyName);
    }
    if graph.input_width == 0 {
        return Err(CompileError::ZeroWidth);
    }
    if graph.ops.is_empty() {
        return Err(CompileError::NoOperators);
    }
    let mut width = graph.input_width;
    for op in &graph.ops {
        if let Op::Dense { outputs } = op {
            if *outputs == 0 {
                return Err(CompileError::ZeroWidth);
            }
            width = *outputs;
        }
    }
    Ok(width)
}

/// Compiles the description for `target`. The artifact is the magic, the version, the target tag, the name, the input
/// width, the operators and finally the output width, so a runtime can check its shapes without the description.
pub fn compile(graph: &Graph, target: Target) -> Result<Vec<u8>, CompileError> {
    let outputs = output_width(graph)?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(target.tag());
    put_u32(&mut out, graph.name.len() as u32);
    out.extend_from_slice(graph.name.as_bytes());
    put_u32(&mut out, graph.input_width);
    put_u32(&mut out, graph.ops.len() as u32);
    for op in &graph.ops {
        match op {
            Op::Dense { outputs } => {
                out.push(0);
                put_u32(&mut out, *outputs);
            }
            Op::Relu => out.push(1),
            Op::Softmax => out.push(2),
        }
    }
    put_u32(&mut out, outputs);
    Ok(out)
}

/// Makes sure the cache holds the artifact of `graph` for `target`, compiling it when it does not. Returns whether this
/// call compiled. The artifact is then read with `cache.lookup(&graph.name, target.name())`, which is None only when an
/// artifact larger than the cache limit was not kept.
pub fn prepare(cache: &mut SpecializationCache, graph: &Graph, target: Target, policy: Policy) -> Result<bool, CompileError> {
    if cache.policy_of(&graph.name, target.name()).is_some() {
        return Ok(false);
    }
    let artifact = compile(graph, target)?;
    cache.store(&graph.name, target.name(), artifact, policy);
    Ok(true)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        Graph { name: "tiny".into(), input_width: 4, ops: vec![Op::Dense { outputs: 3 }, Op::Relu, Op::Softmax] }
    }

    #[test]
    fn the_output_width_is_the_last_dense_width() {
        assert_eq!(output_width(&graph()), Ok(3));
        let two = Graph { ops: vec![Op::Dense { outputs: 8 }, Op::Dense { outputs: 2 }], ..graph() };
        assert_eq!(output_width(&two), Ok(2));
    }

    #[test]
    fn a_bad_description_is_refused_before_anything_is_written() {
        assert_eq!(output_width(&Graph { name: String::new(), ..graph() }), Err(CompileError::EmptyName));
        assert_eq!(output_width(&Graph { input_width: 0, ..graph() }), Err(CompileError::ZeroWidth));
        assert_eq!(output_width(&Graph { ops: vec![], ..graph() }), Err(CompileError::NoOperators));
        assert_eq!(
            output_width(&Graph { ops: vec![Op::Dense { outputs: 0 }], ..graph() }),
            Err(CompileError::ZeroWidth)
        );
    }

    #[test]
    fn the_same_description_compiles_to_the_same_bytes_and_starts_with_the_magic() {
        let a = compile(&graph(), Target::Cpu).unwrap();
        let b = compile(&graph(), Target::Cpu).unwrap();
        assert_eq!(a, b);
        assert_eq!(&a[..4], MAGIC);
        assert_eq!(a[4], VERSION);
        // The target is part of the artifact.
        assert_ne!(a, compile(&graph(), Target::Gpu).unwrap());
        // The output width closes the artifact.
        assert_eq!(&a[a.len() - 4..], &3u32.to_le_bytes());
    }

    #[test]
    fn prepare_compiles_once_per_model_and_target() {
        let mut cache = SpecializationCache::new(1 << 16);
        assert_eq!(prepare(&mut cache, &graph(), Target::Cpu, Policy::Purgeable), Ok(true));
        assert_eq!(prepare(&mut cache, &graph(), Target::Cpu, Policy::Purgeable), Ok(false));
        assert_eq!(prepare(&mut cache, &graph(), Target::Gpu, Policy::Persistent), Ok(true));
        assert_eq!(cache.len(), 2);
        let held = cache.lookup("tiny", Target::Cpu.name()).map(|b| b.to_vec());
        assert_eq!(held, Some(compile(&graph(), Target::Cpu).unwrap()));
    }
}
