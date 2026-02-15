use crate::InfraError;
use crate::env::BrainAtlasEnvInfra;
use crate::pg::BrainAtlasPostgresql;
use services::{EnvInfra, Postgres, Query, QueryResult};

pub struct BrainAtlasInfra {
    pg: BrainAtlasPostgresql,
    env: BrainAtlasEnvInfra,
}

impl BrainAtlasInfra {
    pub fn new() -> Self {
        let pg = BrainAtlasPostgresql::new();
        let env = BrainAtlasEnvInfra::new();
        Self { pg, env }
    }
}

impl EnvInfra for BrainAtlasInfra {
    type Error = InfraError;
    fn get(&self, key: &str) -> Result<String, Self::Error> {
        self.env.get(key)
    }
}

#[async_trait::async_trait]
impl Postgres for BrainAtlasInfra {
    type Error = InfraError;

    async fn execute_query(
        &self,
        database_uri: &str,
        query: Query,
    ) -> Result<QueryResult, Self::Error> {
        self.pg.execute_query(database_uri, query).await
    }
}
