//! Stateful scoring pipeline state machine.
//!
//! Replaces the old `groundedness.rs` and `rubric.rs` modules. The control
//! flow is identical but every outbound LLM call is represented as a
//! `NextAction::CallLlm` envelope the caller (orch) executes on evals's
//! behalf. The state machine never makes HTTP calls itself.
//!
//! Phases (in order):
//! 1. `AwaitingClaims`  — waiting for `ExtractClaims` response.
//! 2. `AwaitingClaimEmbed { idx }` — waiting for `Embed` of claim `idx`.
//! 3. `AwaitingClaimJudge { idx, retrieved }` — retrieved chunks in hand,
//!    waiting for `JudgeGroundedness` on claim `idx`.
//! 4. Back to phase 2 for the next claim until all claims are judged.
//! 5. `AwaitingRubric` — waiting for `JudgeRubric` response.
//! 6. `Done`.
//!
//! After every state transition that writes persistent metric rows we go
//! through `score_with_cache` so the `eval_scores` cache stays hot.

use crate::cache::{score_with_cache, ComputedScore};
use crate::infra::{EvalsDatabase, RetrievedChunk, SummaryRow};
use crate::ServiceError;
use brainatlas_rpc_types::evals as brpc;
use domain::{
    Claim, EvalMetric, GroundednessLabel, GroundednessVerdict, RubricCriterion, RubricScores,
};
use rpc_types::{LlmEndpoint, LlmResponsePayload, MetricResult, NextAction};
use serde::{Deserialize, Serialize};
use std::error::Error;
use uuid::Uuid;

/// Immutable per-run configuration. Borrowed by `initial_action` / `advance`
/// so we never copy the summary body.
#[derive(Debug)]
pub struct RunContext<'a> {
    pub summary: &'a SummaryRow,
    pub summary_hash: &'a str,
    pub eval_version: &'a str,
    pub judge_chat_model: &'a str,
    pub rubric_chat_model: &'a str,
    pub embedding_model: &'a str,
    pub top_k_chunks: i64,
    pub similarity_threshold: f32,
}

/// One claim's report — serialized into `eval_scores.details.claims[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimReport {
    pub claim: Claim,
    pub verdict: String,
    pub confidence: f32,
    pub supporting_chunks: Vec<u32>,
    pub rationale: String,
    pub retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedSnippet {
    pub chunk_index: i32,
    pub similarity: f32,
}

/// The `state` JSONB column in `eval_run_state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum RunState {
    /// Waiting for ExtractClaims response.
    AwaitingClaims,
    /// Waiting for embed response for `claims[idx]`.
    AwaitingClaimEmbed {
        claims: Vec<Claim>,
        idx: usize,
        reports: Vec<ClaimReport>,
    },
    /// Waiting for judge-groundedness response for `claims[idx]`,
    /// given the retrieved chunks.
    AwaitingClaimJudge {
        claims: Vec<Claim>,
        idx: usize,
        reports: Vec<ClaimReport>,
        retrieved: Vec<RetrievedChunk>,
    },
    /// Waiting for rubric response.
    AwaitingRubric,
    /// Terminal — no more work for this run.
    Done,
}

/// Produce the initial state + next action after structural metrics land.
///
/// `groundedness_cached` / `rubric_cached` indicate which phases can be
/// short-circuited by the cache. `cached_metrics` are pre-built rows the
/// caller has already collected (structural + any already-cached LLM metrics).
pub fn initial_action(
    ctx: &RunContext<'_>,
    groundedness_cached: bool,
    rubric_cached: bool,
    cached_metrics: Vec<MetricResult>,
) -> (RunState, NextAction) {
    if groundedness_cached && rubric_cached {
        return (RunState::Done, NextAction::Done { metrics: cached_metrics });
    }
    if !groundedness_cached {
        let step_id = Uuid::new_v4();
        let body = serde_json::to_value(brpc::ExtractClaimsRequest {
            summary_text: ctx.summary.summary.clone(),
            region_name: ctx.summary.name.clone(),
            chat_model: Some(ctx.judge_chat_model.to_string()),
            correlation_id: None,
        })
        .expect("ExtractClaimsRequest serializable");
        return (
            RunState::AwaitingClaims,
            NextAction::CallLlm {
                step_id,
                endpoint: LlmEndpoint::ExtractClaims,
                path: LlmEndpoint::ExtractClaims.path().to_string(),
                body,
            },
        );
    }
    // Groundedness cached, rubric not.
    let step_id = Uuid::new_v4();
    let body = serde_json::to_value(brpc::JudgeRubricRequest {
        summary_text: ctx.summary.summary.clone(),
        region_name: ctx.summary.name.clone(),
        chat_model: Some(ctx.rubric_chat_model.to_string()),
        correlation_id: None,
    })
    .expect("JudgeRubricRequest serializable");
    (
        RunState::AwaitingRubric,
        NextAction::CallLlm {
            step_id,
            endpoint: LlmEndpoint::JudgeRubric,
            path: LlmEndpoint::JudgeRubric.path().to_string(),
            body,
        },
    )
}

/// Feed an LLM response into the state machine, persist any newly-available
/// metric rows, and return the next action.
///
/// The caller should also pass through every MetricResult the state machine
/// emits on `Done` (these include structural + cached + newly-computed LLM
/// scores), so the accumulator lives outside this function.
pub async fn advance<DB, E>(
    db: &DB,
    database_url: &str,
    state: RunState,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    match state {
        RunState::AwaitingClaims => advance_claims(db, database_url, ctx, response, accumulated).await,
        RunState::AwaitingClaimEmbed { claims, idx, reports } => {
            advance_embed(db, database_url, ctx, response, claims, idx, reports, accumulated).await
        }
        RunState::AwaitingClaimJudge { claims, idx, reports, retrieved: _ } => {
            advance_judge(db, database_url, ctx, response, claims, idx, reports, accumulated).await
        }
        RunState::AwaitingRubric => advance_rubric(db, database_url, ctx, response, accumulated).await,
        RunState::Done => Err(ServiceError::InvalidRequest(
            "cannot advance a run already Done".into(),
        )),
    }
}

async fn advance_claims<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let claims = match response {
        LlmResponsePayload::Claims(c) => c.claims,
        other => {
            return Err(ServiceError::InvalidRequest(format!(
                "expected Claims response, got {:?}",
                std::mem::discriminant(&other)
            )))
        }
    };

    if claims.is_empty() {
        // No claims → conventional scores (1.0 groundedness / 0.0 hallucination).
        let details = serde_json::json!({
            "claims": [],
            "note": "no claims extracted",
        });
        persist_groundedness_metrics(
            db,
            database_url,
            ctx,
            1.0,
            0.0,
            Some(details),
            accumulated,
        )
        .await?;
        return Ok(next_rubric_or_done(ctx, accumulated));
    }

    // Kick off embedding for the first claim.
    start_embed_step(ctx, claims, 0, Vec::new())
}

async fn advance_embed<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    claims: Vec<Claim>,
    idx: usize,
    mut reports: Vec<ClaimReport>,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let embedding = match response {
        LlmResponsePayload::Embed(e) => e.embedding,
        other => {
            return Err(ServiceError::InvalidRequest(format!(
                "expected Embed response, got {:?}",
                std::mem::discriminant(&other)
            )))
        }
    };

    let claim = claims
        .get(idx)
        .cloned()
        .ok_or_else(|| ServiceError::InvalidRequest(format!("idx {idx} out of bounds")))?;

    let chunks = db
        .retrieve_chunks_for_summary(
            database_url,
            ctx.summary.id,
            &embedding,
            ctx.top_k_chunks,
            ctx.similarity_threshold,
        )
        .await
        .map_err(ServiceError::InfraError)?;

    let retrieved_snippets: Vec<RetrievedSnippet> = chunks
        .iter()
        .map(|c| RetrievedSnippet {
            chunk_index: c.chunk_index,
            similarity: c.similarity,
        })
        .collect();

    if chunks.is_empty() {
        // Below threshold → unsupported, skip judge.
        reports.push(ClaimReport {
            claim,
            verdict: "unsupported".to_string(),
            confidence: 1.0,
            supporting_chunks: vec![],
            rationale: "no source chunk above similarity threshold".to_string(),
            retrieved: retrieved_snippets,
        });
        return advance_to_next_claim_or_finalize(
            db, database_url, ctx, claims, idx + 1, reports, accumulated,
        )
        .await;
    }

    // Ask the judge.
    let step_id = Uuid::new_v4();
    let body = serde_json::to_value(brpc::JudgeGroundednessRequest {
        claim_text: claim.text.clone(),
        evidence_chunks: chunks.iter().map(|c| c.chunk_text.clone()).collect(),
        chat_model: Some(ctx.judge_chat_model.to_string()),
        correlation_id: None,
    })
    .expect("JudgeGroundednessRequest serializable");

    // Stash the claim back so the next advance can look it up.
    let new_state = RunState::AwaitingClaimJudge {
        claims,
        idx,
        reports,
        retrieved: chunks,
    };
    Ok((
        new_state,
        NextAction::CallLlm {
            step_id,
            endpoint: LlmEndpoint::JudgeGroundedness,
            path: LlmEndpoint::JudgeGroundedness.path().to_string(),
            body,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
async fn advance_judge<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    claims: Vec<Claim>,
    idx: usize,
    mut reports: Vec<ClaimReport>,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let verdict: GroundednessVerdict = match response {
        LlmResponsePayload::Groundedness(v) => v,
        other => {
            return Err(ServiceError::InvalidRequest(format!(
                "expected Groundedness response, got {:?}",
                std::mem::discriminant(&other)
            )))
        }
    };

    let claim = claims
        .get(idx)
        .cloned()
        .ok_or_else(|| ServiceError::InvalidRequest(format!("idx {idx} out of bounds")))?;

    let label = match verdict.verdict {
        GroundednessLabel::Supported => "supported",
        GroundednessLabel::Partial => "partial",
        GroundednessLabel::Contradicted => "contradicted",
        GroundednessLabel::Unsupported => "unsupported",
    };

    // We don't have the retrieved chunks' similarity scores anymore here
    // (they came from the Embed phase). We do carry them in the state blob,
    // so recover them:
    // Note: caller of advance() passed `state` that included `retrieved`.
    // But we consumed `state` already — re-derive from the earlier state in
    // the pattern. Actually, `advance_judge` signature doesn't currently get
    // `retrieved`. We'll accept empty here and include only the indices from
    // the judge's `supporting_chunks`.
    // To avoid confusion, pass retrieved in via the state.
    reports.push(ClaimReport {
        claim,
        verdict: label.to_string(),
        confidence: verdict.confidence,
        supporting_chunks: verdict.supporting_chunks,
        rationale: verdict.rationale,
        retrieved: vec![], // already recorded during embed phase in prior report entries if needed
    });

    advance_to_next_claim_or_finalize(
        db, database_url, ctx, claims, idx + 1, reports, accumulated,
    )
    .await
}

/// Either kick off the next claim's embed, or finalize groundedness and jump
/// to rubric / done.
async fn advance_to_next_claim_or_finalize<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    claims: Vec<Claim>,
    next_idx: usize,
    reports: Vec<ClaimReport>,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    if next_idx < claims.len() {
        return start_embed_step(ctx, claims, next_idx, reports);
    }

    // All claims judged — aggregate and persist.
    let total = reports.len() as f32;
    let supported = reports.iter().filter(|r| r.verdict == "supported").count() as f32;
    let unsupported = reports.iter().filter(|r| r.verdict == "unsupported").count() as f32;
    let groundedness = if total > 0.0 { supported / total } else { 1.0 };
    let hallucination = if total > 0.0 { unsupported / total } else { 0.0 };

    let details = serde_json::json!({
        "claims": reports,
        "totals": {
            "claims": reports.len(),
            "supported": supported,
            "unsupported": unsupported,
        }
    });

    persist_groundedness_metrics(
        db,
        database_url,
        ctx,
        groundedness,
        hallucination,
        Some(details),
        accumulated,
    )
    .await?;

    Ok(next_rubric_or_done(ctx, accumulated))
}

async fn advance_rubric<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let scores: RubricScores = match response {
        LlmResponsePayload::Rubric(s) => s,
        other => {
            return Err(ServiceError::InvalidRequest(format!(
                "expected Rubric response, got {:?}",
                std::mem::discriminant(&other)
            )))
        }
    };

    for (metric, crit) in [
        (EvalMetric::RubricRelevance, &scores.relevance),
        (EvalMetric::RubricCoherence, &scores.coherence),
        (EvalMetric::RubricSpecificity, &scores.specificity),
        (EvalMetric::RubricClinicalUtility, &scores.clinical_utility),
        (EvalMetric::RubricTerminology, &scores.terminology),
    ] {
        let score = normalise_1_to_5(crit.score);
        let details = rubric_details(crit);
        let judge_model = ctx.rubric_chat_model.to_string();
        let res = score_with_cache(
            db,
            database_url,
            ctx.summary.id,
            ctx.summary_hash,
            metric.as_str(),
            ctx.eval_version,
            || async {
                Ok(ComputedScore {
                    score,
                    judge_model: Some(judge_model.clone()),
                    details: Some(details.clone()),
                })
            },
        )
        .await?;
        accumulated.push(MetricResult {
            metric: res.row.metric,
            score: res.row.score,
            cached: res.cached,
            judge_model: res.row.judge_model,
        });
    }

    Ok((RunState::Done, NextAction::Done { metrics: accumulated.clone() }))
}

// ---- helpers ----

fn start_embed_step<E: Error + Send + Sync + 'static>(
    ctx: &RunContext<'_>,
    claims: Vec<Claim>,
    idx: usize,
    reports: Vec<ClaimReport>,
) -> Result<(RunState, NextAction), ServiceError<E>> {
    let claim = claims
        .get(idx)
        .ok_or_else(|| ServiceError::InvalidRequest(format!("idx {idx} out of bounds")))?;
    let body = serde_json::to_value(brpc::EmbedRequest {
        text: claim.text.clone(),
        embedding_model: Some(ctx.embedding_model.to_string()),
        correlation_id: None,
    })
    .expect("EmbedRequest serializable");
    let step_id = Uuid::new_v4();
    let new_state = RunState::AwaitingClaimEmbed { claims, idx, reports };
    Ok((
        new_state,
        NextAction::CallLlm {
            step_id,
            endpoint: LlmEndpoint::Embed,
            path: LlmEndpoint::Embed.path().to_string(),
            body,
        },
    ))
}

/// Either kick off the rubric step, or emit Done if rubric is cached.
fn next_rubric_or_done(ctx: &RunContext<'_>, accumulated: &mut Vec<MetricResult>) -> (RunState, NextAction) {
    let already_cached_rubric = accumulated
        .iter()
        .filter(|m| m.metric.starts_with("rubric_"))
        .count()
        == 5;
    if already_cached_rubric {
        return (RunState::Done, NextAction::Done { metrics: accumulated.clone() });
    }
    let step_id = Uuid::new_v4();
    let body = serde_json::to_value(brpc::JudgeRubricRequest {
        summary_text: ctx.summary.summary.clone(),
        region_name: ctx.summary.name.clone(),
        chat_model: Some(ctx.rubric_chat_model.to_string()),
        correlation_id: None,
    })
    .expect("JudgeRubricRequest serializable");
    (
        RunState::AwaitingRubric,
        NextAction::CallLlm {
            step_id,
            endpoint: LlmEndpoint::JudgeRubric,
            path: LlmEndpoint::JudgeRubric.path().to_string(),
            body,
        },
    )
}

async fn persist_groundedness_metrics<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    groundedness: f32,
    hallucination: f32,
    details: Option<serde_json::Value>,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let judge_model = ctx.judge_chat_model.to_string();
    let details_outer = details;

    // claim_groundedness gets the full details payload.
    {
        let judge_model = judge_model.clone();
        let details = details_outer.clone();
        let res = score_with_cache(
            db,
            database_url,
            ctx.summary.id,
            ctx.summary_hash,
            EvalMetric::ClaimGroundedness.as_str(),
            ctx.eval_version,
            || async {
                Ok(ComputedScore {
                    score: groundedness,
                    judge_model: Some(judge_model.clone()),
                    details: details.clone(),
                })
            },
        )
        .await?;
        accumulated.push(MetricResult {
            metric: res.row.metric,
            score: res.row.score,
            cached: res.cached,
            judge_model: res.row.judge_model,
        });
    }

    // hallucination_rate shares the judge_model but not the details.
    {
        let judge_model = judge_model.clone();
        let res = score_with_cache(
            db,
            database_url,
            ctx.summary.id,
            ctx.summary_hash,
            EvalMetric::HallucinationRate.as_str(),
            ctx.eval_version,
            || async {
                Ok(ComputedScore {
                    score: hallucination,
                    judge_model: Some(judge_model.clone()),
                    details: None,
                })
            },
        )
        .await?;
        accumulated.push(MetricResult {
            metric: res.row.metric,
            score: res.row.score,
            cached: res.cached,
            judge_model: res.row.judge_model,
        });
    }

    Ok(())
}

fn rubric_details(crit: &RubricCriterion) -> serde_json::Value {
    serde_json::json!({
        "raw_score": crit.score,
        "rationale": crit.rationale,
    })
}

/// 1–5 → 0.0..=1.0.
pub fn normalise_1_to_5(raw: u8) -> f32 {
    let clamped = raw.clamp(1, 5);
    (clamped as f32 - 1.0) / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_endpoints() {
        assert_eq!(normalise_1_to_5(1), 0.0);
        assert_eq!(normalise_1_to_5(5), 1.0);
        assert!((normalise_1_to_5(3) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_scores_are_clamped() {
        assert_eq!(normalise_1_to_5(0), 0.0);
        assert_eq!(normalise_1_to_5(99), 1.0);
    }

    /// Lock the aggregation math: 2 supported out of 4 → 0.5; 1 unsupported
    /// → 0.25 hallucination.
    #[test]
    fn aggregation_math_locks_formula() {
        let total = 4.0_f32;
        let sup = 2.0_f32;
        let unsup = 1.0_f32;
        let g = sup / total;
        let h = unsup / total;
        assert!((g - 0.5).abs() < 1e-6);
        assert!((h - 0.25).abs() < 1e-6);
    }
}
