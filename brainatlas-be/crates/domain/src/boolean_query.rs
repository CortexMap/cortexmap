use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BooleanQuery {
    /// A simple term query - searches for a single term
    Term(String),

    /// A phrase query - searches for an exact phrase
    Phrase(String),

    /// A wildcard query - supports * and ? wildcards
    Wildcard(String),

    /// Field-specific query
    Field(FieldQuery),

    /// AND operation - all sub-queries must match
    And(Vec<BooleanQuery>),

    /// OR operation - at least one sub-query must match
    Or(Vec<BooleanQuery>),

    /// NOT operation - negates the sub-query
    Not(NotQuery),

    /// Boost operation - increases relevance score
    Boost(BoostQuery),

    /// Range query - matches values within a range
    Range(RangeQuery),
}

/// Field-specific query for searching in particular fields
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct FieldQuery {
    /// The field name to search in
    pub name: String,

    /// The value to search for
    pub value: String,

    /// Optional boost factor for this field
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost: Option<f32>,
}

/// NOT query wrapper - needed for proper serialization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct NotQuery {
    /// The query to negate
    pub query: Box<BooleanQuery>,
}

/// Boost query for relevance scoring
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct BoostQuery {
    /// The query to boost
    pub query: Box<BooleanQuery>,

    /// The boost factor (multiplier for relevance score)
    pub factor: f32,
}

/// Range query for numeric or date ranges
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct RangeQuery {
    /// The field to apply the range to
    pub field: String,

    /// Lower bound (inclusive if specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gte: Option<String>,

    /// Lower bound (exclusive if specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt: Option<String>,

    /// Upper bound (inclusive if specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lte: Option<String>,

    /// Upper bound (exclusive if specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lt: Option<String>,
}

impl BooleanQuery {
    /// Create a simple term query
    pub fn term(term: impl Into<String>) -> Self {
        BooleanQuery::Term(term.into())
    }

    /// Create a phrase query
    pub fn phrase(phrase: impl Into<String>) -> Self {
        BooleanQuery::Phrase(phrase.into())
    }

    /// Create a wildcard query
    pub fn wildcard(pattern: impl Into<String>) -> Self {
        BooleanQuery::Wildcard(pattern.into())
    }

    /// Create an AND query
    pub fn and(queries: Vec<BooleanQuery>) -> Self {
        BooleanQuery::And(queries)
    }

    /// Create an OR query
    pub fn or(queries: Vec<BooleanQuery>) -> Self {
        BooleanQuery::Or(queries)
    }

    /// Create a field query
    pub fn field(name: impl Into<String>, value: impl Into<String>) -> Self {
        BooleanQuery::Field(FieldQuery {
            name: name.into(),
            value: value.into(),
            boost: None,
        })
    }

    /// Create a boost query
    pub fn boost(query: BooleanQuery, factor: f32) -> Self {
        BooleanQuery::Boost(BoostQuery {
            query: Box::new(query),
            factor,
        })
    }

    fn to_string_inner(&self) -> String {
        match self {
            BooleanQuery::Term(term) => {
                if term.contains(' ') || term.contains('"') {
                    format!("\"{}\"", term.replace('"', "\\\""))
                } else {
                    term.clone()
                }
            }

            BooleanQuery::Phrase(phrase) => {
                format!("\"{}\"", phrase.replace('"', "\\\""))
            }

            BooleanQuery::Wildcard(pattern) => pattern.clone(),

            BooleanQuery::Field(field_query) => {
                let value = if field_query.value.contains(' ') {
                    format!("\"{}\"", field_query.value.replace('"', "\\\""))
                } else {
                    field_query.value.clone()
                };

                let base = format!("{}:{}", field_query.name, value);

                if let Some(boost) = field_query.boost {
                    format!("{}^{}", base, boost)
                } else {
                    base
                }
            }

            BooleanQuery::And(queries) => {
                if queries.is_empty() {
                    return String::new();
                }

                let query_strings: Vec<String> =
                    queries.iter().map(|q| q.to_string_inner()).collect();

                if queries.len() == 1 {
                    query_strings[0].clone()
                } else {
                    format!("({})", query_strings.join(" AND "))
                }
            }

            BooleanQuery::Or(queries) => {
                if queries.is_empty() {
                    return String::new();
                }

                let query_strings: Vec<String> =
                    queries.iter().map(|q| q.to_string_inner()).collect();

                if queries.len() == 1 {
                    query_strings[0].clone()
                } else {
                    format!("({})", query_strings.join(" OR "))
                }
            }

            BooleanQuery::Not(not_query) => {
                format!("NOT {}", not_query.query.to_string_inner())
            }

            BooleanQuery::Boost(boost_query) => {
                format!(
                    "{}^{}",
                    boost_query.query.to_string_inner(),
                    boost_query.factor
                )
            }

            BooleanQuery::Range(range_query) => {
                let lower = if let Some(ref gte) = range_query.gte {
                    format!("[{}", gte)
                } else if let Some(ref gt) = range_query.gt {
                    format!("{{{}", gt)
                } else {
                    "[*".to_string()
                };

                let upper = if let Some(ref lte) = range_query.lte {
                    format!("{}]", lte)
                } else if let Some(ref lt) = range_query.lt {
                    format!("{}}}", lt)
                } else {
                    "*]".to_string()
                };

                format!("{}:{} TO {}", range_query.field, lower, upper)
            }
        }
    }

    pub fn to_query_string(&self) -> String {
        self.to_string_inner().replace(" ", "+")
    }
}

impl std::ops::Not for BooleanQuery {
    type Output = BooleanQuery;

    fn not(self) -> Self::Output {
        BooleanQuery::Not(NotQuery {
            query: Box::new(self),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_escapes_spaces_and_quotes() {
        let query = BooleanQuery::term("motor \"cortex\"");
        assert_eq!(query.to_string_inner(), "\"motor \\\"cortex\\\"\"");
    }

    #[test]
    fn test_field_query_escapes_quotes_and_applies_boost() {
        let query = BooleanQuery::Field(FieldQuery {
            name: "title".to_string(),
            value: "Layer \"2/3\" neurons".to_string(),
            boost: Some(1.5),
        });

        assert_eq!(
            query.to_string_inner(),
            "title:\"Layer \\\"2/3\\\" neurons\"^1.5"
        );
    }

    #[test]
    fn test_single_and_or_queries_do_not_add_parentheses() {
        assert_eq!(
            BooleanQuery::and(vec![BooleanQuery::term("rust")]).to_string_inner(),
            "rust"
        );
        assert_eq!(
            BooleanQuery::or(vec![BooleanQuery::term("axum")]).to_string_inner(),
            "axum"
        );
    }

    #[test]
    fn test_empty_and_or_queries_return_empty_strings() {
        assert_eq!(BooleanQuery::and(vec![]).to_string_inner(), "");
        assert_eq!(BooleanQuery::or(vec![]).to_string_inner(), "");
    }

    #[test]
    fn test_range_query_supports_exclusive_bounds() {
        let query = BooleanQuery::Range(RangeQuery {
            field: "year".to_string(),
            gte: None,
            gt: Some("2020".to_string()),
            lte: None,
            lt: Some("2024".to_string()),
        });

        assert_eq!(query.to_string_inner(), "year:{2020 TO 2024}");
    }

    #[test]
    fn test_to_query_string_replaces_spaces_with_pluses() {
        let query = BooleanQuery::and(vec![
            BooleanQuery::term("motor cortex"),
            !BooleanQuery::field("species", "mouse"),
        ]);

        assert_eq!(
            query.to_query_string(),
            "(\"motor+cortex\"+AND+NOT+species:mouse)"
        );
    }

    // ---------- Gap-fill tests (push coverage ≥ 90%) ----------

    #[test]
    fn test_phrase_escapes_embedded_quotes() {
        let q = BooleanQuery::phrase("a \"b\" c");
        assert_eq!(q.to_string_inner(), "\"a \\\"b\\\" c\"");
    }

    #[test]
    fn test_wildcard_is_passed_through_unchanged() {
        let q = BooleanQuery::wildcard("moto*");
        assert_eq!(q.to_string_inner(), "moto*");
    }

    #[test]
    fn test_field_without_boost_or_spaces() {
        let q = BooleanQuery::field("species", "mouse");
        assert_eq!(q.to_string_inner(), "species:mouse");
    }

    #[test]
    fn test_multi_term_and_wraps_in_parens() {
        let q = BooleanQuery::and(vec![
            BooleanQuery::term("cortex"),
            BooleanQuery::term("neuron"),
        ]);
        assert_eq!(q.to_string_inner(), "(cortex AND neuron)");
    }

    #[test]
    fn test_multi_term_or_wraps_in_parens() {
        let q = BooleanQuery::or(vec![
            BooleanQuery::term("cortex"),
            BooleanQuery::term("neuron"),
        ]);
        assert_eq!(q.to_string_inner(), "(cortex OR neuron)");
    }

    #[test]
    fn test_boost_query_builder_serializes_with_factor() {
        let q = BooleanQuery::boost(BooleanQuery::term("memory"), 2.5);
        assert_eq!(q.to_string_inner(), "memory^2.5");
    }

    #[test]
    fn test_range_inclusive_and_mixed_open_bounds() {
        // Fully inclusive [a TO b]
        let q = BooleanQuery::Range(RangeQuery {
            field: "year".to_string(),
            gte: Some("2000".to_string()),
            gt: None,
            lte: Some("2020".to_string()),
            lt: None,
        });
        assert_eq!(q.to_string_inner(), "year:[2000 TO 2020]");

        // Open below, inclusive above: [* TO lte]
        let q_open_low = BooleanQuery::Range(RangeQuery {
            field: "year".to_string(),
            gte: None,
            gt: None,
            lte: Some("2020".to_string()),
            lt: None,
        });
        assert_eq!(q_open_low.to_string_inner(), "year:[* TO 2020]");

        // Inclusive below, open above: [gte TO *]
        let q_open_high = BooleanQuery::Range(RangeQuery {
            field: "year".to_string(),
            gte: Some("2000".to_string()),
            gt: None,
            lte: None,
            lt: None,
        });
        assert_eq!(q_open_high.to_string_inner(), "year:[2000 TO *]");

        // Fully open: [* TO *]
        let q_full_open = BooleanQuery::Range(RangeQuery {
            field: "year".to_string(),
            gte: None,
            gt: None,
            lte: None,
            lt: None,
        });
        assert_eq!(q_full_open.to_string_inner(), "year:[* TO *]");
    }

    #[test]
    fn test_not_operator_wraps_inner_query() {
        let q = !BooleanQuery::term("cat");
        assert_eq!(q.to_string_inner(), "NOT cat");
    }

    #[test]
    fn test_roundtrip_serde_and_fields() {
        // Exercise the `#[serde(skip_serializing_if = "Option::is_none")]`
        // branches for FieldQuery.boost and RangeQuery.
        let field = BooleanQuery::Field(FieldQuery {
            name: "title".to_string(),
            value: "x".to_string(),
            boost: None,
        });
        let s = serde_json::to_string(&field).unwrap();
        // `boost` is not serialized when None.
        assert!(!s.contains("boost"));

        let back: BooleanQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(back, field);
    }
}
