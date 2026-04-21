//! HTTP wire types for stateless eval-related endpoints exposed by brainatlas-be.
//!
//! These are plain serde structs (no protobuf) because callers are HTTP/JSON
//! clients (`evals-be`) rather than gRPC consumers. Keeping them in `rpc-types`
//! lets both server and any Rust client share the contract.

use serde::{Deserialize, Serialize};

// ---- /api/llm/embed ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub text: String,
    /// Optional override of the embedding model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Opaque caller-supplied ID used to attribute the LLM cost back to the
    /// originating eval run/step or region summary. See the cost tracking
    /// design in `plans/2026-04-20-llm-cost-tracking-v1.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    pub embedding: Vec<f32>,
}

// ---- /api/llm/extract-claims ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractClaimsRequest {
    pub summary_text: String,
    pub region_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/judge-groundedness ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeGroundednessRequest {
    pub claim_text: String,
    pub evidence_chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/judge-rubric ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeRubricRequest {
    pub summary_text: String,
    pub region_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

// ---- /api/llm/usage ----
//
// Aggregate view of the `llm_call_usage` table. Query string parameters map
// 1:1 to `domain::UsageAggregateFilter`; unset parameters are not applied.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAggregateQuery {
    /// Inclusive lower-bound on `created_at`, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Inclusive upper-bound on `created_at`, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Prefix match on `correlation_id`, e.g. `eval:{run_id}:` to aggregate
    /// all steps of an eval run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_tag: Option<String>,
}

// ---- /api/llm/judge-citation ----
//
// Stateless "did the author cite the right chunk?" judge. Distinct from
// `judge-groundedness`: the caller passes exactly ONE chunk (the one the
// author cited for this claim) plus the enclosing sentence as context.
//
// Response reuses `GroundednessVerdict` (from brainatlas-be domain) so
// wire shape stays uniform. `supporting_chunks` is always empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeCitationRequest {
    pub claim_text: String,
    pub sentence_context: String,
    pub chunk_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- EmbedRequest ----

    #[test]
    fn embed_request_roundtrip_full() {
        let r = EmbedRequest {
            text: "hippocampus memory".to_string(),
            embedding_model: Some("openai/text-embedding-3-small".to_string()),
            correlation_id: Some("eval:run-1:step-2".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["text"], "hippocampus memory");
        assert_eq!(v["embedding_model"], "openai/text-embedding-3-small");
        assert_eq!(v["correlation_id"], "eval:run-1:step-2");
        let back: EmbedRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.text, r.text);
        assert_eq!(back.embedding_model, r.embedding_model);
        assert_eq!(back.correlation_id, r.correlation_id);
    }

    #[test]
    fn embed_request_skips_none_fields() {
        let r = EmbedRequest {
            text: "t".to_string(),
            embedding_model: None,
            correlation_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("embedding_model").is_none());
        assert!(v.get("correlation_id").is_none());
    }

    #[test]
    fn embed_request_defaults_missing_fields() {
        let r: EmbedRequest = serde_json::from_str(r#"{"text":"t"}"#).unwrap();
        assert_eq!(r.text, "t");
        assert!(r.embedding_model.is_none());
        assert!(r.correlation_id.is_none());
    }

    // ---- EmbedResponse ----

    #[test]
    fn embed_response_roundtrip() {
        let r = EmbedResponse {
            embedding: vec![0.1, 0.2, 0.3, 0.4],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v["embedding"].is_array());
        let back: EmbedResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.embedding, r.embedding);
    }

    #[test]
    fn embed_response_empty_vector_roundtrip() {
        let r = EmbedResponse { embedding: vec![] };
        let v = serde_json::to_value(&r).unwrap();
        let back: EmbedResponse = serde_json::from_value(v).unwrap();
        assert!(back.embedding.is_empty());
    }

    // ---- ExtractClaimsRequest ----

    #[test]
    fn extract_claims_request_roundtrip_full() {
        let r = ExtractClaimsRequest {
            summary_text: "Summary.".to_string(),
            region_name: "Hippocampus".to_string(),
            chat_model: Some("openai/gpt-4o-mini".to_string()),
            correlation_id: Some("batch:abc".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["summary_text"], "Summary.");
        assert_eq!(v["region_name"], "Hippocampus");
        assert_eq!(v["chat_model"], "openai/gpt-4o-mini");
        let back: ExtractClaimsRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_text, r.summary_text);
        assert_eq!(back.region_name, r.region_name);
        assert_eq!(back.chat_model, r.chat_model);
        assert_eq!(back.correlation_id, r.correlation_id);
    }

    #[test]
    fn extract_claims_request_skips_none_fields() {
        let r = ExtractClaimsRequest {
            summary_text: "s".to_string(),
            region_name: "r".to_string(),
            chat_model: None,
            correlation_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("chat_model").is_none());
        assert!(v.get("correlation_id").is_none());
    }

    #[test]
    fn extract_claims_request_minimal_payload_deserializes() {
        let r: ExtractClaimsRequest =
            serde_json::from_str(r#"{"summary_text":"s","region_name":"r"}"#).unwrap();
        assert_eq!(r.summary_text, "s");
        assert_eq!(r.region_name, "r");
        assert!(r.chat_model.is_none());
        assert!(r.correlation_id.is_none());
    }

    // ---- JudgeGroundednessRequest ----

    #[test]
    fn judge_groundedness_request_roundtrip_full() {
        let r = JudgeGroundednessRequest {
            claim_text: "The hippocampus supports memory.".to_string(),
            evidence_chunks: vec!["chunk 1".to_string(), "chunk 2".to_string()],
            chat_model: Some("openai/gpt-4o-mini".to_string()),
            correlation_id: Some("eval:run-1:step-5".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["evidence_chunks"].as_array().unwrap().len(), 2);
        let back: JudgeGroundednessRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.claim_text, r.claim_text);
        assert_eq!(back.evidence_chunks, r.evidence_chunks);
        assert_eq!(back.chat_model, r.chat_model);
        assert_eq!(back.correlation_id, r.correlation_id);
    }

    #[test]
    fn judge_groundedness_request_skips_none_fields() {
        let r = JudgeGroundednessRequest {
            claim_text: "c".to_string(),
            evidence_chunks: vec![],
            chat_model: None,
            correlation_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("chat_model").is_none());
        assert!(v.get("correlation_id").is_none());
        assert_eq!(v["evidence_chunks"].as_array().unwrap().len(), 0);
    }

    // ---- JudgeRubricRequest ----

    #[test]
    fn judge_rubric_request_roundtrip_full() {
        let r = JudgeRubricRequest {
            summary_text: "A summary.".to_string(),
            region_name: "CTX".to_string(),
            chat_model: Some("gpt-4".to_string()),
            correlation_id: Some("batch:xyz".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: JudgeRubricRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.summary_text, r.summary_text);
        assert_eq!(back.region_name, r.region_name);
        assert_eq!(back.chat_model, r.chat_model);
        assert_eq!(back.correlation_id, r.correlation_id);
    }

    #[test]
    fn judge_rubric_request_skips_none_fields() {
        let r = JudgeRubricRequest {
            summary_text: "s".to_string(),
            region_name: "r".to_string(),
            chat_model: None,
            correlation_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("chat_model").is_none());
        assert!(v.get("correlation_id").is_none());
    }

    // ---- UsageAggregateQuery ----

    #[test]
    fn usage_aggregate_query_default_is_all_none() {
        let q = UsageAggregateQuery::default();
        assert!(q.since.is_none());
        assert!(q.until.is_none());
        assert!(q.model.is_none());
        assert!(q.correlation_id.is_none());
        assert!(q.correlation_id_prefix.is_none());
        assert!(q.region_id.is_none());
        assert!(q.summary_id.is_none());
        assert!(q.batch_id.is_none());
        assert!(q.caller_tag.is_none());
    }

    #[test]
    fn usage_aggregate_query_empty_serialization_has_no_fields() {
        let q = UsageAggregateQuery::default();
        let v = serde_json::to_value(&q).unwrap();
        let obj = v.as_object().unwrap();
        // Every field has skip_serializing_if = "Option::is_none" and Default gives all None.
        assert!(obj.is_empty(), "expected empty object, got: {obj:?}");
    }

    #[test]
    fn usage_aggregate_query_empty_object_deserializes_to_default() {
        let q: UsageAggregateQuery = serde_json::from_str("{}").unwrap();
        let d = UsageAggregateQuery::default();
        // All fields are Option<_>, so compare via serialization.
        assert_eq!(
            serde_json::to_value(&q).unwrap(),
            serde_json::to_value(&d).unwrap()
        );
    }

    #[test]
    fn usage_aggregate_query_roundtrip_full() {
        let q = UsageAggregateQuery {
            since: Some("2026-04-01T00:00:00Z".to_string()),
            until: Some("2026-04-20T23:59:59Z".to_string()),
            model: Some("openai/gpt-4o-mini".to_string()),
            correlation_id: Some("eval:run-1:step-2".to_string()),
            correlation_id_prefix: Some("eval:run-1:".to_string()),
            region_id: Some(42),
            summary_id: Some("00000000-0000-0000-0000-000000000000".to_string()),
            batch_id: Some("batch-abc".to_string()),
            caller_tag: Some("orch".to_string()),
        };
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["region_id"], 42);
        assert_eq!(v["correlation_id_prefix"], "eval:run-1:");
        let back: UsageAggregateQuery = serde_json::from_value(v).unwrap();
        assert_eq!(back.since, q.since);
        assert_eq!(back.until, q.until);
        assert_eq!(back.model, q.model);
        assert_eq!(back.correlation_id, q.correlation_id);
        assert_eq!(back.correlation_id_prefix, q.correlation_id_prefix);
        assert_eq!(back.region_id, q.region_id);
        assert_eq!(back.summary_id, q.summary_id);
        assert_eq!(back.batch_id, q.batch_id);
        assert_eq!(back.caller_tag, q.caller_tag);
    }

    #[test]
    fn usage_aggregate_query_partial_payload() {
        let q: UsageAggregateQuery =
            serde_json::from_str(r#"{"model":"openai/gpt-4o-mini","region_id":7}"#).unwrap();
        assert_eq!(q.model.as_deref(), Some("openai/gpt-4o-mini"));
        assert_eq!(q.region_id, Some(7));
        assert!(q.since.is_none());
        assert!(q.correlation_id.is_none());
    }

    // ---- JudgeCitationRequest ----

    #[test]
    fn judge_citation_request_roundtrip_full() {
        let r = JudgeCitationRequest {
            claim_text: "Hippocampus supports memory.".to_string(),
            sentence_context: "The hippocampus supports memory [chunk:abc].".to_string(),
            chunk_text: "Evidence about hippocampus and memory.".to_string(),
            chat_model: Some("openai/gpt-4o-mini".to_string()),
            correlation_id: Some("eval:run-1:step-9".to_string()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["claim_text"], "Hippocampus supports memory.");
        let back: JudgeCitationRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.claim_text, r.claim_text);
        assert_eq!(back.sentence_context, r.sentence_context);
        assert_eq!(back.chunk_text, r.chunk_text);
        assert_eq!(back.chat_model, r.chat_model);
        assert_eq!(back.correlation_id, r.correlation_id);
    }

    #[test]
    fn judge_citation_request_skips_none_fields() {
        let r = JudgeCitationRequest {
            claim_text: "c".to_string(),
            sentence_context: "s".to_string(),
            chunk_text: "t".to_string(),
            chat_model: None,
            correlation_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("chat_model").is_none());
        assert!(v.get("correlation_id").is_none());
    }
}
