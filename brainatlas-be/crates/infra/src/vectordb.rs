use crate::error::InfraError;
use crate::models::*;
use crate::schema;
use diesel::prelude::*;
use diesel::sql_types::{Float8, Int4, Text};
use domain::{
    ChunkSource, ExistingSummary, NewEmbedding, NewRegionSummary,
    RetrievalScope, SimilarChunk,
};
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
        retrieval_scope: RetrievalScope,
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
            // Search across ALL chunks for this region, regardless of which
            // batch/summary they were ingested with.  The region_id filter is
            // sufficient to prevent cross-region contamination.  Restricting
            // retrieval to the current batch's summary_id caused the LLM to see
            // an empty context whenever the latest fetch produced bad papers
            // (e.g. malformed queries, off-target papers filtered out by the
            // region-mention filter), even though perfectly good chunks from a
            // prior batch exist for the same region.
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
            .bind::<Int4, _>(retrieval_scope.region_id)
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

// ============================================================================
// Infra-level integration tests (Task 4)
//
// These tests require a live Postgres+pgvector instance.  They are gated with
// `#[ignore]` so they do NOT run in ordinary `cargo test` / CI.  To run them
// locally set DATABASE_URL to a valid connection string and execute:
//
//   cargo test -p infra -- --ignored --test-thread=1
//
// Each test inserts rows in a transaction that is rolled back at the end, so
// no permanent data is written.
// ============================================================================
#[cfg(test)]
mod retrieval_scope_integration_tests {
    use super::*;
    use domain::{NewEmbedding, NewRegionSummary, RetrievalFallbackPolicy, RetrievalScope};
    use services::infra::VectorDatabase;
    use uuid::Uuid;

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set to run infra integration tests")
    }

    fn zero_embedding(len: usize) -> Vec<f32> {
        vec![0.0_f32; len]
    }

    fn make_embedding(region_id: i32, summary_id: Uuid, chunk_text: &str) -> NewEmbedding {
        NewEmbedding {
            region_id,
            summary_id,
            chunk_index: 0,
            chunk_text: chunk_text.to_string(),
            embedding: zero_embedding(1536),
            source_pmc_id: None,
            source_uid: None,
            source_s3_key: None,
            source_query: None,
            source_char_start: None,
            source_char_end: None,
        }
    }

    /// With `RetrievalFallbackPolicy::None` the query must return only chunks
    /// that belong to the exact `summary_id` in the scope — not chunks from
    /// any other summary for the same region.
    #[tokio::test]
    #[ignore = "requires live Postgres+pgvector; run with `cargo test -p infra -- --ignored`"]
    async fn search_similar_strict_scope_excludes_other_summaries() {
        let db = BrainAtlasVectorDB::new();
        let url = db_url();
        let region_id = 999_999;

        // Insert the target summary (the one we will scope to)
        let target_summary = NewRegionSummary {
            region_id,
            name: "Test Region".to_string(),
            acronym: Some("TR".to_string()),
            summary: String::new(),
            content_hash: "hash-target".to_string(),
            batch_id: Uuid::new_v4(),
        };
        let target_summary_id = db
            .insert_summary(&url, target_summary)
            .await
            .expect("insert target summary");

        // Insert a chunk for the target summary
        db.insert_embeddings(
            &url,
            vec![make_embedding(region_id, target_summary_id, "target chunk")],
        )
        .await
        .expect("insert target chunk");

        // Insert a second summary for the same region (it becomes active, target is now inactive)
        let other_summary = NewRegionSummary {
            region_id,
            name: "Test Region".to_string(),
            acronym: Some("TR".to_string()),
            summary: String::new(),
            content_hash: "hash-other".to_string(),
            batch_id: Uuid::new_v4(),
        };
        let other_summary_id = db
            .insert_summary(&url, other_summary)
            .await
            .expect("insert other summary");

        db.insert_embeddings(
            &url,
            vec![make_embedding(region_id, other_summary_id, "other chunk")],
        )
        .await
        .expect("insert other chunk");

        // Search strictly scoped to the target (now inactive) summary
        let scope = RetrievalScope::current_summary(region_id, target_summary_id);
        // scope has fallback_policy = None by default from current_summary()
        let results = db
            .search_similar(&url, zero_embedding(1536), scope, 10)
            .await
            .expect("search_similar");

        assert!(
            results.iter().all(|c| c.chunk_text == "target chunk"),
            "strict scope must not return chunks from other summaries; got {:?}",
            results.iter().map(|c| &c.chunk_text).collect::<Vec<_>>()
        );
    }

    /// With `RetrievalFallbackPolicy::ActiveSummary`, when the requested
    /// `summary_id` has no embeddings (e.g. a newly inserted summary before
    /// embedding ingestion completes), the query must retry against the
    /// currently active summary for the region and return its chunks.
    #[tokio::test]
    #[ignore = "requires live Postgres+pgvector; run with `cargo test -p infra -- --ignored`"]
    async fn search_similar_fallback_to_active_summary_when_empty() {
        let db = BrainAtlasVectorDB::new();
        let url = db_url();
        let region_id = 999_998;

        // Insert the active summary and embed it
        let active_summary = NewRegionSummary {
            region_id,
            name: "Fallback Region".to_string(),
            acronym: None,
            summary: String::new(),
            content_hash: "hash-active".to_string(),
            batch_id: Uuid::new_v4(),
        };
        let active_id = db
            .insert_summary(&url, active_summary)
            .await
            .expect("insert active summary");

        db.insert_embeddings(
            &url,
            vec![make_embedding(region_id, active_id, "active chunk")],
        )
        .await
        .expect("insert active chunk");

        // Synthesise a phantom summary_id that has no embeddings at all
        let phantom_id = Uuid::new_v4();

        let scope = RetrievalScope::current_summary(region_id, phantom_id)
            .with_fallback_policy(RetrievalFallbackPolicy::ActiveSummary);

        let results = db
            .search_similar(&url, zero_embedding(1536), scope, 10)
            .await
            .expect("search_similar with fallback");

        assert!(
            !results.is_empty(),
            "fallback must return chunks from the active summary when the requested summary has none"
        );
        assert!(
            results.iter().any(|c| c.chunk_text == "active chunk"),
            "fallback results must include chunks from the active summary"
        );
    }

    /// With `RetrievalFallbackPolicy::None`, when the requested `summary_id`
    /// has no embeddings, the result must be an empty vec — no fallback must
    /// occur.
    #[tokio::test]
    #[ignore = "requires live Postgres+pgvector; run with `cargo test -p infra -- --ignored`"]
    async fn search_similar_none_policy_returns_empty_when_no_chunks() {
        let db = BrainAtlasVectorDB::new();
        let url = db_url();
        let region_id = 999_997;

        // Insert an active summary with chunks so a fallback *could* return results
        let active_summary = NewRegionSummary {
            region_id,
            name: "Region None Policy".to_string(),
            acronym: None,
            summary: String::new(),
            content_hash: "hash-active-none".to_string(),
            batch_id: Uuid::new_v4(),
        };
        let active_id = db
            .insert_summary(&url, active_summary)
            .await
            .expect("insert active summary");

        db.insert_embeddings(
            &url,
            vec![make_embedding(region_id, active_id, "should not appear")],
        )
        .await
        .expect("insert chunk");

        // Use a phantom summary_id with None fallback policy
        let phantom_id = Uuid::new_v4();
        let scope = RetrievalScope::current_summary(region_id, phantom_id);
        // current_summary uses None fallback by default

        let results = db
            .search_similar(&url, zero_embedding(1536), scope, 10)
            .await
            .expect("search_similar no fallback");

        assert!(
            results.is_empty(),
            "None fallback policy must return empty vec when scope has no chunks; got {:?}",
            results.iter().map(|c| &c.chunk_text).collect::<Vec<_>>()
        );
    }

    /// Verifies that the fallback path is skipped when the active summary IS
    /// the same as the requested summary (i.e., `active_summary.id ==
    /// retrieval_scope.summary_id`).  In that case the initial search already
    /// ran against the correct summary, so there is nothing to retry — the
    /// result must remain empty rather than issuing a redundant second query.
    #[tokio::test]
    #[ignore = "requires live Postgres+pgvector; run with `cargo test -p infra -- --ignored`"]
    async fn search_similar_fallback_skipped_when_active_is_same_as_scope() {
        let db = BrainAtlasVectorDB::new();
        let url = db_url();
        let region_id = 999_996;

        // Insert an active summary but do NOT embed it
        let summary = NewRegionSummary {
            region_id,
            name: "Region Same Active".to_string(),
            acronym: None,
            summary: String::new(),
            content_hash: "hash-same-active".to_string(),
            batch_id: Uuid::new_v4(),
        };
        let summary_id = db
            .insert_summary(&url, summary)
            .await
            .expect("insert summary");

        // scope == active summary; embeddings table is empty for this summary
        let scope = RetrievalScope::current_summary(region_id, summary_id)
            .with_fallback_policy(RetrievalFallbackPolicy::ActiveSummary);

        let results = db
            .search_similar(&url, zero_embedding(1536), scope, 10)
            .await
            .expect("search_similar same active");

        assert!(
            results.is_empty(),
            "no redundant retry when active==scope and there are no embeddings; got {:?}",
            results.iter().map(|c| &c.chunk_text).collect::<Vec<_>>()
        );
    }
}
