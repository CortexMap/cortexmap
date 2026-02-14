use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldQuery {
    /// The field name to search in
    pub name: String,

    /// The value to search for
    pub value: String,

    /// Optional boost factor for this field
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost: Option<f32>,
}

/// NOT query wrapper - needed for proper YAML serialization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotQuery {
    /// The query to negate
    pub query: Box<BooleanQuery>,
}

/// Boost query for relevance scoring
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoostQuery {
    /// The query to boost
    pub query: Box<BooleanQuery>,

    /// The boost factor (multiplier for relevance score)
    pub factor: f32,
}

/// Range query for numeric or date ranges
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

    /// Create a NOT query
    pub fn not(query: BooleanQuery) -> Self {
        BooleanQuery::Not(NotQuery {
            query: Box::new(query),
        })
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
                // Escape special characters and quote if contains spaces
                if term.contains(' ') || term.contains('"') {
                    format!("\"{}\"", term.replace('"', "\\\""))
                } else {
                    term.clone()
                }
            }

            BooleanQuery::Phrase(phrase) => {
                // Phrases are always quoted
                format!("\"{}\"", phrase.replace('"', "\\\""))
            }

            BooleanQuery::Wildcard(pattern) => {
                // Wildcards are used as-is
                pattern.clone()
            }

            BooleanQuery::Field(field_query) => {
                let value = if field_query.value.contains(' ') {
                    format!("\"{}\"", field_query.value.replace('"', "\\\""))
                } else {
                    field_query.value.clone()
                };

                let base = format!("{}:{}", field_query.name, value);

                // Add boost if present
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
                // Build range expression
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

    pub fn to_string(&self) -> String {
        self.to_string_inner().replace(" ", "+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_term_query() {
        let config = Config::from_yaml(include_str!("../fixtures/term_query.yaml")).unwrap();
        assert_eq!(
            config.query.unwrap(),
            BooleanQuery::term("machine learning")
        );
    }

    #[test]
    fn test_phrase_query() {
        let config = Config::from_yaml(include_str!("../fixtures/phrase_query.yaml")).unwrap();
        assert_eq!(
            config.query.unwrap(),
            BooleanQuery::phrase("exact phrase match")
        );
    }

    #[test]
    fn test_wildcard_query() {
        let config = Config::from_yaml(include_str!("../fixtures/wildcard_query.yaml")).unwrap();
        assert_eq!(config.query.unwrap(), BooleanQuery::wildcard("rust*"));
    }

    #[test]
    fn test_and_query() {
        let config = Config::from_yaml(include_str!("../fixtures/and_query.yaml")).unwrap();
        assert!(matches!(config.query.unwrap(), BooleanQuery::And(ref q) if q.len() == 2));
    }

    #[test]
    fn test_or_query() {
        let config = Config::from_yaml(include_str!("../fixtures/or_query.yaml")).unwrap();
        assert!(matches!(config.query.unwrap(), BooleanQuery::Or(ref q) if q.len() == 2));
    }

    #[test]
    fn test_not_query() {
        let config = Config::from_yaml(include_str!("../fixtures/not_query.yaml")).unwrap();
        assert!(matches!(config.query.unwrap(), BooleanQuery::Not(_)));
    }

    #[test]
    fn test_field_query() {
        let config = Config::from_yaml(include_str!("../fixtures/field_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::Field(fq) => {
                assert_eq!(fq.name, "title");
                assert_eq!(fq.value, "Introduction");
            }
            _ => panic!("Expected Field query"),
        }
    }

    #[test]
    fn test_field_with_boost() {
        let config = Config::from_yaml(include_str!("../fixtures/field_boost_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::Field(fq) => {
                assert_eq!(fq.boost, Some(1.5));
            }
            _ => panic!("Expected Field query"),
        }
    }

    #[test]
    fn test_boost_query() {
        let config = Config::from_yaml(include_str!("../fixtures/boost_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::Boost(bq) => {
                assert_eq!(bq.factor, 2.0);
            }
            _ => panic!("Expected Boost query"),
        }
    }

    #[test]
    fn test_range_query() {
        let config = Config::from_yaml(include_str!("../fixtures/range_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::Range(rq) => {
                assert_eq!(rq.field, "date");
                assert_eq!(rq.gte, Some("2024-01-01".to_string()));
                assert_eq!(rq.lte, Some("2024-12-31".to_string()));
            }
            _ => panic!("Expected Range query"),
        }
    }

    #[test]
    fn test_complex_nested_query() {
        let config =
            Config::from_yaml(include_str!("../fixtures/complex_nested_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::And(queries) => {
                assert_eq!(queries.len(), 3);
                assert!(matches!(queries[1], BooleanQuery::Or(_)));
                assert!(matches!(queries[2], BooleanQuery::Not(_)));
            }
            _ => panic!("Expected AND query"),
        }
    }

    #[test]
    fn test_real_world_complex_query() {
        let config =
            Config::from_yaml(include_str!("../fixtures/real_world_complex_query.yaml")).unwrap();
        match config.query.unwrap() {
            BooleanQuery::And(queries) => {
                assert_eq!(queries.len(), 4);
                assert!(matches!(queries[0], BooleanQuery::Or(_)));
                assert!(matches!(queries[1], BooleanQuery::Field(_)));
                assert!(matches!(queries[2], BooleanQuery::Not(_)));
                assert!(matches!(queries[3], BooleanQuery::Boost(_)));
            }
            _ => panic!("Expected AND query"),
        }
    }

    #[test]
    fn test_builder_methods() {
        let query = BooleanQuery::and(vec![
            BooleanQuery::term("rust"),
            BooleanQuery::not(BooleanQuery::term("deprecated")),
        ]);
        assert!(matches!(query, BooleanQuery::And(ref q) if q.len() == 2));
    }

    #[test]
    fn test_to_query_string_term() {
        let query = BooleanQuery::term("rust");
        assert_eq!(query.to_string_inner(), "rust");

        let query = BooleanQuery::term("machine learning");
        assert_eq!(query.to_string_inner(), "\"machine learning\"");
    }

    #[test]
    fn test_to_query_string_phrase() {
        let query = BooleanQuery::phrase("exact phrase");
        assert_eq!(query.to_string_inner(), "\"exact phrase\"");
    }

    #[test]
    fn test_to_query_string_wildcard() {
        let query = BooleanQuery::wildcard("rust*");
        assert_eq!(query.to_string_inner(), "rust*");
    }

    #[test]
    fn test_to_query_string_field() {
        let query = BooleanQuery::field("title", "Introduction");
        assert_eq!(query.to_string_inner(), "title:Introduction");

        let query = BooleanQuery::Field(FieldQuery {
            name: "author".to_string(),
            value: "John Doe".to_string(),
            boost: Some(2.0),
        });
        assert_eq!(query.to_string_inner(), "author:\"John Doe\"^2");
    }

    #[test]
    fn test_to_query_string_and() {
        let query = BooleanQuery::and(vec![
            BooleanQuery::term("rust"),
            BooleanQuery::term("programming"),
        ]);
        assert_eq!(query.to_string_inner(), "(rust AND programming)");
    }

    #[test]
    fn test_to_query_string_or() {
        let query = BooleanQuery::or(vec![BooleanQuery::term("rust"), BooleanQuery::term("go")]);
        assert_eq!(query.to_string_inner(), "(rust OR go)");
    }

    #[test]
    fn test_to_query_string_not() {
        let query = BooleanQuery::not(BooleanQuery::term("deprecated"));
        assert_eq!(query.to_string_inner(), "NOT deprecated");
    }

    #[test]
    fn test_to_query_string_boost() {
        let query = BooleanQuery::boost(BooleanQuery::term("important"), 2.5);
        assert_eq!(query.to_string_inner(), "important^2.5");
    }

    #[test]
    fn test_to_query_string_range() {
        let query = BooleanQuery::Range(RangeQuery {
            field: "date".to_string(),
            gte: Some("2024-01-01".to_string()),
            lte: Some("2024-12-31".to_string()),
            gt: None,
            lt: None,
        });
        assert_eq!(query.to_string_inner(), "date:[2024-01-01 TO 2024-12-31]");
    }

    #[test]
    fn test_to_query_string_complex() {
        let query = BooleanQuery::and(vec![
            BooleanQuery::term("rust"),
            BooleanQuery::or(vec![
                BooleanQuery::term("async"),
                BooleanQuery::term("tokio"),
            ]),
            BooleanQuery::not(BooleanQuery::term("outdated")),
        ]);
        assert_eq!(
            query.to_string_inner(),
            "(rust AND (async OR tokio) AND NOT outdated)"
        );
    }

    #[test]
    fn test_neuroscience_query() {
        let config =
            Config::from_yaml(include_str!("../fixtures/neuroscience_query.yaml")).unwrap();

        // Verify structure
        match config.query.unwrap() {
            BooleanQuery::And(queries) => {
                assert_eq!(queries.len(), 3);
                assert!(matches!(queries[0], BooleanQuery::Or(_)));
                assert!(matches!(queries[1], BooleanQuery::Or(_)));
                assert!(matches!(queries[2], BooleanQuery::Or(_)));
            }
            _ => panic!("Expected AND query"),
        }
    }

    #[test]
    fn test_neuroscience_query_string() {
        let config =
            Config::from_yaml(include_str!("../fixtures/neuroscience_query.yaml")).unwrap();
        let query_string = config.query.unwrap().to_string();

        assert_eq!(
            query_string,
            "((\"motor+cortex\"+OR+M1)+AND+(fMRI+OR+optogenetics)+AND+(mouse+OR+human))"
        );
    }
}
