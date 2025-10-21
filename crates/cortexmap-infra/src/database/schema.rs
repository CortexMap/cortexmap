// @generated automatically by Diesel CLI.

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
