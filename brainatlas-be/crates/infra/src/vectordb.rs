use crate::error::InfraError;
use crate::models::*;
use crate::schema;
use diesel::prelude::*;
use domain::{ExistingSummary, NewEmbedding, NewRegionSummary};
use services::infra::VectorDatabase;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

pub struct BrainAtlasVectorDB {
    pool: Arc<Mutex<OnceLock<diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<PgConnection>>>>>,
}

impl BrainAtlasVectorDB {
    pub fn new() -> Self {
        Self {
            pool: Arc::new(Mutex::new(OnceLock::new())),
        }
    }

    async fn run_blocking<F, R>(&self, database_url: &str, f: F) -> Result<R, InfraError>
    where
        F: FnOnce(&mut PgConnection) -> Result<R, diesel::result::Error> + Send + 'static,
        R: Send + 'static,
    {
        let database_url = database_url.to_string();
        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let pool = pool.lock().unwrap();
            let pool = pool.get_or_init(|| {
                let manager = diesel::r2d2::ConnectionManager::<PgConnection>::new(&database_url);
                diesel::r2d2::Pool::builder()
                    .build(manager)
                    .expect("Failed to create pool")
            });

            let mut conn = pool.get().map_err(|e| {
                diesel::result::Error::DatabaseError(
                    diesel::result::DatabaseErrorKind::UnableToSendCommand,
                    Box::new(e.to_string()),
                )
            })?;

            f(&mut conn)
        })
        .await?
        .map_err(InfraError::from)
    }
}

#[async_trait::async_trait]
impl VectorDatabase for BrainAtlasVectorDB {
    type Error = InfraError;

    async fn insert_embeddings(
        &self,
        database_url: &str,
        embeddings: Vec<NewEmbedding>,
    ) -> Result<(), Self::Error> {
        self.run_blocking(database_url, move |conn| {
            use schema::brain_region_embeddings;

            // Convert domain NewEmbedding to DB NewEmbeddingRow
            let rows: Vec<NewEmbeddingRow> = embeddings
                .into_iter()
                .map(|e| NewEmbeddingRow {
                    region_id: e.region_id,
                    summary_id: e.summary_id,
                    chunk_index: e.chunk_index,
                    chunk_text: e.chunk_text,
                    embedding: pgvector::Vector::from(e.embedding),
                })
                .collect();

            diesel::insert_into(brain_region_embeddings::table)
                .values(&rows)
                .execute(conn)?;

            Ok(())
        })
        .await
    }

    async fn insert_summary(
        &self,
        database_url: &str,
        summary: NewRegionSummary,
    ) -> Result<Uuid, Self::Error> {
        self.run_blocking(database_url, move |conn| {
            use schema::region_summary;

            let row = NewRegionSummaryRow {
                region_id: summary.region_id,
                name: summary.name,
                acronym: summary.acronym,
                summary: summary.summary,
                content_hash: Some(summary.content_hash),
            };

            diesel::insert_into(region_summary::table)
                .values(&row)
                .returning(region_summary::id)
                .get_result::<Uuid>(conn)
        })
        .await
    }

    async fn check_content_hash(
        &self,
        database_url: &str,
        region_id_param: i32,
        hash: &str,
    ) -> Result<Option<ExistingSummary>, Self::Error> {
        let hash = hash.to_string();
        self.run_blocking(database_url, move |conn| {
            use schema::region_summary;

            region_summary::table
                .filter(region_summary::region_id.eq(region_id_param))
                .filter(region_summary::content_hash.eq(Some(&hash)))
                .select((region_summary::id, region_summary::summary))
                .first::<(Uuid, Option<String>)>(conn)
                .optional()
                .map(|opt| {
                    opt.and_then(|(summary_id, summary_text)| {
                        summary_text.map(|text| ExistingSummary {
                            summary_id,
                            summary: text,
                        })
                    })
                })
        })
        .await
    }
}
