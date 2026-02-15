use crate::InfraError;
use services::EnvInfra;
use std::collections::HashMap;

pub struct BrainAtlasEnvInfra {
    vars: HashMap<String, String>,
}

impl BrainAtlasEnvInfra {
    pub fn new() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }
}

impl EnvInfra for BrainAtlasEnvInfra {
    type Error = InfraError;
    fn get(&self, key: &str) -> Result<String, Self::Error> {
        self.vars
            .get(key)
            .cloned()
            .ok_or(Self::Error::EnvVarNotFound(key.to_string()))
    }
}
