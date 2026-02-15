// @generated automatically by Diesel CLI.

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
    use pgvector::sql_types::Vector;

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
        region_id -> Int4,
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
    }
}

diesel::table! {
    region_queries (id) {
        id -> Uuid,
        region_id -> Int4,
        query_text -> Text,
        source -> Text,
        priority -> Nullable<Int4>,
        enabled -> Nullable<Bool>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
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
    }
}

diesel::joinable!(brain_region_embeddings -> region_summary (summary_id));
diesel::joinable!(fetch_task_components -> fetch_tasks (task_id));
diesel::joinable!(fetch_task_logs -> fetch_tasks (task_id));
diesel::joinable!(langchain_pg_embedding -> langchain_pg_collection (collection_id));

diesel::allow_tables_to_appear_in_same_query!(
    brain_region_embeddings,
    fetch_task_components,
    fetch_task_logs,
    fetch_tasks,
    langchain_pg_collection,
    langchain_pg_embedding,
    orch_config,
    papers,
    processed_fetch_tasks,
    region_mapping,
    region_processing_batches,
    region_queries,
    region_summary,
);
