use crate::InfraError;
use services::EnvInfra;
use std::collections::HashMap;

pub struct EvalsEnvInfra {
    vars: HashMap<String, String>,
}

impl EvalsEnvInfra {
    pub fn new() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }
}

impl Default for EvalsEnvInfra {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvInfra for EvalsEnvInfra {
    type Error = InfraError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        self.vars
            .get(key)
            .cloned()
            .ok_or(Self::Error::EnvVarNotFound(key.to_string()))
    }
}
