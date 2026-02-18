use cortexmap_infra::{EnvInfra, InfraError};
use std::collections::HashMap;

pub struct FetcherEnvInfra {
    vars: HashMap<String, String>,
}

impl FetcherEnvInfra {
    pub fn new() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }
}

impl EnvInfra for FetcherEnvInfra {
    fn get_env_var(&self, key: &str) -> Result<String, InfraError> {
        self.vars
            .get(key)
            .cloned()
            .ok_or(InfraError::EnvVarNotFound(key.to_string()))
    }
}
