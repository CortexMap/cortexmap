//! Integration test: state-machine driven by orch-simulated LLM responses.
//!
//! Covers:
//! 1. First invocation: no cached metrics → state machine walks
//!    ExtractClaims → Embed → JudgeGroundedness → JudgeRubric and
//!    produces 11 scored metrics.
//! 2. Second invocation on the same (summary_id, eval_version): every
//!    metric already in `eval_scores` cache → `init_score` short-circuits
//!    to `NextAction::Done` with all 11 metrics marked `cached: true`.
//!
//! There is no HTTP and no real DB: an in-memory `EvalsDatabase` plus
//! hand-crafted `LlmResponsePayload` values simulate brainatlas.

use std::sync::Mutex;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDateTime;
use domain::{
    Claim, ClaimsResponse, EvalRun, EvalRunStatus, EvalScore, GroundednessLabel,
    GroundednessVerdict, NewEvalScore, RubricCriterion, RubricScores,
};
use evals_app::{EvalRuntimeConfig, EvalsApp};
use rpc_types::{InitScoreRequest, LlmEndpoint, LlmResponsePayload, NextAction, StepRequest};
use services::{
    EnvInfra, EvalAggregate, EvalsDatabase, RetrievedChunk, SummaryRow, WorstOffenderRow,
};
use uuid::Uuid;

// ---- Mock infra error type ----

#[derive(Debug, thiserror::Error)]
#[error("mock infra error: {0}")]
struct MockError(String);

// ---- In-memory EvalsDatabase with run_state support ----

#[derive(Default)]
struct InMemoryDb {
    summary: Mutex<Option<SummaryRow>>,
    scores: Mutex<Vec<EvalScore>>,
    runs: Mutex<Vec<EvalRun>>,
    // run_id -> (summary_id, eval_version, state_json, pending_step_id)
    run_states: Mutex<Vec<RunStateRow>>,
}

#[derive(Clone)]
struct RunStateRow {
    run_id: Uuid,
    summary_id: Uuid,
    eval_version: String,
    state: serde_json::Value,
    pending_step_id: Option<Uuid>,
}

impl InMemoryDb {
    fn new(summary: SummaryRow) -> Self {
        Self {
            summary: Mutex::new(Some(summary)),
            scores: Mutex::new(Vec::new()),
            runs: Mutex::new(Vec::new()),
            run_states: Mutex::new(Vec::new()),
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
        Ok(vec![RetrievedChunk {
            chunk_index: 1,
            chunk_text: "supporting evidence".to_string(),
            similarity: 0.9,
        }])
    }

    async fn insert_run_state(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        _pending_endpoint: Option<&str>,
    ) -> Result<Uuid, Self::Error> {
        let run_id = Uuid::new_v4();
        self.run_states.lock().unwrap().push(RunStateRow {
            run_id,
            summary_id,
            eval_version: eval_version.to_string(),
            state: state.clone(),
            pending_step_id,
        });
        Ok(run_id)
    }

    async fn load_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
    ) -> Result<Option<(Uuid, String, serde_json::Value, Option<Uuid>)>, Self::Error> {
        let states = self.run_states.lock().unwrap();
        Ok(states.iter().find(|r| r.run_id == run_id).map(|r| {
            (
                r.summary_id,
                r.eval_version.clone(),
                r.state.clone(),
                r.pending_step_id,
            )
        }))
    }

    async fn save_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
        state: &serde_json::Value,
        pending_step_id: Option<Uuid>,
        _pending_endpoint: Option<&str>,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        if let Some(r) = states.iter_mut().find(|r| r.run_id == run_id) {
            r.state = state.clone();
            r.pending_step_id = pending_step_id;
        }
        Ok(())
    }

    async fn delete_run_state(
        &self,
        _database_url: &str,
        run_id: Uuid,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        states.retain(|r| r.run_id != run_id);
        Ok(())
    }

    async fn delete_run_states_for_summary(
        &self,
        _database_url: &str,
        summary_id: Uuid,
        eval_version: &str,
    ) -> Result<(), Self::Error> {
        let mut states = self.run_states.lock().unwrap();
        states.retain(|r| !(r.summary_id == summary_id && r.eval_version == eval_version));
        Ok(())
    }
}

// ---- Stub env ----

struct DummyEnv;

impl EnvInfra for DummyEnv {
    type Error = MockError;
    fn get_env_var(&self, key: &str) -> Result<String, Self::Error> {
        Err(MockError(format!("no env var {}", key)))
    }
}

// ---- Fake LLM responses driven by the test ----

fn fake_claims_response() -> LlmResponsePayload {
    LlmResponsePayload::Claims(ClaimsResponse {
        claims: vec![
            Claim {
                id: 1,
                section: "Overview".to_string(),
                text: "The hippocampus supports declarative memory.".to_string(),
            },
            Claim {
                id: 2,
                section: "Anatomy".to_string(),
                text: "It sits in the medial temporal lobe.".to_string(),
            },
            Claim {
                id: 3,
                section: "Functions".to_string(),
                text: "It is implicated in Alzheimer's disease.".to_string(),
            },
        ],
    })
}

fn fake_embed_response() -> LlmResponsePayload {
    LlmResponsePayload::Embed(brainatlas_rpc_types::evals::EmbedResponse {
        embedding: vec![0.1; 8],
    })
}

fn fake_groundedness_response() -> LlmResponsePayload {
    LlmResponsePayload::Groundedness(GroundednessVerdict {
        verdict: GroundednessLabel::Supported,
        confidence: 0.95,
        supporting_chunks: vec![1],
        rationale: "matches retrieved chunk".to_string(),
    })
}

fn fake_rubric_response() -> LlmResponsePayload {
    let c = |s: u8| RubricCriterion {
        score: s,
        rationale: format!("score {}", s),
    };
    LlmResponsePayload::Rubric(RubricScores {
        relevance: c(5),
        coherence: c(4),
        specificity: c(4),
        clinical_utility: c(4),
        terminology: c(4),
    })
}

fn pick_fake(endpoint: LlmEndpoint) -> LlmResponsePayload {
    match endpoint {
        LlmEndpoint::ExtractClaims => fake_claims_response(),
        LlmEndpoint::Embed => fake_embed_response(),
        LlmEndpoint::JudgeGroundedness => fake_groundedness_response(),
        LlmEndpoint::JudgeRubric => fake_rubric_response(),
    }
}

// ---- Summary fixture ----

fn fixture_summary() -> SummaryRow {
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

fn make_app(db: Arc<InMemoryDb>) -> EvalsApp<InMemoryDb, DummyEnv, MockError> {
    let cfg = EvalRuntimeConfig {
        database_url: "memory://".to_string(),
        eval_version: "v0.1.0".to_string(),
        judge_chat_model: "mock-judge".to_string(),
        rubric_chat_model: "mock-rubric".to_string(),
        embedding_model: "mock-embed".to_string(),
        top_k_chunks: 3,
        similarity_threshold: 0.5,
    };
    EvalsApp {
        db,
        env: Arc::new(DummyEnv),
        config: cfg,
    }
}

/// Drive the state machine to completion, feeding each `CallLlm` step a
/// hand-crafted fake response. Returns the final `metrics` list plus a
/// count of how many `CallLlm` steps were issued.
async fn run_to_done(
    app: &EvalsApp<InMemoryDb, DummyEnv, MockError>,
    summary_id: Uuid,
) -> (Vec<rpc_types::MetricResult>, usize) {
    let init = app
        .init_score(InitScoreRequest {
            summary_id,
            eval_version: None,
        })
        .await
        .expect("init_score failed");

    let mut call_count = 0usize;
    let mut next = init.next;
    let mut run_id = init.run_id;

    for _ in 0..100 {
        match next {
            NextAction::Done { metrics } => return (metrics, call_count),
            NextAction::CallLlm {
                step_id, endpoint, ..
            } => {
                call_count += 1;
                let payload = pick_fake(endpoint);
                let resp = app
                    .step_score(StepRequest {
                        run_id,
                        step_id,
                        llm_response: payload,
                    })
                    .await
                    .expect("step_score failed");
                run_id = resp.run_id;
                next = resp.next;
            }
        }
    }
    panic!("state machine did not reach Done within 100 steps");
}

#[tokio::test]
async fn init_and_step_score_produce_11_metrics_first_run_cache_hit_second() {
    let summary = fixture_summary();
    let summary_id = summary.id;

    let db = Arc::new(InMemoryDb::new(summary));
    let app = make_app(db.clone());

    // ---- First run: cold cache. Walks the full state machine. ----
    let (first_metrics, first_calls) = run_to_done(&app, summary_id).await;

    assert_eq!(
        first_metrics.len(),
        11,
        "first run must produce 11 metrics, got {}",
        first_metrics.len()
    );
    assert!(
        first_calls > 0,
        "first run must have issued at least one CallLlm step"
    );
    // (We don't assert `all !cached` on the first run because, at `Done`, the
    // wire-layer reconstructs the metrics list from the eval_scores cache —
    // which includes both freshly-computed and previously-cached rows. The
    // `cached` flag for structural metrics therefore defaults to `true` in
    // that reconstruction. What matters is the second-run assertion below.)

    // ---- Second run: warm cache. Should Done immediately. ----
    let (second_metrics, second_calls) = run_to_done(&app, summary_id).await;

    assert_eq!(
        second_calls, 0,
        "second run must have issued ZERO CallLlm steps (cache hit), got {}",
        second_calls
    );
    assert_eq!(
        second_metrics.len(),
        11,
        "second run must also produce 11 metrics"
    );
    assert!(
        second_metrics.iter().all(|m| m.cached),
        "every metric on the second run must be cached=true"
    );

    // Per-metric score equality.
    for fm in &first_metrics {
        let sm = second_metrics
            .iter()
            .find(|s| s.metric == fm.metric)
            .unwrap_or_else(|| panic!("metric {} missing from second run", fm.metric));
        assert!(
            (fm.score - sm.score).abs() < 1e-6,
            "metric {} drifted: first={} second={}",
            fm.metric,
            fm.score,
            sm.score
        );
    }
}
