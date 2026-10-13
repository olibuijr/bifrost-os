//! Predicate validation (`NSPredicateValidating`): checks a predicate tree against a policy of allowed keys and
//! operators before anything evaluates it, so a search field or a saved filter cannot reach keys or comparisons the
//! app did not expose. The visitor walks compound predicates (`AND`, `OR`, `NOT`) and every comparison inside.

/// Comparison operators (`NSComparisonPredicate.Operator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    BeginsWith,
    EndsWith,
    Contains,
    Like,
    In,
}

/// A predicate tree: comparisons joined by compound operators.
#[derive(Clone, Debug, PartialEq)]
pub enum Predicate {
    Comparison { key: String, op: Operator, value: String },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

/// Why a predicate was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    DisallowedKey(String),
    DisallowedOperator { key: String, op: Operator },
    Empty,
}

/// Visits every comparison of a predicate (`NSPredicate` visitor).
pub trait PredicateVisitor {
    fn visit_comparison(&mut self, key: &str, op: Operator, value: &str) -> Result<(), ValidationError>;
}

impl Predicate {
    /// Walks the tree depth first, stopping at the first error.
    pub fn accept<V: PredicateVisitor>(&self, visitor: &mut V) -> Result<(), ValidationError> {
        match self {
            Predicate::Comparison { key, op, value } => visitor.visit_comparison(key, *op, value),
            Predicate::And(list) | Predicate::Or(list) => {
                if list.is_empty() {
                    return Err(ValidationError::Empty);
                }
                list.iter().try_for_each(|p| p.accept(visitor))
            }
            Predicate::Not(inner) => inner.accept(visitor),
        }
    }
}

/// Allowed keys and operators. An empty operator list for a key means every operator is allowed on it.
#[derive(Clone, Debug, Default)]
pub struct ValidationPolicy {
    keys: Vec<(String, Vec<Operator>)>,
}

impl ValidationPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allows `key` with any operator.
    pub fn allow_key(mut self, key: &str) -> Self {
        self.keys.push((key.to_string(), vec![]));
        self
    }

    /// Allows `key` with only these operators.
    pub fn allow(mut self, key: &str, ops: &[Operator]) -> Self {
        self.keys.push((key.to_string(), ops.to_vec()));
        self
    }
}

impl PredicateVisitor for ValidationPolicy {
    fn visit_comparison(&mut self, key: &str, op: Operator, _value: &str) -> Result<(), ValidationError> {
        let Some((_, ops)) = self.keys.iter().find(|(k, _)| k == key) else {
            return Err(ValidationError::DisallowedKey(key.to_string()));
        };
        if ops.is_empty() || ops.contains(&op) {
            Ok(())
        } else {
            Err(ValidationError::DisallowedOperator { key: key.to_string(), op })
        }
    }
}

/// `NSPredicateValidating`: a type that decides whether a predicate may run against it.
pub trait PredicateValidating {
    fn validate(&self, predicate: &Predicate) -> Result<(), ValidationError>;
}

impl PredicateValidating for ValidationPolicy {
    fn validate(&self, predicate: &Predicate) -> Result<(), ValidationError> {
        predicate.accept(&mut self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(key: &str, op: Operator) -> Predicate {
        Predicate::Comparison { key: key.into(), op, value: "x".into() }
    }

    #[test]
    fn allowed_comparison_passes() {
        let p = ValidationPolicy::new().allow("title", &[Operator::Contains]);
        assert_eq!(p.validate(&cmp("title", Operator::Contains)), Ok(()));
    }

    #[test]
    fn unknown_key_is_refused() {
        let p = ValidationPolicy::new().allow_key("title");
        assert_eq!(p.validate(&cmp("secret", Operator::Equal)), Err(ValidationError::DisallowedKey("secret".into())));
    }

    #[test]
    fn operator_outside_list_is_refused() {
        let p = ValidationPolicy::new().allow("title", &[Operator::Equal]);
        assert_eq!(
            p.validate(&cmp("title", Operator::Like)),
            Err(ValidationError::DisallowedOperator { key: "title".into(), op: Operator::Like })
        );
    }

    #[test]
    fn nested_compounds_are_walked() {
        let p = ValidationPolicy::new().allow_key("a");
        let ok = Predicate::And(vec![cmp("a", Operator::Equal), Predicate::Not(Box::new(cmp("a", Operator::In)))]);
        assert_eq!(p.validate(&ok), Ok(()));
        let bad = Predicate::Or(vec![cmp("a", Operator::Equal), Predicate::Not(Box::new(cmp("b", Operator::Equal)))]);
        assert_eq!(p.validate(&bad), Err(ValidationError::DisallowedKey("b".into())));
    }

    #[test]
    fn empty_compound_is_refused() {
        assert_eq!(ValidationPolicy::new().validate(&Predicate::And(vec![])), Err(ValidationError::Empty));
    }
}
