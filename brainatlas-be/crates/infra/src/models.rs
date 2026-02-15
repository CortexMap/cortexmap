use crate::schema::{brain_region_embeddings, region_mapping, region_summary};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use diesel::prelude::*;
use domain::{BrainRegionEntry, RegionMapping, Rgb};
use uuid::Uuid;

/// Diesel row model for `region_mapping`. Private to `infra`.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = region_mapping)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegionMappingRow {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub red: Option<i32>,
    pub green: Option<i32>,
    pub blue: Option<i32>,
    pub structure_order: Option<i32>,
    pub parent_region_id: Option<i32>,
    pub parent_acronym: Option<String>,
    pub created_at: Option<chrono::NaiveDateTime>,
}

impl From<RegionMappingRow> for RegionMapping {
    fn from(row: RegionMappingRow) -> Self {
        let color = match (row.red, row.green, row.blue) {
            (Some(r), Some(g), Some(b)) => {
                Some(Rgb::new(r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8))
            }
            _ => None,
        };
        let created_at: DateTime<Utc> = row
            .created_at
            .map(|ndt| Utc.from_utc_datetime(&ndt))
            .unwrap_or_else(Utc::now);

        RegionMapping {
            id: row.id,
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym,
            color,
            structure_order: row.structure_order,
            parent_region_id: row.parent_region_id,
            parent_acronym: row.parent_acronym,
            created_at,
        }
    }
}

/// Diesel row model for `region_summary`. Private to `infra`.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = region_summary)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegionSummaryRow {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub content_hash: Option<String>,
}

impl From<RegionSummaryRow> for BrainRegionEntry {
    fn from(row: RegionSummaryRow) -> Self {
        let created_at = row
            .created_at
            .map(|ndt| Utc.from_utc_datetime(&ndt))
            .unwrap_or_else(Utc::now);
            
        BrainRegionEntry {
            region_id: row.region_id,
            name: row.name,
            acronym: row.acronym.unwrap_or_default(),
            summary: row.summary.unwrap_or_default(),
            created_at,
        }
    }
}

/// Diesel insert model for `region_summary`
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = region_summary)]
pub struct NewRegionSummaryRow {
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub summary: String,
    pub content_hash: Option<String>,
}

/// Diesel insert model for `brain_region_embeddings`
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = brain_region_embeddings)]
pub struct NewEmbeddingRow {
    pub region_id: i32,
    pub summary_id: Uuid,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: pgvector::Vector,
}
