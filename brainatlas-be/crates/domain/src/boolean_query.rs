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
