//! Postgres adapter for the eval pipeline.
//!
//! Owns:
//!  - Cache lookups + writes against `eval_scores`.
//!  - Run-status upserts against `eval_runs`.
//!  - Read-only access to `region_summary` + `brain_region_embeddings`
//!    (those tables are owned by brainatlas-be; we only read them).

use crate::InfraError;
use crate::schema::{eval_runs, eval_scores, region_summary};
use chrono::Utc;
use deadpool_diesel::Runtime;
use deadpool_diesel::postgres::{BuildError, Manager, Pool};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Float4, Float8, Int4, Jsonb, Nullable, Text, Varchar};
use domain::{EvalRun, EvalRunStatus, EvalScore, NewEvalScore};
use services::{
    ChunkRow, EvalAggregate, LoadedRunState, MetricStatsRaw, RetrievedChunk, SummaryRow,
    WorstOffenderRow,
};
use tokio::sync::OnceCell;
use uuid::Uuid;

pub struct EvalsPostgresql {
    pool: OnceCell<Pool>,
}

impl EvalsPostgresql {
    pub fn new() -> Self {
        Self {
            pool: OnceCell::new(),
        }
    }

    async fn pool(&self, database_url: &str) -> Result<&Pool, BuildError> {
        self.pool
            .get_or_try_init(|| async {
                let manager = Manager::new(database_url, Runtime::Tokio1);
                Pool::builder(manager).max_size(10).build()
            })
            .await
    }
}

impl Default for EvalsPostgresql {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Diesel row models ----

#[derive(Queryable, Selectable, Clone, Debug)]
#[diesel(table_name = eval_scores)]
struct EvalScoreRow {
    id: Uuid,
    summary_id: Uuid,
    summary_hash: String,
    metric: String,
    score: f32,
    judge_model: Option<String>,
    details: Option<serde_json::Value>,
    eval_version: String,
    created_at: chrono::NaiveDateTime,
}

impl From<EvalScoreRow> for EvalScore {
    fn from(r: EvalScoreRow) -> Self {
        Self {
            id: r.id,
            summary_id: r.summary_id,
            summary_hash: r.summary_hash,
            metric: r.metric,
            score: r.score,
            judge_model: r.judge_model,
            details: r.details,
            eval_version: r.eval_version,
            created_at: r.created_at,
        }
    }
}

#[derive(Insertable)]
#[diesel(table_name = eval_scores)]
struct InsertEvalScore<'a> {
    summary_id: Uuid,
    summary_hash: &'a str,
    metric: &'a str,
    score: f32,
    judge_model: Option<&'a str>,
    details: Option<serde_json::Value>,
    eval_version: &'a str,
}

#[derive(Queryable, Selectable, Clone, Debug)]
#[diesel(table_name = eval_runs)]
struct EvalRunRow {
    id: Uuid,
    summary_id: Uuid,
    eval_version: String,
    status: String,
    error_message: Option<String>,
    started_at: Option<chrono::NaiveDateTime>,
    completed_at: Option<chrono::NaiveDateTime>,
    created_at: chrono::NaiveDateTime,
}

impl TryFrom<EvalRunRow> for EvalRun {
    type Error = InfraError;
    fn try_from(r: EvalRunRow) -> Result<Self, InfraError> {
        let status: EvalRunStatus = r
            .status
            .parse()
            .map_err(|_| InfraError::InvalidResponse(format!("bad eval_runs.status: {}", r.status)))?;
        Ok(Self {
            id: r.id,
            summary_id: r.summary_id,
            eval_version: r.eval_version,
            status,
            error_message: r.error_message,
            started_at: r.started_at,
            completed_at: r.completed_at,
            created_at: r.created_at,
        })
    }
}

#[derive(Queryable, Selectable, Clone, Debug)]
#[diesel(table_name = region_summary)]
struct SummarySelectRow {
    id: Uuid,
    region_id: i32,
    name: String,
    acronym: Option<String>,
    summary: Option<String>,
}

// ---- EvalsPostgresql impl ----

impl EvalsPostgresql {
    pub async fn lookup_score_by_hash(
        &self,
        database_url: &str,
        summary_hash: &str,
        metric: &str,
        eval_version: &str,
    ) -> Result<Option<EvalScore>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let h = summary_hash.to_string();
        let m = metric.to_string();
        let v = eval_version.to_string();
        let result = conn
            .interact(move |c| {
                eval_scores::table
                    .filter(eval_scores::summary_hash.eq(h))
                    .filter(eval_scores::metric.eq(m))
                    .filter(eval_scores::eval_version.eq(v))
                    .select(EvalScoreRow::as_select())
                    .first::<EvalScoreRow>(c)
                    .optional()
            })
            .await??;
        Ok(result.map(Into::into))
    }

    pub async fn insert_score(
        &self,
        database_url: &str,
        new: NewEvalScore,
    ) -> Result<EvalScore, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let new_clone = new.clone();
        let inserted = conn
            .interact(move |c| {
                let row = InsertEvalScore {
                    summary_id: new_clone.summary_id,
                    summary_hash: &new_clone.summary_hash,
                    metric: &new_clone.metric,
                    score: new_clone.score,
                    judge_model: new_clone.judge_model.as_deref(),
                    details: new_clone.details.clone(),
                    eval_version: &new_clone.eval_version,
                };
                diesel::insert_into(eval_scores::table)
                    .values(&row)
                    .on_conflict((
                        eval_scores::summary_hash,
                        eval_scores::metric,
                        eval_scores::eval_version,
                    ))
                    .do_nothing()
                    .returning(EvalScoreRow::as_returning())
                    .get_result::<EvalScoreRow>(c)
                    .optional()
            })
            .await??;

        if let Some(row) = inserted {
            return Ok(row.into());
        }

        // ON CONFLICT DO NOTHING returned no row → the cache row was inserted
        // by a concurrent writer. Re-select to resolve.
        let existing = self
            .lookup_score_by_hash(database_url, &new.summary_hash, &new.metric, &new.eval_version)
            .await?
            .ok_or(InfraError::NotFound)?;
        Ok(existing)
    }

    pub async fn get_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Option<SummaryRow>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let result = conn
            .interact(move |c| {
                region_summary::table
                    .find(summary_id)
                    .select(SummarySelectRow::as_select())
                    .first::<SummarySelectRow>(c)
                    .optional()
            })
            .await??;

        Ok(result.map(|r| SummaryRow {
            id: r.id,
            region_id: r.region_id,
            name: r.name,
            acronym: r.acronym,
            summary: r.summary.unwrap_or_default(),
        }))
    }

    pub async fn get_scores_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<EvalScore>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let rows = conn
            .interact(move |c| {
                eval_scores::table
                    .filter(eval_scores::summary_id.eq(summary_id))
                    .order(eval_scores::metric.asc())
                    .select(EvalScoreRow::as_select())
                    .load::<EvalScoreRow>(c)
            })
            .await??;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get_eval_aggregate(
        &self,
        database_url: &str,
        eval_version: &str,
    ) -> Result<EvalAggregate, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let ver = eval_version.to_string();

        let aggregate = conn
            .interact(move |c| -> Result<EvalAggregate, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct CountRow {
                    #[diesel(sql_type = BigInt)]
                    count: i64,
                }

                #[derive(QueryableByName)]
                struct PerMetricRow {
                    #[diesel(sql_type = Text)]
                    metric: String,
                    #[diesel(sql_type = Float8)]
                    avg: f64,
                    #[diesel(sql_type = Float4)]
                    min: f32,
                    #[diesel(sql_type = Float4)]
                    max: f32,
                    #[diesel(sql_type = BigInt)]
                    count: i64,
                }

                let total_summaries: Vec<CountRow> = diesel::sql_query(
                    "SELECT COUNT(*) AS count FROM region_summary WHERE summary IS NOT NULL",
                )
                .load(c)?;

                let total_scored: Vec<CountRow> = diesel::sql_query(
                    "SELECT COUNT(DISTINCT summary_id) AS count FROM eval_scores WHERE eval_version = $1",
                )
                .bind::<Text, _>(&ver)
                .load(c)?;

                let per_metric_rows: Vec<PerMetricRow> = diesel::sql_query(
                    "SELECT metric::text,
                            AVG(score)::float8 AS avg,
                            MIN(score)::float4 AS min,
                            MAX(score)::float4 AS max,
                            COUNT(*)::bigint AS count
                     FROM eval_scores
                     WHERE eval_version = $1
                     GROUP BY metric
                     ORDER BY metric",
                )
                .bind::<Text, _>(&ver)
                .load(c)?;

                let mut per_metric = std::collections::HashMap::new();
                for r in per_metric_rows {
                    per_metric.insert(
                        r.metric,
                        MetricStatsRaw {
                            avg: r.avg as f32,
                            min: r.min,
                            max: r.max,
                            count: r.count,
                        },
                    );
                }

                Ok(EvalAggregate {
                    total_summaries: total_summaries.first().map(|r| r.count).unwrap_or(0),
                    total_scored: total_scored.first().map(|r| r.count).unwrap_or(0),
                    per_metric,
                })
            })
            .await??;

        Ok(aggregate)
    }

    pub async fn get_worst_offenders(
        &self,
        database_url: &str,
        metric: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<WorstOffenderRow>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let m = metric.to_string();
        let v = eval_version.to_string();

        let rows = conn
            .interact(move |c| -> Result<Vec<WorstOffenderRow>, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct Row {
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    summary_id: Uuid,
                    #[diesel(sql_type = Nullable<Varchar>)]
                    region_name: Option<String>,
                    #[diesel(sql_type = Text)]
                    metric: String,
                    #[diesel(sql_type = Float4)]
                    score: f32,
                    #[diesel(sql_type = Text)]
                    eval_version: String,
                }

                let rows: Vec<Row> = diesel::sql_query(
                    "SELECT es.summary_id,
                            rs.name AS region_name,
                            es.metric::text,
                            es.score,
                            es.eval_version::text
                     FROM eval_scores es
                     LEFT JOIN region_summary rs ON rs.id = es.summary_id
                     WHERE es.metric = $1 AND es.eval_version = $2
                     ORDER BY es.score ASC
                     LIMIT $3",
                )
                .bind::<Text, _>(&m)
                .bind::<Text, _>(&v)
                .bind::<BigInt, _>(limit)
                .load(c)?;

                Ok(rows
                    .into_iter()
                    .map(|r| WorstOffenderRow {
                        summary_id: r.summary_id,
                        region_name: r.region_name,
                        metric: r.metric,
                        score: r.score,
                        eval_version: r.eval_version,
                    })
                    .collect())
            })
            .await??;
        Ok(rows)
    }

    pub async fn upsert_run(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        status: EvalRunStatus,
        error_message: Option<String>,
    ) -> Result<EvalRun, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let ver = eval_version.to_string();
        let status_str: &'static str = status.into();
        let status_str = status_str.to_string();
        let now = Utc::now().naive_utc();

        let row = conn
            .interact(move |c| -> Result<EvalRunRow, diesel::result::Error> {
                use diesel::insert_into;

                let started_at = match status {
                    EvalRunStatus::Running => Some(now),
                    _ => None,
                };
                let completed_at = match status {
                    EvalRunStatus::Complete | EvalRunStatus::Failed => Some(now),
                    _ => None,
                };

                let inserted = insert_into(eval_runs::table)
                    .values((
                        eval_runs::summary_id.eq(summary_id),
                        eval_runs::eval_version.eq(&ver),
                        eval_runs::status.eq(&status_str),
                        eval_runs::error_message.eq(error_message.clone()),
                        eval_runs::started_at.eq(started_at),
                        eval_runs::completed_at.eq(completed_at),
                    ))
                    .on_conflict((eval_runs::summary_id, eval_runs::eval_version))
                    .do_update()
                    .set((
                        eval_runs::status.eq(&status_str),
                        eval_runs::error_message.eq(error_message.clone()),
                        eval_runs::started_at.eq(diesel::dsl::sql::<Nullable<diesel::sql_types::Timestamp>>(
                            "CASE WHEN excluded.started_at IS NOT NULL THEN excluded.started_at ELSE eval_runs.started_at END",
                        )),
                        eval_runs::completed_at.eq(completed_at),
                    ))
                    .returning(EvalRunRow::as_returning())
                    .get_result::<EvalRunRow>(c)?;

                Ok(inserted)
            })
            .await??;

        EvalRun::try_from(row)
    }

    pub async fn list_unscored_summary_ids(
        &self,
        database_url: &str,
        eval_version: &str,
        limit: i64,
    ) -> Result<Vec<Uuid>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let ver = eval_version.to_string();

        let rows = conn
            .interact(move |c| -> Result<Vec<Uuid>, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct Row {
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    id: Uuid,
                }

                let rows: Vec<Row> = diesel::sql_query(
                    "SELECT rs.id
                     FROM region_summary rs
                     LEFT JOIN eval_runs er
                            ON er.summary_id = rs.id AND er.eval_version = $1
                     WHERE rs.summary IS NOT NULL
                       AND (er.id IS NULL OR er.status = 'failed')
                     ORDER BY rs.created_at ASC NULLS FIRST
                     LIMIT $2",
                )
                .bind::<Text, _>(&ver)
                .bind::<BigInt, _>(limit)
                .load(c)?;

                Ok(rows.into_iter().map(|r| r.id).collect())
            })
            .await??;
        Ok(rows)
    }

    pub async fn retrieve_chunks_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
        embedding: &[f32],
        top_k: i64,
        min_similarity: f32,
    ) -> Result<Vec<RetrievedChunk>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let min_sim = min_similarity as f64;

        let rows = conn
            .interact(move |c| -> Result<Vec<RetrievedChunk>, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct ChunkRow {
                    #[diesel(sql_type = Int4)]
                    chunk_index: i32,
                    #[diesel(sql_type = Text)]
                    chunk_text: String,
                    #[diesel(sql_type = Float8)]
                    similarity: f64,
                }

                let rows: Vec<ChunkRow> = diesel::sql_query(
                    "SELECT chunk_index,
                            chunk_text,
                            (1.0 - (embedding <=> $1::vector))::float8 AS similarity
                     FROM brain_region_embeddings
                     WHERE summary_id = $2
                       AND (1.0 - (embedding <=> $1::vector)) >= $3
                     ORDER BY embedding <=> $1::vector
                     LIMIT $4",
                )
                .bind::<Text, _>(&embedding_str)
                .bind::<diesel::sql_types::Uuid, _>(summary_id)
                .bind::<Float8, _>(min_sim)
                .bind::<BigInt, _>(top_k)
                .load(c)?;

                Ok(rows
                    .into_iter()
                    .map(|r| RetrievedChunk {
                        chunk_index: r.chunk_index,
                        chunk_text: r.chunk_text,
                        similarity: r.similarity as f32,
                    })
                    .collect())
            })
            .await??;
        Ok(rows)
    }

    /// Batch-lookup `brain_region_embeddings` rows by PK. Empty input
    /// short-circuits without a DB round-trip.
    pub async fn load_chunks_by_ids(
        &self,
        database_url: &str,
        chunk_ids: &[Uuid],
    ) -> Result<Vec<ChunkRow>, InfraError> {
        if chunk_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.pool(database_url).await?.get().await?;
        let ids: Vec<Uuid> = chunk_ids.to_vec();
        let rows = conn
            .interact(move |c| -> Result<Vec<ChunkRow>, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct Row {
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    id: Uuid,
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    summary_id: Uuid,
                    #[diesel(sql_type = Int4)]
                    chunk_index: i32,
                    #[diesel(sql_type = Text)]
                    chunk_text: String,
                }

                let rows: Vec<Row> = diesel::sql_query(
                    "SELECT id, summary_id, chunk_index, chunk_text
                     FROM brain_region_embeddings
                     WHERE id = ANY($1)",
                )
                .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(ids)
                .load(c)?;

                Ok(rows
                    .into_iter()
                    .map(|r| ChunkRow {
                        id: r.id,
                        summary_id: r.summary_id,
                        chunk_index: r.chunk_index,
                        chunk_text: r.chunk_text,
                    })
                    .collect())
            })
            .await??;
        Ok(rows)
    }

    // ---- eval_run_state ----

    pub async fn insert_run_state(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<Uuid, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let ver = eval_version.to_string();
        let state = state.clone();
        let endpoint = pending_endpoint.map(|s| s.to_string());

        let run_id = conn
            .interact(move |c| -> Result<Uuid, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct R {
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    run_id: Uuid,
                }
                let rows: Vec<R> = diesel::sql_query(
                    "INSERT INTO eval_run_state
                        (summary_id, eval_version, state, pending_step_id, pending_endpoint)
                     VALUES ($1, $2, $3, $4, $5)
                     RETURNING run_id",
                )
                .bind::<diesel::sql_types::Uuid, _>(summary_id)
                .bind::<Text, _>(&ver)
                .bind::<Jsonb, _>(&state)
                .bind::<Nullable<diesel::sql_types::Uuid>, _>(pending_step_id)
                .bind::<Nullable<Text>, _>(endpoint.as_deref())
                .load(c)?;
                rows.into_iter()
                    .next()
                    .map(|r| r.run_id)
                    .ok_or(diesel::result::Error::NotFound)
            })
            .await??;

        Ok(run_id)
    }

    pub async fn load_run_state(
        &self,
        database_url: &str,
        run_id: Uuid,
    ) -> Result<Option<LoadedRunState>, InfraError> {
        let conn = self.pool(database_url).await?.get().await?;

        let row = conn
            .interact(move |c| -> Result<Option<LoadedRunState>, diesel::result::Error> {
                #[derive(QueryableByName)]
                struct R {
                    #[diesel(sql_type = diesel::sql_types::Uuid)]
                    summary_id: Uuid,
                    #[diesel(sql_type = Text)]
                    eval_version: String,
                    #[diesel(sql_type = Jsonb)]
                    state: serde_json::Value,
                    #[diesel(sql_type = Nullable<diesel::sql_types::Uuid>)]
                    pending_step_id: Option<Uuid>,
                }
                let rows: Vec<R> = diesel::sql_query(
                    "SELECT summary_id, eval_version, state, pending_step_id
                     FROM eval_run_state WHERE run_id = $1",
                )
                .bind::<diesel::sql_types::Uuid, _>(run_id)
                .load(c)?;
                Ok(rows
                    .into_iter()
                    .next()
                    .map(|r| (r.summary_id, r.eval_version, r.state, r.pending_step_id)))
            })
            .await??;

        Ok(row)
    }

    pub async fn save_run_state(
        &self,
        database_url: &str,
        run_id: Uuid,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        pending_endpoint: Option<&str>,
    ) -> Result<(), InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let state = state.clone();
        let endpoint = pending_endpoint.map(|s| s.to_string());

        conn.interact(move |c| -> Result<usize, diesel::result::Error> {
            diesel::sql_query(
                "UPDATE eval_run_state
                 SET state = $2,
                     pending_step_id = $3,
                     pending_endpoint = $4,
                     updated_at = now()
                 WHERE run_id = $1",
            )
            .bind::<diesel::sql_types::Uuid, _>(run_id)
            .bind::<Jsonb, _>(&state)
            .bind::<Nullable<diesel::sql_types::Uuid>, _>(pending_step_id)
            .bind::<Nullable<Text>, _>(endpoint.as_deref())
            .execute(c)
        })
        .await??;

        Ok(())
    }

    pub async fn delete_run_state(
        &self,
        database_url: &str,
        run_id: Uuid,
    ) -> Result<(), InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        conn.interact(move |c| -> Result<usize, diesel::result::Error> {
            diesel::sql_query("DELETE FROM eval_run_state WHERE run_id = $1")
                .bind::<diesel::sql_types::Uuid, _>(run_id)
                .execute(c)
        })
        .await??;
        Ok(())
    }

    pub async fn delete_run_states_for_summary(
        &self,
        database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
    ) -> Result<(), InfraError> {
        let conn = self.pool(database_url).await?.get().await?;
        let ver = eval_version.to_string();
        conn.interact(move |c| -> Result<usize, diesel::result::Error> {
            diesel::sql_query(
                "DELETE FROM eval_run_state WHERE summary_id = $1 AND eval_version = $2",
            )
            .bind::<diesel::sql_types::Uuid, _>(summary_id)
            .bind::<Text, _>(&ver)
            .execute(c)
        })
        .await??;
        Ok(())
    }
}
