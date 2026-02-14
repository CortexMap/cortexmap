// @generated — matches the `region_mapping` and `region_summary` tables in appdb

diesel::table! {
    region_mapping (id) {
        id -> diesel::sql_types::Uuid,
        region_id -> diesel::sql_types::Integer,
        name -> diesel::sql_types::VarChar,
        acronym -> diesel::sql_types::Nullable<diesel::sql_types::VarChar>,
        red -> diesel::sql_types::Nullable<diesel::sql_types::Integer>,
        green -> diesel::sql_types::Nullable<diesel::sql_types::Integer>,
        blue -> diesel::sql_types::Nullable<diesel::sql_types::Integer>,
        structure_order -> diesel::sql_types::Nullable<diesel::sql_types::Integer>,
        parent_region_id -> diesel::sql_types::Nullable<diesel::sql_types::Integer>,
        parent_acronym -> diesel::sql_types::Nullable<diesel::sql_types::VarChar>,
        created_at -> diesel::sql_types::Nullable<diesel::sql_types::Timestamp>,
    }
}

diesel::table! {
    region_summary (id) {
        id -> diesel::sql_types::Uuid,
        region_id -> diesel::sql_types::Integer,
        name -> diesel::sql_types::VarChar,
        acronym -> diesel::sql_types::Nullable<diesel::sql_types::VarChar>,
        summary -> diesel::sql_types::Text,
        created_at -> diesel::sql_types::Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(region_summary, region_mapping);
