// @generated automatically by Diesel CLI.

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

diesel::joinable!(fetch_task_components -> fetch_tasks (task_id));
diesel::joinable!(fetch_task_logs -> fetch_tasks (task_id));

diesel::allow_tables_to_appear_in_same_query!(
    fetch_task_components,
    fetch_task_logs,
    fetch_tasks,
    papers,
);
