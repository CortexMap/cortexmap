use domain::{BrainRegionEntry, RegionMapping};
use uuid::Uuid;

/// All queries the service layer can issue against Postgres.
pub enum Query {
    /// Fetch all rows from `region_mapping`, ordered by `structure_order`.
    ListRegions,
    /// Fetch a single region by UUID primary key.
    GetRegionById(Uuid),
    /// Check whether a row with the given UUID exists.
    RegionExists(Uuid),
}

/// Typed results returned by each query variant.
pub enum QueryResult {
    Regions(Vec<RegionMapping>),
    Region(Vec<BrainRegionEntry>),
    Exists(bool),
}

/// Postgres infra trait — accepts a typed query and executes it.
/// Connection management and DB row conversion are entirely internal to the implementation.
#[async_trait::async_trait]
pub trait Postgres: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn execute_query(
        &self,
        database_uri: &str,
        query: Query,
    ) -> Result<QueryResult, Self::Error>;
}

pub trait EnvInfra {
    type Error: std::error::Error + Send + Sync + 'static;
    fn get(&self, key: &str) -> Result<String, Self::Error>;
}

/// Blanket: any `T: Postgres` automatically satisfies `Infra`.
pub trait Infra:
    Postgres<Error = <Self as Infra>::Error> + EnvInfra<Error = <Self as Infra>::Error>
{
    type Error: std::error::Error + Send + Sync + 'static;
}

impl<E, T> Infra for T
where
    T: Postgres<Error = E> + EnvInfra<Error = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;
}
