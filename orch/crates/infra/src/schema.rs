// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "vector"))]
    pub struct Vector;
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Vector;

    brain_region_embeddings (id) {
        id -> Uuid,
        region_id -> Int4,
        summary_id -> Uuid,
        chunk_index -> Int4,
        chunk_text -> Text,
        embedding -> Vector,
        created_at -> Timestamp,
        #[max_length = 20]
        source_pmc_id -> Nullable<Varchar>,
        #[max_length = 20]
        source_uid -> Nullable<Varchar>,
        source_s3_key -> Nullable<Text>,
        source_query -> Nullable<Text>,
        source_char_start -> Nullable<Int4>,
        source_char_end -> Nullable<Int4>,
    }
}

diesel::table! {
    eval_run_state (run_id) {
        run_id -> Uuid,
        summary_id -> Uuid,
        eval_version -> Text,
        state -> Jsonb,
        pending_step_id -> Nullable<Uuid>,
        pending_endpoint -> Nullable<Text>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
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

diesel::table! {
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
    fetch_task_components (id) {
        id -> Int8,
        task_id -> Int8,
        component_type -> Text,
        status -> Text,
        s3_key -> Nullable<Text>,
        attempt_count -> Int4,
        max_attempts -> Int4,
        error_message -> Nullable<Text>,
        last_attempted_at -> Nullable<Timestamp>,
        completed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    fetch_task_logs (id) {
        id -> Int8,
        task_id -> Int8,
        component_type -> Nullable<Text>,
        log_level -> Text,
        message -> Text,
        metadata -> Nullable<Jsonb>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    fetch_tasks (id) {
        id -> Int8,
        pmc_id -> Text,
        query -> Text,
        status -> Text,
        priority -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        started_at -> Nullable<Timestamp>,
        completed_at -> Nullable<Timestamp>,
        last_processed_at -> Nullable<Timestamp>,
        worker_id -> Nullable<Text>,
        heartbeat_at -> Nullable<Timestamp>,
        worker_version -> Nullable<Text>,
        region_id -> Nullable<Int4>,
        stream_message_id -> Nullable<Text>,
    }
}

diesel::table! {
    langchain_pg_collection (uuid) {
        name -> Nullable<Varchar>,
        cmetadata -> Nullable<Json>,
        uuid -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Vector;

    langchain_pg_embedding (uuid) {
        collection_id -> Nullable<Uuid>,
        embedding -> Nullable<Vector>,
        document -> Nullable<Varchar>,
        cmetadata -> Nullable<Json>,
        custom_id -> Nullable<Varchar>,
        uuid -> Uuid,
    }
}

diesel::table! {
    llm_call_usage (id) {
        id -> Uuid,
        created_at -> Timestamptz,
        #[max_length = 32]
        endpoint -> Varchar,
        #[max_length = 256]
        model -> Varchar,
        prompt_tokens -> Int4,
        completion_tokens -> Int4,
        total_tokens -> Int4,
        cost_usd -> Nullable<Numeric>,
        #[max_length = 128]
        correlation_id -> Nullable<Varchar>,
        region_id -> Nullable<Int4>,
        summary_id -> Nullable<Uuid>,
        batch_id -> Nullable<Uuid>,
        #[max_length = 64]
        caller_tag -> Nullable<Varchar>,
        #[max_length = 128]
        request_id -> Nullable<Varchar>,
    }
}

diesel::table! {
    llm_pricing (id) {
        id -> Uuid,
        #[max_length = 256]
        model -> Varchar,
        input_price_per_million -> Numeric,
        output_price_per_million -> Numeric,
        embedding_price_per_million -> Nullable<Numeric>,
        #[max_length = 8]
        currency -> Varchar,
        effective_from -> Timestamptz,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    orch_config (key) {
        key -> Text,
        value -> Text,
        description -> Nullable<Text>,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    papers (id) {
        id -> Int8,
        pmc_id -> Text,
        s3_key -> Text,
        uid -> Text,
        query -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    processed_fetch_tasks (fetch_task_id) {
        fetch_task_id -> Int8,
        region_id -> Uuid,
        pmc_id -> Text,
        processed_at -> Timestamp,
        brainatlas_status -> Text,
        brainatlas_started_at -> Nullable<Timestamp>,
        brainatlas_completed_at -> Nullable<Timestamp>,
        error_message -> Nullable<Text>,
    }
}

diesel::table! {
    region_mapping (id) {
        id -> Uuid,
        region_id -> Int4,
        #[max_length = 255]
        name -> Varchar,
        #[max_length = 50]
        acronym -> Nullable<Varchar>,
        red -> Nullable<Int4>,
        green -> Nullable<Int4>,
        blue -> Nullable<Int4>,
        structure_order -> Nullable<Int4>,
        parent_region_id -> Nullable<Int4>,
        #[max_length = 50]
        parent_acronym -> Nullable<Varchar>,
        created_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    region_processing_batches (id) {
        id -> Uuid,
        status -> Text,
        fetch_task_ids -> Array<Nullable<Int8>>,
        expected_task_count -> Int4,
        #[max_length = 64]
        content_hash -> Nullable<Varchar>,
        created_at -> Timestamp,
        ready_at -> Nullable<Timestamp>,
        processing_started_at -> Nullable<Timestamp>,
        completed_at -> Nullable<Timestamp>,
        summary_id -> Nullable<Uuid>,
        error_message -> Nullable<Text>,
        region_id -> Uuid,
    }
}

diesel::table! {
    region_queries (id) {
        id -> Uuid,
        query_text -> Text,
        source -> Text,
        priority -> Nullable<Int4>,
        enabled -> Nullable<Bool>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        region_id -> Uuid,
    }
}

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

diesel::joinable!(brain_region_embeddings -> region_summary (summary_id));
diesel::joinable!(eval_runs -> region_summary (summary_id));
diesel::joinable!(eval_scores -> region_summary (summary_id));
diesel::joinable!(fetch_task_components -> fetch_tasks (task_id));
diesel::joinable!(fetch_task_logs -> fetch_tasks (task_id));
diesel::joinable!(langchain_pg_embedding -> langchain_pg_collection (collection_id));
diesel::joinable!(region_processing_batches -> region_mapping (region_id));
diesel::joinable!(region_queries -> region_mapping (region_id));

diesel::allow_tables_to_appear_in_same_query!(
    brain_region_embeddings,
    eval_run_state,
    eval_runs,
    eval_scores,
    fetch_task_components,
    fetch_task_logs,
    fetch_tasks,
    langchain_pg_collection,
    langchain_pg_embedding,
    llm_call_usage,
    llm_pricing,
    orch_config,
    papers,
    processed_fetch_tasks,
    region_mapping,
    region_processing_batches,
    region_queries,
    region_summary,
);
