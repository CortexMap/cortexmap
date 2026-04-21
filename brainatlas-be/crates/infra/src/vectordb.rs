use crate::error::InfraError;
use crate::models::*;
use crate::schema;
use diesel::prelude::*;
use diesel::sql_types::{Float8, Int4, Text};
use domain::{ChunkSource, ExistingSummary, NewEmbedding, NewRegionSummary, SimilarChunk};
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
        // diesel's `.values(&[])` with no rows is not a valid INSERT and will
        // fail at the SQL layer. The knowledge-only summary path passes an
        // empty vec (no sources → no chunks to embed), so we short-circuit.
        if embeddings.is_empty() {
            return Ok(());
        }

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
                    source_pmc_id: e.source_pmc_id,
                    source_uid: e.source_uid,
                    source_s3_key: e.source_s3_key,
                    source_query: e.source_query,
                    source_char_start: e.source_char_start,
                    source_char_end: e.source_char_end,
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
            use diesel::Connection;
            use schema::region_summary;

            let row = NewRegionSummaryRow {
                region_id: summary.region_id,
                name: summary.name,
                acronym: summary.acronym,
                summary: summary.summary,
                content_hash: Some(summary.content_hash),
                batch_id: summary.batch_id,
            };

            // Atomically deactivate any prior active summaries for this region
            // and insert the new one. Without this, region_summary accumulates
            // one row per pipeline cycle (we observed 2,475 rows across 1,194
            // regions before the fix). The partial index
            // `idx_region_summary_active WHERE is_active = true` was always
            // designed for latest-only access; this enforces it.
            conn.transaction::<Uuid, diesel::result::Error, _>(|conn| {
                diesel::update(
                    region_summary::table
                        .filter(region_summary::region_id.eq(row.region_id))
                        .filter(region_summary::is_active.eq(true)),
                )
                .set(region_summary::is_active.eq(false))
                .execute(conn)?;

                diesel::insert_into(region_summary::table)
                    .values(&row)
                    .returning(region_summary::id)
                    .get_result::<Uuid>(conn)
            })
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

    async fn search_similar(
        &self,
        database_url: &str,
        query_embedding: Vec<f32>,
        region_id_param: i32,
        top_k: usize,
    ) -> Result<Vec<SimilarChunk>, Self::Error> {
        let embedding_str = format!(
            "[{}]",
            query_embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        #[derive(QueryableByName)]
        struct SimilarChunkRow {
            #[diesel(sql_type = diesel::sql_types::Uuid)]
            id: uuid::Uuid,
            #[diesel(sql_type = Int4)]
            chunk_index: i32,
            #[diesel(sql_type = Text)]
            chunk_text: String,
            #[diesel(sql_type = Float8)]
            similarity_score: f64,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Varchar>)]
            source_pmc_id: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Varchar>)]
            source_uid: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
            source_s3_key: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
            source_query: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Nullable<Int4>)]
            source_char_start: Option<i32>,
            #[diesel(sql_type = diesel::sql_types::Nullable<Int4>)]
            source_char_end: Option<i32>,
        }

        self.run_blocking(database_url, move |conn| {
            let rows = diesel::sql_query(
                "SELECT id, chunk_index, chunk_text, \
                 1.0 - (embedding <=> $1::vector) AS similarity_score, \
                 source_pmc_id, source_uid, source_s3_key, source_query, \
                 source_char_start, source_char_end \
                 FROM brain_region_embeddings \
                 WHERE region_id = $2 \
                 ORDER BY embedding <=> $1::vector \
                 LIMIT $3",
            )
            .bind::<Text, _>(&embedding_str)
            .bind::<Int4, _>(region_id_param)
            .bind::<diesel::sql_types::BigInt, _>(top_k as i64)
            .load::<SimilarChunkRow>(conn)?;

            Ok(rows
                .into_iter()
                .map(|r| SimilarChunk {
                    id: r.id,
                    chunk_index: r.chunk_index,
                    chunk_text: r.chunk_text,
                    similarity_score: r.similarity_score,
                    source_pmc_id: r.source_pmc_id,
                    source_uid: r.source_uid,
                    source_s3_key: r.source_s3_key,
                    source_query: r.source_query,
                    source_char_start: r.source_char_start,
                    source_char_end: r.source_char_end,
                })
                .collect())
        })
        .await
    }

    async fn update_summary_text(
        &self,
        database_url: &str,
        summary_id_param: Uuid,
        summary_text: &str,
    ) -> Result<(), Self::Error> {
        let text = summary_text.to_string();
        self.run_blocking(database_url, move |conn| {
            use schema::region_summary;

            diesel::update(region_summary::table.filter(region_summary::id.eq(summary_id_param)))
                .set(region_summary::summary.eq(Some(&text)))
                .execute(conn)?;

            Ok(())
        })
        .await
    }

    async fn get_chunk_source(
        &self,
        database_url: &str,
        chunk_id: Uuid,
    ) -> Result<Option<ChunkSource>, Self::Error> {
        self.run_blocking(database_url, move |conn| {
            use schema::brain_region_embeddings;

            let row = brain_region_embeddings::table
                .find(chunk_id)
                .first::<EmbeddingRow>(conn)
                .optional()?;

            Ok(row.map(|r| ChunkSource {
                chunk_id: r.id,
                chunk_text: r.chunk_text,
                source_s3_key: r.source_s3_key,
                source_pmc_id: r.source_pmc_id,
                source_uid: r.source_uid,
                source_query: r.source_query,
                char_start: r.source_char_start,
                char_end: r.source_char_end,
            }))
        })
        .await
    }
}
