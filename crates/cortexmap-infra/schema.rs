// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "vector"))]
    pub struct Vector;
}

diesel::table! {
    papers (id) {
        id -> Int8,
        pmc_id -> Text,
        s3_url -> Text,
        uid -> Text,
        query -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Vector;

    vector_store (id) {
        id -> Text,
        embedding -> Nullable<Vector>,
        metadata -> Nullable<Jsonb>,
        created_at -> Nullable<Timestamp>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(papers, vector_store,);
