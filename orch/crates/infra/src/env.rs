use crate::InfraError;
use services::EnvInfra;
use std::collections::HashMap;

pub struct OrchEnvInfra {
    vars: HashMap<String, String>,
}

impl OrchEnvInfra {
    pub fn new() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }
}

impl EnvInfra for OrchEnvInfra {
    type Error = InfraError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        self.vars
            .get(key)
            .cloned()
            .ok_or(Self::Error::EnvVarNotFound(key.to_string()))
    }
}
