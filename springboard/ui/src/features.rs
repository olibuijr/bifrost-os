//! Typed model inputs and outputs (`MLFeatureValue`, `MLFeatureProvider`, `MLDictionaryFeatureProvider`,
//! `MLBatchProvider`, `MLArrayBatchProvider`): named values that a model reads and writes. A model declares a
//! kind for each name, and [`check`] refuses a provider that lacks one or holds the wrong kind. Batches group
//! providers so that several predictions can be run as one request.
use crate::tensor::Tensor;
use std::collections::BTreeMap;

/// The kind of a value, as a model declares it (`MLFeatureType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Int,
    Double,
    Text,
    Tensor,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    Int(i64),
    Double(f64),
    Text(String),
    Tensor(Tensor),
}

impl FeatureValue {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Int(_) => Kind::Int,
            Self::Double(_) => Kind::Double,
            Self::Text(_) => Kind::Text,
            Self::Tensor(_) => Kind::Tensor,
        }
    }

    /// Accessors are exact: an integer is not read as a double, so a model's declared kind is what counts.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_double(&self) -> Option<f64> {
        match self {
            Self::Double(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_tensor(&self) -> Option<&Tensor> {
        match self {
            Self::Tensor(t) => Some(t),
            _ => None,
        }
    }
}

/// One input or output that a model declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureSpec {
    pub name: String,
    pub kind: Kind,
}

/// A set of named values (`MLFeatureProvider`).
pub trait FeatureProvider {
    fn value(&self, name: &str) -> Option<&FeatureValue>;
    /// The names in sorted order.
    fn names(&self) -> Vec<&str>;
}

/// A provider backed by a dictionary (`MLDictionaryFeatureProvider`). Setting a name again replaces its value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DictionaryProvider {
    values: BTreeMap<String, FeatureValue>,
}

impl DictionaryProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: &str, value: FeatureValue) -> Self {
        self.values.insert(name.to_owned(), value);
        self
    }
}

impl FeatureProvider for DictionaryProvider {
    fn value(&self, name: &str) -> Option<&FeatureValue> {
        self.values.get(name)
    }

    fn names(&self) -> Vec<&str> {
        self.values.keys().map(String::as_str).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureError {
    /// The provider has no value for a declared name.
    Missing(String),
    /// The provider holds a value of another kind than the one declared.
    WrongKind { name: String, expected: Kind, found: Kind },
}

/// Checks that the provider holds every declared feature with its declared kind. Extra names are ignored, as
/// a model ignores inputs it does not declare.
pub fn check(provider: &dyn FeatureProvider, specs: &[FeatureSpec]) -> Result<(), FeatureError> {
    for spec in specs {
        let value = provider
            .value(&spec.name)
            .ok_or_else(|| FeatureError::Missing(spec.name.clone()))?;
        if value.kind() != spec.kind {
            return Err(FeatureError::WrongKind {
                name: spec.name.clone(),
                expected: spec.kind,
                found: value.kind(),
            });
        }
    }
    Ok(())
}

/// A group of providers run together (`MLBatchProvider`).
pub trait BatchProvider {
    fn count(&self) -> usize;
    fn provider(&self, index: usize) -> Option<&dyn FeatureProvider>;
}

/// A batch held in a list (`MLArrayBatchProvider`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArrayBatch {
    providers: Vec<DictionaryProvider>,
}

impl ArrayBatch {
    pub fn new(providers: Vec<DictionaryProvider>) -> Self {
        Self { providers }
    }
}

impl BatchProvider for ArrayBatch {
    fn count(&self) -> usize {
        self.providers.len()
    }

    fn provider(&self, index: usize) -> Option<&dyn FeatureProvider> {
        self.providers.get(index).map(|p| p as &dyn FeatureProvider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, kind: Kind) -> FeatureSpec {
        FeatureSpec { name: name.into(), kind }
    }

    #[test]
    fn each_value_reports_its_kind_and_reads_back_only_as_that_kind() {
        let n = FeatureValue::Int(4);
        assert_eq!(n.kind(), Kind::Int);
        assert_eq!(n.as_int(), Some(4));
        assert_eq!(n.as_double(), None, "an integer is not silently read as a double");
        let t = FeatureValue::Text("cat".into());
        assert_eq!((t.kind(), t.as_text()), (Kind::Text, Some("cat")));
        let m = FeatureValue::Tensor(Tensor::zeros(&[2, 2]));
        assert_eq!(m.as_tensor().map(Tensor::rank), Some(2));
    }

    #[test]
    fn a_dictionary_provider_looks_up_names_and_lists_them_sorted() {
        let p = DictionaryProvider::new()
            .with("score", FeatureValue::Double(0.5))
            .with("label", FeatureValue::Text("dog".into()))
            .with("score", FeatureValue::Double(0.75));
        assert_eq!(p.names(), vec!["label", "score"]);
        assert_eq!(p.value("score").and_then(FeatureValue::as_double), Some(0.75), "the last set wins");
        assert!(p.value("missing").is_none());
    }

    #[test]
    fn check_accepts_declared_kinds_and_ignores_extra_names() {
        let p = DictionaryProvider::new()
            .with("width", FeatureValue::Int(28))
            .with("image", FeatureValue::Tensor(Tensor::zeros(&[28, 28])))
            .with("unused", FeatureValue::Text("x".into()));
        assert_eq!(check(&p, &[spec("width", Kind::Int), spec("image", Kind::Tensor)]), Ok(()));
    }

    #[test]
    fn check_reports_a_missing_input() {
        let p = DictionaryProvider::new().with("width", FeatureValue::Int(28));
        assert_eq!(
            check(&p, &[spec("width", Kind::Int), spec("height", Kind::Int)]),
            Err(FeatureError::Missing("height".into()))
        );
    }

    #[test]
    fn check_reports_a_value_of_the_wrong_kind() {
        let p = DictionaryProvider::new().with("width", FeatureValue::Text("28".into()));
        assert_eq!(
            check(&p, &[spec("width", Kind::Int)]),
            Err(FeatureError::WrongKind { name: "width".into(), expected: Kind::Int, found: Kind::Text })
        );
    }

    #[test]
    fn a_batch_counts_its_providers_and_refuses_an_index_past_the_end() {
        let batch = ArrayBatch::new(vec![
            DictionaryProvider::new().with("n", FeatureValue::Int(1)),
            DictionaryProvider::new().with("n", FeatureValue::Int(2)),
        ]);
        assert_eq!(batch.count(), 2);
        let second = batch.provider(1).and_then(|p| p.value("n")).and_then(FeatureValue::as_int);
        assert_eq!(second, Some(2));
        assert!(batch.provider(2).is_none());
    }
}
