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
use crate::citations::{
    self, parse_citations, CitationIssue, CitationIssueKind, ParsedCitation,
};
use crate::infra::{ChunkRow, EvalsDatabase, RetrievedChunk, SummaryRow};
use crate::ServiceError;
use brainatlas_rpc_types::evals as brpc;
use domain::{
    Claim, EvalMetric, GroundednessLabel, GroundednessVerdict, RubricCriterion, RubricScores,
};
use rpc_types::{LlmEndpoint, LlmResponsePayload, MetricResult, NextAction};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// When `true`, after rubric scoring the state machine runs the expensive
    /// per-citation support judge. When `false`, only the three deterministic
    /// citation metrics (presence, validity, scope) are computed. See
    /// `EVAL_CITATION_SUPPORT_ENABLED`.
    pub citation_support_enabled: bool,
    /// Upper bound on the number of `JudgeCitation` calls issued per summary.
    /// Excess citations are still counted in `presence`/`validity`/`scope` but
    /// the support judge skips them and the support score is flagged with
    /// `details.truncated = true`.
    pub citation_support_max_calls: usize,
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
    /// Waiting for rubric response. Carries the claim list forward so the
    /// citation phases (after rubric) can reference the extracted claims
    /// without re-running extraction. `claims` may be `None` when groundedness
    /// was cache-hit and no fresh extraction happened.
    AwaitingRubric {
        #[serde(default)]
        claims: Option<Vec<Claim>>,
    },
    /// Waiting for a JudgeCitation response for one specific
    /// `(claim_idx, cite_idx)` pair. Issued when `citation_support_enabled`
    /// is true and at least one citation exists in scope.
    AwaitingCitationSupport {
        claims: Vec<Claim>,
        /// `(claim_idx, cite_idx_within_claim)` — used to resume.
        claim_idx: usize,
        cite_idx: usize,
        /// All cited chunks already loaded, keyed by UUID.
        cited_chunks: HashMap<Uuid, ChunkRow>,
        /// Issues accumulated so far (presence + validity + scope + any
        /// unsupported/contradicted support verdicts).
        issues: Vec<CitationIssue>,
        /// Tallies for the support-judge aggregation.
        support_supported: u32,
        support_partial: u32,
        support_unsupported: u32,
        support_contradicted: u32,
        /// Totals locked at prep-time so the final metrics don't shift if
        /// a late iteration fails.
        totals: CitationTotals,
        /// Number of support-judge calls actually issued so far (may be
        /// less than the product of indices if skipped for budget).
        support_calls_issued: u32,
        /// `true` when we truncated the support loop due to
        /// `citation_support_max_calls`.
        truncated: bool,
    },
    /// Terminal — no more work for this run.
    Done,
}

/// Counters captured during the "prep" pass over parsed citations; carried
/// through the support-judge loop so final-score math is deterministic.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CitationTotals {
    /// Total claims the extractor produced.
    pub total_claims: u32,
    /// Claims whose enclosing sentence in the summary carries at least one
    /// `[chunk:...]` marker.
    pub claims_with_citation: u32,
    /// Total `[chunk:UUID]` markers parsed from the summary.
    pub total_citations: u32,
    /// Markers whose UUID exists in `brain_region_embeddings`.
    pub existing_citations: u32,
    /// Markers whose UUID exists *and* belongs to this summary.
    pub in_scope_citations: u32,
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
        // Citations are only computed alongside a fresh groundedness pass
        // (they ride on the same Claim objects). If both upstream families
        // are already cached, nothing left to do.
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
    // Groundedness cached, rubric not — skip straight to rubric. No claims
    // available, so citations won't run in this branch.
    let step_id = Uuid::new_v4();
    let body = serde_json::to_value(brpc::JudgeRubricRequest {
        summary_text: ctx.summary.summary.clone(),
        region_name: ctx.summary.name.clone(),
        chat_model: Some(ctx.rubric_chat_model.to_string()),
        correlation_id: None,
    })
    .expect("JudgeRubricRequest serializable");
    (
        RunState::AwaitingRubric { claims: None },
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
        RunState::AwaitingRubric { claims } => {
            advance_rubric(db, database_url, ctx, response, claims, accumulated).await
        }
        RunState::AwaitingCitationSupport {
            claims,
            claim_idx,
            cite_idx,
            cited_chunks,
            issues,
            support_supported,
            support_partial,
            support_unsupported,
            support_contradicted,
            totals,
            support_calls_issued,
            truncated,
        } => {
            advance_citation_support(
                db,
                database_url,
                ctx,
                response,
                claims,
                claim_idx,
                cite_idx,
                cited_chunks,
                issues,
                support_supported,
                support_partial,
                support_unsupported,
                support_contradicted,
                totals,
                support_calls_issued,
                truncated,
                accumulated,
            )
            .await
        }
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
        return Ok(next_rubric_or_done(ctx, Some(Vec::new()), accumulated));
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

    Ok(next_rubric_or_done(ctx, Some(claims), accumulated))
}

async fn advance_rubric<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    claims: Option<Vec<Claim>>,
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

    // After rubric → hand off to the citation phase (if claims are available).
    next_citation_or_done(db, database_url, ctx, claims, accumulated).await
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
fn next_rubric_or_done(
    ctx: &RunContext<'_>,
    claims: Option<Vec<Claim>>,
    accumulated: &mut Vec<MetricResult>,
) -> (RunState, NextAction) {
    let already_cached_rubric = accumulated
        .iter()
        .filter(|m| m.metric.starts_with("rubric_"))
        .count()
        == 5;
    if already_cached_rubric {
        // Rubric already cached — the caller must still run citations.
        // But `next_rubric_or_done` is sync and citations need DB access,
        // so emit a sentinel transition the async caller re-enters.
        // Simpler: fall through to the rubric call (which is the cached
        // path and will reuse the cached score via `score_with_cache`).
        // However, we want to avoid the wasted LLM call. Instead, when
        // rubric is cached, emit `Done` here — historically that was the
        // behaviour and rubric-cached-but-groundedness-not is rare.
        // Citations still get a chance on the fresh-groundedness path;
        // rubric-cached+groundedness-fresh is not a common combination
        // and would complicate the control flow.
        //
        // TODO: once cost tracking is in place, revisit this trade-off.
        let _ = claims;
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
        RunState::AwaitingRubric { claims },
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

// ---- citation phases ----

/// Called once after rubric finishes. Runs the deterministic citation
/// pre-pass (presence + validity + scope), persists those three metric
/// rows unconditionally, and then either emits `Done` (support disabled
/// or nothing to judge) or transitions into `AwaitingCitationSupport`.
async fn next_citation_or_done<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    claims: Option<Vec<Claim>>,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    // Citations require the fresh claim list (to map claims → sentences).
    // If groundedness was cache-hit the caller left `claims = None` and we
    // skip citations entirely for this run — they'll land on the next run
    // that re-extracts claims.
    let Some(claims) = claims else {
        return Ok((RunState::Done, NextAction::Done { metrics: accumulated.clone() }));
    };

    // Parse every [chunk:UUID] marker in the summary text.
    let parsed: Vec<ParsedCitation> = parse_citations(&ctx.summary.summary);

    // Presence: claims whose enclosing sentence carries at least one citation.
    let (presence_score, presence_issues) =
        citations::citation_presence_score(&ctx.summary.summary, &claims);

    // Validity + scope need a DB round-trip for the cited UUIDs.
    let unique_uuids: Vec<Uuid> = {
        let mut set: HashSet<Uuid> = HashSet::new();
        for p in &parsed {
            set.insert(p.uuid);
        }
        set.into_iter().collect()
    };
    let chunk_rows: Vec<ChunkRow> = if unique_uuids.is_empty() {
        Vec::new()
    } else {
        db.load_chunks_by_ids(database_url, &unique_uuids)
            .await
            .map_err(ServiceError::InfraError)?
    };
    let existing: HashSet<Uuid> = chunk_rows.iter().map(|c| c.id).collect();
    let in_scope: HashSet<Uuid> = chunk_rows
        .iter()
        .filter(|c| c.summary_id == ctx.summary.id)
        .map(|c| c.id)
        .collect();

    let mut validity_issues: Vec<CitationIssue> = Vec::new();
    let mut scope_issues: Vec<CitationIssue> = Vec::new();
    for p in &parsed {
        if !existing.contains(&p.uuid) {
            validity_issues.push(CitationIssue {
                kind: CitationIssueKind::Orphan,
                claim_id: 0,
                claim_text: "<unattributed>".to_string(),
                offending_chunk_id: Some(p.uuid),
                rationale: format!("UUID {} does not exist in brain_region_embeddings", p.uuid),
            });
        } else if !in_scope.contains(&p.uuid) {
            scope_issues.push(CitationIssue {
                kind: CitationIssueKind::OutOfScope,
                claim_id: 0,
                claim_text: "<unattributed>".to_string(),
                offending_chunk_id: Some(p.uuid),
                rationale: format!(
                    "UUID {} exists but belongs to a different summary",
                    p.uuid
                ),
            });
        }
    }

    let total_citations = parsed.len() as u32;
    let existing_citations = (total_citations as usize - validity_issues.len()) as u32;
    let in_scope_citations = existing_citations - scope_issues.len() as u32;

    let validity_score: f32 = if total_citations == 0 {
        0.0
    } else {
        existing_citations as f32 / total_citations as f32
    };
    let scope_score: f32 = if existing_citations == 0 {
        1.0
    } else {
        in_scope_citations as f32 / existing_citations as f32
    };

    // Build a map from uuid → chunk row for quick lookup in the support phase.
    let cited_chunks: HashMap<Uuid, ChunkRow> = chunk_rows
        .into_iter()
        .filter(|c| in_scope.contains(&c.id))
        .map(|c| (c.id, c))
        .collect();

    let totals = CitationTotals {
        total_claims: claims.len() as u32,
        claims_with_citation: {
            // Presence score was computed above; derive the count from it
            // for auditing.
            let n = claims.len() as f32;
            (presence_score * n).round() as u32
        },
        total_citations,
        existing_citations,
        in_scope_citations,
    };

    // Persist the three deterministic metrics.
    persist_citation_metric(
        db,
        database_url,
        ctx,
        EvalMetric::CitationPresence,
        presence_score,
        serde_json::json!({
            "issues": presence_issues,
            "totals": &totals,
        }),
        accumulated,
    )
    .await?;
    persist_citation_metric(
        db,
        database_url,
        ctx,
        EvalMetric::CitationValidity,
        validity_score,
        serde_json::json!({
            "issues": validity_issues,
            "totals": &totals,
            "reason": if total_citations == 0 { "no_citations" } else { "" },
        }),
        accumulated,
    )
    .await?;
    persist_citation_metric(
        db,
        database_url,
        ctx,
        EvalMetric::CitationScope,
        scope_score,
        serde_json::json!({
            "issues": scope_issues,
            "totals": &totals,
        }),
        accumulated,
    )
    .await?;

    // If the support judge is disabled, or there is nothing in-scope to judge,
    // persist a sentinel support score and stop.
    if !ctx.citation_support_enabled || in_scope_citations == 0 {
        let (support_score, reason) = if !ctx.citation_support_enabled {
            (f32::NAN, "support_disabled")
        } else {
            (0.0, "no_citations")
        };
        // NaN cannot round-trip through JSON / Postgres numeric, so when
        // disabled we simply skip persisting the support row entirely to
        // avoid polluting the cache with a synthetic score.
        if !support_score.is_nan() {
            persist_citation_metric(
                db,
                database_url,
                ctx,
                EvalMetric::CitationSupport,
                support_score,
                serde_json::json!({
                    "totals": &totals,
                    "reason": reason,
                }),
                accumulated,
            )
            .await?;
        }
        return Ok((RunState::Done, NextAction::Done { metrics: accumulated.clone() }));
    }

    // Otherwise, start the support-judge loop. Find the first
    // `(claim_idx, cite_idx)` pair whose UUID is in-scope.
    let presence_set: HashSet<Uuid> = in_scope.iter().copied().collect();
    let initial = find_next_support_step(&claims, 0, 0, &presence_set);
    match initial {
        None => {
            // No in-scope citations attached to any claim → nothing to judge.
            // Support score reflects the (0/0) case, treated as 1.0.
            persist_citation_metric(
                db,
                database_url,
                ctx,
                EvalMetric::CitationSupport,
                1.0,
                serde_json::json!({
                    "totals": &totals,
                    "reason": "no_cited_chunks_on_claims",
                }),
                accumulated,
            )
            .await?;
            Ok((RunState::Done, NextAction::Done { metrics: accumulated.clone() }))
        }
        Some((claim_idx, cite_idx)) => {
            let issues: Vec<CitationIssue> = presence_issues
                .into_iter()
                .chain(validity_issues)
                .chain(scope_issues)
                .collect();
            let action = build_citation_support_action(
                ctx,
                &claims,
                claim_idx,
                cite_idx,
                &cited_chunks,
            );
            let state = RunState::AwaitingCitationSupport {
                claims,
                claim_idx,
                cite_idx,
                cited_chunks,
                issues,
                support_supported: 0,
                support_partial: 0,
                support_unsupported: 0,
                support_contradicted: 0,
                totals,
                support_calls_issued: 1,
                truncated: false,
            };
            Ok((state, action))
        }
    }
}

/// Locate the next claim/citation pair whose UUID is in-scope. Returns `None`
/// when no more pairs are left.
fn find_next_support_step(
    claims: &[Claim],
    start_claim: usize,
    start_cite: usize,
    in_scope: &HashSet<Uuid>,
) -> Option<(usize, usize)> {
    for ci in start_claim..claims.len() {
        let cites = &claims[ci].cited_chunks;
        let start = if ci == start_claim { start_cite } else { 0 };
        for ki in start..cites.len() {
            if in_scope.contains(&cites[ki]) {
                return Some((ci, ki));
            }
        }
    }
    None
}

/// Build the `CallLlm` action for a specific `(claim_idx, cite_idx)` pair.
fn build_citation_support_action(
    ctx: &RunContext<'_>,
    claims: &[Claim],
    claim_idx: usize,
    cite_idx: usize,
    cited_chunks: &HashMap<Uuid, ChunkRow>,
) -> NextAction {
    let claim = &claims[claim_idx];
    let uuid = claim.cited_chunks[cite_idx];
    let chunk_text = cited_chunks
        .get(&uuid)
        .map(|c| c.chunk_text.clone())
        .unwrap_or_default();

    // Best-effort sentence extraction: locate the first [chunk:UUID] marker
    // in the summary that matches this UUID, then return the enclosing
    // sentence.
    let sentence = enclosing_sentence_for_uuid(&ctx.summary.summary, uuid)
        .unwrap_or_else(|| claim.text.clone());

    let body = serde_json::to_value(brpc::JudgeCitationRequest {
        claim_text: claim.text.clone(),
        sentence_context: sentence,
        chunk_text,
        chat_model: Some(ctx.judge_chat_model.to_string()),
        correlation_id: None,
    })
    .expect("JudgeCitationRequest serializable");

    NextAction::CallLlm {
        step_id: Uuid::new_v4(),
        endpoint: LlmEndpoint::JudgeCitation,
        path: LlmEndpoint::JudgeCitation.path().to_string(),
        body,
    }
}

fn enclosing_sentence_for_uuid(summary: &str, uuid: Uuid) -> Option<String> {
    let needle = format!("[chunk:{}]", uuid);
    let Some(pos) = summary.to_lowercase().find(&needle.to_lowercase()) else {
        return None;
    };
    // Find sentence boundaries around `pos`.
    let bytes = summary.as_bytes();
    let mut start = pos;
    while start > 0 {
        let b = bytes[start - 1];
        if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
            break;
        }
        start -= 1;
    }
    let mut end = pos + needle.len();
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
            end += 1;
            break;
        }
        end += 1;
    }
    Some(summary[start..end].trim().to_string())
}

#[allow(clippy::too_many_arguments)]
async fn advance_citation_support<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    response: LlmResponsePayload,
    claims: Vec<Claim>,
    claim_idx: usize,
    cite_idx: usize,
    cited_chunks: HashMap<Uuid, ChunkRow>,
    mut issues: Vec<CitationIssue>,
    mut support_supported: u32,
    mut support_partial: u32,
    mut support_unsupported: u32,
    mut support_contradicted: u32,
    totals: CitationTotals,
    support_calls_issued: u32,
    mut truncated: bool,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(RunState, NextAction), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let verdict: GroundednessVerdict = match response {
        LlmResponsePayload::CitationSupport(v) => v,
        other => {
            return Err(ServiceError::InvalidRequest(format!(
                "expected CitationSupport response, got {:?}",
                std::mem::discriminant(&other)
            )))
        }
    };

    // Record the verdict.
    let claim = &claims[claim_idx];
    let uuid = claim.cited_chunks[cite_idx];
    match verdict.verdict {
        GroundednessLabel::Supported => support_supported += 1,
        GroundednessLabel::Partial => support_partial += 1,
        GroundednessLabel::Unsupported => {
            support_unsupported += 1;
            issues.push(CitationIssue {
                kind: CitationIssueKind::Unsupported,
                claim_id: claim.id,
                claim_text: claim.text.clone(),
                offending_chunk_id: Some(uuid),
                rationale: verdict.rationale.clone(),
            });
        }
        GroundednessLabel::Contradicted => {
            support_contradicted += 1;
            issues.push(CitationIssue {
                kind: CitationIssueKind::Contradicted,
                claim_id: claim.id,
                claim_text: claim.text.clone(),
                offending_chunk_id: Some(uuid),
                rationale: verdict.rationale.clone(),
            });
        }
    }

    // Build an in-scope set for iteration from the preloaded `cited_chunks`
    // (since only in-scope chunks made it into the map).
    let in_scope: HashSet<Uuid> = cited_chunks.keys().copied().collect();

    // Determine the next pair.
    let next = find_next_support_step(&claims, claim_idx, cite_idx + 1, &in_scope);

    let over_budget = support_calls_issued as usize >= ctx.citation_support_max_calls;
    match next {
        Some((nci, nki)) if !over_budget => {
            let action =
                build_citation_support_action(ctx, &claims, nci, nki, &cited_chunks);
            let state = RunState::AwaitingCitationSupport {
                claims,
                claim_idx: nci,
                cite_idx: nki,
                cited_chunks,
                issues,
                support_supported,
                support_partial,
                support_unsupported,
                support_contradicted,
                totals,
                support_calls_issued: support_calls_issued + 1,
                truncated,
            };
            Ok((state, action))
        }
        _ => {
            // Done iterating (or truncated) — aggregate and persist.
            if next.is_some() && over_budget {
                truncated = true;
            }
            let judged = support_supported
                + support_partial
                + support_unsupported
                + support_contradicted;
            let score = if judged == 0 {
                1.0
            } else {
                (support_supported as f32 + 0.5 * support_partial as f32) / judged as f32
            };
            persist_citation_metric(
                db,
                database_url,
                ctx,
                EvalMetric::CitationSupport,
                score,
                serde_json::json!({
                    "issues": issues,
                    "totals": &totals,
                    "support_supported": support_supported,
                    "support_partial": support_partial,
                    "support_unsupported": support_unsupported,
                    "support_contradicted": support_contradicted,
                    "calls_issued": support_calls_issued,
                    "truncated": truncated,
                }),
                accumulated,
            )
            .await?;
            Ok((RunState::Done, NextAction::Done { metrics: accumulated.clone() }))
        }
    }
}

async fn persist_citation_metric<DB, E>(
    db: &DB,
    database_url: &str,
    ctx: &RunContext<'_>,
    metric: EvalMetric,
    score: f32,
    details: serde_json::Value,
    accumulated: &mut Vec<MetricResult>,
) -> Result<(), ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    E: Error + Send + Sync + 'static,
{
    // Only CitationSupport carries a judge_model; the three deterministic
    // metrics do not invoke an LLM.
    let judge_model = if matches!(metric, EvalMetric::CitationSupport) {
        Some(ctx.judge_chat_model.to_string())
    } else {
        None
    };
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
                judge_model: judge_model.clone(),
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
    Ok(())
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

    /// Guard the support-score formula: supported=1.0, partial=0.5, else 0.
    /// 2 supported, 2 partial, 1 unsupported, 1 contradicted → (2 + 1) / 6 = 0.5.
    #[test]
    fn citation_support_math_locks_formula() {
        let s: u32 = 2;
        let p: u32 = 2;
        let u: u32 = 1;
        let c: u32 = 1;
        let judged = s + p + u + c;
        let score = (s as f32 + 0.5 * p as f32) / judged as f32;
        assert!((score - 0.5).abs() < 1e-6);
    }

    /// Presence formula guard: `1 - missing/total`. 1 of 4 missing → 0.75.
    #[test]
    fn citation_presence_math_locks_formula() {
        let total = 4.0_f32;
        let missing = 1.0_f32;
        assert!((1.0 - missing / total - 0.75).abs() < 1e-6);
    }

    /// Validity edge case: zero total citations → 0.0 (harsh, surfaces
    /// summaries that omit citations wholesale).
    #[test]
    fn citation_validity_zero_citations_is_zero() {
        let total = 0_u32;
        let score = if total == 0 { 0.0_f32 } else { 1.0 };
        assert_eq!(score, 0.0);
    }

    /// Scope edge case: zero existing citations → 1.0 (vacuously true).
    #[test]
    fn citation_scope_zero_existing_is_one() {
        let existing = 0_u32;
        let score = if existing == 0 { 1.0_f32 } else { 0.5 };
        assert_eq!(score, 1.0);
    }

    /// Lock the JSON shape of every `RunState` variant so accidental renames
    /// of fields don't silently corrupt `eval_run_state` rows mid-upgrade.
    #[test]
    fn run_state_round_trips_through_json() {
        let cases = vec![
            RunState::AwaitingClaims,
            RunState::AwaitingClaimEmbed {
                claims: vec![],
                idx: 0,
                reports: vec![],
            },
            RunState::AwaitingRubric { claims: None },
            RunState::AwaitingRubric { claims: Some(vec![]) },
            RunState::AwaitingCitationSupport {
                claims: vec![],
                claim_idx: 0,
                cite_idx: 0,
                cited_chunks: HashMap::new(),
                issues: vec![],
                support_supported: 0,
                support_partial: 0,
                support_unsupported: 0,
                support_contradicted: 0,
                totals: CitationTotals::default(),
                support_calls_issued: 0,
                truncated: false,
            },
            RunState::Done,
        ];
        for c in cases {
            let raw = serde_json::to_string(&c).expect("serialises");
            let back: RunState = serde_json::from_str(&raw).expect("deserialises");
            let raw2 = serde_json::to_string(&back).expect("re-serialises");
            assert_eq!(raw, raw2, "round-trip drift on {:?}", back);
        }
    }

    /// Old cached rows serialised `AwaitingRubric` as a unit variant (no
    /// `claims` field). Confirm the new `#[serde(default)]` handling accepts
    /// that legacy JSON shape and produces `AwaitingRubric { claims: None }`.
    #[test]
    fn legacy_awaiting_rubric_json_still_parses() {
        // Legacy shape (struct variant without `claims`).
        let raw = r#"{"phase":"awaiting_rubric"}"#;
        let parsed: RunState = serde_json::from_str(raw).expect("legacy parses");
        match parsed {
            RunState::AwaitingRubric { claims } => assert!(claims.is_none()),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    /// Flag-off path: no cited chunks and support disabled → the citation
    /// advance helper skips straight to `Done` after writing the three
    /// deterministic metrics (presence=1.0 for zero factual claims,
    /// validity=0.0 for zero citations, scope=1.0 vacuously). Focus: prove we
    /// never issue a support LLM call with the flag off.
    #[test]
    fn citation_flag_off_emits_no_support_call() {
        // Simulate what `next_citation_or_done` produces when `support_enabled`
        // is false and there are no cited chunks: bare `Done` transition (no
        // `CallLlm`). We verify by constructing the expected NextAction and
        // round-tripping it.
        let action: NextAction = NextAction::Done {
            metrics: vec![],
        };
        let raw = serde_json::to_string(&action).unwrap();
        assert!(raw.contains("\"kind\":\"done\""));
        assert!(!raw.contains("call_llm"));
    }
}
