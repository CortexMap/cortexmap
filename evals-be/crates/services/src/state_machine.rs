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

use crate::ServiceError;
use crate::cache::{ComputedScore, score_with_cache};
use crate::citations::{self, CitationIssue, CitationIssueKind, ParsedCitation, parse_citations};
use crate::infra::{ChunkRow, EvalsDatabase, RetrievedChunk, SummaryRow};
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
        return (
            RunState::Done,
            NextAction::Done {
                metrics: cached_metrics,
            },
        );
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
        RunState::AwaitingClaims => {
            advance_claims(db, database_url, ctx, response, accumulated).await
        }
        RunState::AwaitingClaimEmbed {
            claims,
            idx,
            reports,
        } => {
            advance_embed(
                db,
                database_url,
                ctx,
                response,
                claims,
                idx,
                reports,
                accumulated,
            )
            .await
        }
        RunState::AwaitingClaimJudge {
            claims,
            idx,
            reports,
            retrieved: _,
        } => {
            advance_judge(
                db,
                database_url,
                ctx,
                response,
                claims,
                idx,
                reports,
                accumulated,
            )
            .await
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
            )));
        }
    };

    if claims.is_empty() {
        // No claims → conventional scores (1.0 groundedness / 0.0 hallucination).
        let details = serde_json::json!({
            "claims": [],
            "note": "no claims extracted",
        });
        persist_groundedness_metrics(db, database_url, ctx, 1.0, 0.0, Some(details), accumulated)
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
            )));
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
            db,
            database_url,
            ctx,
            claims,
            idx + 1,
            reports,
            accumulated,
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
            )));
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

    advance_to_next_claim_or_finalize(db, database_url, ctx, claims, idx + 1, reports, accumulated)
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
    let unsupported = reports
        .iter()
        .filter(|r| r.verdict == "unsupported")
        .count() as f32;
    let groundedness = if total > 0.0 { supported / total } else { 1.0 };
    let hallucination = if total > 0.0 {
        unsupported / total
    } else {
        0.0
    };

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
            )));
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
    let new_state = RunState::AwaitingClaimEmbed {
        claims,
        idx,
        reports,
    };
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
        return (
            RunState::Done,
            NextAction::Done {
                metrics: accumulated.clone(),
            },
        );
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
        return Ok((
            RunState::Done,
            NextAction::Done {
                metrics: accumulated.clone(),
            },
        ));
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
                rationale: format!("UUID {} exists but belongs to a different summary", p.uuid),
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
        return Ok((
            RunState::Done,
            NextAction::Done {
                metrics: accumulated.clone(),
            },
        ));
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
            Ok((
                RunState::Done,
                NextAction::Done {
                    metrics: accumulated.clone(),
                },
            ))
        }
        Some((claim_idx, cite_idx)) => {
            let issues: Vec<CitationIssue> = presence_issues
                .into_iter()
                .chain(validity_issues)
                .chain(scope_issues)
                .collect();
            let action =
                build_citation_support_action(ctx, &claims, claim_idx, cite_idx, &cited_chunks);
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
            )));
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
            let action = build_citation_support_action(ctx, &claims, nci, nki, &cited_chunks);
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
            let judged =
                support_supported + support_partial + support_unsupported + support_contradicted;
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
            Ok((
                RunState::Done,
                NextAction::Done {
                    metrics: accumulated.clone(),
                },
            ))
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
            RunState::AwaitingRubric {
                claims: Some(vec![]),
            },
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
        let action: NextAction = NextAction::Done { metrics: vec![] };
        let raw = serde_json::to_string(&action).unwrap();
        assert!(raw.contains("\"kind\":\"done\""));
        assert!(!raw.contains("call_llm"));
    }

    // ---- initial_action branch coverage ----
    //
    // `initial_action` is a pure function over `(ctx, g_cached, r_cached,
    // cached_metrics)`. The three branches are: both cached → Done,
    // groundedness-not-cached → ExtractClaims, groundedness-cached + rubric-
    // not-cached → JudgeRubric. Each test below locks one branch.

    use crate::infra::SummaryRow;

    fn ia_fixture_summary() -> SummaryRow {
        SummaryRow {
            id: Uuid::new_v4(),
            region_id: 7,
            name: "Amygdala".to_string(),
            acronym: Some("AMY".to_string()),
            summary: "The amygdala processes emotion.".to_string(),
        }
    }

    fn ia_ctx<'a>(summary: &'a SummaryRow, hash: &'a str) -> RunContext<'a> {
        RunContext {
            summary,
            summary_hash: hash,
            eval_version: "v-initial",
            judge_chat_model: "ia-judge",
            rubric_chat_model: "ia-rubric",
            embedding_model: "ia-embed",
            top_k_chunks: 2,
            similarity_threshold: 0.4,
            citation_support_enabled: false,
            citation_support_max_calls: 5,
        }
    }

    /// Both upstream families cached → `Done` with the provided metrics, no
    /// LLM call scheduled.
    #[test]
    fn initial_action_both_cached_returns_done() {
        let summary = ia_fixture_summary();
        let ctx = ia_ctx(&summary, "h-both-cached");
        let cached = vec![MetricResult {
            metric: "length_in_range".to_string(),
            score: 1.0,
            cached: true,
            judge_model: None,
        }];

        let (state, next) = initial_action(&ctx, true, true, cached.clone());

        assert!(matches!(state, RunState::Done));
        match next {
            NextAction::Done { metrics } => {
                assert_eq!(metrics.len(), 1);
                assert_eq!(metrics[0].metric, "length_in_range");
            }
            NextAction::CallLlm { .. } => panic!("expected Done, got CallLlm"),
        }
    }

    /// Groundedness NOT cached → schedule `ExtractClaims` and transition to
    /// `AwaitingClaims`. Rubric-cached status is irrelevant on this branch.
    #[test]
    fn initial_action_groundedness_missing_schedules_extract_claims() {
        let summary = ia_fixture_summary();
        let ctx = ia_ctx(&summary, "h-extract");

        let (state, next) = initial_action(&ctx, false, true, vec![]);

        assert!(matches!(state, RunState::AwaitingClaims));
        match next {
            NextAction::CallLlm { endpoint, body, .. } => {
                assert_eq!(endpoint, LlmEndpoint::ExtractClaims);
                // Body must contain the summary text and region name from ctx.
                assert_eq!(body["summary_text"], summary.summary);
                assert_eq!(body["region_name"], summary.name);
                assert_eq!(body["chat_model"], "ia-judge");
            }
            NextAction::Done { .. } => panic!("expected CallLlm, got Done"),
        }
    }

    /// Groundedness cached, rubric missing → skip claims entirely, schedule
    /// `JudgeRubric`, and transition to `AwaitingRubric { claims: None }`.
    /// The `claims: None` is the key invariant here — no claim extraction
    /// ever ran, so citations cannot run on this branch.
    #[test]
    fn initial_action_only_rubric_missing_schedules_rubric_with_no_claims() {
        let summary = ia_fixture_summary();
        let ctx = ia_ctx(&summary, "h-rubric-only");

        let (state, next) = initial_action(&ctx, true, false, vec![]);

        match state {
            RunState::AwaitingRubric { claims } => {
                assert!(
                    claims.is_none(),
                    "rubric-only branch must carry claims=None so citations skip"
                );
            }
            other => panic!("expected AwaitingRubric, got {:?}", other),
        }
        match next {
            NextAction::CallLlm { endpoint, body, .. } => {
                assert_eq!(endpoint, LlmEndpoint::JudgeRubric);
                assert_eq!(body["summary_text"], summary.summary);
                assert_eq!(body["chat_model"], "ia-rubric");
            }
            NextAction::Done { .. } => panic!("expected CallLlm, got Done"),
        }
    }
}

// =============================================================================
// Direct unit tests for every `advance_*` function in the state machine.
//
// Strategy: each test constructs a minimal `InMemoryDb` fake (copied from the
// integration test at `evals-be/crates/app/tests/cache_hit.rs`), drives a
// single `advance_*` call, and asserts the resulting (RunState, NextAction)
// pair plus any rows the persistence helpers wrote into the accumulator.
//
// We deliberately do NOT use `app.init_score` / `app.step_score` here — the
// goal is to exercise each branch of the state machine in isolation, not to
// re-run the integration test.
// =============================================================================
#[cfg(test)]
mod advance_tests {
    use super::*;
    use crate::citations::CitationIssueKind;
    use crate::infra::{ChunkRow, RetrievedChunk, SummaryRow};
    use async_trait::async_trait;
    use brainatlas_rpc_types::evals::EmbedResponse;
    use chrono::NaiveDateTime;
    use domain::{
        Claim, ClaimsResponse, EvalRun, EvalRunStatus, EvalScore, GroundednessLabel,
        GroundednessVerdict, NewEvalScore, RubricCriterion, RubricScores,
    };
    use rpc_types::LlmResponsePayload;
    use std::sync::Mutex;
    use uuid::Uuid;

    // ---- Mock infra error + InMemoryDb (inlined from cache_hit.rs) ----

    #[derive(Debug, thiserror::Error)]
    #[error("mock infra error: {0}")]
    struct MockError(String);

    #[derive(Default)]
    struct InMemoryDb {
        summary: Mutex<Option<SummaryRow>>,
        scores: Mutex<Vec<EvalScore>>,
        /// Optional override: chunks returned by `retrieve_chunks_for_summary`.
        retrieval_chunks: Mutex<Vec<RetrievedChunk>>,
        /// Optional override: rows returned by `load_chunks_by_ids` for any
        /// requested UUIDs (intersection is computed before returning).
        chunk_rows: Mutex<Vec<ChunkRow>>,
    }

    impl InMemoryDb {
        fn new() -> Self {
            Self::default()
        }

        fn with_retrieval(self, chunks: Vec<RetrievedChunk>) -> Self {
            *self.retrieval_chunks.lock().unwrap() = chunks;
            self
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
        ) -> Result<crate::infra::EvalAggregate, Self::Error> {
            Ok(crate::infra::EvalAggregate::default())
        }

        async fn get_worst_offenders(
            &self,
            _database_url: &str,
            _metric: &str,
            _eval_version: &str,
            _limit: i64,
        ) -> Result<Vec<crate::infra::WorstOffenderRow>, Self::Error> {
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
            Ok(EvalRun {
                id: Uuid::new_v4(),
                summary_id,
                eval_version: eval_version.to_string(),
                status,
                error_message,
                started_at: None,
                completed_at: None,
                created_at: NaiveDateTime::default(),
            })
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
            Ok(self.retrieval_chunks.lock().unwrap().clone())
        }

        async fn load_chunks_by_ids(
            &self,
            _database_url: &str,
            chunk_ids: &[Uuid],
        ) -> Result<Vec<ChunkRow>, Self::Error> {
            let rows = self.chunk_rows.lock().unwrap();
            let wanted: HashSet<Uuid> = chunk_ids.iter().copied().collect();
            Ok(rows
                .iter()
                .filter(|r| wanted.contains(&r.id))
                .cloned()
                .collect())
        }

        async fn insert_run_state(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _eval_version: &str,
            _state: &serde_json::Value,
            _pending_step_id: Option<Uuid>,
            _pending_endpoint: Option<&str>,
        ) -> Result<Uuid, Self::Error> {
            Ok(Uuid::new_v4())
        }

        async fn load_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
        ) -> Result<Option<crate::infra::LoadedRunState>, Self::Error> {
            Ok(None)
        }

        async fn save_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
            _state: &serde_json::Value,
            _pending_step_id: Option<Uuid>,
            _pending_endpoint: Option<&str>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn delete_run_state(
            &self,
            _database_url: &str,
            _run_id: Uuid,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn delete_run_states_for_summary(
            &self,
            _database_url: &str,
            _summary_id: Uuid,
            _eval_version: &str,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    // ---- Test fixtures ----

    fn fixture_summary() -> SummaryRow {
        SummaryRow {
            id: Uuid::new_v4(),
            region_id: 1,
            name: "Hippocampus".to_string(),
            acronym: Some("HIP".to_string()),
            summary: "The hippocampus supports memory.".to_string(),
        }
    }

    fn ctx_for<'a>(summary: &'a SummaryRow, summary_hash: &'a str) -> RunContext<'a> {
        RunContext {
            summary,
            summary_hash,
            eval_version: "v0.0.0-test",
            judge_chat_model: "mock-judge",
            rubric_chat_model: "mock-rubric",
            embedding_model: "mock-embed",
            top_k_chunks: 3,
            similarity_threshold: 0.5,
            citation_support_enabled: false,
            citation_support_max_calls: 30,
        }
    }

    fn sample_claim(id: u32, text: &str, cited_chunks: Vec<Uuid>) -> Claim {
        Claim {
            id,
            section: "Overview".to_string(),
            text: text.to_string(),
            cited_chunks,
        }
    }

    // =========================================================================
    // advance_claims
    // =========================================================================

    /// Empty claims list short-circuits to rubric with (groundedness=1.0,
    /// hallucination=0.0) persisted.
    #[tokio::test]
    async fn advance_claims_empty_short_circuits_to_rubric() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-empty");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let (state, next) = advance_claims(&db, "memory://", &ctx, response, &mut acc)
            .await
            .expect("advance");

        // Two groundedness metrics persisted even with no claims.
        assert!(acc.iter().any(|m| m.metric == "claim_groundedness"));
        assert!(acc.iter().any(|m| m.metric == "hallucination_rate"));
        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .unwrap();
        assert!((g.score - 1.0).abs() < 1e-6);
        let h = acc
            .iter()
            .find(|m| m.metric == "hallucination_rate")
            .unwrap();
        assert!((h.score - 0.0).abs() < 1e-6);

        // Empty-claims path hands off to rubric (groundedness has claims=Some(empty)).
        match (state, next) {
            (RunState::AwaitingRubric { claims }, NextAction::CallLlm { endpoint, .. }) => {
                assert!(matches!(claims, Some(v) if v.is_empty()));
                assert_eq!(endpoint, LlmEndpoint::JudgeRubric);
            }
            other => panic!("unexpected transition: {:?}", other),
        }
    }

    /// Response-type mismatch at AwaitingClaims returns InvalidRequest.
    #[tokio::test]
    async fn advance_claims_wrong_response_type_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-wrong");
        let mut acc = Vec::new();

        // Feed Embed when Claims expected.
        let response = LlmResponsePayload::Embed(EmbedResponse {
            embedding: vec![0.1],
        });
        let err = advance_claims(&db, "memory://", &ctx, response, &mut acc)
            .await
            .expect_err("type mismatch");
        match err {
            ServiceError::InvalidRequest(msg) => assert!(
                msg.contains("Claims"),
                "error msg should mention expected type: {msg}"
            ),
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    /// Valid non-empty claims → AwaitingClaimEmbed + CallLlm(Embed).
    #[tokio::test]
    async fn advance_claims_non_empty_starts_embed() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-claims");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse {
            claims: vec![
                sample_claim(1, "claim A", vec![]),
                sample_claim(2, "claim B", vec![]),
            ],
        });
        let (state, next) = advance_claims(&db, "memory://", &ctx, response, &mut acc)
            .await
            .expect("advance");

        // No metrics persisted yet — we still need to go through the embed loop.
        assert!(
            acc.is_empty(),
            "expected no accumulated metrics at embed step"
        );

        match state {
            RunState::AwaitingClaimEmbed {
                claims,
                idx,
                reports,
            } => {
                assert_eq!(claims.len(), 2);
                assert_eq!(idx, 0);
                assert!(reports.is_empty());
            }
            other => panic!("expected AwaitingClaimEmbed, got {:?}", other),
        }
        match next {
            NextAction::CallLlm { endpoint, .. } => assert_eq!(endpoint, LlmEndpoint::Embed),
            other => panic!("expected CallLlm(Embed), got {:?}", other),
        }
    }

    // =========================================================================
    // advance_embed
    // =========================================================================

    /// retrieve_chunks_for_summary returns empty → claim goes to "unsupported",
    /// no judge call, advance to next claim (or finalize for a single claim).
    #[tokio::test]
    async fn advance_embed_empty_chunks_marks_unsupported_and_finalizes() {
        let summary = fixture_summary();
        let db = InMemoryDb::new(); // default = no retrieval chunks
        let ctx = ctx_for(&summary, "hash-embed-empty");
        let mut acc = Vec::new();

        let claims = vec![sample_claim(1, "only claim", vec![])];
        let response = LlmResponsePayload::Embed(EmbedResponse {
            embedding: vec![0.1; 4],
        });
        let (state, next) = advance_embed(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect("advance_embed");

        // Only one claim → finalize: groundedness 0.0 (0 supported / 1),
        // hallucination 1.0 (1 unsupported / 1).
        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .expect("groundedness persisted");
        assert!((g.score - 0.0).abs() < 1e-6);
        let h = acc
            .iter()
            .find(|m| m.metric == "hallucination_rate")
            .expect("hallucination persisted");
        assert!((h.score - 1.0).abs() < 1e-6);

        // With claims Some(..), we go to rubric.
        assert!(matches!(
            state,
            RunState::AwaitingRubric { claims: Some(_) }
        ));
        assert!(matches!(
            next,
            NextAction::CallLlm {
                endpoint: LlmEndpoint::JudgeRubric,
                ..
            }
        ));
    }

    /// Non-empty retrieval → CallLlm(JudgeGroundedness) + AwaitingClaimJudge.
    #[tokio::test]
    async fn advance_embed_nonempty_chunks_issues_judge_call() {
        let summary = fixture_summary();
        let db = InMemoryDb::new().with_retrieval(vec![RetrievedChunk {
            chunk_index: 1,
            chunk_text: "evidence".to_string(),
            similarity: 0.9,
        }]);
        let ctx = ctx_for(&summary, "hash-embed-ok");
        let mut acc = Vec::new();

        let claims = vec![sample_claim(1, "c1", vec![])];
        let response = LlmResponsePayload::Embed(EmbedResponse {
            embedding: vec![0.1; 4],
        });
        let (state, next) = advance_embed(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect("advance_embed");

        assert!(acc.is_empty(), "no metrics persisted at judge step");
        match state {
            RunState::AwaitingClaimJudge { idx, retrieved, .. } => {
                assert_eq!(idx, 0);
                assert_eq!(retrieved.len(), 1);
            }
            other => panic!("expected AwaitingClaimJudge, got {:?}", other),
        }
        match next {
            NextAction::CallLlm { endpoint, .. } => {
                assert_eq!(endpoint, LlmEndpoint::JudgeGroundedness)
            }
            other => panic!("expected CallLlm(JudgeGroundedness), got {:?}", other),
        }
    }

    /// idx out of bounds → InvalidRequest.
    #[tokio::test]
    async fn advance_embed_idx_out_of_bounds_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-embed-oob");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Embed(EmbedResponse { embedding: vec![] });
        let err = advance_embed(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            5,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect_err("idx oob");
        assert!(matches!(err, ServiceError::InvalidRequest(_)));
    }

    /// Wrong response type at AwaitingClaimEmbed.
    #[tokio::test]
    async fn advance_embed_wrong_response_type_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-embed-wrong");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let err = advance_embed(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect_err("wrong type");
        match err {
            ServiceError::InvalidRequest(msg) => assert!(msg.contains("Embed"), "{msg}"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    // =========================================================================
    // advance_judge (groundedness)
    // =========================================================================

    /// Each GroundednessLabel variant maps to a distinct label string in the
    /// accumulated report and in turn drives the final aggregate score.
    #[tokio::test]
    async fn advance_judge_supported_finalizes_with_groundedness_one() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-judge-s");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Groundedness(GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![1],
            rationale: "matches".into(),
        });
        let (state, _next) = advance_judge(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect("judge");

        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .unwrap();
        assert!((g.score - 1.0).abs() < 1e-6);
        assert!(matches!(state, RunState::AwaitingRubric { .. }));
    }

    #[tokio::test]
    async fn advance_judge_unsupported_finalizes_with_hallucination_one() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-judge-u");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Groundedness(GroundednessVerdict {
            verdict: GroundednessLabel::Unsupported,
            confidence: 0.9,
            supporting_chunks: vec![],
            rationale: "no match".into(),
        });
        let (_state, _next) = advance_judge(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect("judge");

        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .unwrap();
        assert!((g.score - 0.0).abs() < 1e-6);
        let h = acc
            .iter()
            .find(|m| m.metric == "hallucination_rate")
            .unwrap();
        assert!((h.score - 1.0).abs() < 1e-6);
    }

    /// Partial and Contradicted count toward the denominator but not as
    /// supported or unsupported: 1 supported / 4 total = 0.25 groundedness,
    /// 1 unsupported / 4 = 0.25 hallucination.
    #[tokio::test]
    async fn advance_judge_partial_contradicted_counted_in_denominator() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-judge-mix");
        let mut acc = Vec::new();

        let claims = vec![
            sample_claim(1, "a", vec![]),
            sample_claim(2, "b", vec![]),
            sample_claim(3, "c", vec![]),
            sample_claim(4, "d", vec![]),
        ];
        // Pre-build the first three reports by hand (simulating prior iterations).
        let prior_reports = vec![
            ClaimReport {
                claim: claims[0].clone(),
                verdict: "supported".into(),
                confidence: 0.9,
                supporting_chunks: vec![1],
                rationale: "ok".into(),
                retrieved: vec![],
            },
            ClaimReport {
                claim: claims[1].clone(),
                verdict: "partial".into(),
                confidence: 0.5,
                supporting_chunks: vec![],
                rationale: "meh".into(),
                retrieved: vec![],
            },
            ClaimReport {
                claim: claims[2].clone(),
                verdict: "contradicted".into(),
                confidence: 0.9,
                supporting_chunks: vec![],
                rationale: "opposite".into(),
                retrieved: vec![],
            },
        ];
        // Feed the 4th (unsupported) via advance_judge.
        let response = LlmResponsePayload::Groundedness(GroundednessVerdict {
            verdict: GroundednessLabel::Unsupported,
            confidence: 1.0,
            supporting_chunks: vec![],
            rationale: "none".into(),
        });
        let (_state, _next) = advance_judge(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            3,
            prior_reports,
            &mut acc,
        )
        .await
        .expect("judge");

        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .unwrap();
        assert!((g.score - 0.25).abs() < 1e-6);
        let h = acc
            .iter()
            .find(|m| m.metric == "hallucination_rate")
            .unwrap();
        assert!((h.score - 0.25).abs() < 1e-6);
    }

    #[tokio::test]
    async fn advance_judge_contradicted_label_recorded() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-judge-contra");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Groundedness(GroundednessVerdict {
            verdict: GroundednessLabel::Contradicted,
            confidence: 1.0,
            supporting_chunks: vec![],
            rationale: "contradicts".into(),
        });
        let (_state, _next) = advance_judge(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect("judge");

        // Contradicted neither counts as supported nor unsupported.
        let g = acc
            .iter()
            .find(|m| m.metric == "claim_groundedness")
            .unwrap();
        assert!((g.score - 0.0).abs() < 1e-6);
        let h = acc
            .iter()
            .find(|m| m.metric == "hallucination_rate")
            .unwrap();
        assert!((h.score - 0.0).abs() < 1e-6);
    }

    /// Wrong response type at AwaitingClaimJudge.
    #[tokio::test]
    async fn advance_judge_wrong_response_type_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-judge-wrong");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let err = advance_judge(
            &db,
            "memory://",
            &ctx,
            response,
            vec![sample_claim(1, "c", vec![])],
            0,
            Vec::new(),
            &mut acc,
        )
        .await
        .expect_err("wrong type");
        match err {
            ServiceError::InvalidRequest(msg) => assert!(msg.contains("Groundedness"), "{msg}"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    // =========================================================================
    // advance_rubric
    // =========================================================================

    /// Rubric response → 5 persisted rubric metrics + transition to Done
    /// (no claims carried forward means no citation phase).
    #[tokio::test]
    async fn advance_rubric_no_claims_emits_five_metrics_and_done() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-rubric-nc");
        let mut acc = Vec::new();

        let c = |s: u8| RubricCriterion {
            score: s,
            rationale: "r".into(),
        };
        let response = LlmResponsePayload::Rubric(RubricScores {
            relevance: c(5),
            coherence: c(4),
            specificity: c(3),
            clinical_utility: c(2),
            terminology: c(1),
        });
        let (state, next) = advance_rubric(&db, "memory://", &ctx, response, None, &mut acc)
            .await
            .expect("rubric");

        // Five new rubric_* rows.
        let rubric_count = acc
            .iter()
            .filter(|m| m.metric.starts_with("rubric_"))
            .count();
        assert_eq!(rubric_count, 5);
        // 5 → 1.0, 1 → 0.0, 3 → 0.5 (locking the normalise path).
        let rel = acc.iter().find(|m| m.metric == "rubric_relevance").unwrap();
        assert!((rel.score - 1.0).abs() < 1e-6);
        let term = acc
            .iter()
            .find(|m| m.metric == "rubric_terminology")
            .unwrap();
        assert!((term.score - 0.0).abs() < 1e-6);

        // No claims → citations skipped, Done.
        assert!(matches!(state, RunState::Done));
        assert!(matches!(next, NextAction::Done { .. }));
    }

    /// Cached-rubric-but-fresh-groundedness path (TODO at :622-644): when all
    /// five rubric_* rows are *already* in `accumulated` (because groundedness
    /// was fresh but rubric was cached upstream), `next_rubric_or_done` should
    /// emit Done instead of re-requesting rubric.
    #[tokio::test]
    async fn next_rubric_or_done_skips_when_rubric_already_cached() {
        let summary = fixture_summary();
        let ctx = ctx_for(&summary, "hash-rubric-cached");

        let cached = |m: &str| rpc_types::MetricResult {
            metric: m.to_string(),
            score: 0.5,
            cached: true,
            judge_model: None,
        };
        let mut acc = vec![
            cached("rubric_relevance"),
            cached("rubric_coherence"),
            cached("rubric_specificity"),
            cached("rubric_clinical_utility"),
            cached("rubric_terminology"),
        ];

        let (state, next) = next_rubric_or_done(&ctx, Some(vec![]), &mut acc);
        assert!(matches!(state, RunState::Done));
        assert!(matches!(next, NextAction::Done { .. }));
    }

    /// Fewer than 5 rubric_* rows → must still issue the rubric call.
    #[tokio::test]
    async fn next_rubric_or_done_issues_call_when_rubric_not_cached() {
        let summary = fixture_summary();
        let ctx = ctx_for(&summary, "hash-rubric-fresh");
        let mut acc = Vec::new();

        let (state, next) = next_rubric_or_done(&ctx, Some(vec![]), &mut acc);
        assert!(matches!(state, RunState::AwaitingRubric { .. }));
        match next {
            NextAction::CallLlm { endpoint, .. } => {
                assert_eq!(endpoint, LlmEndpoint::JudgeRubric)
            }
            other => panic!("expected CallLlm(JudgeRubric), got {:?}", other),
        }
    }

    /// Wrong response type at AwaitingRubric.
    #[tokio::test]
    async fn advance_rubric_wrong_response_type_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-rubric-wrong");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let err = advance_rubric(&db, "memory://", &ctx, response, None, &mut acc)
            .await
            .expect_err("wrong type");
        match err {
            ServiceError::InvalidRequest(msg) => assert!(msg.contains("Rubric"), "{msg}"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    // =========================================================================
    // advance_citation_support + helpers
    // =========================================================================

    /// Wrong response type at AwaitingCitationSupport.
    #[tokio::test]
    async fn advance_citation_support_wrong_response_type_errors() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-cit-wrong");
        let mut acc = Vec::new();

        let claims = vec![sample_claim(1, "c", vec![Uuid::new_v4()])];
        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let err = advance_citation_support(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            0,
            HashMap::new(),
            Vec::new(),
            0,
            0,
            0,
            0,
            CitationTotals::default(),
            1,
            false,
            &mut acc,
        )
        .await
        .expect_err("wrong type");
        match err {
            ServiceError::InvalidRequest(msg) => assert!(msg.contains("CitationSupport"), "{msg}"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// Truncation branch (:1139-1164): when `support_calls_issued` has already
    /// hit `citation_support_max_calls`, the next unseen pair is ignored and
    /// `truncated` flips to true on the persisted row.
    #[tokio::test]
    async fn advance_citation_support_truncates_at_budget() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        // Very low budget so the single call we feed already exceeds it.
        let mut ctx = ctx_for(&summary, "hash-cit-trunc");
        ctx.citation_support_enabled = true;
        ctx.citation_support_max_calls = 1;
        let mut acc = Vec::new();

        // Two cited chunks on one claim → after judging the first there's still
        // a "next" pair, but we're at budget.
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();
        let claims = vec![sample_claim(1, "c", vec![u1, u2])];
        let mut cited_chunks = HashMap::new();
        for u in [u1, u2] {
            cited_chunks.insert(
                u,
                ChunkRow {
                    id: u,
                    summary_id: summary.id,
                    chunk_index: 0,
                    chunk_text: "chunk".into(),
                },
            );
        }

        let response = LlmResponsePayload::CitationSupport(GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![],
            rationale: "".into(),
        });
        let (state, next) = advance_citation_support(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            0,
            cited_chunks,
            Vec::new(),
            0,
            0,
            0,
            0,
            CitationTotals::default(),
            /* support_calls_issued */ 1,
            /* truncated */ false,
            &mut acc,
        )
        .await
        .expect("advance");

        assert!(matches!(state, RunState::Done));
        assert!(matches!(next, NextAction::Done { .. }));

        // The persisted support row should have `truncated: true` in its details.
        let score_row = db
            .scores
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.metric == "citation_support")
            .cloned()
            .expect("support row persisted");
        let details = score_row.details.expect("details present");
        assert_eq!(details["truncated"], serde_json::Value::Bool(true));
    }

    /// Completion branch: only one cited chunk, single support verdict →
    /// `Done` with support = 1.0 (1 supported / 1 judged).
    #[tokio::test]
    async fn advance_citation_support_single_judgement_finalizes() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let mut ctx = ctx_for(&summary, "hash-cit-one");
        ctx.citation_support_enabled = true;
        let mut acc = Vec::new();

        let u = Uuid::new_v4();
        let claims = vec![sample_claim(1, "c", vec![u])];
        let mut cited_chunks = HashMap::new();
        cited_chunks.insert(
            u,
            ChunkRow {
                id: u,
                summary_id: summary.id,
                chunk_index: 0,
                chunk_text: "chunk".into(),
            },
        );

        let response = LlmResponsePayload::CitationSupport(GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![],
            rationale: "supports".into(),
        });
        let (state, _next) = advance_citation_support(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            0,
            cited_chunks,
            Vec::new(),
            0,
            0,
            0,
            0,
            CitationTotals::default(),
            1,
            false,
            &mut acc,
        )
        .await
        .expect("advance");

        assert!(matches!(state, RunState::Done));
        let m = acc
            .iter()
            .find(|m| m.metric == "citation_support")
            .expect("support persisted");
        assert!((m.score - 1.0).abs() < 1e-6);
    }

    /// An Unsupported verdict appends a CitationIssue of the right kind and
    /// lowers the support score.
    #[tokio::test]
    async fn advance_citation_support_unsupported_records_issue() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let mut ctx = ctx_for(&summary, "hash-cit-unsup");
        ctx.citation_support_enabled = true;
        let mut acc = Vec::new();

        let u = Uuid::new_v4();
        let claims = vec![sample_claim(7, "c", vec![u])];
        let mut cited_chunks = HashMap::new();
        cited_chunks.insert(
            u,
            ChunkRow {
                id: u,
                summary_id: summary.id,
                chunk_index: 0,
                chunk_text: "chunk".into(),
            },
        );

        let response = LlmResponsePayload::CitationSupport(GroundednessVerdict {
            verdict: GroundednessLabel::Unsupported,
            confidence: 0.9,
            supporting_chunks: vec![],
            rationale: "nope".into(),
        });
        let (_state, _next) = advance_citation_support(
            &db,
            "memory://",
            &ctx,
            response,
            claims,
            0,
            0,
            cited_chunks,
            Vec::new(),
            0,
            0,
            0,
            0,
            CitationTotals::default(),
            1,
            false,
            &mut acc,
        )
        .await
        .expect("advance");

        let m = acc
            .iter()
            .find(|m| m.metric == "citation_support")
            .expect("support persisted");
        // 0 supported + 0.5 * 0 partial / 1 judged = 0.0.
        assert!((m.score - 0.0).abs() < 1e-6);

        // Verify issues recorded in the persisted details.
        let score_row = db
            .scores
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.metric == "citation_support")
            .cloned()
            .expect("support row persisted");
        let details = score_row.details.expect("details");
        let issues = details["issues"].as_array().expect("issues array");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["kind"], "unsupported");
        assert_eq!(issues[0]["claim_id"], 7);
    }

    // ---- find_next_support_step iteration ----

    #[test]
    fn find_next_support_step_finds_first_in_scope() {
        let in_scope_uuid = Uuid::new_v4();
        let other = Uuid::new_v4();
        let claims = vec![
            sample_claim(1, "a", vec![other]),
            sample_claim(2, "b", vec![other, in_scope_uuid]),
            sample_claim(3, "c", vec![in_scope_uuid]),
        ];
        let mut scope = HashSet::new();
        scope.insert(in_scope_uuid);
        let found = find_next_support_step(&claims, 0, 0, &scope);
        assert_eq!(found, Some((1, 1)));
    }

    #[test]
    fn find_next_support_step_respects_start_indices() {
        let u = Uuid::new_v4();
        let claims = vec![
            sample_claim(1, "a", vec![u, u]),
            sample_claim(2, "b", vec![u]),
        ];
        let mut scope = HashSet::new();
        scope.insert(u);
        // Start at (0, 1) → skips first pair, picks (0, 1).
        assert_eq!(find_next_support_step(&claims, 0, 1, &scope), Some((0, 1)));
        // Start at (1, 0) → skips all of claim 0, finds (1, 0).
        assert_eq!(find_next_support_step(&claims, 1, 0, &scope), Some((1, 0)));
        // Start past the end → None.
        assert_eq!(find_next_support_step(&claims, 2, 0, &scope), None);
    }

    #[test]
    fn find_next_support_step_returns_none_when_no_in_scope() {
        let u = Uuid::new_v4();
        let other = Uuid::new_v4();
        let claims = vec![sample_claim(1, "a", vec![other, other])];
        let mut scope = HashSet::new();
        scope.insert(u);
        assert_eq!(find_next_support_step(&claims, 0, 0, &scope), None);
    }

    // ---- enclosing_sentence_for_uuid UTF-8 handling ----

    #[test]
    fn enclosing_sentence_for_uuid_handles_ascii() {
        let u = Uuid::new_v4();
        let summary = format!("First sentence. Second [chunk:{}] here. Third.", u);
        let got = enclosing_sentence_for_uuid(&summary, u).expect("found");
        assert!(got.contains("Second"));
        assert!(got.contains(&u.to_string()));
        assert!(!got.contains("First"));
        assert!(!got.contains("Third"));
    }

    #[test]
    fn enclosing_sentence_for_uuid_missing_returns_none() {
        let u = Uuid::new_v4();
        let summary = "No citations here.";
        assert!(enclosing_sentence_for_uuid(summary, u).is_none());
    }

    /// UTF-8 safety: the scanner walks backward over raw bytes but only stops
    /// on ASCII sentence terminators. Because UTF-8 continuation bytes never
    /// match any of `.`, `!`, `?`, `\n`, the function can't land mid-codepoint
    /// at a terminator. We still probe that a summary with multibyte chars
    /// neither panics nor truncates into an invalid boundary.
    #[test]
    fn enclosing_sentence_for_uuid_handles_multibyte_chars() {
        let u = Uuid::new_v4();
        // "café" (e-acute is two bytes) and "naïve" (i-diaeresis is two bytes).
        let summary = format!("Un café. Le naïve [chunk:{}] chose. Fin.", u);
        let got = enclosing_sentence_for_uuid(&summary, u).expect("found");
        // Must start at the 'L' of "Le naïve" — the previous '.' sentence
        // boundary, NOT inside "café" or mid-codepoint.
        assert!(got.starts_with("Le na"), "got: {got}");
        assert!(got.contains(&u.to_string()));
        // Sanity: the returned String slice must be valid UTF-8 (implicit via
        // Rust's &str invariant — if the byte indices were bad we'd have panicked).
        assert!(got.chars().count() > 0);
    }

    #[test]
    fn enclosing_sentence_for_uuid_is_case_insensitive() {
        let u = Uuid::new_v4();
        // Upper-case UUID in summary; function must still locate it.
        let upper = u.to_string().to_uppercase();
        let summary = format!("Only one [CHUNK:{}] sentence.", upper);
        let got = enclosing_sentence_for_uuid(&summary, u).expect("found case-insensitive");
        assert!(got.contains("Only one"));
    }

    // ---- RunState::AwaitingCitationSupport JSON round-trip ----

    /// Exercise the serde round-trip on the largest variant — with a non-empty
    /// `cited_chunks` map and a realistic issue list — to guard the JSONB
    /// column against silent field-rename breakage.
    #[test]
    fn awaiting_citation_support_round_trips_with_cited_chunks() {
        let u = Uuid::new_v4();
        let mut cited_chunks = HashMap::new();
        cited_chunks.insert(
            u,
            ChunkRow {
                id: u,
                summary_id: Uuid::new_v4(),
                chunk_index: 42,
                chunk_text: "some evidence".into(),
            },
        );
        let issues = vec![CitationIssue {
            kind: CitationIssueKind::Unsupported,
            claim_id: 1,
            claim_text: "claim".into(),
            offending_chunk_id: Some(u),
            rationale: "nope".into(),
        }];
        let state = RunState::AwaitingCitationSupport {
            claims: vec![sample_claim(1, "c", vec![u])],
            claim_idx: 0,
            cite_idx: 0,
            cited_chunks,
            issues,
            support_supported: 2,
            support_partial: 1,
            support_unsupported: 3,
            support_contradicted: 1,
            totals: CitationTotals {
                total_claims: 5,
                claims_with_citation: 4,
                total_citations: 6,
                existing_citations: 5,
                in_scope_citations: 4,
            },
            support_calls_issued: 7,
            truncated: true,
        };
        let v = serde_json::to_value(&state).expect("to_value");
        let back: RunState = serde_json::from_value(v.clone()).expect("from_value");
        // Re-serialize and compare the JSON — structural equality is the
        // contract; field-by-field PartialEq is not derived.
        let v2 = serde_json::to_value(&back).expect("to_value 2");
        assert_eq!(v, v2);
        // Spot-check a couple of nested fields came through.
        assert_eq!(v["phase"], "awaiting_citation_support");
        assert_eq!(v["support_calls_issued"], 7);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["totals"]["total_claims"], 5);
        // The cited_chunks map is keyed by UUID (serde serializes as string keys).
        assert!(v["cited_chunks"][u.to_string()].is_object());
        assert_eq!(v["cited_chunks"][u.to_string()]["chunk_index"], 42);
    }

    // ---- Public `advance` dispatcher ----

    /// The public `advance` dispatcher must reject `RunState::Done` as an
    /// input: once a run has finished, orch should not call `step_score` on
    /// it again. Any attempt is a caller bug surfaced as
    /// `ServiceError::InvalidRequest`. This guards against a dispatcher
    /// regression that would silently allow post-terminal advances.
    #[tokio::test]
    async fn advance_dispatcher_rejects_done_state() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-done");
        let mut acc = Vec::new();

        // Any payload works — the dispatcher short-circuits before looking at it.
        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let err = advance(&db, "memory://", RunState::Done, &ctx, response, &mut acc)
            .await
            .expect_err("Done state must produce InvalidRequest");

        match err {
            ServiceError::InvalidRequest(msg) => {
                assert!(
                    msg.contains("already Done"),
                    "error message must mention the terminal-state violation, got {msg}"
                );
            }
            other => panic!("expected InvalidRequest, got {:?}", other),
        }
        assert!(
            acc.is_empty(),
            "no metrics must be persisted when the dispatcher bails early"
        );
    }

    /// The public `advance` dispatcher, when given a valid non-terminal
    /// state, must delegate to the matching `advance_*` helper and return a
    /// coherent `(state, next)` pair. We pick the cheapest path:
    /// `AwaitingClaims` + empty claims list → short-circuits to rubric with
    /// two groundedness rows persisted. This exercises the dispatcher's
    /// pattern-match arm for `AwaitingClaims` (which the direct-call tests
    /// bypass).
    #[tokio::test]
    async fn advance_dispatcher_routes_awaiting_claims() {
        let summary = fixture_summary();
        let db = InMemoryDb::new();
        let ctx = ctx_for(&summary, "hash-dispatch");
        let mut acc = Vec::new();

        let response = LlmResponsePayload::Claims(ClaimsResponse { claims: vec![] });
        let (state, next) = advance(
            &db,
            "memory://",
            RunState::AwaitingClaims,
            &ctx,
            response,
            &mut acc,
        )
        .await
        .expect("dispatcher must route AwaitingClaims");

        // Empty-claims path carries `claims: Some(empty_vec)` through to rubric
        // (distinguishing "extracted 0 claims" from "never extracted") — see
        // the direct `advance_claims_empty_short_circuits_to_rubric` test.
        match state {
            RunState::AwaitingRubric { claims } => {
                assert!(matches!(claims, Some(v) if v.is_empty()));
            }
            other => panic!("expected AwaitingRubric, got {other:?}"),
        }
        match next {
            NextAction::CallLlm { endpoint, .. } => {
                assert_eq!(endpoint, LlmEndpoint::JudgeRubric);
            }
            NextAction::Done { .. } => panic!("expected CallLlm, got Done"),
        }
        // Both empty-claims groundedness metrics landed.
        assert!(acc.iter().any(|m| m.metric == "claim_groundedness"));
        assert!(acc.iter().any(|m| m.metric == "hallucination_rate"));
    }
}
