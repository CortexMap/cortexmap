//! Step 5.5 — cache-hit integration test.
//!
//! Verifies the read-through `eval_scores` cache as a correctness contract:
//!
//! 1. `score_summary` invoked twice on the same `summary_id` returns identical
//!    score values for every metric.
//! 2. The second invocation reports `cached: true` for every metric.
//! 3. The mocked brainatlas client receives LLM calls only on the first
//!    invocation — the second pays zero LLM tokens.
//!
//! No DB or HTTP is involved: an in-memory `EvalsDatabase` and a counting
//! `BrainatlasClient` mock plug straight into `EvalsApp::score_summary`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use brainatlas_rpc_types::evals as brpc;
use chrono::NaiveDateTime;
use evals_app::{EvalRuntimeConfig, EvalsApp};
use domain::{
    Claim, ClaimsResponse, EvalRun, EvalRunStatus, EvalScore, GroundednessLabel,
    GroundednessVerdict, NewEvalScore, RubricCriterion, RubricScores,
};
use services::{
    BrainatlasClient, EnvInfra, EvalAggregate, EvalsDatabase, RetrievedChunk, SummaryRow,
    WorstOffenderRow,
};
use uuid::Uuid;

// ---- Mock infra error type ----

#[derive(Debug, thiserror::Error)]
#[error("mock infra error: {0}")]
struct MockError(String);

// ---- In-memory EvalsDatabase ----

#[derive(Default)]
struct InMemoryDb {
    /// Single fixture summary served by `get_summary`.
    summary: Mutex<Option<SummaryRow>>,
    /// All persisted score rows.
    scores: Mutex<Vec<EvalScore>>,
    /// All persisted run rows (latest wins).
    runs: Mutex<Vec<EvalRun>>,
}

impl InMemoryDb {
    fn new(summary: SummaryRow) -> Self {
        Self {
            summary: Mutex::new(Some(summary)),
            scores: Mutex::new(Vec::new()),
            runs: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl EvalsDatabase for InMemoryDb {
    type Error = MockError;

    async fn lookup_score_by_hash(
        &self,
        _database_url: &str,
        summary_hash: &str,
        metric: &str,
        eval_version: &str,
    ) -> Result<Option<EvalScore>, Self::Error> {
        let scores = self.scores.lock().unwrap();
        Ok(scores
            .iter()
            .find(|s| {
                s.summary_hash == summary_hash
                    && s.metric == metric
                    && s.eval_version == eval_version
            })
            .cloned())
    }

    async fn insert_score(
        &self,
        _database_url: &str,
        new: NewEvalScore,
    ) -> Result<EvalScore, Self::Error> {
        let mut scores = self.scores.lock().unwrap();
        // Honor `ON CONFLICT DO NOTHING`: if a row already exists for this
        // cache key, return the existing row (concurrent writer semantics).
        if let Some(existing) = scores.iter().find(|s| {
            s.summary_hash == new.summary_hash
                && s.metric == new.metric
                && s.eval_version == new.eval_version
        }) {
            return Ok(existing.clone());
        }
        let row = EvalScore {
            id: Uuid::new_v4(),
            summary_id: new.summary_id,
            summary_hash: new.summary_hash,
            metric: new.metric,
            score: new.score,
            judge_model: new.judge_model,
            details: new.details,
            eval_version: new.eval_version,
            created_at: NaiveDateTime::default(),
        };
        scores.push(row.clone());
        Ok(row)
    }

    async fn get_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
    ) -> Result<Option<SummaryRow>, Self::Error> {
        let s = self.summary.lock().unwrap();
        Ok(s.as_ref().filter(|r| r.id == summary_id).cloned())
    }

    async fn get_scores_for_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
    ) -> Result<Vec<EvalScore>, Self::Error> {
        Ok(self
            .scores
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.summary_id == summary_id)
            .cloned()
            .collect())
    }

    async fn get_eval_aggregate(
        &self,
        _database_url: &str,
        _eval_version: &str,
    ) -> Result<EvalAggregate, Self::Error> {
        Ok(EvalAggregate::default())
    }

    async fn get_worst_offenders(
        &self,
        _database_url: &str,
        _metric: &str,
        _eval_version: &str,
        _limit: i64,
    ) -> Result<Vec<WorstOffenderRow>, Self::Error> {
        Ok(Vec::new())
    }

    async fn upsert_run(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        status: EvalRunStatus,
        error_message: Option<String>,
    ) -> Result<EvalRun, Self::Error> {
        let row = EvalRun {
            id: Uuid::new_v4(),
            summary_id,
            eval_version: eval_version.to_string(),
            status,
            error_message,
            started_at: None,
            completed_at: None,
            created_at: NaiveDateTime::default(),
        };
        self.runs.lock().unwrap().push(row.clone());
        Ok(row)
    }

    async fn list_unscored_summary_ids(
        &self,
        _database_url: &str,
        _eval_version: &str,
        _limit: i64,
    ) -> Result<Vec<Uuid>, Self::Error> {
        Ok(Vec::new())
    }

    async fn retrieve_chunks_for_summary(
        &self,
        _database_url: &str,
        _summary_id: Uuid,
        _embedding: &[f32],
        _top_k: i64,
        _min_similarity: f32,
    ) -> Result<Vec<RetrievedChunk>, Self::Error> {
        // Always return one supporting chunk so the judge LLM is invoked
        // (rather than the threshold short-circuit that produces "unsupported"
        // verdicts without an LLM call). This is the path the cache test
        // needs to exercise.
        Ok(vec![RetrievedChunk {
            chunk_index: 1,
            chunk_text: "supporting evidence".to_string(),
            similarity: 0.9,
        }])
    }
}

// ---- Counting brainatlas client ----

#[derive(Default)]
struct CountingBrainatlas {
    extract_calls: AtomicUsize,
    embed_calls: AtomicUsize,
    judge_groundedness_calls: AtomicUsize,
    judge_rubric_calls: AtomicUsize,
}

impl CountingBrainatlas {
    fn total_llm_calls(&self) -> usize {
        self.extract_calls.load(Ordering::SeqCst)
            + self.embed_calls.load(Ordering::SeqCst)
            + self.judge_groundedness_calls.load(Ordering::SeqCst)
            + self.judge_rubric_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BrainatlasClient for CountingBrainatlas {
    type Error = MockError;

    async fn extract_claims(
        &self,
        _base_url: &str,
        _req: brpc::ExtractClaimsRequest,
    ) -> Result<ClaimsResponse, Self::Error> {
        self.extract_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ClaimsResponse {
            claims: vec![Claim {
                id: 1,
                section: "Overview".to_string(),
                text: "The hippocampus supports declarative memory.".to_string(),
            }],
        })
    }

    async fn embed(
        &self,
        _base_url: &str,
        _req: brpc::EmbedRequest,
    ) -> Result<brpc::EmbedResponse, Self::Error> {
        self.embed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(brpc::EmbedResponse {
            embedding: vec![0.1; 8],
        })
    }

    async fn judge_groundedness(
        &self,
        _base_url: &str,
        _req: brpc::JudgeGroundednessRequest,
    ) -> Result<GroundednessVerdict, Self::Error> {
        self.judge_groundedness_calls.fetch_add(1, Ordering::SeqCst);
        Ok(GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.95,
            supporting_chunks: vec![1],
            rationale: "matches retrieved chunk".to_string(),
        })
    }

    async fn judge_rubric(
        &self,
        _base_url: &str,
        _req: brpc::JudgeRubricRequest,
    ) -> Result<RubricScores, Self::Error> {
        self.judge_rubric_calls.fetch_add(1, Ordering::SeqCst);
        let c = |s: u8| RubricCriterion {
            score: s,
            rationale: format!("score {}", s),
        };
        Ok(RubricScores {
            relevance: c(5),
            coherence: c(4),
            specificity: c(3),
            clinical_utility: c(5),
            terminology: c(4),
        })
    }

    async fn check_health(&self, _base_url: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---- Stub env (no env vars needed by the test path) ----

struct DummyEnv;

impl EnvInfra for DummyEnv {
    type Error = MockError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        Err(MockError(format!("no env var {}", key)))
    }
}

// ---- The actual test ----

fn fixture_summary() -> SummaryRow {
    // Realistic-shaped multi-section text so structural metrics actually score
    // > 0 (section completeness counts headings; length must land in range).
    let body: String = "## Overview\nThe hippocampus is a brain region.\n\n\
                        ## Anatomy & Connectivity\nIt sits in the medial temporal lobe.\n\n\
                        ## Functions\nIt supports declarative memory.\n\n\
                        ## Associated Disorders\nIt is implicated in Alzheimer's.\n\n\
                        ## Symptoms of Damage or Dysfunction\nMemory loss is typical.\n\n\
                        ## Research Highlights\nMany papers exist.\n"
        .repeat(8);
    SummaryRow {
        id: Uuid::new_v4(),
        region_id: 1,
        name: "Hippocampus".to_string(),
        acronym: Some("HIP".to_string()),
        summary: body,
    }
}

#[tokio::test]
async fn cache_short_circuits_second_run() {
    let summary = fixture_summary();
    let summary_id = summary.id;

    let db = Arc::new(InMemoryDb::new(summary));
    let brainatlas = Arc::new(CountingBrainatlas::default());
    let env = Arc::new(DummyEnv);

    // Build a runtime config directly so we don't need real env vars.
    let cfg = EvalRuntimeConfig {
        database_url: "memory://".to_string(),
        brainatlas_base_url: "http://mock-brainatlas".to_string(),
        eval_version: "v1.0".to_string(),
        judge_chat_model: "mock-judge".to_string(),
        rubric_chat_model: "mock-rubric".to_string(),
        embedding_model: "mock-embed".to_string(),
        top_k_chunks: 3,
        similarity_threshold: 0.5,
    };

    let app = EvalsApp {
        db: db.clone(),
        brainatlas: brainatlas.clone(),
        env,
        config: cfg,
    };

    // ---- First call: cold cache. Expect LLM calls > 0. ----
    let first = app
        .score_summary(summary_id, None)
        .await
        .expect("first score_summary failed");

    let first_total_llm = brainatlas.total_llm_calls();
    assert!(
        first_total_llm > 0,
        "first run must have invoked the LLM at least once (got {})",
        first_total_llm
    );
    assert!(
        first.metrics.iter().all(|m| !m.cached),
        "every metric on the first run should be a cache miss"
    );
    assert!(
        !first.metrics.is_empty(),
        "first run must produce at least one metric"
    );

    // ---- Second call: warm cache. ----
    let calls_before_second = brainatlas.total_llm_calls();
    let second = app
        .score_summary(summary_id, None)
        .await
        .expect("second score_summary failed");
    let calls_after_second = brainatlas.total_llm_calls();

    // (a) Identical scores per metric.
    assert_eq!(
        first.metrics.len(),
        second.metrics.len(),
        "metric count differs between runs: first={}, second={}",
        first.metrics.len(),
        second.metrics.len()
    );
    for first_m in &first.metrics {
        let second_m = second
            .metrics
            .iter()
            .find(|m| m.metric == first_m.metric)
            .unwrap_or_else(|| panic!("metric {} missing from second run", first_m.metric));
        assert!(
            (first_m.score - second_m.score).abs() < 1e-6,
            "metric {} score drift: first={} second={}",
            first_m.metric,
            first_m.score,
            second_m.score
        );
    }

    // (b) Every metric on the second run reports cached=true.
    for m in &second.metrics {
        assert!(
            m.cached,
            "metric {} on second run must be cached, got cached={}",
            m.metric, m.cached
        );
    }

    // (c) The brainatlas mock saw zero new LLM calls during the second run.
    assert_eq!(
        calls_before_second, calls_after_second,
        "second run must perform zero LLM work \
         (calls before = {}, calls after = {})",
        calls_before_second, calls_after_second
    );
}
