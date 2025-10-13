use serde::{Deserialize, Serialize};
use crate::config::BooleanQuery;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub query: Option<BooleanQuery>,
}


impl Config {
    /// Parse configuration from a YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}
