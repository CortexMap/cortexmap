// @generated-by-hand: schema for the tables this service touches.
//
// `eval_scores` and `eval_runs` are owned by evals-be (defined in
// `migrations/2026-04-19-000001-create_eval_scores`). The other tables are
// owned by brainatlas-be and accessed read-only.

diesel::table! {
    use diesel::sql_types::*;

    eval_scores (id) {
        id -> Uuid,
        summary_id -> Uuid,
        #[max_length = 64]
        summary_hash -> Varchar,
        #[max_length = 64]
        metric -> Varchar,
        score -> Float4,
        #[max_length = 128]
        judge_model -> Nullable<Varchar>,
        details -> Nullable<Jsonb>,
        #[max_length = 16]
        eval_version -> Varchar,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    eval_runs (id) {
        id -> Uuid,
        summary_id -> Uuid,
        #[max_length = 16]
        eval_version -> Varchar,
        #[max_length = 16]
        status -> Varchar,
        error_message -> Nullable<Text>,
        started_at -> Nullable<Timestamp>,
        completed_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
    }
}

// ---- Read-only tables (owned by brainatlas-be) ----

diesel::table! {
    region_summary (id) {
        id -> Uuid,
        region_id -> Int4,
        #[max_length = 255]
        name -> Varchar,
        #[max_length = 50]
        acronym -> Nullable<Varchar>,
        summary -> Nullable<Text>,
        created_at -> Nullable<Timestamp>,
        #[max_length = 64]
        content_hash -> Nullable<Varchar>,
        is_active -> Bool,
        batch_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use pgvector::sql_types::Vector;

    brain_region_embeddings (id) {
        id -> Uuid,
        region_id -> Int4,
        summary_id -> Uuid,
        chunk_index -> Int4,
        chunk_text -> Text,
        embedding -> Vector,
        created_at -> Timestamp,
    }
}
