use crate::models::{RegionMappingRow, RegionSummaryRow};
use crate::schema::region_mapping::dsl;
use crate::schema::region_summary::dsl as summary_dsl;
use crate::InfraError;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use deadpool_diesel::Runtime;
use diesel::prelude::*;
use services::{Postgres, Query, QueryResult};
use tokio::sync::OnceCell;

pub struct BrainAtlasPostgresql {
    pool: OnceCell<Pool>,
}

impl BrainAtlasPostgresql {
    pub fn new() -> Self {
        Self {
            pool: OnceCell::new(),
        }
    }

    /// Returns the cached pool, initialising it from `database_uri` on the first call.
    async fn pool(&self, database_uri: &str) -> Result<&Pool, BuildError> {
        self.pool
            .get_or_try_init(|| async {
                let manager = Manager::new(database_uri, Runtime::Tokio1);
                Pool::builder(manager).max_size(10).build()
            })
            .await
    }
}

#[async_trait::async_trait]
impl Postgres for BrainAtlasPostgresql {
    type Error = InfraError;

    async fn execute_query(&self, database_uri: &str, query: Query) -> Result<QueryResult, Self::Error> {
        let conn = self.pool(database_uri).await?.get().await?;

        match query {
            Query::ListRegions => {
                let rows = conn
                    .interact(|c| {
                        dsl::region_mapping
                            .order(dsl::structure_order.asc().nulls_last())
                            .load::<RegionMappingRow>(c)
                    })
                    .await??;
                Ok(QueryResult::Regions(
                    rows.into_iter().map(Into::into).collect(),
                ))
            }

            Query::GetRegionById(id) => {
                // `id` is the UUID from `region_mapping`. We look up the integer
                // `region_id` from that row, then query `region_summary` by that integer.
                let region_id: i32 = conn
                    .interact(move |c| {
                        dsl::region_mapping
                            .filter(dsl::id.eq(id))
                            .select(dsl::region_id)
                            .first::<i32>(c)
                    })
                    .await??;

                let rows = conn
                    .interact(move |c| {
                        summary_dsl::region_summary
                            .filter(summary_dsl::region_id.eq(region_id))
                            .order(summary_dsl::created_at.desc())
                            .load::<RegionSummaryRow>(c)
                    })
                    .await??;
                Ok(QueryResult::Region(rows.into_iter().map(Into::into).collect()))
            }

            Query::RegionExists(id) => {
                let exists = conn
                    .interact(move |c| {
                        diesel::select(diesel::dsl::exists(
                            dsl::region_mapping.filter(dsl::id.eq(id)),
                        ))
                        .get_result::<bool>(c)
                    })
                    .await??;
                Ok(QueryResult::Exists(exists))
            }
        }
    }
}
