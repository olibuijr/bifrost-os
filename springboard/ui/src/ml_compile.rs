//! The compile step of the model runtime: a [`Model`] (read from its text form or built in code) becomes one compact,
//! checked binary, and the binary becomes a model again without any text parsing. The runtime service compiles a model
//! once when it is installed and loads the compiled form on every launch, so a damaged or truncated file is refused up
//! front instead of failing in the middle of a prediction.
//!
//! Layout (all numbers little endian): the magic `BMLC`, a format byte (1), the input name (u16 length and UTF-8), the
//! input length (u32), the output name (same form), the layer count (u32), then each layer as a tag byte (1 dense,
//! 2 relu, 3 softmax) and, for dense, the output count (u32), the weight count (u32) with that many `f32`, and the bias
//! count (u32) with that many `f32`. A 64-bit FNV-1a checksum of everything before it closes the file. Counts are
//! checked against the bytes that are left before anything is allocated, so a hostile count cannot reserve memory.

use crate::ml_model::{Layer, Model, ModelError};

/// The first four bytes of a compiled model.
pub const MAGIC: [u8; 4] = *b"BMLC";
/// The format byte. A file with another value is refused.
pub const FORMAT: u8 = 1;
/// File extension of a compiled model.
pub const EXTENSION: &str = "bmlc";

const TAG_DENSE: u8 = 1;
const TAG_RELU: u8 = 2;
const TAG_SOFTMAX: u8 = 3;

/// Why a compiled model could not be made or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// The model has no output name, so it cannot predict and is not compiled.
    NoOutput,
    /// A name is longer than 65535 bytes.
    NameTooLong,
    /// The data does not start with the magic.
    BadMagic,
    /// The format byte is not one this build reads.
    UnsupportedFormat(u8),
    /// The data ends early, or a count runs past the end.
    Truncated,
    /// The checksum does not match: the file is damaged.
    Checksum,
    /// A layer tag is unknown.
    BadLayer(u8),
    /// A name is not valid UTF-8.
    BadName,
    /// Bytes remain after the last layer.
    TrailingBytes,
    /// The layers do not form a valid model (shape mismatch), or the text form did not parse.
    Model(ModelError),
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn put_name(out: &mut Vec<u8>, name: &str) -> Result<(), CompileError> {
    let len = u16::try_from(name.len()).map_err(|_| CompileError::NameTooLong)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    Ok(())
}

fn put_floats(out: &mut Vec<u8>, values: &[f32]) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Compiles `model` to its binary form.
pub fn compile(model: &Model) -> Result<Vec<u8>, CompileError> {
    let output = model.output_name().ok_or(CompileError::NoOutput)?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(FORMAT);
    put_name(&mut out, model.input_name())?;
    out.extend_from_slice(&(model.input_len() as u32).to_le_bytes());
    put_name(&mut out, output)?;
    out.extend_from_slice(&(model.layers().len() as u32).to_le_bytes());
    for layer in model.layers() {
        match layer {
            Layer::Dense { outputs, weights, bias } => {
                out.push(TAG_DENSE);
                out.extend_from_slice(&(*outputs as u32).to_le_bytes());
                put_floats(&mut out, weights);
                put_floats(&mut out, bias);
            }
            Layer::Relu => out.push(TAG_RELU),
            Layer::Softmax => out.push(TAG_SOFTMAX),
        }
    }
    let sum = fnv1a(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// Compiles a model from its text form (`Model::parse`). A text that does not parse is the parse error.
pub fn compile_text(text: &str) -> Result<Vec<u8>, CompileError> {
    compile(&Model::parse(text).map_err(CompileError::Model)?)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CompileError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(CompileError::Truncated)?;
        let slice = &self.data[self.at..end];
        self.at = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, CompileError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, CompileError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, CompileError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn name(&mut self) -> Result<String, CompileError> {
        let len = usize::from(self.u16()?);
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| CompileError::BadName)
    }
    fn floats(&mut self) -> Result<Vec<f32>, CompileError> {
        let count = self.u32()? as usize;
        let bytes = self.take(count.checked_mul(4).ok_or(CompileError::Truncated)?)?;
        Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }
}

/// Reads a compiled model. The checksum is verified before anything else is read.
pub fn decompile(data: &[u8]) -> Result<Model, CompileError> {
    if data.len() < MAGIC.len() || data[..MAGIC.len()] != MAGIC {
        return Err(CompileError::BadMagic);
    }
    if data.len() < MAGIC.len() + 1 + 8 {
        return Err(CompileError::Truncated);
    }
    let (body, tail) = data.split_at(data.len() - 8);
    if fnv1a(body) != u64::from_le_bytes(tail.try_into().unwrap()) {
        return Err(CompileError::Checksum);
    }
    let mut r = Reader { data: body, at: MAGIC.len() };
    let format = r.u8()?;
    if format != FORMAT {
        return Err(CompileError::UnsupportedFormat(format));
    }
    let input = r.name()?;
    let input_len = r.u32()? as usize;
    let output = r.name()?;
    let layers = r.u32()?;
    let mut model = Model::new(&input, input_len).map_err(CompileError::Model)?.with_output(&output);
    for _ in 0..layers {
        match r.u8()? {
            TAG_DENSE => {
                let outputs = r.u32()? as usize;
                let weights = r.floats()?;
                let bias = r.floats()?;
                model = model.dense(outputs, weights, bias).map_err(CompileError::Model)?;
            }
            TAG_RELU => model = model.relu(),
            TAG_SOFTMAX => model = model.softmax(),
            other => return Err(CompileError::BadLayer(other)),
        }
    }
    if r.at != body.len() {
        return Err(CompileError::TrailingBytes);
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::{DictionaryProvider, FeatureValue};
    use crate::ml_tensor::Tensor;

    fn model() -> Model {
        Model::new("x", 2)
            .unwrap()
            .dense(3, vec![1.0, -1.0, 0.5, 0.25, 2.0, -0.5], vec![0.1, 0.0, -0.1])
            .unwrap()
            .relu()
            .dense(2, vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0], vec![0.0, 0.0])
            .unwrap()
            .softmax()
            .with_output("y")
    }

    fn run(m: &Model) -> Vec<f32> {
        let input = DictionaryProvider::new().with("x", FeatureValue::Tensor(Tensor::new(&[2], vec![1.0, 2.0]).unwrap()));
        m.predict(&input).unwrap().value("y").and_then(FeatureValue::as_tensor).unwrap().values().to_vec()
    }

    fn header(input_len: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.push(FORMAT);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.push(b'x');
        b.extend_from_slice(&input_len.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.push(b'y');
        b.extend_from_slice(&1u32.to_le_bytes());
        b
    }

    fn sealed(mut body: Vec<u8>) -> Vec<u8> {
        let sum = fnv1a(&body);
        body.extend_from_slice(&sum.to_le_bytes());
        body
    }

    #[test]
    fn a_compiled_model_reads_back_equal_and_predicts_the_same() {
        let m = model();
        let bytes = compile(&m).unwrap();
        assert_eq!(&bytes[..4], b"BMLC");
        let back = decompile(&bytes).unwrap();
        assert_eq!(back, m);
        assert_eq!(run(&back), run(&m));
    }

    #[test]
    fn a_model_without_an_output_is_not_compiled() {
        let m = Model::new("x", 1).unwrap();
        assert_eq!(compile(&m), Err(CompileError::NoOutput));
    }

    #[test]
    fn text_compiles_to_the_same_bytes_as_the_built_model() {
        let text = "input x 2\ndense 1 1,1 0\nrelu\noutput y\n";
        let built = Model::new("x", 2).unwrap().dense(1, vec![1.0, 1.0], vec![0.0]).unwrap().relu().with_output("y");
        assert_eq!(compile_text(text).unwrap(), compile(&built).unwrap());
        assert!(matches!(compile_text("nonsense"), Err(CompileError::Model(ModelError::Parse { line: 1 }))));
    }

    #[test]
    fn damaged_data_is_refused_before_it_is_read() {
        let mut bytes = compile(&model()).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        assert_eq!(decompile(&bytes), Err(CompileError::Checksum));
    }

    #[test]
    fn short_and_foreign_data_is_refused() {
        assert_eq!(decompile(b"nope-nope-nope"), Err(CompileError::BadMagic));
        assert_eq!(decompile(b""), Err(CompileError::BadMagic));
        assert_eq!(decompile(b"BMLC\x01"), Err(CompileError::Truncated));
        let bytes = compile(&model()).unwrap();
        for cut in [bytes.len() - 1, bytes.len() / 2, 9] {
            assert!(decompile(&bytes[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn an_unknown_format_or_layer_is_refused_even_with_a_good_checksum() {
        let mut body = compile(&model()).unwrap();
        body.truncate(body.len() - 8);
        body[4] = 2;
        assert_eq!(decompile(&sealed(body)), Err(CompileError::UnsupportedFormat(2)));

        let mut tiny = header(1);
        tiny.push(9);
        assert_eq!(decompile(&sealed(tiny)), Err(CompileError::BadLayer(9)));
    }

    #[test]
    fn a_huge_weight_count_is_refused_without_allocating() {
        let mut body = header(1);
        body.push(TAG_DENSE);
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decompile(&sealed(body)), Err(CompileError::Truncated));
    }

    #[test]
    fn a_layer_shape_that_does_not_fit_is_a_model_error() {
        let mut body = header(2);
        body.push(TAG_DENSE);
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes()); // one weight, but two inputs need two
        body.extend_from_slice(&1.0f32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&0.0f32.to_le_bytes());
        assert_eq!(decompile(&sealed(body)), Err(CompileError::Model(ModelError::LayerSize)));
    }

    #[test]
    fn trailing_bytes_after_the_last_layer_are_refused() {
        let mut body = compile(&model()).unwrap();
        body.truncate(body.len() - 8);
        body.push(0);
        assert_eq!(decompile(&sealed(body)), Err(CompileError::TrailingBytes));
    }
}
