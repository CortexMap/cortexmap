//! HTTP wire types for evals-be's public API.
//!
//! evals-be is a pure state-machine: orch drives an `init`/`step` loop that
//! makes all outbound LLM calls on behalf of evals. Every `NextAction::CallLlm`
//! carries the exact body orch should POST to brainatlas, plus an opaque
//! `step_id` the next `/step` request must echo back for idempotency.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---- POST /api/evals/score/init ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitScoreRequest {
    pub summary_id: Uuid,
    /// Optional override of the eval_version. Defaults to `ConfigKey::EvalVersion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitScoreResponse {
    pub run_id: Uuid,
    pub summary_id: Uuid,
    pub eval_version: String,
    pub next: NextAction,
}

// ---- POST /api/evals/score/step ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRequest {
    pub run_id: Uuid,
    /// Echoes the `step_id` returned on the previous `CallLlm` action.
    pub step_id: Uuid,
    pub llm_response: LlmResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResponse {
    pub run_id: Uuid,
    pub next: NextAction,
}

// ---- State machine action envelope ----

/// What orch should do next. Either make an LLM call on evals's behalf, or
/// stop because the run is complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextAction {
    CallLlm {
        /// Unique per outstanding step. Must be echoed back in the next
        /// `StepRequest`.
        step_id: Uuid,
        endpoint: LlmEndpoint,
        /// The path orch should POST to (relative to the brainatlas base URL).
        path: String,
        /// The exact JSON body orch should POST. Shape depends on `endpoint`.
        body: serde_json::Value,
    },
    Done {
        metrics: Vec<MetricResult>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmEndpoint {
    ExtractClaims,
    Embed,
    JudgeGroundedness,
    JudgeRubric,
    JudgeCitation,
}

impl LlmEndpoint {
    /// Path relative to the brainatlas base URL.
    pub fn path(self) -> &'static str {
        match self {
            LlmEndpoint::ExtractClaims => "/brainatlas-be/api/llm/extract-claims",
            LlmEndpoint::Embed => "/brainatlas-be/api/llm/embed",
            LlmEndpoint::JudgeGroundedness => "/brainatlas-be/api/llm/judge-groundedness",
            LlmEndpoint::JudgeRubric => "/brainatlas-be/api/llm/judge-rubric",
            LlmEndpoint::JudgeCitation => "/brainatlas-be/api/llm/judge-citation",
        }
    }
}

/// One of the five possible LLM response shapes orch can feed back to evals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmResponsePayload {
    Claims(domain::ClaimsResponse),
    Embed(brainatlas_rpc_types::evals::EmbedResponse),
    Groundedness(domain::GroundednessVerdict),
    Rubric(domain::RubricScores),
    /// Reuses `GroundednessVerdict`; the citation judge returns the same
    /// verdict shape but with an empty `supporting_chunks` list.
    CitationSupport(domain::GroundednessVerdict),
}

// ---- MetricResult (unchanged) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricResult {
    pub metric: String,
    pub score: f32,
    /// `true` if the score was returned from the cache (no compute / LLM call).
    pub cached: bool,
    pub judge_model: Option<String>,
}

// ---- GET /api/evals/scores/:summary_id ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoresForSummaryResponse {
    pub summary_id: Uuid,
    pub scores: Vec<ScoreEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
    pub judge_model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub created_at: String,
}

// ---- GET /api/evals/summary?eval_version=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummaryResponse {
    pub eval_version: String,
    pub total_summaries: i64,
    pub total_scored: i64,
    pub per_metric: HashMap<String, MetricStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricStats {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    pub count: i64,
}

// ---- GET /api/evals/worst?metric=...&limit=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorstOffender {
    pub summary_id: Uuid,
    pub region_name: Option<String>,
    pub metric: String,
    pub score: f32,
    pub eval_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorstOffendersResponse {
    pub metric: String,
    pub limit: i64,
    pub entries: Vec<WorstOffender>,
}

// ---- GET /api/evals/unscored?eval_version=...&limit=... ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnscoredResponse {
    pub eval_version: String,
    pub limit: i64,
    pub summary_ids: Vec<Uuid>,
}

// ---- POST /api/evals/batch ----

/// Trigger eval runs for a list of summaries in the background.
///
/// Each `summary_id` gets its own independent `tokio::spawn`-ed eval task,
/// identical to calling `POST /score/init` per ID. Repeated calls with
/// overlapping IDs are intentional and each produce a fresh batch — no
/// deduplication occurs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEvalRequest {
    /// One or more summary IDs to evaluate. Must be non-empty.
    pub summary_ids: Vec<Uuid>,
    /// Optional eval version override. Defaults to the server-configured
    /// `EVAL_VERSION` env var when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_version: Option<String>,
}

/// Immediate response returned before any background eval task has completed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEvalResponse {
    /// Ephemeral correlation UUID generated server-side. Useful for log
    /// correlation — not persisted and not queryable after this response.
    pub batch_eval_id: Uuid,
    /// Echo of every `summary_id` that was accepted into the background queue,
    /// in the order they were received.
    pub accepted: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use brainatlas_rpc_types::evals::EmbedResponse;
    use domain::{
        Claim, ClaimsResponse, GroundednessLabel, GroundednessVerdict, RubricCriterion,
        RubricScores,
    };

    // ---- InitScoreRequest ----

    #[test]
    fn init_score_request_roundtrip_with_version() {
        let r = InitScoreRequest {
            summary_id: Uuid::new_v4(),
            eval_version: Some("v2".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["eval_version"], "v2");
        let back: InitScoreRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_id, r.summary_id);
        assert_eq!(back.eval_version.as_deref(), Some("v2"));
    }

    #[test]
    fn init_score_request_skips_eval_version_when_none() {
        let r = InitScoreRequest {
            summary_id: Uuid::new_v4(),
            eval_version: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("eval_version").is_none());
        // And deserialization tolerates the missing field.
        let back: InitScoreRequest =
            serde_json::from_str(&format!(r#"{{"summary_id":"{}"}}"#, r.summary_id)).unwrap();
        assert_eq!(back.summary_id, r.summary_id);
        assert!(back.eval_version.is_none());
    }

    // ---- InitScoreResponse ----

    #[test]
    fn init_score_response_roundtrip_done() {
        let r = InitScoreResponse {
            run_id: Uuid::new_v4(),
            summary_id: Uuid::new_v4(),
            eval_version: "v1".to_string(),
            next: NextAction::Done { metrics: vec![] },
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["next"]["kind"], "done");
        let back: InitScoreResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.run_id, r.run_id);
        assert!(matches!(back.next, NextAction::Done { .. }));
    }

    // ---- StepRequest / StepResponse ----

    #[test]
    fn step_request_roundtrip() {
        let r = StepRequest {
            run_id: Uuid::new_v4(),
            step_id: Uuid::new_v4(),
            llm_response: LlmResponsePayload::Embed(EmbedResponse {
                embedding: vec![0.1, 0.2, 0.3],
            }),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["llm_response"]["kind"], "embed");
        let back: StepRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.run_id, r.run_id);
        assert_eq!(back.step_id, r.step_id);
        match back.llm_response {
            LlmResponsePayload::Embed(EmbedResponse { embedding }) => {
                assert_eq!(embedding.len(), 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn step_response_roundtrip() {
        let r = StepResponse {
            run_id: Uuid::new_v4(),
            next: NextAction::Done { metrics: vec![] },
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: StepResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.run_id, r.run_id);
        assert!(matches!(back.next, NextAction::Done { .. }));
    }

    // ---- NextAction ----

    #[test]
    fn next_action_call_llm_roundtrip() {
        let step_id = Uuid::new_v4();
        let n = NextAction::CallLlm {
            step_id,
            endpoint: LlmEndpoint::ExtractClaims,
            path: "/brainatlas-be/api/llm/extract-claims".to_string(),
            body: serde_json::json!({"summary_text": "foo", "region_name": "bar"}),
        };
        let v = serde_json::to_value(&n).unwrap();
        // Tagged enum: `kind` field present with snake_case discriminant.
        assert_eq!(v["kind"], "call_llm");
        assert_eq!(v["endpoint"], "extract_claims");
        assert_eq!(v["path"], "/brainatlas-be/api/llm/extract-claims");
        assert_eq!(v["body"]["summary_text"], "foo");

        let back: NextAction = serde_json::from_value(v).unwrap();
        match back {
            NextAction::CallLlm {
                step_id: sid,
                endpoint,
                path,
                body,
            } => {
                assert_eq!(sid, step_id);
                assert_eq!(endpoint, LlmEndpoint::ExtractClaims);
                assert_eq!(path, "/brainatlas-be/api/llm/extract-claims");
                assert_eq!(body["region_name"], "bar");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn next_action_done_roundtrip_empty() {
        let n = NextAction::Done { metrics: vec![] };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["kind"], "done");
        assert!(v["metrics"].as_array().unwrap().is_empty());
        let back: NextAction = serde_json::from_value(v).unwrap();
        assert!(matches!(back, NextAction::Done { ref metrics } if metrics.is_empty()));
    }

    #[test]
    fn next_action_done_roundtrip_with_metrics() {
        let n = NextAction::Done {
            metrics: vec![
                MetricResult {
                    metric: "rubric_relevance".to_string(),
                    score: 1.0,
                    cached: true,
                    judge_model: Some("gpt-4o-mini".to_string()),
                },
                MetricResult {
                    metric: "claim_groundedness".to_string(),
                    score: 0.5,
                    cached: false,
                    judge_model: None,
                },
            ],
        };
        let v = serde_json::to_value(&n).unwrap();
        let back: NextAction = serde_json::from_value(v).unwrap();
        match back {
            NextAction::Done { metrics } => {
                assert_eq!(metrics.len(), 2);
                assert_eq!(metrics[0].metric, "rubric_relevance");
                assert!(metrics[0].cached);
                assert_eq!(metrics[0].judge_model.as_deref(), Some("gpt-4o-mini"));
                assert_eq!(metrics[1].metric, "claim_groundedness");
                assert!(!metrics[1].cached);
                assert!(metrics[1].judge_model.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    // ---- LlmEndpoint ----

    #[test]
    fn llm_endpoint_all_variants_roundtrip_snake_case() {
        let cases = [
            (LlmEndpoint::ExtractClaims, "extract_claims"),
            (LlmEndpoint::Embed, "embed"),
            (LlmEndpoint::JudgeGroundedness, "judge_groundedness"),
            (LlmEndpoint::JudgeRubric, "judge_rubric"),
            (LlmEndpoint::JudgeCitation, "judge_citation"),
        ];
        for (ep, wire) in cases {
            let v = serde_json::to_value(ep).unwrap();
            assert_eq!(v, wire, "wrong wire name for {ep:?}");
            let back: LlmEndpoint = serde_json::from_value(v).unwrap();
            assert_eq!(back, ep);
        }
    }

    #[test]
    fn llm_endpoint_path_helper_is_stable() {
        assert_eq!(
            LlmEndpoint::ExtractClaims.path(),
            "/brainatlas-be/api/llm/extract-claims"
        );
        assert_eq!(LlmEndpoint::Embed.path(), "/brainatlas-be/api/llm/embed");
        assert_eq!(
            LlmEndpoint::JudgeGroundedness.path(),
            "/brainatlas-be/api/llm/judge-groundedness"
        );
        assert_eq!(
            LlmEndpoint::JudgeRubric.path(),
            "/brainatlas-be/api/llm/judge-rubric"
        );
        assert_eq!(
            LlmEndpoint::JudgeCitation.path(),
            "/brainatlas-be/api/llm/judge-citation"
        );
    }

    // ---- LlmResponsePayload: every variant must roundtrip ----

    fn sample_claims() -> ClaimsResponse {
        ClaimsResponse {
            claims: vec![Claim {
                id: 1,
                section: "Overview".to_string(),
                text: "The hippocampus supports memory.".to_string(),
                cited_chunks: vec![],
            }],
        }
    }

    fn sample_verdict() -> GroundednessVerdict {
        GroundednessVerdict {
            verdict: GroundednessLabel::Supported,
            confidence: 0.9,
            supporting_chunks: vec![1, 2],
            rationale: "Good match".to_string(),
        }
    }

    fn sample_rubric() -> RubricScores {
        RubricScores {
            relevance: RubricCriterion {
                score: 5,
                rationale: "".to_string(),
            },
            coherence: RubricCriterion {
                score: 4,
                rationale: "".to_string(),
            },
            specificity: RubricCriterion {
                score: 3,
                rationale: "".to_string(),
            },
            clinical_utility: RubricCriterion {
                score: 5,
                rationale: "".to_string(),
            },
            terminology: RubricCriterion {
                score: 4,
                rationale: "".to_string(),
            },
        }
    }

    #[test]
    fn llm_response_payload_claims_roundtrip() {
        let p = LlmResponsePayload::Claims(sample_claims());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "claims");
        let back: LlmResponsePayload = serde_json::from_value(v).unwrap();
        match back {
            LlmResponsePayload::Claims(c) => {
                assert_eq!(c.claims.len(), 1);
                assert_eq!(c.claims[0].text, "The hippocampus supports memory.");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn llm_response_payload_embed_roundtrip() {
        let p = LlmResponsePayload::Embed(EmbedResponse {
            embedding: vec![0.0, 1.0, 2.0],
        });
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "embed");
        let back: LlmResponsePayload = serde_json::from_value(v).unwrap();
        match back {
            LlmResponsePayload::Embed(e) => {
                assert_eq!(e.embedding, vec![0.0, 1.0, 2.0]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn llm_response_payload_groundedness_roundtrip() {
        let p = LlmResponsePayload::Groundedness(sample_verdict());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "groundedness");
        assert_eq!(v["verdict"], "supported");
        let back: LlmResponsePayload = serde_json::from_value(v).unwrap();
        match back {
            LlmResponsePayload::Groundedness(g) => {
                assert!(matches!(g.verdict, GroundednessLabel::Supported));
                assert_eq!(g.supporting_chunks, vec![1, 2]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn llm_response_payload_rubric_roundtrip() {
        let p = LlmResponsePayload::Rubric(sample_rubric());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "rubric");
        assert_eq!(v["relevance"]["score"], 5);
        let back: LlmResponsePayload = serde_json::from_value(v).unwrap();
        match back {
            LlmResponsePayload::Rubric(r) => {
                assert_eq!(r.relevance.score, 5);
                assert_eq!(r.terminology.score, 4);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn llm_response_payload_citation_support_roundtrip() {
        // Citation reuses GroundednessVerdict — the wire "kind" is citation_support.
        let p = LlmResponsePayload::CitationSupport(sample_verdict());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "citation_support");
        let back: LlmResponsePayload = serde_json::from_value(v).unwrap();
        match back {
            LlmResponsePayload::CitationSupport(g) => {
                assert!(matches!(g.verdict, GroundednessLabel::Supported));
            }
            _ => panic!("wrong variant"),
        }
    }

    // ---- MetricResult ----

    #[test]
    fn metric_result_roundtrip_with_judge() {
        let m = MetricResult {
            metric: "rubric_relevance".to_string(),
            score: 0.75,
            cached: false,
            judge_model: Some("gpt-4o-mini".to_string()),
        };
        let v = serde_json::to_value(&m).unwrap();
        let back: MetricResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.metric, m.metric);
        assert!((back.score - 0.75).abs() < 1e-6);
        assert!(!back.cached);
        assert_eq!(back.judge_model.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn metric_result_roundtrip_null_judge() {
        let m = MetricResult {
            metric: "claim_support".to_string(),
            score: 1.0,
            cached: true,
            judge_model: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        // No skip_serializing_if on judge_model → null appears.
        assert!(v.get("judge_model").is_some());
        let back: MetricResult = serde_json::from_value(v).unwrap();
        assert!(back.judge_model.is_none());
        assert!(back.cached);
    }

    // ---- ScoresForSummaryResponse / ScoreEntry ----

    #[test]
    fn score_entry_roundtrip() {
        let e = ScoreEntry {
            metric: "rubric_relevance".to_string(),
            score: 0.5,
            eval_version: "v1".to_string(),
            judge_model: Some("gpt-4o-mini".to_string()),
            details: Some(serde_json::json!({"nested": 1})),
            created_at: "2026-04-20T12:00:00Z".to_string(),
        };
        let v = serde_json::to_value(&e).unwrap();
        let back: ScoreEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.metric, e.metric);
        assert_eq!(back.judge_model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(back.details.as_ref().unwrap()["nested"], 1);
    }

    #[test]
    fn scores_for_summary_response_roundtrip() {
        let r = ScoresForSummaryResponse {
            summary_id: Uuid::new_v4(),
            scores: vec![ScoreEntry {
                metric: "m".to_string(),
                score: 0.1,
                eval_version: "v1".to_string(),
                judge_model: None,
                details: None,
                created_at: "2026-04-20T12:00:00Z".to_string(),
            }],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: ScoresForSummaryResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_id, r.summary_id);
        assert_eq!(back.scores.len(), 1);
    }

    // ---- EvalSummaryResponse / MetricStats ----

    #[test]
    fn metric_stats_roundtrip() {
        let s = MetricStats {
            avg: 0.8,
            min: 0.1,
            max: 1.0,
            count: 42,
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: MetricStats = serde_json::from_value(v).unwrap();
        assert!((back.avg - 0.8).abs() < 1e-6);
        assert_eq!(back.count, 42);
    }

    #[test]
    fn eval_summary_response_roundtrip() {
        let mut per_metric = HashMap::new();
        per_metric.insert(
            "rubric_relevance".to_string(),
            MetricStats {
                avg: 0.8,
                min: 0.2,
                max: 1.0,
                count: 10,
            },
        );
        let r = EvalSummaryResponse {
            eval_version: "v1".to_string(),
            total_summaries: 100,
            total_scored: 80,
            per_metric,
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: EvalSummaryResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.eval_version, "v1");
        assert_eq!(back.total_summaries, 100);
        assert_eq!(back.total_scored, 80);
        assert_eq!(back.per_metric.len(), 1);
        assert!((back.per_metric["rubric_relevance"].avg - 0.8).abs() < 1e-6);
    }

    // ---- WorstOffender / WorstOffendersResponse ----

    #[test]
    fn worst_offender_roundtrip() {
        let w = WorstOffender {
            summary_id: Uuid::new_v4(),
            region_name: Some("Hippocampus".to_string()),
            metric: "rubric_relevance".to_string(),
            score: 0.1,
            eval_version: "v1".to_string(),
        };
        let v = serde_json::to_value(&w).unwrap();
        let back: WorstOffender = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_id, w.summary_id);
        assert_eq!(back.region_name.as_deref(), Some("Hippocampus"));
    }

    #[test]
    fn worst_offenders_response_roundtrip() {
        let r = WorstOffendersResponse {
            metric: "rubric_relevance".to_string(),
            limit: 10,
            entries: vec![],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: WorstOffendersResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.metric, "rubric_relevance");
        assert_eq!(back.limit, 10);
        assert!(back.entries.is_empty());
    }

    // ---- UnscoredResponse ----

    #[test]
    fn unscored_response_roundtrip() {
        let ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let r = UnscoredResponse {
            eval_version: "v1".to_string(),
            limit: 25,
            summary_ids: ids.clone(),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: UnscoredResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.eval_version, "v1");
        assert_eq!(back.limit, 25);
        assert_eq!(back.summary_ids, ids);
    }
}
