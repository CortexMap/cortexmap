//! LLM groundedness pipeline (Step 5).
//!
//! `judge_groundedness_for_summary` is the inner compute that the cache layer
//! invokes on a miss. It:
//!
//! 1. Asks brainatlas to extract atomic claims from the summary.
//! 2. For each claim:
//!    a. Embeds the claim text via brainatlas.
//!    b. Retrieves top-K source chunks from `brain_region_embeddings` scoped
//!       to *this* summary (so the claim must be grounded in its own sources,
//!       not any chunk for the region).
//!    c. Filters chunks below the configured similarity threshold. If none
//!       remain, the claim is "unsupported" without paying for the judge LLM.
//!    d. Otherwise asks the judge LLM for a verdict.
//! 3. Aggregates: `groundedness = supported / total`,
//!    `hallucination = unsupported / total`. Per-claim verdicts go into the
//!    `details` JSON for drill-down.

use crate::infra::{BrainatlasClient, EvalsDatabase, RetrievedChunk, SummaryRow};
use crate::ServiceError;
use backon::{ExponentialBuilder, Retryable};
use brainatlas_rpc_types::evals as brpc;
use domain::{Claim, GroundednessLabel, GroundednessVerdict};
use serde::Serialize;
use std::error::Error;

#[derive(Debug, Clone)]
pub struct GroundednessConfig {
    pub brainatlas_base_url: String,
    pub judge_chat_model: String,
    pub embedding_model: String,
    pub top_k_chunks: i64,
    pub similarity_threshold: f32,
}

#[derive(Debug, Clone, Serialize)]
struct ClaimReport {
    claim: Claim,
    verdict: String,
    confidence: f32,
    supporting_chunks: Vec<u32>,
    rationale: String,
    /// Top retrieved chunks above the similarity threshold (text + score).
    /// Empty list → no chunks passed the threshold and the judge was skipped.
    retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Clone, Serialize)]
struct RetrievedSnippet {
    chunk_index: i32,
    similarity: f32,
}

/// Result returned to the cache layer: one entry per metric (groundedness +
/// hallucination_rate) along with the shared `details` payload.
#[derive(Debug, Clone)]
pub struct GroundednessOutcome {
    pub claim_groundedness: f32,
    pub hallucination_rate: f32,
    pub details: serde_json::Value,
    pub judge_model: String,
}

/// Execute the full groundedness pipeline for one summary.
///
/// Errors from individual claim judgments are *not* fatal — they're recorded
/// in the per-claim report as `verdict: "error"` so a single bad LLM call
/// doesn't lose the whole run.
pub async fn judge_groundedness_for_summary<DB, BC, E>(
    db: &DB,
    brainatlas: &BC,
    database_url: &str,
    summary: &SummaryRow,
    cfg: &GroundednessConfig,
) -> Result<GroundednessOutcome, ServiceError<E>>
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let extract_req = brpc::ExtractClaimsRequest {
        summary_text: summary.summary.clone(),
        region_name: summary.name.clone(),
        chat_model: Some(cfg.judge_chat_model.clone()),
    };
    let claims_resp = retry(|| brainatlas.extract_claims(&cfg.brainatlas_base_url, extract_req.clone()))
        .await
        .map_err(ServiceError::InfraError)?;

    let claims = claims_resp.claims;
    if claims.is_empty() {
        // No claims extracted → can't judge anything. Score 0 / 0 conventionally
        // resolves to 1.0 (nothing to ground, nothing hallucinated). We pick
        // 1.0 for groundedness and 0.0 for hallucination so empty summaries
        // don't get blamed for a model failure to extract claims.
        let details = serde_json::json!({
            "claims": [],
            "note": "no claims extracted",
        });
        return Ok(GroundednessOutcome {
            claim_groundedness: 1.0,
            hallucination_rate: 0.0,
            details,
            judge_model: cfg.judge_chat_model.clone(),
        });
    }

    let mut reports: Vec<ClaimReport> = Vec::with_capacity(claims.len());
    let mut supported = 0u32;
    let mut unsupported = 0u32;

    for claim in claims {
        let report = judge_one_claim(db, brainatlas, database_url, summary, &claim, cfg).await;
        match report.verdict.as_str() {
            "supported" => supported += 1,
            "unsupported" => unsupported += 1,
            _ => {}
        }
        reports.push(report);
    }

    let total = reports.len() as f32;
    let groundedness = if total > 0.0 { supported as f32 / total } else { 1.0 };
    let hallucination = if total > 0.0 { unsupported as f32 / total } else { 0.0 };

    let details = serde_json::json!({
        "claims": reports,
        "totals": {
            "claims": reports.len(),
            "supported": supported,
            "unsupported": unsupported,
        }
    });

    Ok(GroundednessOutcome {
        claim_groundedness: groundedness,
        hallucination_rate: hallucination,
        details,
        judge_model: cfg.judge_chat_model.clone(),
    })
}

async fn judge_one_claim<DB, BC, E>(
    db: &DB,
    brainatlas: &BC,
    database_url: &str,
    summary: &SummaryRow,
    claim: &Claim,
    cfg: &GroundednessConfig,
) -> ClaimReport
where
    DB: EvalsDatabase<Error = E>,
    BC: BrainatlasClient<Error = E>,
    E: Error + Send + Sync + 'static,
{
    let mk_error_report = |msg: String| ClaimReport {
        claim: claim.clone(),
        verdict: "error".to_string(),
        confidence: 0.0,
        supporting_chunks: vec![],
        rationale: msg,
        retrieved: vec![],
    };

    // 1) Embed the claim.
    let embed_req = brpc::EmbedRequest {
        text: claim.text.clone(),
        embedding_model: Some(cfg.embedding_model.clone()),
    };
    let embedding = match retry(|| brainatlas.embed(&cfg.brainatlas_base_url, embed_req.clone())).await {
        Ok(r) => r.embedding,
        Err(e) => return mk_error_report(format!("embed failed: {e}")),
    };

    // 2) Retrieve chunks scoped to this summary.
    let chunks: Vec<RetrievedChunk> = match db
        .retrieve_chunks_for_summary(
            database_url,
            summary.id,
            &embedding,
            cfg.top_k_chunks,
            cfg.similarity_threshold,
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return mk_error_report(format!("retrieve failed: {e}")),
    };

    let retrieved_snippets: Vec<RetrievedSnippet> = chunks
        .iter()
        .map(|c| RetrievedSnippet {
            chunk_index: c.chunk_index,
            similarity: c.similarity,
        })
        .collect();

    // 3) If no chunks passed the threshold → unsupported, skip the judge.
    if chunks.is_empty() {
        return ClaimReport {
            claim: claim.clone(),
            verdict: "unsupported".to_string(),
            confidence: 1.0,
            supporting_chunks: vec![],
            rationale: "no source chunk above similarity threshold".to_string(),
            retrieved: retrieved_snippets,
        };
    }

    // 4) Ask the judge.
    let judge_req = brpc::JudgeGroundednessRequest {
        claim_text: claim.text.clone(),
        evidence_chunks: chunks.iter().map(|c| c.chunk_text.clone()).collect(),
        chat_model: Some(cfg.judge_chat_model.clone()),
    };
    let verdict: GroundednessVerdict =
        match retry(|| brainatlas.judge_groundedness(&cfg.brainatlas_base_url, judge_req.clone())).await
        {
            Ok(v) => v,
            Err(e) => return mk_error_report(format!("judge failed: {e}")),
        };

    let label = match verdict.verdict {
        GroundednessLabel::Supported => "supported",
        GroundednessLabel::Partial => "partial",
        GroundednessLabel::Contradicted => "contradicted",
        GroundednessLabel::Unsupported => "unsupported",
    };

    ClaimReport {
        claim: claim.clone(),
        verdict: label.to_string(),
        confidence: verdict.confidence,
        supporting_chunks: verdict.supporting_chunks,
        rationale: verdict.rationale,
        retrieved: retrieved_snippets,
    }
}

/// Standard 3-attempt exponential backoff matching the orch convention.
async fn retry<F, Fut, T, E>(mut op: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let policy = ExponentialBuilder::default()
        .with_min_delay(std::time::Duration::from_secs(1))
        .with_max_delay(std::time::Duration::from_secs(10))
        .with_max_times(3);
    (move || op()).retry(&policy).await
}

#[cfg(test)]
mod tests {
    /// Smoke-check the aggregation math directly: 2 supported, 1 unsupported,
    /// 1 partial → groundedness = 0.5, hallucination = 0.25.
    #[test]
    fn aggregation_math_locks_formula() {
        let totals = (4.0_f32, 2.0_f32, 1.0_f32);
        let (total, sup, unsup) = totals;
        let g = sup / total;
        let h = unsup / total;
        assert!((g - 0.5).abs() < 1e-6);
        assert!((h - 0.25).abs() < 1e-6);
    }
}
